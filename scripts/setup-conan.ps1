# ==============================================================================
# DoLogger — Conan Setup Script for Windows (PowerShell 7+)
# ==============================================================================
# Usage:
#   pwsh scripts/setup-conan.ps1
#   pwsh scripts/setup-conan.ps1 -DryRun
#   pwsh scripts/setup-conan.ps1 -Detect
# ==============================================================================
param(
    [switch]$DryRun,
    [switch]$Detect,
    [string]$Profile = ""
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$ProfilesDir = Join-Path $ProjectDir ".conan\profiles"
$BuildDir = if ($env:BUILD_DIR) { $env:BUILD_DIR } else { Join-Path $ProjectDir "build" }

function Write-Banner {
    Write-Host ""
    Write-Host "=============================================" -ForegroundColor Cyan
    Write-Host " DoLogger — Conan Dependency Setup (Windows)" -ForegroundColor Cyan
    Write-Host "=============================================" -ForegroundColor Cyan
    Write-Host ""
}

function Get-DetectedProfile {
    $arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { "x86" }
    return "windows-msvc-$arch"
}

function Test-ConanInstalled {
    $conan = Get-Command conan -ErrorAction SilentlyContinue
    if (-not $conan) {
        Write-Host "[ERROR] Conan not found in PATH." -ForegroundColor Red
        Write-Host ""
        Write-Host "Install Conan 2.x:"
        Write-Host "  pip install conan"
        Write-Host "  pipx install conan    (recommended)"
        Write-Host ""
        Write-Host "Then run: conan profile detect"
        exit 1
    }
    $ver = (conan --version 2>&1 | Select-Object -First 1) -replace '.*?(\d+\.\d+).*', '$1'
    $major = [int]($ver -split '\.')[0]
    if ($major -lt 2) {
        Write-Host "[ERROR] Conan 2.x required (found $ver)." -ForegroundColor Red
        Write-Host "Upgrade: pip install --upgrade conan"
        exit 1
    }
    Write-Host "  Conan version: $ver" -ForegroundColor Green
}

# --- Main ---
Write-Banner

$ProfileName = if ($Profile) { $Profile } else { Get-DetectedProfile }

if ($Detect) {
    Write-Output $ProfileName
    exit 0
}

$ProfilePath = Join-Path $ProfilesDir $ProfileName

if (-not (Test-Path $ProfilePath)) {
    Write-Host "[ERROR] Profile not found: $ProfilePath" -ForegroundColor Red
    Write-Host ""
    Write-Host "Available profiles:"
    Get-ChildItem $ProfilesDir -File | ForEach-Object { Write-Host "  $($_.Name)" }
    Write-Host ""
    exit 1
}

Write-Host "  Platform detected: $(Get-DetectedProfile)"
Write-Host "  Selected profile:  $ProfileName"
Write-Host "  Build directory:   $BuildDir"
Write-Host "  Profile path:      $ProfilePath"
Write-Host ""

Test-ConanInstalled

if ($DryRun) {
    Write-Host "[DRY-RUN] Would execute:" -ForegroundColor Yellow
    Write-Host "  conan install `"$ProjectDir`" --output-folder=`"$BuildDir`" --profile:host=`"$ProfilePath`" --profile:build=`"$ProfilePath`" --build=missing"
    Write-Host ""
    Write-Host "[DRY-RUN] Then for CMake:" -ForegroundColor Yellow
    Write-Host "  cmake -B `"$BuildDir`" -DCMAKE_TOOLCHAIN_FILE=`"$BuildDir\conan_toolchain.cmake`" -DCMAKE_BUILD_TYPE=Release"
    Write-Host ""
    exit 0
}

# --- Detect default Conan profile ---
Write-Host "[1/3] Detecting default Conan profile..." -ForegroundColor Cyan
$hasDefault = conan profile show default 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "  No default profile found — running 'conan profile detect'..."
    conan profile detect
}
Write-Host ""

# --- Install dependencies ---
Write-Host "[2/3] Installing C dependencies via Conan..." -ForegroundColor Cyan
Write-Host "  (This may take several minutes on first run — libraries are cached after)"
Write-Host ""

conan install $ProjectDir `
    --output-folder="$BuildDir" `
    --profile:host="$ProfilePath" `
    --profile:build="$ProfilePath" `
    --build=missing

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "[ERROR] Conan install failed." -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host ""
Write-Host "  Dependencies installed successfully." -ForegroundColor Green
Write-Host ""

# --- Post-install summary ---
Write-Host "[3/3] Build instructions" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Next steps:"
Write-Host ""
Write-Host "  # Build Rust core + CLI only" -ForegroundColor Green
Write-Host "  cargo build --release"
Write-Host ""
Write-Host "  # Build C/C++ plugins (uses Conan toolchain)" -ForegroundColor Green
Write-Host "  cmake -B `"$BuildDir`" -DCMAKE_TOOLCHAIN_FILE=`"$BuildDir\conan_toolchain.cmake`" -DCMAKE_BUILD_TYPE=Release"
Write-Host "  cmake --build `"$BuildDir`" --target dologger_plugins"
Write-Host ""
Write-Host "  # Build everything (Rust + C/C++ + Go)" -ForegroundColor Green
Write-Host "  pwsh scripts/build-all.ps1"
Write-Host ""
Write-Host "  # Quick: build all plugins only" -ForegroundColor Green
Write-Host "  pwsh scripts/build-plugins.ps1"
Write-Host ""
Write-Host "=============================================" -ForegroundColor Cyan
Write-Host "  Conan setup complete!" -ForegroundColor Green
Write-Host "=============================================" -ForegroundColor Cyan
Write-Host ""
