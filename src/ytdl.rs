use serde_json::Value;
use std::process::Stdio;
use tokio::process::Command;

pub struct Resolved {
    pub url: String,
    pub title: String,
}

/// เรียก yt-dlp เพื่อดึง Direct Audio URL (query อาจเป็น URL หรือ `ytsearch1:ชื่อเพลง`)
pub async fn resolve(query: &str) -> Result<Resolved, String> {
    let bin = std::env::var("YT_DLP_PATH").unwrap_or_else(|_| "yt-dlp".into());
    let mut cmd = Command::new(&bin);
    cmd.args([
        "-j",
        "--no-warnings",
        "--no-playlist",
        "--playlist-items",
        "1",
        "-f",
        "bestaudio/best",
        "--js-runtimes",
        "node",
        "--",
        query,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let out = cmd
        .output()
        .await
        .map_err(|e| format!("เรียก yt-dlp ไม่สำเร็จ: {e}"))?;
    if !out.status.success() {
        return Err(fail_msg(&bin, &out));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or("ไม่พบผลการค้นหา")?;
    let v: Value = serde_json::from_str(line).map_err(|e| format!("อ่านผลลัพธ์ yt-dlp ไม่ได้: {e}"))?;
    let url = v["url"].as_str().ok_or("ไม่พบ stream URL")?.to_string();
    let title = v["title"].as_str().unwrap_or("Unknown").to_string();
    let ua = v["http_headers"]["User-Agent"].as_str().map(str::to_string);
    wait_until_ready(&url, ua.as_deref()).await;
    Ok(Resolved { url, title })
}

/// URL แบบ YouTube playlist ล้วน (`/playlist?list=...` หรือมี list= แต่ไม่มี v=)
pub fn is_youtube_playlist(text: &str) -> bool {
    let t = text.trim();
    let yt = t.contains("youtube.com/") || t.contains("music.youtube.com/");
    yt && t.contains("list=") && (t.contains("/playlist") || !(t.contains("?v=") || t.contains("&v=")))
}

pub struct PlaylistEntry {
    pub title: String,
    pub url: String,
}

/// ดึงรายการเพลงใน playlist แบบเร็ว (ไม่ resolve stream) สูงสุด `limit` รายการ
pub async fn resolve_playlist(url: &str, limit: usize) -> Result<Vec<PlaylistEntry>, String> {
    let bin = std::env::var("YT_DLP_PATH").unwrap_or_else(|_| "yt-dlp".into());
    let mut cmd = Command::new(&bin);
    cmd.args([
        "--flat-playlist",
        "-j",
        "--no-warnings",
        "--playlist-end",
        &limit.to_string(),
        "--",
        url,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);

    let out = cmd
        .output()
        .await
        .map_err(|e| format!("เรียก yt-dlp ไม่สำเร็จ: {e}"))?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(fail_msg(&bin, &out));
    }
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let title = v["title"].as_str().unwrap_or("Unknown").to_string();
        if title == "[Private video]" || title == "[Deleted video]" {
            continue;
        }
        let url = match v["url"].as_str() {
            Some(u) if u.starts_with("http") => u.to_string(),
            _ => match v["id"].as_str() {
                Some(id) => format!("https://www.youtube.com/watch?v={id}"),
                None => continue,
            },
        };
        entries.push(PlaylistEntry { title, url });
    }
    Ok(entries)
}

fn fail_msg(bin: &str, out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("(ไม่มี error output)");
    format!("yt-dlp ล้มเหลว (exit={:?}, bin={bin}): {last}", out.status.code())
}

/// URL ที่ yt-dlp เพิ่งออกให้ มักตอบ 403 อยู่ ~2 วินาทีแรก (ffmpeg จะเปิดไม่ได้และเพลงเงียบ)
/// จึงรอจนกว่า URL จะตอบ 2xx ก่อนส่งต่อให้ ffmpeg (รอสูงสุด ~10 วินาที)
async fn wait_until_ready(url: &str, ua: Option<&str>) {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    });
    for _ in 0..20 {
        let mut req = client.get(url).header("Range", "bytes=0-0");
        if let Some(ua) = ua {
            req = req.header("User-Agent", ua);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => return,
            Ok(r) if r.status().as_u16() == 403 => {}
            _ => return, // error อื่น ปล่อยให้ ffmpeg ลองเอง
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
