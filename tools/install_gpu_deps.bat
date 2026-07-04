@echo off
:: GPU Dependency Installer for ONNX Runtime
::
:: Double-click to install system-wide (requires admin) — recommended.
:: Or drag-and-drop an app folder to install locally (no admin needed).

set TARGET=%~1

if "%TARGET%"=="" (
    echo Installing GPU dependencies system-wide...
    echo This requires Administrator privileges.
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File ""%~dp0install_gpu_deps.ps1"" -SystemWide'"
) else (
    echo Installing GPU dependencies to: %TARGET%
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1" -Local "%TARGET%"
)
pause
