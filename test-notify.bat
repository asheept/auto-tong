@echo off
chcp 65001 >nul
echo [Auto-Tong] notify test
echo.
echo Log: %APPDATA%\auto-tong\auto-tong.log
echo.
echo OK: "fs watch" message appears after adding zip to Drive folder
echo NG: Only "polling" message appears
echo.
set RUST_LOG=info
"C:\autu-tong-target\release\auto-tong.exe"
pause
