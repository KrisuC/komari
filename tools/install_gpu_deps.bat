@echo off
:: Double-click this file to install GPU dependencies.
:: It will launch the PowerShell script with the correct settings.
::
:: To specify a custom target folder, drag-and-drop your app folder onto this file,
:: or run from command line:
::   install_gpu_deps.bat "C:\Path\To\App"

set TARGET=%~1
if "%TARGET%"=="" (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1"
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1" -Local "%TARGET%"
)
pause
