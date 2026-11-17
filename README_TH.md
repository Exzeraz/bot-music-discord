# Music Bot

บอทเพลง Discord เขียนด้วย Rust ([serenity](https://github.com/serenity-rs/serenity) + [songbird](https://github.com/serenity-rs/songbird)) เล่นเสียงจากลิงก์ YouTube และ Spotify ผ่าน `yt-dlp` และ `ffmpeg`

🇬🇧 [English](README.md)

## ความสามารถ
- เล่นจาก YouTube URL, คำค้นหา หรือ Spotify URL
- คิวเพลง, ข้าม, หยุดชั่วคราว/เล่นต่อ, หยุด, สุ่ม, วนซ้ำ (off / song / queue)
- ปรับเสียง (0-100) และความเร็ว (0.1-1000)
- ใช้ได้ทั้งคำสั่ง `!` และ `/`

## สิ่งที่ต้องมี
- [Rust](https://rustup.rs/) (stable)
- `yt-dlp` และ `ffmpeg` (ดูด้านล่าง)
- Discord bot token ([Developer Portal](https://discord.com/developers/applications))

## วิธีติดตั้ง
1. วาง `yt-dlp_x86.exe` และ `ffmpeg.exe` ไว้ในโฟลเดอร์ [`bin/`](bin) (ไฟล์ใหญ่ จึงไม่ถูกเก็บใน git) **หรือ** ติดตั้งลงเครื่องแล้วไม่ต้องกำหนด path
2. คัดลอกไฟล์ตัวอย่างแล้วกรอกค่า:
   ```
   copy .env.example .env
   ```
3. รันบอท:
   ```
   run.bat
   ```
   (หรือ `cargo run --release`)

## การตั้งค่า (`.env`)
| ตัวแปร | จำเป็น | คำอธิบาย |
|---|---|---|
| `DISCORD_TOKEN` | ใช่ | โทเคนบอท Discord |
| `SPOTIFY_CLIENT_ID` / `SPOTIFY_CLIENT_SECRET` | ไม่ | เปิดใช้งานลิงก์ Spotify |
| `YT_DLP_PATH` | ไม่ | path ของ yt-dlp (ค่าเริ่มต้น: `yt-dlp` จาก `PATH`) |
| `FFMPEG_PATH` | ไม่ | path ของ ffmpeg (ค่าเริ่มต้น: `ffmpeg` จาก `PATH`) |

## คำสั่ง
`play` (`p`), `skip` (`s`), `pause`, `resume`, `stop`, `loop`, `queue` (`q`), `np`, `shuffle`, `vol`, `speed`, `help`

## โครงสร้างโปรเจกต์
```
bin/        ไฟล์ yt-dlp และ ffmpeg (ไม่เก็บใน git)
src/        โค้ดบอท (main, player, spotify, ytdl, log)
.env        ค่าตั้งค่าในเครื่อง (ไม่เก็บใน git)
```

## สัญญาอนุญาต

[The Unlicense](LICENSE) - สาธารณสมบัติ (public domain)
