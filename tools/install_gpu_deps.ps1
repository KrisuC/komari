# install_gpu_deps.ps1
# Installs CUDA 12.x and cuDNN 9.x runtime DLLs needed for ONNX Runtime GPU inference.
#
# Usage:
#   Double-click the file, or run:
#   powershell -ExecutionPolicy Bypass -File install_gpu_deps.ps1 -Local "C:\Path\To\App"
#
# Options:
#   -SystemWide    Install DLLs to C:\Windows\System32 (requires admin)
#   -Local <path>  Copy DLLs next to a specific exe (no admin needed)
#   -SkipCuda      Don't install CUDA DLLs
#   -SkipCudnn     Don't install cuDNN DLLs

param(
    [switch]$SystemWide,
    [string]$Local,
    [switch]$SkipCuda,
    [switch]$SkipCudnn
)

# Force output immediately, even if the window is about to close
$Host.UI.RawUI.WindowTitle = "GPU Dependency Installer"

# Wrap everything so we pause on ANY error
try {
$ErrorActionPreference = "Continue"
$ScriptStartTime = Get-Date

# ---- RESOLVE TARGET DIRECTORY ----
Write-Host "Step 1/5: Resolving target directory..." -ForegroundColor Cyan

if ($Local) {
    $TargetDir = (Resolve-Path $Local -ErrorAction SilentlyContinue)
    if (-not $TargetDir) {
        Write-Host "  ERROR: Directory not found: $Local" -ForegroundColor Red
        Write-Host "  Press any key to exit..." -ForegroundColor Yellow
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
        exit 1
    }
    $TargetDir = $TargetDir.Path
    Write-Host "  Using provided path: $TargetDir" -ForegroundColor Green
} elseif ($SystemWide) {
    $TargetDir = "$env:SystemRoot\System32"
    $isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole] "Administrator")
    if (-not $isAdmin) {
        Write-Host "  ERROR: SystemWide install requires Administrator privileges." -ForegroundColor Red
        Write-Host "  Right-click PowerShell → Run as Administrator, then try again." -ForegroundColor Yellow
        Write-Host "  Press any key to exit..." -ForegroundColor Yellow
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
        exit 1
    }
    Write-Host "  System-wide install to: $TargetDir" -ForegroundColor Green
} else {
    Write-Host "  ERROR: No target specified." -ForegroundColor Red
    Write-Host ""
    Write-Host "  Use -SystemWide to install globally, or -Local <path> for a specific app." -ForegroundColor Yellow
    Write-Host "  Example: powershell -ExecutionPolicy Bypass -File install_gpu_deps.ps1 -SystemWide" -ForegroundColor White
    Write-Host "  Example: powershell -ExecutionPolicy Bypass -File install_gpu_deps.ps1 -Local 'C:\MyApp'" -ForegroundColor White
    Write-Host ""
    Write-Host "  Use the .bat file to avoid this: just double-click install_gpu_deps.bat" -ForegroundColor Cyan
    throw "No target directory specified. Use -SystemWide or -Local."
}

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  GPU Dependency Installer" -ForegroundColor Cyan
Write-Host "  ONNX Runtime 1.22 / CUDA 12.x / cuDNN 9.x" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  Started at : $ScriptStartTime" -ForegroundColor Gray
Write-Host "  Target     : $TargetDir" -ForegroundColor Gray
Write-Host "  PC name    : $env:COMPUTERNAME" -ForegroundColor Gray
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# ---- STEP 2: SCAN SYSTEM ----
Write-Host "Step 2/5: Scanning system for existing GPU libraries..." -ForegroundColor Cyan
Write-Host ""

# Check GPU
Write-Host "  Checking GPU..." -ForegroundColor White
try {
    $gpuInfo = & nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>$null
    if ($gpuInfo) {
        Write-Host "    Found: $gpuInfo" -ForegroundColor Green
    } else {
        Write-Host "    WARNING: nvidia-smi not found. Is an NVIDIA GPU present?" -ForegroundColor Yellow
        Write-Host "    Driver check: nvidia-smi is not available on this machine." -ForegroundColor Yellow
    }
} catch {
    Write-Host "    WARNING: Could not run nvidia-smi. Is an NVIDIA GPU present?" -ForegroundColor Yellow
}

# Check CUDA Toolkit installations
Write-Host ""
Write-Host "  Looking for CUDA Toolkit installations..." -ForegroundColor White
$cudaInstallDirs = @(Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA" -ErrorAction SilentlyContinue)
if ($cudaInstallDirs.Count -gt 0) {
    foreach ($dir in $cudaInstallDirs) {
        $versionFile = "$($dir.FullName)\version.txt"
        if (Test-Path $versionFile) {
            $cudaVersion = (Get-Content $versionFile -Raw).Trim()
            Write-Host "    Found CUDA $cudaVersion at $($dir.FullName)" -ForegroundColor Green
        } else {
            Write-Host "    Found CUDA at $($dir.FullName)" -ForegroundColor Green
        }
        $dllCount = @(Get-ChildItem "$($dir.FullName)\bin\*64_*.dll" -ErrorAction SilentlyContinue).Count
        Write-Host "      Contains $dllCount CUDA runtime DLLs" -ForegroundColor Gray
    }
} else {
    Write-Host "    No CUDA Toolkit installation found." -ForegroundColor Yellow
    Write-Host "    (This is fine — the script can download what's needed)" -ForegroundColor Gray
}

# Check if Python is available (needed for cuDNN auto-download)
Write-Host ""
Write-Host "  Looking for Python (for cuDNN download)..." -ForegroundColor White
$python = $null
foreach ($cmd in @('python', 'python3', 'py')) {
    $found = Get-Command $cmd -ErrorAction SilentlyContinue
    if ($found) {
        $python = $found
        $pyVersion = & $python.Source --version 2>&1
        Write-Host "    Found: $pyVersion at $($python.Source)" -ForegroundColor Green
        break
    }
}
if (-not $python) {
    Write-Host "    Python not found on PATH." -ForegroundColor Yellow
    Write-Host "    Checking winget for Python installation..." -ForegroundColor Gray
}

# Check if target dir already has some DLLs
Write-Host ""
Write-Host "  DLLs already in target directory:" -ForegroundColor White
$existingGpuDlls = @(Get-ChildItem $TargetDir -Filter "*64_*.dll" -ErrorAction SilentlyContinue) + @(Get-ChildItem $TargetDir -Filter "cudnn*.dll" -ErrorAction SilentlyContinue)
if ($existingGpuDlls.Count -gt 0) {
    foreach ($dll in $existingGpuDlls) {
        $sizeMB = [math]::Round($dll.Length / 1MB, 1)
        Write-Host "    $($dll.Name) ($sizeMB MB)" -ForegroundColor Green
    }
} else {
    Write-Host "    (none yet — will install)" -ForegroundColor Gray
}

# ---- HELPER FUNCTIONS ----
function Find-Dll {
    param([string]$Name)
    if (Test-Path "$script:TargetDir\$Name") { return "$script:TargetDir\$Name" }
    foreach ($dir in Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA" -ErrorAction SilentlyContinue) {
        if (Test-Path "$($dir.FullName)\bin\$Name") { return "$($dir.FullName)\bin\$Name" }
    }
    $found = (Get-Command $Name -ErrorAction SilentlyContinue)
    if ($found) { return $found.Source }
    return $null
}

function Test-Dlls {
    param([string[]]$Names)
    $missing = @()
    foreach ($n in $Names) {
        if (-not (Find-Dll $n)) { $missing += $n }
    }
    return $missing
}

# ---- STEP 3: CUDA ----
Write-Host ""
Write-Host "Step 3/5: Installing CUDA runtime DLLs..." -ForegroundColor Cyan
Write-Host ""

$cudaDlls = @(
    "cudart64_12.dll",
    "cublas64_12.dll",
    "cublasLt64_12.dll",
    "cufft64_11.dll",
    "curand64_10.dll",
    "cusparse64_12.dll",
    "cusolver64_11.dll",
    "nvrtc64_12.dll",
    "nvrtc-builtins64_12.dll",
    "nvJitLink_12.dll"
)

if ($SkipCuda) {
    Write-Host "  SKIPPED (requested by user)" -ForegroundColor Gray
} else {
    $missingCuda = Test-Dlls $cudaDlls
    Write-Host "  Checking $($cudaDlls.Count) required DLLs..." -ForegroundColor White

    if ($missingCuda.Count -eq 0) {
        Write-Host "  All CUDA DLLs already present — nothing to do." -ForegroundColor Green
    } else {
        Write-Host "  Missing $($missingCuda.Count) DLL(s):" -ForegroundColor Yellow
        foreach ($dll in $missingCuda) {
            Write-Host "    - $dll" -ForegroundColor Yellow
        }

        $cudaInstall = @(Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v*\bin" -ErrorAction SilentlyContinue) |
            Sort-Object -Descending |
            Select-Object -First 1

        if ($cudaInstall) {
            Write-Host "  Using CUDA installation at: $($cudaInstall.Parent.FullName)" -ForegroundColor Green
            Write-Host "  Copying DLLs to target directory..." -ForegroundColor White
            $copiedCount = 0
            $totalSize = 0

            # Copy requested DLLs
            foreach ($dll in $missingCuda) {
                $src = "$($cudaInstall.FullName)\$dll"
                if (Test-Path $src) {
                    $size = (Get-Item $src).Length
                    $totalSize += $size
                    $sizeMB = [math]::Round($size / 1MB, 1)
                    Copy-Item $src $TargetDir -Force
                    Write-Host "    Copied $dll ($sizeMB MB)" -ForegroundColor Green
                    $copiedCount++
                } else {
                    Write-Host "    NOT FOUND in CUDA: $dll" -ForegroundColor Yellow
                }
            }

            # Also copy any extra *64_*.dll that might be useful
            Write-Host "  Copying additional CUDA runtime DLLs..." -ForegroundColor White
            foreach ($dll in Get-ChildItem "$($cudaInstall.FullName)\*64_*.dll" -Name) {
                if (-not (Test-Path "$TargetDir\$dll")) {
                    $size = (Get-Item "$($cudaInstall.FullName)\$dll").Length
                    $sizeMB = [math]::Round($size / 1MB, 1)
                    Copy-Item "$($cudaInstall.FullName)\$dll" $TargetDir -Force
                    Write-Host "    Copied extra: $dll ($sizeMB MB)" -ForegroundColor Green
                    $copiedCount++
                    $totalSize += $size
                }
            }

            $totalSizeMB = [math]::Round($totalSize / 1MB, 1)
            Write-Host "  Done. Copied $copiedCount DLL(s) ($totalSizeMB MB total)." -ForegroundColor Green
        } else {
            Write-Host "  No CUDA Toolkit found locally." -ForegroundColor Yellow
            Write-Host "  NOTE: Full CUDA installation is recommended." -ForegroundColor Yellow
            Write-Host "  Download CUDA Toolkit 12.x from:" -ForegroundColor Yellow
            Write-Host "    https://developer.nvidia.com/cuda-downloads" -ForegroundColor White
            Write-Host "  Set CUDA_PATH or re-run this script after installing." -ForegroundColor White
        }
    }
}

# ---- STEP 4: cuDNN ----
Write-Host ""
Write-Host "Step 4/5: Installing cuDNN 9.x DLLs..." -ForegroundColor Cyan
Write-Host ""

$cudnnDlls = @(
    "cudnn64_9.dll",
    "cudnn_ops64_9.dll",
    "cudnn_cnn64_9.dll",
    "cudnn_adv64_9.dll",
    "cudnn_eng64_9.dll",
    "cudnn_graph64_9.dll"
)

if ($SkipCudnn) {
    Write-Host "  SKIPPED (requested by user)" -ForegroundColor Gray
} else {
    $missingCudnn = Test-Dlls $cudnnDlls
    Write-Host "  Checking $($cudnnDlls.Count) required DLLs..." -ForegroundColor White

    if ($missingCudnn.Count -eq 0) {
        Write-Host "  All cuDNN DLLs already present — nothing to do." -ForegroundColor Green
    } else {
        Write-Host "  Missing $($missingCudnn.Count) DLL(s):" -ForegroundColor Yellow
        foreach ($dll in $missingCudnn) {
            Write-Host "    - $dll" -ForegroundColor Yellow
        }

        # Try to get Python if not already found
        if (-not $python) {
            Write-Host "  Attempting to install Python via winget..." -ForegroundColor White
            try {
                winget install Python.Python.3.12 --accept-package-agreements --accept-source-agreements 2>$null
                # Refresh PATH in this session
                $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
                foreach ($cmd in @('python', 'python3', 'py')) {
                    $found = Get-Command $cmd -ErrorAction SilentlyContinue
                    if ($found) { $python = $found; break }
                }
                if ($python) {
                    Write-Host "    Python installed: $(& $python.Source --version 2>&1)" -ForegroundColor Green
                } else {
                    Write-Host "    Python installed but not found on PATH — you may need to restart your terminal." -ForegroundColor Yellow
                }
            } catch {
                Write-Host "    winget failed. Is winget installed?" -ForegroundColor Red
            }
        }

        if ($python) {
            Write-Host "  Downloading cuDNN via pip (nvidia-cudnn-cu12)..." -ForegroundColor White
            Write-Host "  (This may take a minute — the package is ~600MB)" -ForegroundColor Gray
            Write-Host "  NOTE: No NVIDIA account required — this is a public PyPI package." -ForegroundColor Gray
            Write-Host ""

            try {
                $pipArgs = @("-m", "pip", "install", "nvidia-cudnn-cu12", "--disable-pip-version-check", "--progress-bar", "on")
                & $python.Source @pipArgs 2>&1 | ForEach-Object {
                    if ($_ -match "Successfully|already satisfied|Downloading|Installing") {
                        Write-Host "  pip: $_" -ForegroundColor Gray
                    }
                }

                if ($LASTEXITCODE -ne 0) {
                    Write-Host "  WARNING: pip install may have failed (exit code: $LASTEXITCODE)." -ForegroundColor Yellow
                    Write-Host "  Trying with --user flag..." -ForegroundColor Yellow
                    & $python.Source -m pip install nvidia-cudnn-cu12 --user --disable-pip-version-check 2>&1 | Out-Null
                }

                # Find where pip installed it
                Write-Host "  Locating cuDNN package..." -ForegroundColor White
                $pipShow = & $python.Source -m pip show nvidia-cudnn-cu12 2>$null | Out-String
                if ($pipShow -match "Location:\s+(.+)") {
                    $cudnnPackage = Join-Path $matches[1].Trim() "nvidia\cudnn"
                    Write-Host "  Package location: $cudnnPackage" -ForegroundColor Gray

                    $dllPaths = @()
                    if (Test-Path "$cudnnPackage\bin") {
                        $dllPaths = @(Get-ChildItem "$cudnnPackage\bin\*.dll")
                    } elseif (Test-Path "$cudnnPackage\Lib\x64") {
                        $dllPaths = @(Get-ChildItem "$cudnnPackage\Lib\x64\*.dll")
                    } else {
                        $dllPaths = @(Get-ChildItem $cudnnPackage -Recurse -Filter "*.dll" -ErrorAction SilentlyContinue)
                    }

                    if ($dllPaths.Count -gt 0) {
                        $copiedCount = 0
                        $totalSize = 0
                        foreach ($dll in $dllPaths) {
                            $sizeMB = [math]::Round($dll.Length / 1MB, 1)
                            Copy-Item $dll.FullName $TargetDir -Force
                            Write-Host "    Copied $($dll.Name) ($sizeMB MB)" -ForegroundColor Green
                            $copiedCount++
                            $totalSize += $dll.Length
                        }
                        $totalSizeMB = [math]::Round($totalSize / 1MB, 1)
                        Write-Host "  Done. Copied $copiedCount DLL(s) ($totalSizeMB MB total)." -ForegroundColor Green
                    } else {
                        Write-Host "  WARNING: No DLLs found in cuDNN package at $cudnnPackage" -ForegroundColor Red
                        Write-Host "  Listing contents for debugging:" -ForegroundColor Yellow
                        Get-ChildItem $cudnnPackage -Directory | ForEach-Object { Write-Host "    $($_.Name)/" -ForegroundColor Gray }
                    }
                } else {
                    Write-Host "  ERROR: Could not locate nvidia-cudnn-cu12 after pip install." -ForegroundColor Red
                    Write-Host "  Pip show output:" -ForegroundColor Yellow
                    Write-Host $pipShow -ForegroundColor Gray
                }
            } catch {
                Write-Host "  ERROR: pip command failed: $_" -ForegroundColor Red
            }
        } else {
            Write-Host "  ERROR: Python is required to auto-download cuDNN." -ForegroundColor Red
            Write-Host "  Install Python manually (https://python.org) then re-run." -ForegroundColor Red
            Write-Host "  Or download cuDNN manually from:" -ForegroundColor Red
            Write-Host "    https://developer.nvidia.com/cudnn" -ForegroundColor White
            Write-Host "  (NVIDIA account required for manual download)" -ForegroundColor Gray
        }
    }
}

# ---- STEP 5: VERIFY ----
Write-Host ""
Write-Host "Step 5/5: Verifying installation..." -ForegroundColor Cyan
Write-Host ""

$allDlls = $cudaDlls + $cudnnDlls
$stillMissing = Test-Dlls $allDlls

Write-Host "  Checking all $($allDlls.Count) required DLLs:" -ForegroundColor White

$allGpuDlls = @(Get-ChildItem $TargetDir -Filter "*64_*.dll" -ErrorAction SilentlyContinue) + @(Get-ChildItem $TargetDir -Filter "cudnn*.dll" -ErrorAction SilentlyContinue)
$totalInstalledSize = 0
foreach ($dll in $allGpuDlls | Sort-Object Name) {
    $sizeMB = [math]::Round($dll.Length / 1MB, 1)
    $status = if ($dll.Name -in $stillMissing) { "MISSING" } else { "OK" }
    $color = if ($status -eq "OK") { "Green" } else { "Red" }
    Write-Host "    [$status] $($dll.Name) ($sizeMB MB)" -ForegroundColor $color
    $totalInstalledSize += $dll.Length
}

$totalInstalledSizeMB = [math]::Round($totalInstalledSize / 1MB, 1)
Write-Host ""
Write-Host "  Total GPU DLLs in target: $($allGpuDlls.Count) file(s) ($totalInstalledSizeMB MB)" -ForegroundColor White

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
if ($stillMissing.Count -gt 0) {
    Write-Host "  RESULT: PARTIAL INSTALL" -ForegroundColor Yellow
    Write-Host "============================================" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "  Missing $($stillMissing.Count) DLL(s):" -ForegroundColor Yellow
    foreach ($dll in $stillMissing) {
        Write-Host "    - $dll" -ForegroundColor Yellow
    }
    Write-Host ""
    Write-Host "  The app will still RUN, but may fall back to CPU for" -ForegroundColor Yellow
    Write-Host "  some operations if these DLLs are needed." -ForegroundColor Yellow
} else {
    Write-Host "  RESULT: ALL DEPENDENCIES INSTALLED" -ForegroundColor Green
    Write-Host "============================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "  GPU acceleration should now be fully functional." -ForegroundColor Green
}

Write-Host ""
Write-Host "  Target: $TargetDir" -ForegroundColor Cyan
$ScriptEndTime = Get-Date
$elapsed = [math]::Round(($ScriptEndTime - $ScriptStartTime).TotalSeconds, 1)
Write-Host "  Elapsed: $elapsed seconds" -ForegroundColor Gray
Write-Host ""

# ---- PAUSE ----
Write-Host "============================================" -ForegroundColor DarkGray
Write-Host "  Press any key to close this window..." -ForegroundColor DarkGray
Write-Host "============================================" -ForegroundColor DarkGray
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")

} catch {
    # Catch ALL errors and show them before pausing
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Red
    Write-Host "  SCRIPT ERROR" -ForegroundColor Red
    Write-Host "============================================" -ForegroundColor Red
    Write-Host ""
    Write-Host "  $_" -ForegroundColor Red
    Write-Host ""
    if ($_.Exception.StackTrace) {
        Write-Host "  Stack trace:" -ForegroundColor Yellow
        Write-Host "  $($_.Exception.StackTrace)" -ForegroundColor Yellow
    }
    Write-Host ""
    Write-Host "============================================" -ForegroundColor DarkGray
    Write-Host "  Press any key to close this window..." -ForegroundColor DarkGray
    Write-Host "============================================" -ForegroundColor DarkGray
    $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    exit 1
}
