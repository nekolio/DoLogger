# ==============================================================================
# DoLogger — Build All Non-Rust Plugins (Windows PowerShell)
# ==============================================================================
param(
    [ValidateSet("Debug", "Release")]
    [string]$BuildType = "Debug",
    [ValidateSet("all", "c", "cpp", "go")]
    [string]$Filter = "all"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = if ($env:BUILD_DIR) { $env:BUILD_DIR } else { Join-Path $ProjectDir "build" }

Write-Host ""
Write-Host "=============================================" -ForegroundColor Cyan
Write-Host " DoLogger — Plugin Build (Windows)" -ForegroundColor Cyan
Write-Host "=============================================" -ForegroundColor Cyan
Write-Host "  Build type: $BuildType"
Write-Host "  Build dir:  $BuildDir"
Write-Host ""

# ---------------------------------------------------------------------------
# C/C++ Plugins
# ---------------------------------------------------------------------------
function Build-CPlugins {
    Write-Host "[C/C++ Plugins]" -ForegroundColor White
    $count = 0

    $cmakeArgs = @("-DCMAKE_BUILD_TYPE=$BuildType")
    $toolchain = Join-Path $BuildDir "conan_toolchain.cmake"
    if (Test-Path $toolchain) {
        $cmakeArgs += "-DCMAKE_TOOLCHAIN_FILE=$toolchain"
        Write-Host "  Using Conan toolchain: $toolchain"
    } else {
        Write-Host "  No Conan toolchain found — run 'pwsh scripts/setup-conan.ps1' first for C deps" -ForegroundColor Yellow
    }

    $cmakeFiles = Get-ChildItem -Path "$ProjectDir\plugins" -Recurse -Filter "CMakeLists.txt" -ErrorAction SilentlyContinue
    foreach ($cmf in $cmakeFiles) {
        $pluginDir = $cmf.DirectoryName
        $pluginName = $cmf.Directory.Name
        $pluginBuildDir = Join-Path $BuildDir "plugins\$pluginName"

        Write-Host "  → $pluginName ($pluginDir)" -ForegroundColor Green
        & cmake -B $pluginBuildDir -S $pluginDir @cmakeArgs 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host "    CMake configure failed for $pluginName" -ForegroundColor Red
            continue
        }
        & cmake --build $pluginBuildDir --config $BuildType
        $count++
    }

    Write-Host ""
    Write-Host "  Built $count C/C++ plugin(s)" -ForegroundColor Green
}

# ---------------------------------------------------------------------------
# Go Plugins
# ---------------------------------------------------------------------------
function Build-GoPlugins {
    Write-Host "[Go Plugins]" -ForegroundColor White
    $count = 0

    $goCmd = Get-Command go -ErrorAction SilentlyContinue
    if (-not $goCmd) {
        Write-Host "  Go not found — skipping Go plugins" -ForegroundColor Yellow
        return
    }
    Write-Host "  Go version: $(go version)"

    $goMods = Get-ChildItem -Path "$ProjectDir\plugins" -Recurse -Filter "go.mod" -ErrorAction SilentlyContinue
    foreach ($gm in $goMods) {
        $pluginDir = $gm.DirectoryName
        $pluginName = $gm.Directory.Name
        $goFiles = Get-ChildItem -Path $pluginDir -Filter "*.go" -ErrorAction SilentlyContinue
        $isCgo = $false
        foreach ($gf in $goFiles) {
            if ((Get-Content $gf.FullName -Raw) -match 'import "C"') {
                $isCgo = $true
                break
            }
        }
        if (-not $isCgo) { continue }

        $outName = "dologger-plugin-$pluginName.dll"
        Write-Host "  → $pluginName → $outName" -ForegroundColor Green

        $env:CGO_ENABLED = "1"
        Push-Location $pluginDir
        try {
            & go build -buildmode=c-shared -o $outName . 2>&1
        } finally {
            Pop-Location
        }
        $count++
    }

    Write-Host ""
    Write-Host "  Built $count Go plugin(s)" -ForegroundColor Green
}

# --- Main ---
if ($Filter -in @("all", "c", "cpp")) {
    Build-CPlugins
}
if ($Filter -in @("all", "go")) {
    Build-GoPlugins
}

Write-Host "=============================================" -ForegroundColor Cyan
Write-Host "  Plugin build complete!" -ForegroundColor Green
Write-Host "=============================================" -ForegroundColor Cyan
Write-Host ""
