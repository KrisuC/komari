# Building komari

## Prerequisites

| Tool | Version | Purpose | Install |
|---|---|---|---|
| Rust | nightly-2025-12-21 | Compiler (edition 2024) | `winget install Rustlang.Rustup` then `rustup toolchain install nightly-2025-12-21 --profile minimal` |
| Dioxus CLI | 0.7.9 | Asset bundling, desktop shell | `cargo install dioxus-cli --version 0.7.9 --locked` |
| LLVM | 21.1.0 | libclang for OpenCV bindings | `winget install LLVM.LLVM --version 21.1.0` |
| OpenCV 4 | 4.12.0 | Computer vision | Via vcpkg (see below) |
| vcpkg | latest | C++ package manager | `git clone --depth 1 https://github.com/microsoft/vcpkg.git %USERPROFILE%\vcpkg` then `bootstrap-vcpkg.bat` |
| protobuf | 35.0 | Protocol buffer compiler | `winget install Google.Protobuf` |
| Node.js | ≥ 20 | TailwindCSS build (npx) | `winget install OpenJS.NodeJS` |
| Visual Studio | 2022 Community | MSVC linker & CRT | `winget install Microsoft.VisualStudio.2022.Community` |

### OpenCV via vcpkg

```powershell
cd %USERPROFILE%\vcpkg
.\bootstrap-vcpkg.bat
.\vcpkg.exe install "opencv4[contrib,nonfree]:x64-windows-static"
```

This builds OpenCV from source (~15-30 minutes).

### Node.js dependencies

```bash
cd ui
npm install
```

## Environment setup

Create `%USERPROFILE%\.cargo\config.toml`:

```toml
[env]
OPENCV_DISABLE_PROBES = "environment,pkg_config,cmake,vcpkg_cmake"
VCPKGRS_TRIPLET = "x64-windows-static"
VCPKG_ROOT = "C:\\Users\\<your-username>\\vcpkg"
OPENCV_MSVC_CRT = "static"
LIBCLANG_PATH = "C:\\Program Files\\LLVM\\bin"

[http]
proxy = "http://127.0.0.1:7890"     # If using a local proxy (clash/v2ray)

[build]
target = "x86_64-pc-windows-msvc"
```

If you use a proxy, set the environment variables before building:

```bash
export HTTP_PROXY="http://127.0.0.1:7890"
export HTTPS_PROXY="http://127.0.0.1:7890"
```

## libclang.dll conflict

PlasticSCM installs an incompatible `libclang.dll` that appears earlier on `PATH`. This causes OpenCV bindings to crash with `0xc000007b`.

**Fix:** Always prepend LLVM to `PATH` before building:

```bash
export PATH="/c/Program Files/LLVM/bin:$PATH"
export LIBCLANG_PATH="/c/Program Files/LLVM/bin"
```

## Dioxus version mismatch

The project uses **dioxus 0.7.2** but the installed **dx CLI is 0.7.9**. This causes `dx build` to fail at the linker step with:

```
CVTRES : fatal error CVT1100: duplicate resource. type:VERSION, name:1, language:0x0409
```

### Workaround

Create an empty Windows resource file before building to prevent the duplicate:

**In PowerShell:**

```powershell
$winres = "target\dx\ui\<profile>\windows\.winres"
mkdir $winres -Force
"" > "$winres\empty.rc"
& 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\rc.exe' /fo "$winres\empty.res" "$winres\empty.rc"
& 'C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.50.35717\bin\HostX64\x64\lib.exe' /OUT:"$winres\resource.lib" "$winres\empty.res"
attrib +R "$winres\resource.lib"
```

Replace `<profile>` with `release` or `debug` depending on the build target.

> **Note:** Adjust the Windows SDK and MSVC version numbers to match your installation.

## Building

### Release

```bash
# Set up environment
export PATH="/c/Program Files/LLVM/bin:$HOME/.cargo/bin:$PATH"
export HTTP_PROXY="http://127.0.0.1:7890"
export HTTPS_PROXY="http://127.0.0.1:7890"
export LIBCLANG_PATH="/c/Program Files/LLVM/bin"

# Run the empty resource workaround first (PowerShell, see above)

# Build
dx build --release --package ui
```

Output: `target\dx\ui\release\windows\app\ui.exe`

### Debug

```bash
dx build --package ui
```

Output: `target\dx\ui\debug\windows\app\ui.exe` (includes console window for log output)

### Shortcuts

`ui_release.bat` and `ui_debug.bat` at the project root launch the respective builds.

## Troubleshooting

### "failed to run custom build command for opencv" (0xc000007b)
Wrong `libclang.dll` on PATH. Ensure LLVM `bin/` is first in `PATH` and `LIBCLANG_PATH` is set.

### "cargo metadata took too long to respond"  
Network/slow proxy. Ensure `HTTP_PROXY`/`HTTPS_PROXY` are set correctly, or try `--offline` if dependencies are already cached.

### "couldn't determine visual studio generator"
Missing VS environment. Run from a terminal with VS tools (Developer Command Prompt) or ensure vcvars was sourced.

### "failed to run custom build command for aws-lc-sys"
This only affects `cargo install dioxus-cli --version 0.7.2`. Use dx 0.7.9 with the empty resource workaround instead.

### TailwindCSS build fails
Run `npm install` in the `ui/` directory first to install the `@tailwindcss/cli` package.
