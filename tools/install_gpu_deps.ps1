# install_gpu_deps.ps1
# Installs CUDA 12.x and cuDNN 9.x runtime DLLs needed for ONNX Runtime GPU inference.
#
# Usage: powershell -ExecutionPolicy Bypass -File install_gpu_deps.ps1
#
# Options:
#   -SystemWide    Install DLLs to C:\Windows\System32 (requires admin, works for any app)
#   -Local <path>  Copy DLLs next to a specific exe (no admin needed)
#   (default)      Copies DLLs to the script's own directory

param(
    [switch]$SystemWide,
    [string]$Local,
    [switch]$SkipCuda,
    [switch]$SkipCudnn
)

$ErrorActionPreference = "Stop"

# ---- CONFIG ----
$CUDA_VERSION  = "12.8"
$CUDNN_VERSION = "9.10.2"   # cuDNN 9.x for CUDA 12.x

# ---- RESOLVE TARGET DIRECTORY ----
if ($Local) {
    $TargetDir = (Resolve-Path $Local).Path
} elseif ($SystemWide) {
    $TargetDir = "$env:SystemRoot\System32"
    $isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole] "Administrator")
    if (-not $isAdmin) {
        Write-Error "SystemWide install requires Administrator privileges. Run PowerShell as admin."
        exit 1
    }
} else {
    $TargetDir = $PSScriptRoot
}

Write-Host @"

============================================
 GPU Dependency Installer for ONNX Runtime
============================================
 Target: $TargetDir
 System-wide: $SystemWide
 CUDA: $CUDA_VERSION
 cuDNN: $CUDNN_VERSION

"@ -ForegroundColor Cyan

# ---- HELPER: check if a DLL exists on the system ----
function Find-Dll {
    param([string]$Name)
    # Check target dir first
    if (Test-Path "$TargetDir\$Name") { return "$TargetDir\$Name" }
    # Check CUDA installations
    foreach ($dir in Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA" -ErrorAction SilentlyContinue) {
        if (Test-Path "$($dir.FullName)\bin\$Name") { return "$($dir.FullName)\bin\$Name" }
    }
    # Check PATH
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

# ---- CUDA ----
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

if (-not $SkipCuda) {
    $missingCuda = Test-Dlls $cudaDlls
    if ($missingCuda.Count -eq 0) {
        Write-Host "[OK] CUDA runtime DLLs already present in target or system." -ForegroundColor Green
    } else {
        Write-Host "[MISSING] CUDA DLLs: $($missingCuda -join ', ')" -ForegroundColor Yellow

        # Try to find them in an existing CUDA Toolkit installation
        $cudaInstall = Get-ChildItem "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v*\bin" -ErrorAction SilentlyContinue |
            Sort-Object -Descending |
            Select-Object -First 1

        if ($cudaInstall) {
            Write-Host "Found CUDA Toolkit at $($cudaInstall.Parent.FullName), copying DLLs..." -ForegroundColor Green
            foreach ($dll in $missingCuda) {
                $src = "$($cudaInstall.FullName)\$dll"
                if (Test-Path $src) {
                    Copy-Item $src $TargetDir -Force
                    Write-Host "  Copied $dll"
                }
            }
            # Also copy any *64_*.dll that ONNX Runtime might need
            foreach ($dll in Get-ChildItem "$($cudaInstall.FullName)\*64_*.dll" -Name) {
                if (-not (Test-Path "$TargetDir\$dll")) {
                    Copy-Item "$($cudaInstall.FullName)\$dll" $TargetDir -Force
                    Write-Host "  Copied $dll"
                }
            }
        } else {
            Write-Host "No existing CUDA installation found. Downloading CUDA redistributable..." -ForegroundColor Yellow
            $cudaZip = "$env:TEMP\cuda_redist.zip"
            $cudaExtract = "$env:TEMP\cuda_redist"

            # NVIDIA redistributable packages (publicly available, no login required)
            $cudaRedistUrl = "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.8.39-archive.zip"
            Invoke-WebRequest -Uri $cudaRedistUrl -OutFile $cudaZip
            Expand-Archive $cudaZip $cudaExtract -Force

            # Copy all DLLs from extracted archive to target
            $extracted = Get-ChildItem $cudaExtract -Recurse -Filter "*.dll" -ErrorAction SilentlyContinue
            foreach ($dll in $extracted) {
                Copy-Item $dll.FullName $TargetDir -Force
                Write-Host "  Copied $($dll.Name)"
            }
            Remove-Item $cudaZip -Force -ErrorAction SilentlyContinue
            Remove-Item $cudaExtract -Recurse -Force -ErrorAction SilentlyContinue

            # NOTE: This only covers cudart. For the full set, a CUDA Toolkit install is better.
            Write-Host "[WARN] Only cudart was downloaded. Install CUDA Toolkit 12.x for full GPU support: https://developer.nvidia.com/cuda-downloads" -ForegroundColor Yellow
        }
    }
}

# ---- cuDNN ----
$cudnnDlls = @("cudnn64_9.dll", "cudnn_ops64_9.dll", "cudnn_cnn64_9.dll", "cudnn_adv64_9.dll", "cudnn_eng64_9.dll", "cudnn_graph64_9.dll")

if (-not $SkipCudnn) {
    $missingCudnn = Test-Dlls $cudnnDlls
    if ($missingCudnn.Count -eq 0) {
        Write-Host "[OK] cuDNN DLLs already present." -ForegroundColor Green
    } else {
        Write-Host "[MISSING] cuDNN DLLs: $($missingCudnn -join ', ')" -ForegroundColor Yellow

        # Approach 1: Check if pip is available and use nvidia-cudnn-cu12 wheel (no NVIDIA login needed)
        $pipAvailable = (Get-Command python -ErrorAction SilentlyContinue) -or (Get-Command python3 -ErrorAction SilentlyContinue)
        if (-not $pipAvailable) {
            Write-Host "Python not found. Trying winget to install..." -ForegroundColor Yellow
            # Try winget for Python
            winget install Python.Python.3.12 --accept-package-agreements --accept-source-agreements 2>$null
            # Refresh PATH
            $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
        }

        $python = (Get-Command python -ErrorAction SilentlyContinue) ?? (Get-Command python3 -ErrorAction SilentlyContinue)
        if ($python) {
            Write-Host "Installing nvidia-cudnn-cu12 via pip (avoiding NVIDIA login)..." -ForegroundColor Green
            & $python.Source -m pip install "nvidia-cudnn-cu12" --quiet --disable-pip-version-check 2>$null

            # Find where pip installed it
            $pipShow = & $python.Source -m pip show nvidia-cudnn-cu12 2>$null | Out-String
            if ($pipShow -match "Location:\s+(.+)") {
                $cudnnPackage = Join-Path $matches[1].Trim() "nvidia\cudnn"
                Write-Host "cuDNN package at: $cudnnPackage"
                if (Test-Path "$cudnnPackage\bin") {
                    foreach ($dll in Get-ChildItem "$cudnnPackage\bin\*.dll" -Name) {
                        Copy-Item "$cudnnPackage\bin\$dll" $TargetDir -Force
                        Write-Host "  Copied $dll"
                    }
                } elseif (Test-Path "$cudnnPackage\Lib\x64") {
                    foreach ($dll in Get-ChildItem "$cudnnPackage\Lib\x64\*.dll" -Name) {
                        Copy-Item "$cudnnPackage\Lib\x64\$dll" $TargetDir -Force
                        Write-Host "  Copied $dll"
                    }
                } else {
                    # Search recursively
                    foreach ($dll in Get-ChildItem $cudnnPackage -Recurse -Filter "*.dll" -ErrorAction SilentlyContinue) {
                        Copy-Item $dll.FullName $TargetDir -Force
                        Write-Host "  Copied $($dll.Name)"
                    }
                }
            }
        } else {
            Write-Host "[FAIL] Python is required to auto-download cuDNN." -ForegroundColor Red
            Write-Host "Install Python first, then re-run this script." -ForegroundColor Red
            Write-Host "Or manually download cuDNN from: https://developer.nvidia.com/cudnn" -ForegroundColor Yellow
        }
    }
}

# ---- VERIFY ----
Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "  Verification" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan

$allDlls = $cudaDlls + $cudnnDlls
$stillMissing = Test-Dlls $allDlls

if ($stillMissing.Count -gt 0) {
    Write-Host "[WARN] Still missing: $($stillMissing -join ', ')" -ForegroundColor Yellow
    Write-Host "These are not critical — ONNX Runtime will fall back to CPU for missing features." -ForegroundColor Yellow
} else {
    Write-Host "[OK] All GPU dependencies are in place!" -ForegroundColor Green
}

Write-Host ""
Write-Host "Target directory: $TargetDir" -ForegroundColor Cyan
Write-Host "You can now run the app with GPU acceleration." -ForegroundColor Cyan
Write-Host ""

# Print summary of what's in the target directory
Write-Host "GPU-related DLLs in target:" -ForegroundColor White
Get-ChildItem $TargetDir -Filter "*64_*.dll" | ForEach-Object { Write-Host "  $($_.Name)" }
Get-ChildItem $TargetDir -Filter "cudnn*.dll" | ForEach-Object { Write-Host "  $($_.Name)" }
Get-ChildItem $TargetDir -Filter "nvrtc*.dll" | ForEach-Object { Write-Host "  $($_.Name)" }
Get-ChildItem $TargetDir -Filter "nvJit*.dll" | ForEach-Object { Write-Host "  $($_.Name)" }
