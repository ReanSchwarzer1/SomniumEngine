$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$exe = Join-Path $repo "target\release\hello_engine.exe"

$controlled = @(
    "SOMNIUM_FSR", "SOMNIUM_CAS", "SOMNIUM_BLOOM", "SOMNIUM_RESTIR",
    "SOMNIUM_RESTIR_GI", "SOMNIUM_RT_REFLECT", "SOMNIUM_PATH_TRACER",
    "SOMNIUM_CPU_FRUSTUM", "SOMNIUM_CASCADE_CULL", "SOMNIUM_CAMERA_YAW",
    "SOMNIUM_CAMERA_PITCH", "SOMNIUM_AUDIT_YAW_JUMP_FRAME",
    "SOMNIUM_AUDIT_YAW_JUMP_DEGREES", "SOMNIUM_CAPTURE_AFTER_WATER",
    "SOMNIUM_CAPTURE_AFTER_TAA", "SOMNIUM_KIT_VIEW", "SOMNIUM_VOLUMETRICS",
    "SOMNIUM_LIGHT_SHAFTS", "SOMNIUM_SUN_ELEVATION", "SOMNIUM_SUN_AZIMUTH"
)

function Invoke-Followup {
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
    Write-Host "FOLLOWUP $Name frame=$Frame"
    & $exe
    if ($LASTEXITCODE -ne 0) { throw "$Name exited with $LASTEXITCODE" }
}

$neutral = @{
    SOMNIUM_FSR="0"; SOMNIUM_CAS="0"; SOMNIUM_BLOOM="0";
    SOMNIUM_RESTIR="0"; SOMNIUM_RESTIR_GI="0";
    SOMNIUM_RT_REFLECT="0"; SOMNIUM_CAPTURE_AFTER_TAA="1"
}

foreach ($frame in 96, 97) {
    $waterOn = $neutral.Clone(); $waterOn.SOMNIUM_RT_REFLECT="1"; $waterOn.SOMNIUM_CAPTURE_AFTER_WATER="1"
    Invoke-Followup -Name "water_v2_on_f$frame" -Frame $frame -Environment $waterOn
    $waterOff = $neutral.Clone(); $waterOff.SOMNIUM_CAPTURE_AFTER_WATER="1"
    Invoke-Followup -Name "water_v2_off_f$frame" -Frame $frame -Environment $waterOff
}

$old = $neutral.Clone(); $old.SOMNIUM_PATH_TRACER="1"; $old.SOMNIUM_CAMERA_YAW="-90"
Invoke-Followup -Name "path_old_yaw" -Frame 25 -Environment $old
$new = $neutral.Clone(); $new.SOMNIUM_PATH_TRACER="1"; $new.SOMNIUM_CAMERA_YAW="-70"
Invoke-Followup -Name "path_new_yaw" -Frame 25 -Environment $new
$jump = $old.Clone(); $jump.SOMNIUM_AUDIT_YAW_JUMP_FRAME="24"; $jump.SOMNIUM_AUDIT_YAW_JUMP_DEGREES="20"
Invoke-Followup -Name "path_yaw_jump_immediate" -Frame 25 -Environment $jump
Invoke-Followup -Name "path_yaw_jump_converged" -Frame 72 -Environment $jump

$crOn = $neutral.Clone()
Invoke-Followup -Name "cr_isolated_on" -Frame 64 -Environment $crOn
$crOff = $neutral.Clone(); $crOff.SOMNIUM_CPU_FRUSTUM="0"; $crOff.SOMNIUM_CASCADE_CULL="0"
Invoke-Followup -Name "cr_isolated_off" -Frame 64 -Environment $crOff

$shaft = $neutral.Clone(); $shaft.SOMNIUM_KIT_VIEW="walk"; $shaft.SOMNIUM_CAMERA_YAW="-90"; $shaft.SOMNIUM_CAMERA_PITCH="0"; $shaft.SOMNIUM_SUN_ELEVATION="6"; $shaft.SOMNIUM_SUN_AZIMUTH="270"; $shaft.SOMNIUM_VOLUMETRICS="1"; $shaft.SOMNIUM_LIGHT_SHAFTS="1"
Invoke-Followup -Name "shafts_accept_on" -Frame 72 -Environment $shaft
$shaftOff = $shaft.Clone(); $shaftOff.SOMNIUM_LIGHT_SHAFTS="0"
Invoke-Followup -Name "shafts_accept_off" -Frame 72 -Environment $shaftOff

$fsr = $neutral.Clone(); $fsr.SOMNIUM_FSR="1"; $fsr.Remove("SOMNIUM_CAPTURE_AFTER_TAA")
Invoke-Followup -Name "fsr_on" -Frame 72 -Environment $fsr

Write-Host "FOLLOWUP complete"
