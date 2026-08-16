$ErrorActionPreference = "Stop"

$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$exe = Join-Path $repo "target\release\hello_engine.exe"

$controlled = @(
    "SOMNIUM_FSR", "SOMNIUM_CAS", "SOMNIUM_BLOOM", "SOMNIUM_RESTIR",
    "SOMNIUM_RESTIR_GI", "SOMNIUM_RT_REFLECT", "SOMNIUM_RT_REFRACT",
    "SOMNIUM_PATH_TRACER", "SOMNIUM_VOLUMETRICS", "SOMNIUM_LIGHT_SHAFTS",
    "SOMNIUM_SUN_ELEVATION", "SOMNIUM_SUN_AZIMUTH", "SOMNIUM_CAPTURE_AFTER_WATER",
    "SOMNIUM_CAPTURE_AFTER_TAA", "SOMNIUM_KIT_VIEW", "SOMNIUM_CAMERA_POS",
    "SOMNIUM_CAMERA_YAW", "SOMNIUM_CAMERA_PITCH"
)

function Invoke-Repro {
    param([string]$Name, [int]$Frame, [hashtable]$Environment)
    foreach ($key in $controlled) {
        [Environment]::SetEnvironmentVariable($key, $null, "Process")
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, "Process")
    }
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_DISPLAY_PNG", (Join-Path $PSScriptRoot "$Name.png"), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_AUDIT_LOG", (Join-Path $PSScriptRoot "$Name.audit.txt"), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_FRAME", $Frame.ToString(), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_QUIT", "1", "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_MAXIMIZE", "1", "Process")
    Write-Host "REPRO $Name frame=$Frame"
    & $exe
    if ($LASTEXITCODE -ne 0) { throw "$Name exited with $LASTEXITCODE" }
}

$neutral = @{
    SOMNIUM_FSR="0"; SOMNIUM_CAS="0"; SOMNIUM_BLOOM="0";
    SOMNIUM_RESTIR="0"; SOMNIUM_RESTIR_GI="0";
    SOMNIUM_RT_REFLECT="0"; SOMNIUM_VOLUMETRICS="0";
    SOMNIUM_CAPTURE_AFTER_TAA="1"
}

foreach ($frame in 2, 16, 64) {
    $path = $neutral.Clone(); $path.SOMNIUM_PATH_TRACER="1"
    Invoke-Repro -Name "after_path_f$frame" -Frame $frame -Environment $path
}

$waterOn = $neutral.Clone(); $waterOn.SOMNIUM_RT_REFLECT="1"; $waterOn.SOMNIUM_CAPTURE_AFTER_WATER="1"
Invoke-Repro -Name "after_water_rt_on" -Frame 64 -Environment $waterOn
$waterOff = $neutral.Clone(); $waterOff.SOMNIUM_CAPTURE_AFTER_WATER="1"
Invoke-Repro -Name "after_water_rt_off" -Frame 64 -Environment $waterOff

$nightOn = $neutral.Clone(); $nightOn.SOMNIUM_FSR="1"; $nightOn.SOMNIUM_VOLUMETRICS="1"; $nightOn.SOMNIUM_SUN_ELEVATION="-18"; $nightOn.Remove("SOMNIUM_CAPTURE_AFTER_TAA")
Invoke-Repro -Name "after_night_fsr_on" -Frame 64 -Environment $nightOn
$nightOff = $nightOn.Clone(); $nightOff.SOMNIUM_FSR="0"; $nightOff.SOMNIUM_CAPTURE_AFTER_TAA="1"
Invoke-Repro -Name "after_night_fsr_off" -Frame 64 -Environment $nightOff

Write-Host "REPRO complete"
