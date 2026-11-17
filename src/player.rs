use crate::{log, ytdl};
use async_trait::async_trait;
use serenity::http::Http;
use serenity::model::id::{ChannelId, GuildId};
use songbird::input::core::io::ReadOnlySource;
use songbird::input::{ChildContainer, Input, RawAdapter};
use songbird::{Event, EventContext, EventHandler as VoiceEventHandler, Songbird, TrackEvent};
use songbird::tracks::TrackHandle;
use std::collections::{HashMap, VecDeque};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::Mutex;

pub const MAX_QUEUE: usize = 100;

#[derive(Clone, Debug)]
pub struct Song {
    pub title: String,
    /// Direct audio URL (None = lazy, ต้องไปดึงจาก `query` ตอนจะเล่น)
    pub url: Option<String>,
    pub query: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopMode {
    Off,
    Song,
    Queue,
}

pub struct GuildPlayer {
    pub queue: VecDeque<Song>,
    pub volume: f32,
    pub speed: f64,
    pub current: Option<Song>,
    pub track: Option<TrackHandle>,
    pub started: Instant,
    pub elapsed: f64,
    pub paused: bool,
    pub play_id: u64,
    pub loop_mode: LoopMode,
    pub skipping: bool,
    pub text_channel: ChannelId,
    /// query ของเพลงที่กำลังเตรียมล่วงหน้า (กันทำซ้ำ)
    pub prefetching: Option<String>,
}

impl GuildPlayer {
    fn new(text_channel: ChannelId) -> Self {
        Self {
            queue: VecDeque::new(),
            volume: 0.5,
            speed: 1.0,
            current: None,
            track: None,
            started: Instant::now(),
            elapsed: 0.0,
            paused: false,
            play_id: 0,
            loop_mode: LoopMode::Off,
            skipping: false,
            text_channel,
            prefetching: None,
        }
    }

    /// รีเซ็ตสถานะทั้งหมด (volume / speed คงไว้)
    pub fn reset(&mut self) {
        self.play_id += 1;
        self.queue.clear();
        self.current = None;
        self.track = None;
        self.paused = false;
        self.elapsed = 0.0;
        self.loop_mode = LoopMode::Off;
        self.skipping = false;
        self.prefetching = None;
    }
}

/// ตัวกลางที่แชร์ให้ทุก command / event
pub struct Core {
    pub sb: Arc<Songbird>,
    pub http: Arc<Http>,
    players: StdMutex<HashMap<GuildId, Arc<Mutex<GuildPlayer>>>>,
}

/// FFmpeg atempo รองรับ 0.5-2.0 ต่อ 1 filter จึงต้องต่อหลายตัวเพื่อความเร็วสูง/ต่ำมาก
pub fn atempo_filter(speed: f64) -> String {
    let mut filters = Vec::new();
    let mut cur = speed;
    while cur > 2.0 {
        filters.push("atempo=2.0".to_string());
        cur /= 2.0;
    }
    while cur < 0.5 {
        filters.push("atempo=0.5".to_string());
        cur /= 0.5;
    }
    filters.push(format!("atempo={cur:.4}"));
    filters.join(",")
}

fn spawn_ffmpeg(url: &str, speed: f64, seek: f64) -> std::io::Result<Input> {
    let bin = std::env::var("FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".into());
    let mut cmd = Command::new(bin);
    cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
    if seek > 0.0 {
        cmd.args(["-ss", &format!("{seek:.2}")]);
    }
    // -icy 0: ไม่ส่ง header Icy-MetaData เพราะ YouTube (googlevideo) จะตอบ 403 ถ้ามี header นี้
    cmd.args(["-icy", "0", "-i", url, "-vn"]);
    if (speed - 1.0).abs() > f64::EPSILON {
        cmd.args(["-filter:a", &atempo_filter(speed)]);
    }
    cmd.args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-loglevel", "error", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn()?;
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    log::error(format!("[ffmpeg] {line}"));
                }
            }
        });
    }
    let source = ReadOnlySource::new(ChildContainer::from(child));
    Ok(RawAdapter::new(source, 48000, 2).into())
}

struct TrackEnd {
    core: Arc<Core>,
    guild: GuildId,
    play_id: u64,
}

#[async_trait]
impl VoiceEventHandler for TrackEnd {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(list) = ctx {
            for (state, _) in list.iter() {
                log::info(format!("[track] playing={:?} position={:?}", state.playing, state.position));
            }
        }
        let core = self.core.clone();
        let (g, id) = (self.guild, self.play_id);
        tokio::spawn(async move { core.play_next(g, id).await });
        None
    }
}

impl Core {
    pub fn new(sb: Arc<Songbird>, http: Arc<Http>) -> Arc<Self> {
        Arc::new(Self { sb, http, players: StdMutex::new(HashMap::new()) })
    }

    pub fn player(&self, guild: GuildId, text_channel: ChannelId) -> Arc<Mutex<GuildPlayer>> {
        let mut map = self.players.lock().unwrap();
        let p = map
            .entry(guild)
            .or_insert_with(|| Arc::new(Mutex::new(GuildPlayer::new(text_channel))));
        p.clone()
    }

    pub fn existing_player(&self, guild: GuildId) -> Option<Arc<Mutex<GuildPlayer>>> {
        self.players.lock().unwrap().get(&guild).cloned()
    }

    pub async fn say(&self, channel: ChannelId, text: impl Into<String>) {
        if let Err(e) = channel.say(&self.http, text.into()).await {
            log::error(format!("ส่งข้อความไม่สำเร็จ: {e}"));
        }
    }

    pub fn is_connected(&self, guild: GuildId) -> bool {
        self.sb.get(guild).is_some()
    }

    /// เริ่มเล่นเพลง (ผู้เรียกต้องถือ lock ของ GuildPlayer อยู่) — คืน true ถ้าสำเร็จ
    pub async fn start_song(self: &Arc<Self>, guild: GuildId, p: &mut GuildPlayer, song: Song, seek: f64) -> bool {
        let Some(call) = self.sb.get(guild) else {
            log::error("ไม่สามารถเล่นเพลงได้: ไม่ได้เชื่อมต่อกับห้องเสียง (Not connected to voice)");
            return false;
        };
        let Some(url) = song.url.clone() else {
            log::error("start_song: เพลงยังไม่มี URL");
            return false;
        };

        let input = match spawn_ffmpeg(&url, p.speed, seek) {
            Ok(i) => i,
            Err(e) => {
                log::error(format!("เปิด ffmpeg ไม่สำเร็จ: {e}"));
                return false;
            }
        };

        p.current = Some(song);
        p.started = Instant::now();
        if seek == 0.0 {
            p.elapsed = 0.0;
        }
        p.paused = false;
        p.play_id += 1;
        let play_id = p.play_id;

        let handle = {
            let mut c = call.lock().await;
            c.play_only_input(input)
        };
        let _ = handle.set_volume(p.volume);
        for ev in [TrackEvent::End, TrackEvent::Error] {
            let _ = handle.add_event(
                Event::Track(ev),
                TrackEnd { core: self.clone(), guild, play_id },
            );
        }
        p.track = Some(handle);
        self.prefetch_next(guild);
        true
    }

    /// เตรียมเพลงถัดไปในคิว 1 เพลงล่วงหน้า (แกะ stream URL ระหว่างที่เพลงปัจจุบันเล่นอยู่)
    /// เรียกซ้ำได้ปลอดภัย: ถ้าเพลงหน้าคิวพร้อมแล้วหรือกำลังเตรียมอยู่ จะไม่ทำอะไร
    pub fn prefetch_next(self: &Arc<Self>, guild: GuildId) {
        let core = self.clone();
        tokio::spawn(async move {
            let Some(pl) = core.existing_player(guild) else { return };
            let query = {
                let mut p = pl.lock().await;
                let Some(front) = p.queue.front().cloned() else { return };
                if front.url.is_some() || p.prefetching.as_deref() == Some(front.query.as_str()) {
                    return;
                }
                p.prefetching = Some(front.query.clone());
                front.query
            };

            let res = ytdl::resolve(&query).await;

            let mut p = pl.lock().await;
            p.prefetching = None;
            match res {
                Ok(r) => {
                    // หาตามชื่อ query เผื่อคิวถูกสลับ/ข้ามระหว่างรอ
                    if let Some(s) = p.queue.iter_mut().find(|s| s.query == query && s.url.is_none()) {
                        log::info(format!("เตรียมเพลงถัดไปล่วงหน้าแล้ว: {}", r.title));
                        s.url = Some(r.url);
                        s.title = r.title;
                    }
                }
                Err(e) => log::error(format!("เตรียมเพลงล่วงหน้าไม่สำเร็จ ({query}): {e}")),
            }
            // ถ้าหน้าคิวเปลี่ยนไประหว่างรอ (skip/shuffle) ให้เตรียมตัวใหม่ต่อ
            let again = p.queue.front().map(|s| s.url.is_none() && s.query != query).unwrap_or(false);
            drop(p);
            if again {
                core.prefetch_next(guild);
            }
        });
    }

    /// เรียกเมื่อเพลงจบ/พัง (หรือ skip) เพื่อเล่นเพลงถัดไป
    pub async fn play_next(self: Arc<Self>, guild: GuildId, mut id: u64) {
        let Some(pl) = self.existing_player(guild) else { return };
        let mut first = true;

        loop {
            let (mut song, channel) = {
                let mut p = pl.lock().await;
                if p.play_id != id || !self.is_connected(guild) {
                    return;
                }
                p.play_id += 1; // claim: กัน event ซ้ำ (End + Error)
                let channel = p.text_channel;

                let was_skipped = std::mem::take(&mut p.skipping);

                if first {
                    // 1. Song loop
                    if p.loop_mode == LoopMode::Song && !was_skipped {
                        if let Some(song) = p.current.clone() {
                            let title = song.title.clone();
                            if self.start_song(guild, &mut p, song, 0.0).await {
                                let speed = p.speed;
                                drop(p);
                                self.say(channel, format!("🔂 กำลังวนซ้ำเพลงเดิม: **{title}** (ความเร็ว x{speed})")).await;
                                log::info(format!("วนซ้ำเพลงเดิม: {title}"));
                            }
                            return;
                        }
                    }
                    // 2. Queue loop
                    if p.loop_mode == LoopMode::Queue {
                        if let Some(cur) = p.current.clone() {
                            if !was_skipped || !p.queue.is_empty() {
                                p.queue.push_back(cur);
                            }
                        }
                    }
                }
                first = false;

                match p.queue.pop_front() {
                    Some(s) => (s, channel),
                    None => {
                        p.current = None;
                        p.track = None;
                        p.elapsed = 0.0;
                        p.paused = false;
                        log::info("คิวเพลงว่างเปล่า (Queue empty).");
                        return;
                    }
                }
            };

            // Lazy queue: ดึง stream URL นอก lock
            if song.url.is_none() {
                match ytdl::resolve(&song.query).await {
                    Ok(r) => {
                        song.url = Some(r.url);
                        song.title = r.title;
                    }
                    Err(e) => {
                        log::error(format!("ไม่สามารถดึงข้อมูลสตรีมสำหรับ {}: {e}", song.title));
                        self.say(channel, format!("⚠️ ไม่สามารถดึงสตรีมเพลง: **{}** ได้ (กำลังข้ามไปยังเพลงถัดไป)", song.title)).await;
                        id = pl.lock().await.play_id;
                        continue;
                    }
                }
            }

            let mut p = pl.lock().await;
            if p.play_id != id + 1 || !self.is_connected(guild) {
                log::info("สถานะเปลี่ยนระหว่างโหลดเพลง ยกเลิกการเล่นเพลงถัดไป");
                return;
            }
            let title = song.title.clone();
            if self.start_song(guild, &mut p, song, 0.0).await {
                let speed = p.speed;
                drop(p);
                self.say(channel, format!("กำลังเล่นเพลงถัดไป: **{title}** (ความเร็ว x{speed})")).await;
                log::info(format!("ถัดไปกำลังเล่นเพลง: {title}"));
                return;
            }
            // เปิดเพลงไม่สำเร็จ → ข้าม
            id = p.play_id;
        }
    }
}

#[cfg(test)]
mod tests {
    use songbird::input::codecs::{get_codec_registry, get_probe};
    use songbird::input::core::io::ReadOnlySource;
    use songbird::input::{Input, RawAdapter};

    /// กันพลาด: ถ้า symphonia ไม่ได้เปิด feature "pcm" เสียงจะเงียบ (no compatible track found)
    #[tokio::test]
    async fn raw_f32_stream_is_playable() {
        let src = ReadOnlySource::new(std::io::Cursor::new(vec![0u8; 48000 * 8]));
        let input: Input = RawAdapter::new(src, 48000, 2).into();
        input
            .make_playable_async(get_codec_registry(), get_probe())
            .await
            .expect("raw f32 input must be decodable");
    }
}
