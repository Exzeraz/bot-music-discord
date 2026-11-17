use base64::Engine;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct SpotifyInfo {
    pub kind: String,
    pub title: String,
    pub tracks: Vec<String>,
}

pub struct SpotifyHelper {
    client_id: Option<String>,
    client_secret: Option<String>,
    token: Mutex<Option<(String, Instant)>>,
    http: reqwest::Client,
}

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"spotify\.com/(?:intl-[a-zA-Z0-9-]+/)?(track|playlist|album|episode)/([a-zA-Z0-9]+)").unwrap()
    })
}

fn uri_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^spotify:(track|playlist|album|episode):([a-zA-Z0-9]+)").unwrap())
}

fn next_data_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?s)<script\s+id="__NEXT_DATA__"[^>]*>(.*?)</script>"#).unwrap())
}

fn make_query(title: &str, artists: &str) -> String {
    format!("{title} - {artists}")
        .trim_matches(|c| c == ' ' || c == '-')
        .to_string()
}

fn artists_of(v: &Value) -> String {
    v["artists"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

impl SpotifyHelper {
    pub fn new(client_id: Option<String>, client_secret: Option<String>) -> Self {
        Self {
            client_id: client_id.filter(|s| !s.is_empty()),
            client_secret: client_secret.filter(|s| !s.is_empty()),
            token: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    pub fn is_spotify_url(text: &str) -> bool {
        url_re().is_match(text) || uri_re().is_match(text) || text.contains("spotify.link/")
    }

    async fn api_token(&self) -> Option<String> {
        let (id, secret) = (self.client_id.as_ref()?, self.client_secret.as_ref()?);
        let mut guard = self.token.lock().await;
        if let Some((t, exp)) = guard.as_ref() {
            if Instant::now() < *exp {
                return Some(t.clone());
            }
        }
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
        let resp = self
            .http
            .post("https://accounts.spotify.com/api/token")
            .header("Authorization", format!("Basic {auth}"))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .ok()?;
        let v: Value = resp.json().await.ok()?;
        let t = v["access_token"].as_str()?.to_string();
        let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60);
        *guard = Some((t.clone(), Instant::now() + Duration::from_secs(ttl)));
        Some(t)
    }

    async fn parse_url(&self, input: &str) -> Option<(String, String)> {
        let mut text = input.to_string();
        if text.contains("spotify.link/") {
            if let Ok(r) = self
                .http
                .get(&text)
                .header("User-Agent", "Mozilla/5.0")
                .send()
                .await
            {
                text = r.url().to_string();
            }
        }
        if let Some(c) = uri_re().captures(&text) {
            return Some((c[1].to_string(), c[2].to_string()));
        }
        url_re()
            .captures(&text)
            .map(|c| (c[1].to_string(), c[2].to_string()))
    }

    async fn api_get(&self, token: &str, url: &str) -> Option<Value> {
        let r = self.http.get(url).bearer_auth(token).send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        r.json().await.ok()
    }

    async fn via_api(&self, token: &str, kind: &str, id: &str) -> Option<SpotifyInfo> {
        match kind {
            "track" => {
                let d = self
                    .api_get(token, &format!("https://api.spotify.com/v1/tracks/{id}"))
                    .await?;
                let title = d["name"].as_str().unwrap_or("").to_string();
                let q = make_query(&title, &artists_of(&d));
                Some(SpotifyInfo { kind: "track".into(), title, tracks: vec![q] })
            }
            "album" => {
                let d = self
                    .api_get(token, &format!("https://api.spotify.com/v1/albums/{id}"))
                    .await?;
                let tracks = d["tracks"]["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|t| make_query(t["name"].as_str().unwrap_or(""), &artists_of(t)))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(SpotifyInfo {
                    kind: "album".into(),
                    title: d["name"].as_str().unwrap_or("").into(),
                    tracks,
                })
            }
            "playlist" => {
                let d = self
                    .api_get(
                        token,
                        &format!("https://api.spotify.com/v1/playlists/{id}?fields=name,tracks.items(track(name,artists))"),
                    )
                    .await?;
                let tracks = d["tracks"]["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|i| !i["track"].is_null())
                            .map(|i| {
                                make_query(i["track"]["name"].as_str().unwrap_or(""), &artists_of(&i["track"]))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(SpotifyInfo {
                    kind: "playlist".into(),
                    title: d["name"].as_str().unwrap_or("").into(),
                    tracks,
                })
            }
            _ => None,
        }
    }

    pub async fn get_info(&self, input: &str) -> Option<SpotifyInfo> {
        let (kind, id) = self.parse_url(input).await?;

        // 1. Official API (ถ้ามี credentials)
        if let Some(token) = self.api_token().await {
            if let Some(info) = self.via_api(&token, &kind, &id).await {
                return Some(info);
            }
        }

        // 2. Fallback: Embed scraper (ไม่ต้องใช้ API key)
        let html = self
            .http
            .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
            .header("User-Agent", UA)
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        let json = next_data_re().captures(&html)?.get(1)?.as_str().to_string();
        let data: Value = serde_json::from_str(&json).ok()?;
        let entity = &data["props"]["pageProps"]["state"]["data"]["entity"];
        let name = entity["name"]
            .as_str()
            .or_else(|| entity["title"].as_str())
            .unwrap_or("Spotify Item")
            .to_string();

        let mut tracks = Vec::new();
        if kind == "track" || kind == "episode" {
            let title = entity["title"]
                .as_str()
                .or_else(|| entity["name"].as_str())
                .unwrap_or("");
            let a = artists_of(entity);
            let artist = if a.is_empty() {
                entity["subtitle"].as_str().unwrap_or("").to_string()
            } else {
                a
            };
            tracks.push(make_query(title, &artist));
        } else if let Some(list) = entity["trackList"].as_array() {
            for t in list {
                let Some(title) = t["title"].as_str().or_else(|| t["name"].as_str()) else {
                    continue;
                };
                let a = artists_of(t);
                let sub = if a.is_empty() {
                    t["subtitle"].as_str().unwrap_or("").to_string()
                } else {
                    a
                };
                tracks.push(make_query(title, &sub));
            }
        }
        Some(SpotifyInfo { kind, title: name, tracks })
    }
}
