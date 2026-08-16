$ErrorActionPreference = "Stop"

$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$exe = Join-Path $repo "target\release\hello_engine.exe"

function Invoke-NightFsr {
    param([string]$Name, [string]$Fsr, [string]$Preset, [string]$Elevation = "-18")
    $values = @{
        SOMNIUM_FSR=$Fsr; SOMNIUM_CAS="0"; SOMNIUM_BLOOM="0";
        SOMNIUM_RESTIR="0"; SOMNIUM_RESTIR_GI="0"; SOMNIUM_RT_REFLECT="0";
        SOMNIUM_VOLUMETRICS="1"; SOMNIUM_SUN_ELEVATION=$Elevation;
        SOMNIUM_VIEWPORT_RES=$Preset; SOMNIUM_CAPTURE_FRAME="64";
        SOMNIUM_CAPTURE_QUIT="1"; SOMNIUM_MAXIMIZE="1";
        SOMNIUM_CAPTURE_DISPLAY_PNG=(Join-Path $PSScriptRoot "$Name.png");
        SOMNIUM_AUDIT_LOG=(Join-Path $PSScriptRoot "$Name.audit.txt")
    }
    foreach ($entry in $values.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, "Process")
    }
    Write-Host "FSR-CONTRACT $Name preset=$Preset fsr=$Fsr"
    & $exe
    if ($LASTEXITCODE -ne 0) { throw "$Name exited with $LASTEXITCODE" }
}

Invoke-NightFsr -Name "after_fsr_fallback_native_off" -Fsr "0" -Preset "0"
Invoke-NightFsr -Name "after_fsr_fallback_native_on" -Fsr "1" -Preset "0"
Invoke-NightFsr -Name "after_fsr_fallback_900p_off" -Fsr "0" -Preset "3"
Invoke-NightFsr -Name "after_fsr_fallback_900p_on" -Fsr "1" -Preset "3"
Invoke-NightFsr -Name "after_fsr_day_off" -Fsr "0" -Preset "3" -Elevation "35"
Invoke-NightFsr -Name "after_fsr_day_on" -Fsr "1" -Preset "3" -Elevation "35"

Write-Host "FSR-CONTRACT complete"
