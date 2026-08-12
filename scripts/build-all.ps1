# ==============================================================================
# DoLogger — Full Project Build (Windows PowerShell)
# ==============================================================================
param(
    [switch]$Release,
    [switch]$CoreOnly,
    [switch]$RunTests
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildFlag = if ($Release) { "--release" } else { "" }
$BuildType = if ($Release) { "release" } else { "debug" }

Push-Location $ProjectDir
try {
    Write-Host ""
    Write-Host "╔══════════════════════════════════════════════════╗" -ForegroundColor Cyan
    Write-Host "║     DoLogger — Full Project Build (Windows)     ║" -ForegroundColor Cyan
    Write-Host "╠══════════════════════════════════════════════════╣" -ForegroundColor Cyan
    Write-Host ("║  Build type: {0,-36} ║" -f $BuildType) -ForegroundColor Cyan
    Write-Host ("║  Target:     {0,-36} ║" -f $(if ($CoreOnly) { "Rust only" } else { "Rust + plugins" })) -ForegroundColor Cyan
    Write-Host "╚══════════════════════════════════════════════════╝" -ForegroundColor Cyan
    Write-Host ""

    $step = 0
    $total = if ($CoreOnly) { 2 } else { 3 }
    if ($RunTests) { $total++ }

    # --- Step 1: Check prerequisites ---
    $step++
    Write-Host "[$step/$total] Checking prerequisites..." -ForegroundColor Green
    Write-Host "  Rust:  $(rustc --version)"
    Write-Host "  Cargo: $(cargo --version)"
    Write-Host "  CMake: $(cmake --version | Select-Object -First 1)"

    # --- Step 2: Rust core + CLI ---
    $step++
    Write-Host "[$step/$total] Building Rust core + CLI..." -ForegroundColor Green
    cargo build $BuildFlag
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed" }

    # --- Step 3: Non-Rust plugins ---
    if (-not $CoreOnly) {
        $step++
        Write-Host "[$step/$total] Building non-Rust plugins..." -ForegroundColor Green
        $pluginArgs = @{}
        if ($Release) { $pluginArgs.BuildType = "Release" }
        & pwsh -File "$ScriptDir\build-plugins.ps1" @pluginArgs
    }

    # --- Step 4: Tests ---
    if ($RunTests) {
        $step++
        Write-Host "[$step/$total] Running all tests..." -ForegroundColor Green
        cargo test $BuildFlag
    }

    Write-Host ""
    Write-Host "╔══════════════════════════════════════════════════╗" -ForegroundColor Cyan
    Write-Host "║  Build complete!                                ║" -ForegroundColor Cyan
    Write-Host "╠══════════════════════════════════════════════════╣" -ForegroundColor Cyan
    Write-Host ("║  Core:   target/{0}/dologger_core.dll           ║" -f $BuildType) -ForegroundColor Cyan
    Write-Host ("║  CLI:    target/{0}/dologctl.exe                ║" -f $BuildType) -ForegroundColor Cyan
    Write-Host "╚══════════════════════════════════════════════════╝" -ForegroundColor Cyan
    Write-Host ""
} finally {
    Pop-Location
}
