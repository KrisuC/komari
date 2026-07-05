@echo off
REM Auto-elevate to admin if not already running as admin
net session >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo Requesting administrator privileges...
    powershell -Command "Start-Process '%~f0' -Verb RunAs -WorkingDirectory '%~dp0'"
    exit /b
)

REM Ensure 64-bit LLVM is found before the 32-bit PlasticSCM libclang.dll
set "PATH=C:\Program Files\LLVM\bin;%PATH%"
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"

echo === Building debug ===
dx build --package ui
if %ERRORLEVEL% neq 0 (
    echo DEBUG BUILD FAILED
    exit /b %ERRORLEVEL%
)

echo === Building release ===
set "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true"
dx build --package ui --release
if %ERRORLEVEL% neq 0 (
    echo RELEASE BUILD FAILED
    exit /b %ERRORLEVEL%
)

echo === Both builds succeeded ===

REM Copy examples and kmbox script to release output
echo === Copying extras to release ===
xcopy /E /I /Y "%~dp0examples" "%~dp0target\dx\ui\release\windows\app\examples"
copy /Y "%~dp0kmbox_example.bat" "%~dp0target\dx\ui\release\windows\app\"

REM Launch the debug build (runs as admin since we elevated)
echo === Launching debug build ===
start "" "%~dp0target\dx\ui\debug\windows\app\ui.exe"
