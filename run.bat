@echo off
cd /d "%~dp0"

echo [INFO] Building (skips if up to date)...
cargo build --release
if errorlevel 1 (
    echo [ERROR] Build failed. If the bot is already running, close it first.
    pause
    exit /b 1
)

echo [INFO] Starting Rust bot...
"target\release\music-bot.exe"

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] Bot exited with code %errorlevel%.
    pause
)
