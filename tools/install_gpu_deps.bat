@echo off
setlocal enabledelayedexpansion

set TARGET=%~1

:: If no target and not admin, self-elevate
if "%TARGET%"=="" (
    net session >nul 2>&1
    if !errorlevel! neq 0 (
        echo Requesting Administrator privileges...
        powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process '%~f0' -Verb RunAs -Wait"
        exit /b
    )

    echo Installing GPU dependencies system-wide...
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1" -SystemWide
) else (
    echo Installing GPU dependencies to: %TARGET%
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1" -Local "%TARGET%"
)
pause
