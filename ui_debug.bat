@echo off
REM Auto-elevate to admin if not already running as admin
net session >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo Requesting administrator privileges...
    powershell -Command "Start-Process '%~f0' -Verb RunAs -WorkingDirectory '%~dp0'"
    exit /b
)
start "" "%~dp0target\dx\ui\debug\windows\app\ui.exe"
