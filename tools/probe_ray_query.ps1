# Launch the editor once with hardware ray tracing forced on, under a hard
# memory ceiling.
#
#   ./tools/probe_ray_query.ps1 [-CapMb 3072] [-TimeoutSeconds 45]
#
# # Why this script exists
#
# `ray_query_compiler_is_safe` disables ray queries on NVIDIA + Vulkan because
# that driver's shader compiler consumed more than 47 GB compiling one of
# Somnium's ray-query pipelines and never finished. The important word is
# *never*: it does not crash, it does not error, it allocates until the machine
# is unusable. Testing a fix for it by simply launching the editor has already
# cost this project two incidents.
#
# So the probe owns the process. It polls private bytes, kills at the ceiling,
# and reports the peak either way. A failed attempt costs a few seconds and a
# few gigabytes instead of the desktop.
#
# Verdicts:
#   exited            the process ended on its own inside the window
#   held-under-cap    still running at the deadline, never crossed the ceiling
#                     -- this is success: it got past pipeline compilation
#   runaway-over-cap  the compile is still unbounded; the fix did not work
[CmdletBinding()]
param(
    [int]$CapMb = 3072,
    [int]$TimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target/release/hello_engine.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    throw "No release binary at $exe. Build it first: cargo build --release -p hello_engine"
}

# Refuse to add to an existing pile. The original incident was several engines
# alive at once, and a probe that quietly became the fifth would be worse than
# no probe.
$existing = Get-Process -Name 'hello_engine' -ErrorAction SilentlyContinue
if ($existing) {
    throw "hello_engine is already running (PID $($existing.Id -join ', ')). Close it first."
}

$logDir = Join-Path $repo '.probe'
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
$out = Join-Path $logDir 'ray-query.stdout.log'
$err = Join-Path $logDir 'ray-query.stderr.log'
Remove-Item -LiteralPath $out, $err -Force -ErrorAction SilentlyContinue

$env:SOMNIUM_FORCE_RAY_QUERY = '1'
Write-Host "Forcing ray queries on, cap ${CapMb} MB, deadline ${TimeoutSeconds}s..."

$proc = Start-Process -FilePath $exe -WorkingDirectory $repo `
    -RedirectStandardOutput $out -RedirectStandardError $err -PassThru

$peak = 0.0
$verdict = 'held-under-cap'
$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
try {
    while (-not $proc.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
        $proc.Refresh()
        if ($proc.HasExited) { break }
        $mb = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
        if ($mb -gt $peak) { $peak = $mb }
        if ($mb -ge $CapMb) {
            $verdict = 'runaway-over-cap'
            break
        }
    }
} finally {
    # Always. An early failure in the loop must not leave a forced-ray-query
    # process running unattended -- that is the exact thing being guarded here.
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    $proc.WaitForExit()
    Remove-Item Env:SOMNIUM_FORCE_RAY_QUERY -ErrorAction SilentlyContinue
}
if ($proc.HasExited -and $verdict -eq 'held-under-cap' -and $peak -lt $CapMb) {
    if ($proc.ExitCode -ne 0) { $verdict = 'exited-with-error' }
}

Write-Host ""
Write-Host "verdict=$verdict peak_private_mb=$peak cap_mb=$CapMb"
Write-Host ""
Write-Host '--- stdout (tail) ---'
if (Test-Path -LiteralPath $out) { Get-Content -LiteralPath $out -Tail 30 }
Write-Host '--- stderr (tail) ---'
if (Test-Path -LiteralPath $err) { Get-Content -LiteralPath $err -Tail 30 }
