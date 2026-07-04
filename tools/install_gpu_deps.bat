@echo off
set TARGET=%~1
set LOG=%TEMP%\gpu_deps_install.log

if "%TARGET%"=="" (
    echo Installing GPU dependencies system-wide...
    echo This requires Administrator privileges (UAC popup incoming).
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File ""%~dp0install_gpu_deps.ps1"" -SystemWide'"
    echo.
    if exist "%LOG%" (
        echo --- INSTALL LOG ---
        type "%LOG%"
        echo --- END OF LOG ---
        del "%LOG%"
    ) else (
        echo WARNING: No log file found. The elevated window may not have run.
    )
) else (
    echo Installing GPU dependencies to: %TARGET%
    echo.
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_gpu_deps.ps1" -Local "%TARGET%"
)
pause
