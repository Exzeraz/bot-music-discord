mod log;
mod player;
mod spotify;
mod ytdl;

use player::{Core, LoopMode, Song, MAX_QUEUE};
use rand::seq::SliceRandom;
use serenity::async_trait;
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
};
use serenity::model::application::{Command, CommandDataOptionValue, CommandOptionType, Interaction};
use serenity::model::channel::Message;
use serenity::model::gateway::{GatewayIntents, Ready};
use serenity::model::id::{ChannelId, GuildId, UserId};
use serenity::model::voice::VoiceState;
use serenity::prelude::*;
use songbird::{Songbird, SerenityInit};
use spotify::SpotifyHelper;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// ข้อมูลผู้สั่งคำสั่ง (ใช้ร่วมกันทั้ง `!คำสั่ง` และ `/คำสั่ง`)
struct Req {
    channel_id: ChannelId,
    author_id: UserId,
    author_name: String,
}

fn slash_commands() -> Vec<CreateCommand> {
    let text = |name: &str, desc: &str, opt: &str, opt_desc: &str, required: bool| {
        CreateCommand::new(name.to_string()).description(desc.to_string()).add_option(
            CreateCommandOption::new(CommandOptionType::String, opt.to_string(), opt_desc.to_string()).required(required),
        )
    };
    let simple = |name: &str, desc: &str| CreateCommand::new(name.to_string()).description(desc.to_string());
    vec![
        text("play", "เล่นเพลงจาก YouTube / Spotify", "query", "ชื่อเพลง / YouTube URL / Spotify URL", true),
        text("p", "เล่นเพลง (ย่อของ /play)", "query", "ชื่อเพลง / YouTube URL / Spotify URL", true),
        simple("skip", "ข้ามเพลงที่กำลังเล่น"),
        simple("s", "ข้ามเพลง (ย่อของ /skip)"),
        simple("pause", "หยุดเพลงชั่วคราว"),
        simple("resume", "เล่นเพลงต่อ"),
        simple("stop", "หยุดเล่นและล้างคิว"),
        text("loop", "ตั้งค่าการวนซ้ำ", "mode", "off / song / queue (เว้นว่างเพื่อสลับ)", false),
        simple("queue", "ดูรายการเพลงในคิว"),
        simple("q", "ดูคิว (ย่อของ /queue)"),
        simple("np", "ดูเพลงที่กำลังเล่น"),
        simple("shuffle", "สุ่มลำดับเพลงในคิว"),
        CreateCommand::new("vol").description("ปรับระดับเสียง 0-100").add_option(
            CreateCommandOption::new(CommandOptionType::Integer, "level", "0-100").required(true).min_int_value(0).max_int_value(100),
        ),
        CreateCommand::new("speed").description("ปรับความเร็วเพลง 0.1-1000").add_option(
            CreateCommandOption::new(CommandOptionType::Number, "value", "0.1-1000").required(true).min_number_value(0.1).max_number_value(1000.0),
        ),
        simple("help", "แสดงรายการคำสั่ง"),
    ]
}

struct Handler {
    core: OnceLock<Arc<Core>>,
    spotify: SpotifyHelper,
}

impl Handler {
    fn core(&self, ctx: &Context, sb: &Arc<Songbird>) -> Arc<Core> {
        self.core
            .get_or_init(|| Core::new(sb.clone(), ctx.http.clone()))
            .clone()
    }
}

/// หาห้องเสียงที่ผู้ใช้อยู่ (ไม่ถือ cache lock ข้าม await)
fn user_voice_channel(ctx: &Context, guild: GuildId, msg: &Req) -> Option<ChannelId> {
    let g = ctx.cache.guild(guild)?;
    g.voice_states.get(&msg.author_id).and_then(|v| v.channel_id)
}

fn bot_voice_channel(core: &Core, guild: GuildId) -> Option<u64> {
    let call = core.sb.get(guild)?;
    let c = call.try_lock().ok()?;
    c.current_channel().map(|c| c.0.get())
}

fn fmt_loop(m: LoopMode) -> &'static str {
    match m {
        LoopMode::Off => "ปิด ➡️",
        LoopMode::Song => "เพลงปัจจุบัน 🔂",
        LoopMode::Queue => "ทั้งคิว 🔁",
    }
}

async fn send_help(ctx: &Context, msg: &Req) {
    let embed = CreateEmbed::new()
        .title("🎵 คำสั่งทั้งหมดของ MusicBot")
        .description("นี่คือรายการคำสั่งที่คุณสามารถใช้งานได้:")
        .colour(serenity::model::Colour::BLUE)
        .field("!play <ชื่อเพลง / YouTube URL / Spotify URL>", "เล่นเพลงจาก YouTube หรือ Spotify (รองรับ Track, Album, Playlist)", false)
        .field("!skip", "ข้ามเพลงที่กำลังเล่นอยู่", false)
        .field("!pause", "หยุดเพลงชั่วคราว", true)
        .field("!resume", "เล่นเพลงต่อ", true)
        .field("!stop", "หยุดเล่นและล้างคิวทั้งหมด", false)
        .field("!loop [off/song/queue] (หรือ !repeat)", "ตั้งค่าการวนซ้ำ (เพลงเดียว / ทั้งคิว / ปิด หรือพิมพ์ `!loop` เพื่อสลับ)", false)
        .field("!queue (หรือ !list)", "ดูรายการเพลงในคิว", false)
        .field("!np (หรือ !nowplaying)", "ดูเพลงที่กำลังเล่นอยู่ตอนนี้", false)
        .field("!shuffle (หรือ !random)", "สุ่มสลับลำดับเพลงในคิว", false)
        .field("!vol <0-100>", "ปรับระดับเสียง", false)
        .field("!speed <0.1-1000.0>", "ปรับความเร็วการเล่นเพลง (รองรับได้สูงสุด 1000 เท่า 🚀)", false)
        .footer(serenity::builder::CreateEmbedFooter::new("บอทจะออกจากห้องอัตโนมัติหากไม่มีคนอยู่"));
    let _ = msg.channel_id.send_message(&ctx.http, CreateMessage::new().embed(embed)).await;
}

impl Handler {
    async fn cmd_play(&self, ctx: &Context, core: &Arc<Core>, msg: &Req, guild: GuildId, search: &str) {
        if search.is_empty() {
            return core.say(msg.channel_id, "ใช้: `!play <ชื่อเพลง / URL>`").await;
        }
        let Some(vc) = user_voice_channel(ctx, guild, msg) else {
            log::info(format!("ผู้ใช้ {} ไม่อยู่ในห้องเสียง", msg.author_name));
            return core.say(msg.channel_id, "ต้องอยู่ในช่องเสียงก่อน!").await;
        };

        // เชื่อมต่อ / ย้ายห้อง
        if bot_voice_channel(core, guild) != Some(vc.get()) {
            log::info(format!("กำลังเชื่อมต่อห้องเสียง: {vc}"));
            if let Err(e) = core.sb.join(guild, vc).await {
                log::error(format!("เชื่อมต่อห้องเสียงไม่สำเร็จ: {e}"));
                return core.say(msg.channel_id, format!("เกิดข้อผิดพลาด: {e}")).await;
            }
            log::success("เชื่อมต่อห้องเสียงสำเร็จแล้ว!");
        }

        let player = core.player(guild, msg.channel_id);
        player.lock().await.text_channel = msg.channel_id;
        let _typing = msg.channel_id.start_typing(&ctx.http);

        // --- Spotify ---
        if SpotifyHelper::is_spotify_url(search) {
            log::info(format!("ตรวจพบลิงก์ Spotify: {search}"));
            let info = match self.spotify.get_info(search).await {
                Some(i) if !i.tracks.is_empty() => i,
                _ => {
                    log::error(format!("ไม่สามารถดึงข้อมูลเพลงจาก Spotify: {search}"));
                    return core.say(msg.channel_id, "❌ ไม่สามารถดึงข้อมูลเพลงจาก Spotify ได้ กรุณาตรวจสอบลิงก์อีกครั้ง").await;
                }
            };
            let kind_cap = {
                let mut c = info.kind.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            };

            if info.tracks.len() == 1 {
                let q = format!("ytsearch1:{}", info.tracks[0]);
                let r = match ytdl::resolve(&q).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error(format!("yt-dlp: {e}"));
                        return core.say(msg.channel_id, format!("❌ ไม่พบเพลง **{}** บน YouTube", info.tracks[0])).await;
                    }
                };
                let song = Song { title: r.title.clone(), url: Some(r.url), query: q };
                let mut p = player.lock().await;
                if p.current.is_some() {
                    if p.queue.len() >= MAX_QUEUE {
                        return core.say(msg.channel_id, "คิวเต็มแล้ว! (สูงสุด 100 เพลง)").await;
                    }
                    p.queue.push_back(song);
                    let n = p.queue.len();
                    drop(p);
                    core.prefetch_next(guild);
                    core.say(msg.channel_id, format!("🟢 เพิ่มเพลงจาก Spotify: **{}** ลงในคิว | (เพลงในคิว: {n})", r.title)).await;
                    log::success(format!("เพิ่มเพลง Spotify ลงในคิว: {}", r.title));
                } else if core.start_song(guild, &mut p, song, 0.0).await {
                    let speed = p.speed;
                    drop(p);
                    core.say(msg.channel_id, format!("🟢 กำลังเล่นเพลงจาก Spotify: **{}** (ความเร็ว x{speed})", r.title)).await;
                    log::success(format!("เริ่มเล่นเพลง Spotify: {}", r.title));
                }
                return;
            }

            // Playlist / Album
            let mut remaining: &[String] = &info.tracks;
            let active = player.lock().await.current.is_some();
            if !active {
                let q = format!("ytsearch1:{}", info.tracks[0]);
                if let Ok(r) = ytdl::resolve(&q).await {
                    let title = r.title.clone();
                    let song = Song { title: r.title, url: Some(r.url), query: q };
                    let mut p = player.lock().await;
                    if p.current.is_none() && core.start_song(guild, &mut p, song, 0.0).await {
                        drop(p);
                        core.say(msg.channel_id, format!("🟢 เริ่มเล่นจาก Spotify {kind_cap} **{}**: **{title}**", info.title)).await;
                        log::success(format!("เริ่มเล่นเพลงแรกจาก Spotify {}: {title}", info.kind));
                        remaining = &info.tracks[1..];
                    }
                }
            }
            let mut p = player.lock().await;
            let mut added = 0;
            for t in remaining {
                if p.queue.len() >= MAX_QUEUE {
                    break;
                }
                p.queue.push_back(Song { title: t.clone(), url: None, query: format!("ytsearch1:{t}") });
                added += 1;
            }
            let total = p.queue.len();
            drop(p);
            core.prefetch_next(guild);
            core.say(msg.channel_id, format!("🟢 เพิ่ม **{added} เพลง** จาก Spotify {kind_cap} **{}** ลงในคิวแล้ว! (รวมทั้งหมดในคิว: {total} เพลง)", info.title)).await;
            log::success(format!("เพิ่ม {added} เพลงจาก Spotify {} '{}' ลงในคิว", info.kind, info.title));
            return;
        }

        // --- YouTube Playlist ---
        if ytdl::is_youtube_playlist(search) {
            log::info(format!("ตรวจพบ YouTube Playlist: {search}"));
            let entries = match ytdl::resolve_playlist(search, MAX_QUEUE).await {
                Ok(e) if !e.is_empty() => e,
                Ok(_) => return core.say(msg.channel_id, "❌ Playlist นี้ว่างเปล่า หรือเข้าถึงไม่ได้ (อาจเป็น Private)").await,
                Err(e) => {
                    log::error(format!("ดึง YouTube Playlist ไม่สำเร็จ: {e}"));
                    return core.say(msg.channel_id, "❌ ไม่สามารถดึง Playlist จาก YouTube ได้ กรุณาตรวจสอบลิงก์อีกครั้ง").await;
                }
            };
            let total_found = entries.len();
            let mut rest: &[ytdl::PlaylistEntry] = &entries;
            let active = player.lock().await.current.is_some();
            if !active {
                if let Ok(r) = ytdl::resolve(&entries[0].url).await {
                    let title = r.title.clone();
                    let song = Song { title: r.title, url: Some(r.url), query: entries[0].url.clone() };
                    let mut p = player.lock().await;
                    if p.current.is_none() && core.start_song(guild, &mut p, song, 0.0).await {
                        drop(p);
                        core.say(msg.channel_id, format!("🟢 เริ่มเล่นจาก YouTube Playlist: **{title}**")).await;
                        log::success(format!("เริ่มเล่นเพลงแรกจาก YouTube Playlist: {title}"));
                        rest = &entries[1..];
                    }
                }
            }
            let mut p = player.lock().await;
            let mut added = 0;
            for e in rest {
                if p.queue.len() >= MAX_QUEUE {
                    break;
                }
                p.queue.push_back(Song { title: e.title.clone(), url: None, query: e.url.clone() });
                added += 1;
            }
            let total = p.queue.len();
            drop(p);
            core.prefetch_next(guild);
            core.say(msg.channel_id, format!("🟢 เพิ่ม **{added} เพลง** จาก YouTube Playlist ลงในคิวแล้ว! (รวมทั้งหมดในคิว: {total} เพลง{})", if total_found >= MAX_QUEUE { ", จำกัดสูงสุด 100 เพลง" } else { "" })).await;
            log::success(format!("เพิ่ม {added} เพลงจาก YouTube Playlist ลงในคิว"));
            return;
        }

        // --- YouTube / ค้นหาทั่วไป ---
        let query = if search.starts_with("http://") || search.starts_with("https://") {
            search.to_string()
        } else {
            format!("ytsearch1:{search}")
        };
        let r = match ytdl::resolve(&query).await {
            Ok(r) => r,
            Err(e) => {
                log::error(format!("ไม่พบผลการค้นหาสำหรับ {search}: {e}"));
                return core.say(msg.channel_id, "ไม่พบผลการค้นหา!").await;
            }
        };
        let song = Song { title: r.title.clone(), url: Some(r.url), query };
        let mut p = player.lock().await;
        if p.current.is_some() {
            if p.queue.len() >= MAX_QUEUE {
                return core.say(msg.channel_id, "คิวเต็มแล้ว! (สูงสุด 100 เพลง)").await;
            }
            p.queue.push_back(song);
            let n = p.queue.len();
            drop(p);
            core.prefetch_next(guild);
            core.say(msg.channel_id, format!("เพิ่มลงในคิว: **{}** | ขณะนี้มี {n} เพลงในคิว", r.title)).await;
            log::success(format!("เพิ่มลงในคิวสำเร็จ: {}", r.title));
        } else if core.start_song(guild, &mut p, song, 0.0).await {
            let speed = p.speed;
            drop(p);
            core.say(msg.channel_id, format!("กำลังเล่น: **{}** (ความเร็ว x{speed})", r.title)).await;
            log::success(format!("เริ่มเล่นเพลง: {}", r.title));
        }
    }

    async fn cmd_speed(&self, core: &Arc<Core>, msg: &Req, guild: GuildId, arg: &str) {
        let Ok(s) = arg.parse::<f64>() else {
            return core.say(msg.channel_id, "กรุณาระบุความเร็วระหว่าง 0.1 ถึง 1000.0 (เช่น `!speed 1.5`, `!speed 10`, หรือ `!speed 1000`)").await;
        };
        if !(0.1..=1000.0).contains(&s) {
            return core.say(msg.channel_id, "กรุณาระบุความเร็วระหว่าง 0.1 ถึง 1000.0 (เช่น `!speed 1.5`, `!speed 10`, หรือ `!speed 1000`)").await;
        }
        let player = core.player(guild, msg.channel_id);
        let mut p = player.lock().await;
        let old = p.speed;
        p.speed = s;

        if core.is_connected(guild) && p.current.is_some() {
            if !p.paused {
                p.elapsed += p.started.elapsed().as_secs_f64() * old;
            }
            let seek = p.elapsed;
            let song = p.current.clone().unwrap();
            p.play_id += 1; // ทำให้ callback ของ player เดิมใช้ไม่ได้
            if core.start_song(guild, &mut p, song, seek).await {
                drop(p);
                core.say(msg.channel_id, format!("ปรับความเร็วแบบ Real-time เป็น: **x{s}** ⚡ (ตำแหน่ง {:02}:{:02})", seek as u64 / 60, seek as u64 % 60)).await;
                log::success(format!("เปลี่ยนความเร็วการเล่นแบบ Real-time เป็น x{s} (seek={seek:.2}s)"));
            }
        } else {
            drop(p);
            core.say(msg.channel_id, format!("ปรับความเร็วเป็น: **x{s}** ⚡ (จะมีผลในเพลงถัดไป)")).await;
        }
    }

    async fn cmd_loop(&self, core: &Arc<Core>, msg: &Req, guild: GuildId, arg: &str) {
        let player = core.player(guild, msg.channel_id);
        let mut p = player.lock().await;
        if arg.is_empty() {
            p.loop_mode = match p.loop_mode {
                LoopMode::Off => LoopMode::Song,
                LoopMode::Song => LoopMode::Queue,
                LoopMode::Queue => LoopMode::Off,
            };
        } else {
            match arg.to_lowercase().as_str() {
                "song" | "single" | "one" | "track" | "1" | "เพลง" => p.loop_mode = LoopMode::Song,
                "queue" | "all" | "q" | "คิว" | "ทั้งหมด" => p.loop_mode = LoopMode::Queue,
                "off" | "disable" | "stop" | "none" | "0" | "ปิด" => p.loop_mode = LoopMode::Off,
                "status" | "check" => {}
                _ => {
                    drop(p);
                    return core.say(msg.channel_id, concat!(
                        "❌ รูปแบบโหมดไม่ถูกต้อง!\n💡 คำสั่งที่รองรับ:\n",
                        "• `!loop` หรือ `!repeat` : สลับโหมดวนซ้ำ (ปิด -> เพลงปัจจุบัน -> ทั้งคิว -> ปิด)\n",
                        "• `!loop song` : วนซ้ำเฉพาะเพลงที่กำลังเล่น 🔂\n",
                        "• `!loop queue` : วนซ้ำทุกเพลงในคิว 🔁\n",
                        "• `!loop off` : ปิดการวนซ้ำ ➡️"
                    )).await;
                }
            }
        }
        let (title, desc) = match p.loop_mode {
            LoopMode::Off => ("ปิด (Off) ➡️", "บอทจะเล่นเพลงตามลำดับคิวและหยุดเมื่อเพลงหมด"),
            LoopMode::Song => ("เพลงปัจจุบัน (Song) 🔂", "บอทจะเล่นเพลงปัจจุบันซ้ำไปเรื่อยๆ (ข้ามได้ด้วย `!skip`)"),
            LoopMode::Queue => ("ทั้งคิว (Queue) 🔁", "บอทจะนำเพลงที่เล่นจบไปต่อท้ายคิวเพื่อวนเล่นใหม่"),
        };
        let mode = p.loop_mode;
        drop(p);
        core.say(msg.channel_id, format!("🔁 โหมดการวนซ้ำ: **{title}**\n📝 *{desc}*")).await;
        log::success(format!("ตั้งค่า loop_mode เป็น: {mode:?}"));
    }

    async fn cmd_queue(&self, core: &Arc<Core>, msg: &Req, guild: GuildId) {
        let player = core.player(guild, msg.channel_id);
        let p = player.lock().await;
        let ls = fmt_loop(p.loop_mode);
        if p.queue.is_empty() {
            return core.say(msg.channel_id, format!("ขณะนี้ไม่มีเพลงในคิว (โหมดวนซ้ำ: **{ls}**)")).await;
        }
        let mut text = format!("**รายการเพลงในคิว:** (โหมดวนซ้ำ: **{ls}**)\n");
        for (i, s) in p.queue.iter().take(10).enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, s.title));
        }
        if p.queue.len() > 10 {
            text.push_str(&format!("และอีก {} เพลง...", p.queue.len() - 10));
        }
        drop(p);
        core.say(msg.channel_id, text).await;
    }

    async fn handle(&self, ctx: &Context, msg: &Req, guild: GuildId, cmd: &str, arg: &str) {
        let sb = songbird::get(ctx).await.expect("Songbird not registered");
        let core = self.core(ctx, &sb);
        let user = msg.author_name.as_str();

        match cmd {
            "help" | "h" => {
                log::command(user, "help", "แสดงรายการคำสั่ง");
                send_help(ctx, msg).await;
            }
            "play" | "p" => {
                log::command(user, "play", &format!("ค้นหา/เล่น: {arg}"));
                self.cmd_play(ctx, &core, msg, guild, arg).await;
            }
            "speed" => {
                log::command(user, "speed", &format!("ตั้งค่าความเร็วเป็น x{arg}"));
                self.cmd_speed(&core, msg, guild, arg).await;
            }
            "skip" | "s" => {
                log::command(user, "skip", "ข้ามเพลงปัจจุบัน");
                if !core.is_connected(guild) {
                    return core.say(msg.channel_id, "ไม่ได้เชื่อมต่อในช่องเสียง!").await;
                }
                if user_voice_channel(ctx, guild, msg).map(|c| c.get()) != bot_voice_channel(&core, guild) {
                    return core.say(msg.channel_id, "ต้องอยู่ในช่องเสียงเดียวกับบอทก่อน!").await;
                }
                let player = core.player(guild, msg.channel_id);
                let mut p = player.lock().await;
                if let (Some(track), true) = (p.track.clone(), p.current.is_some()) {
                    p.skipping = true;
                    p.elapsed = 0.0;
                    p.paused = false;
                    let _ = track.stop(); // ทำให้ event End ทำงาน → play_next
                    drop(p);
                    core.say(msg.channel_id, "ข้ามเพลงแล้ว ⏭️").await;
                    log::success("ข้ามเพลงเสร็จสมบูรณ์");
                } else {
                    drop(p);
                    core.say(msg.channel_id, "ไม่ได้เล่นเพลงอยู่!").await;
                }
            }
            "loop" | "repeat" | "l" => {
                log::command(user, "loop", &format!("ตั้งค่าโหมด loop: {arg}"));
                self.cmd_loop(&core, msg, guild, arg).await;
            }
            "queue" | "list" | "q" => {
                log::command(user, "queue", "ขอดูรายการคิวเพลง");
                self.cmd_queue(&core, msg, guild).await;
            }
            "shuffle" | "random" => {
                log::command(user, "shuffle", "สับเปลี่ยนคิวเพลงแบบสุ่ม");
                let player = core.player(guild, msg.channel_id);
                let mut p = player.lock().await;
                if p.queue.len() < 2 {
                    drop(p);
                    return core.say(msg.channel_id, "ต้องมีเพลงในคิวอย่างน้อย 2 เพลงถึงจะสุ่มได้!").await;
                }
                p.queue.make_contiguous().shuffle(&mut rand::thread_rng());
                drop(p);
                core.prefetch_next(guild);
                core.say(msg.channel_id, "สลับลำดับเพลงในคิวแบบสุ่มเรียบร้อยแล้ว! 🎲").await;
                log::success("สับเปลี่ยนคิวเพลงเสร็จสิ้น");
            }
            "pause" => {
                log::command(user, "pause", "หยุดเพลงชั่วคราว");
                let player = core.player(guild, msg.channel_id);
                let mut p = player.lock().await;
                match (p.track.clone(), p.current.is_some(), p.paused) {
                    (Some(track), true, false) => {
                        p.elapsed += p.started.elapsed().as_secs_f64() * p.speed;
                        p.paused = true;
                        let _ = track.pause();
                        drop(p);
                        core.say(msg.channel_id, "หยุดเพลงชั่วคราวแล้ว ⏸️").await;
                        log::success("หยุดเพลงชั่วคราวแล้ว");
                    }
                    _ => {
                        drop(p);
                        core.say(msg.channel_id, "ไม่ได้เล่นเพลงอยู่ หรือเพลงหยุดอยู่แล้ว").await;
                    }
                }
            }
            "resume" => {
                log::command(user, "resume", "เล่นเพลงต่อ");
                let player = core.player(guild, msg.channel_id);
                let mut p = player.lock().await;
                match (p.track.clone(), p.paused) {
                    (Some(track), true) => {
                        p.started = std::time::Instant::now();
                        p.paused = false;
                        let _ = track.play();
                        drop(p);
                        core.say(msg.channel_id, "เล่นเพลงต่อแล้ว ▶️").await;
                        log::success("เล่นเพลงต่อเรียบร้อย");
                    }
                    _ => {
                        drop(p);
                        core.say(msg.channel_id, "เพลงไม่ได้หยุดอยู่").await;
                    }
                }
            }
            "np" | "nowplaying" => {
                log::command(user, "nowplaying", "ขอดูเพลงที่กำลังเล่น");
                let player = core.player(guild, msg.channel_id);
                let p = player.lock().await;
                let text = match &p.current {
                    Some(s) => {
                        let speed = if (p.speed - 1.0).abs() > f64::EPSILON { format!(" (ความเร็ว x{})", p.speed) } else { String::new() };
                        let badge = match p.loop_mode {
                            LoopMode::Song => " [🔂 วนซ้ำเพลงนี้]",
                            LoopMode::Queue => " [🔁 วนซ้ำทั้งคิว]",
                            LoopMode::Off => "",
                        };
                        format!("🎶 กำลังเล่น: **{}**{speed}{badge}", s.title)
                    }
                    None => "ขณะนี้ไม่มีการเล่นเพลง".to_string(),
                };
                drop(p);
                core.say(msg.channel_id, text).await;
            }
            "vol" | "volume" | "v" => {
                log::command(user, "volume", &format!("ปรับความดังเสียงเป็น {arg}%"));
                let Some(v) = arg.parse::<i32>().ok().filter(|v| (0..=100).contains(v)) else {
                    return core.say(msg.channel_id, "กรุณาระบุระดับเสียงระหว่าง 0-100").await;
                };
                let player = core.player(guild, msg.channel_id);
                let mut p = player.lock().await;
                p.volume = v as f32 / 100.0;
                if let Some(t) = &p.track {
                    let _ = t.set_volume(p.volume);
                }
                drop(p);
                core.say(msg.channel_id, format!("ปรับระดับเสียงเป็น: **{v}%** 🔊")).await;
                log::success(format!("ปรับระดับเสียงเสร็จสมบูรณ์: {v}%"));
            }
            "stop" => {
                log::command(user, "stop", "หยุดบอทและล้างคิว");
                if core.is_connected(guild) {
                    if let Some(pl) = core.existing_player(guild) {
                        pl.lock().await.reset();
                    }
                    if let Err(e) = core.sb.remove(guild).await {
                        log::error(format!("เกิดข้อผิดพลาดในการตัดการเชื่อมต่อใน !stop: {e}"));
                    }
                    core.say(msg.channel_id, "เลิกเล่นและล้างคิวแล้ว ⏹️").await;
                    log::success("บอทออกจากห้องเสียงและรีเซ็ตค่าคิวแล้ว");
                } else {
                    core.say(msg.channel_id, "บอทไม่ได้เชื่อมต่ออยู่ในช่องเสียง").await;
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        use colored::Colorize;
        match Command::set_global_commands(&ctx.http, slash_commands()).await {
            Ok(c) => log::success(format!("ลงทะเบียน Slash Commands แล้ว {} คำสั่ง", c.len())),
            Err(e) => log::error(format!("ลงทะเบียน Slash Commands ไม่สำเร็จ: {e}")),
        }
        let sb_ok = "Enabled (songbird, pure-Rust crypto — ไม่ต้องใช้ libsodium)".green();
        println!();
        println!("{}", "🎵 DISCORD MUSIC BOT (Rust) 🎵".white().bold().on_blue());
        println!("  Bot Account            : {} ({})", ready.user.name, ready.user.id);
        println!("  Total Guilds Connected : {}", ready.guilds.len());
        println!("  Prefix                 : !");
        println!("  Voice Engine           : {sb_ok}");
        println!("  Discord DAVE           : {}", "Available (davey)".green());
        println!("  Spotify Support        : {}", "Enabled (Track/Album/Playlist)".green());
        println!("  Operating System       : {}", std::env::consts::OS);
        println!();
        log::success(format!("บอทเริ่มทำงานสำเร็จแล้วในชื่อ: {}", ready.user.name));
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }
        let Some(guild) = msg.guild_id else { return };
        let Some(rest) = msg.content.strip_prefix('!') else { return };
        let (cmd, arg) = match rest.split_once(char::is_whitespace) {
            Some((c, a)) => (c.to_lowercase(), a.trim().to_string()),
            None => (rest.to_lowercase(), String::new()),
        };
        let req = Req { channel_id: msg.channel_id, author_id: msg.author.id, author_name: msg.author.name.clone() };
        self.handle(&ctx, &req, guild, &cmd, &arg).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(cmd) = interaction else { return };
        let Some(guild) = cmd.guild_id else {
            let _ = cmd
                .create_response(&ctx.http, CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content("ใช้ได้เฉพาะในเซิร์ฟเวอร์").ephemeral(true),
                ))
                .await;
            return;
        };
        let arg = cmd
            .data
            .options
            .first()
            .map(|o| match &o.value {
                CommandDataOptionValue::String(s) => s.clone(),
                CommandDataOptionValue::Integer(i) => i.to_string(),
                CommandDataOptionValue::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();
        // ตอบ Discord ภายใน 3 วิ แล้วให้ handler ส่งผลลัพธ์จริงลงห้องแชทตามปกติ
        let _ = cmd
            .create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content(format!("⏳ `/{} {}`", cmd.data.name, arg)),
            ))
            .await;
        let req = Req { channel_id: cmd.channel_id, author_id: cmd.user.id, author_name: cmd.user.name.clone() };
        self.handle(&ctx, &req, guild, &cmd.data.name, &arg).await;
    }

    async fn voice_state_update(&self, ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        let Some(guild) = new.guild_id else { return };
        let Some(sb) = songbird::get(&ctx).await else { return };
        let core = self.core(&ctx, &sb);
        let me = ctx.cache.current_user().id;

        // 1. บอทถูกตัดการเชื่อมต่อ (โดนเตะ ฯลฯ)
        if new.user_id == me {
            if let (Some(o), None) = (&old, new.channel_id) {
                if o.channel_id.is_some() {
                    if let Some(pl) = core.existing_player(guild) {
                        pl.lock().await.reset();
                    }
                    log::info("บอทถูกตัดการเชื่อมต่อจากห้องเสียง ทำการรีเซ็ตคิวและสถานะเรียบร้อย");
                }
            }
            return;
        }

        // 2. ผู้ใช้ออกจากห้อง → ถ้าไม่เหลือคนจริงๆ ให้ออกตาม
        let Some(before) = old.and_then(|o| o.channel_id) else { return };
        if new.channel_id == Some(before) {
            return;
        }
        if bot_voice_channel(&core, guild) != Some(before.get()) {
            return;
        }
        let humans = {
            let Some(g) = ctx.cache.guild(guild) else { return };
            g.voice_states
                .values()
                .filter(|v| v.channel_id == Some(before))
                .filter(|v| v.user_id != me)
                .filter(|v| !v.member.as_ref().map(|m| m.user.bot).unwrap_or(false))
                .count()
        };
        if humans == 0 {
            if let Some(pl) = core.existing_player(guild) {
                pl.lock().await.reset();
            }
            if let Err(e) = core.sb.remove(guild).await {
                log::error(format!("เกิดข้อผิดพลาดในการตัดการเชื่อมต่อห้องว่าง: {e}"));
            }
            log::info("ออกจากห้องเสียงอัตโนมัติเนื่องจากห้องว่างเปล่า");
        }
    }
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "songbird=info,serenity=warn,warn".into()),
        )
        .init();
    // โหมดทดสอบ: music-bot --ytdl-test <url|ytsearch1:...>
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|a| a == "--ytdl-test").unwrap_or(false) {
        let q = args.get(2).cloned().unwrap_or_default();
        match ytdl::resolve(&q).await {
            Ok(r) => println!("OK: {}
{}", r.title, r.url),
            Err(e) => println!("ERR: {e}"),
        }
        return;
    }
    let token = std::env::var("DISCORD_TOKEN").expect("ไม่พบ DISCORD_TOKEN ในไฟล์ .env");
    let spotify = SpotifyHelper::new(
        std::env::var("SPOTIFY_CLIENT_ID").ok(),
        std::env::var("SPOTIFY_CLIENT_SECRET").ok(),
    );

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::MESSAGE_CONTENT;

    let manager = Songbird::serenity();
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler { core: OnceLock::new(), spotify })
        .register_songbird_with(manager)
        .await
        .expect("สร้าง Discord client ไม่สำเร็จ");

    log::info("กำลังเชื่อมต่อเข้าสู่ระบบ Discord Gateway...");
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        log::info("กำลังปิดบอทตามคำสั่งผู้ใช้งาน (Ctrl+C)");
        shard_manager.shutdown_all().await;
    });

    if let Err(e) = client.start().await {
        log::error(format!("Client error: {e}"));
    }
    let _ = Duration::from_secs(0);
}
