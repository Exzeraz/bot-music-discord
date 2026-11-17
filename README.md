# Music Bot

A Discord music bot written in Rust ([serenity](https://github.com/serenity-rs/serenity) + [songbird](https://github.com/serenity-rs/songbird)). Plays audio from YouTube and Spotify links via `yt-dlp` and `ffmpeg`.

🇹🇭 [อ่านภาษาไทย](README_TH.md)

## Features
- Play from YouTube URL, search term, or Spotify URL
- Queue, skip, pause/resume, stop, shuffle, loop (off / song / queue)
- Volume (0-100) and playback speed (0.1-1000)
- Both `!prefix` and `/slash` commands

## Requirements
- [Rust](https://rustup.rs/) (stable)
- `yt-dlp` and `ffmpeg` (see below)
- A Discord bot token ([Developer Portal](https://discord.com/developers/applications))

## Setup
1. Put `yt-dlp_x86.exe` and `ffmpeg.exe` in the [`bin/`](bin) folder (they are git-ignored because of their size), **or** install them system-wide and leave the paths unset.
2. Copy the example config and fill it in:
   ```
   copy .env.example .env
   ```
3. Run:
   ```
   run.bat
   ```
   (or `cargo run --release`)

## Configuration (`.env`)
| Variable | Required | Description |
|---|---|---|
| `DISCORD_TOKEN` | yes | Discord bot token |
| `SPOTIFY_CLIENT_ID` / `SPOTIFY_CLIENT_SECRET` | no | Enables Spotify links |
| `YT_DLP_PATH` | no | Path to yt-dlp. Default: `yt-dlp` from `PATH` |
| `FFMPEG_PATH` | no | Path to ffmpeg. Default: `ffmpeg` from `PATH` |

## Commands
`play` (`p`), `skip` (`s`), `pause`, `resume`, `stop`, `loop`, `queue` (`q`), `np`, `shuffle`, `vol`, `speed`, `help`.

## Project layout
```
bin/        yt-dlp and ffmpeg binaries (git-ignored)
src/        bot source (main, player, spotify, ytdl, log)
.env        your local config (git-ignored)
```

## License

[The Unlicense](LICENSE) - public domain.
