# check-smoke.ps1 — release artifact smoke test for Windows.
#
# Verifies that the built release artifacts actually run:
#   1. dologctl.exe starts and reports its version
#   2. dologger_core.dll loads and dologger_version() returns a string
#      (pure PowerShell P/Invoke probe)
#   3. a foreign-language (Python ctypes) host can drive the full C ABI
#      lifecycle — init, log, config, shutdown
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tests/smoke/check-smoke.ps1 [-ArtifactDir <dir>]
param(
    [string]$ArtifactDir = ""
)

$ErrorActionPreference = "Stop"
$script:Failures = 0

function Note($msg)  { Write-Host ""; Write-Host "== $msg" -ForegroundColor Cyan }
function Pass($msg)  { Write-Host "  [PASS] $msg" -ForegroundColor Green }
function Fail($msg)  { Write-Host "  [FAIL] $msg" -ForegroundColor Red; $script:Failures++ }

# ── Locate artifacts ────────────────────────────────────────────────
if (-not $ArtifactDir) {
    if (Test-Path "release-artifacts") { $ArtifactDir = "release-artifacts" }
    else { $ArtifactDir = "target\release" }
}
Note "Artifact directory: $ArtifactDir"

$Exe = Join-Path $ArtifactDir "dologctl-windows-x86_64.exe"
if (-not (Test-Path $Exe)) { $Exe = Join-Path $ArtifactDir "dologctl.exe" }
$Dll = Join-Path $ArtifactDir "dologger_core.dll"

# ── 1. CLI executable runs ──────────────────────────────────────────
Note "1. CLI executable"
if (Test-Path $Exe) {
    $out = & $Exe version 2>&1
    if ($LASTEXITCODE -eq 0 -and ($out -join "`n") -match "dologctl") {
        Pass "dologctl.exe version ran (exit $LASTEXITCODE)"
        ($out | Select-Object -First 4) | ForEach-Object { Write-Host "       $_" }
    } else {
        Fail "dologctl.exe version failed (exit $LASTEXITCODE)"
    }
} else {
    Fail "dologctl.exe not found (looked in $ArtifactDir)"
}

# ── 2. DLL loads + dologger_version() via P/Invoke ──────────────────
Note "2. dologger_core.dll via P/Invoke"
if (Test-Path $Dll) {
    try {
        $dllAbs = (Resolve-Path $Dll).Path
        # Escape backslashes for the C# string literal
        $dllCs = $dllAbs.Replace('\', '\\')
        $source = @"
using System;
using System.Runtime.InteropServices;
public static class DologgerProbe {
    [DllImport("$dllCs", CallingConvention = CallingConvention.Cdecl)]
    public static extern IntPtr dologger_version();
}
"@
        Add-Type -TypeDefinition $source
        $ptr = [DologgerProbe]::dologger_version()
        if ($ptr -ne [IntPtr]::Zero) {
            $version = [Runtime.InteropServices.Marshal]::PtrToStringAnsi($ptr)
            if ($version -match "^\d+\.\d+\.\d+") {
                Pass "dologger_version() = $version"
            } else {
                Fail "unexpected version string: '$version'"
            }
        } else {
            Fail "dologger_version() returned NULL"
        }
    } catch {
        Fail "P/Invoke probe failed: $($_.Exception.Message)"
    }
} else {
    Fail "dologger_core.dll not found (looked in $ArtifactDir)"
}

# ── 3. Foreign-language C ABI lifecycle (Python ctypes) ─────────────
Note "3. C ABI via Python ctypes"
if (Test-Path $Dll) {
    $py = (Get-Command python -ErrorAction SilentlyContinue)
    if (-not $py) { $py = (Get-Command python3 -ErrorAction SilentlyContinue) }
    if (-not $py) { $py = (Get-Command py -ErrorAction SilentlyContinue) }
    if ($py) {
        $script = Join-Path $PSScriptRoot "c_abi_smoke.py"
        & $py.Source $script $Dll
        if ($LASTEXITCODE -eq 0) {
            Pass "full C ABI lifecycle via ctypes"
        } else {
            Fail "C ABI lifecycle via ctypes"
        }
    } else {
        Fail "python not found — C ABI cross-language check skipped"
    }
}

# ── Summary ─────────────────────────────────────────────────────────
Write-Host ""
if ($script:Failures -eq 0) {
    Write-Host "SMOKE TEST: ALL PASSED"
    exit 0
} else {
    Write-Host "SMOKE TEST: $($script:Failures) FAILURE(S)"
    exit 1
}
