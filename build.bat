@echo off
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
