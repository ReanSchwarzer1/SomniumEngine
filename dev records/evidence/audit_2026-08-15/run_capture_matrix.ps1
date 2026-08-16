$ErrorActionPreference = "Stop"

$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$exe = Join-Path $repo "target\release\hello_engine.exe"
$evidence = $PSScriptRoot

$controlled = @(
    "SOMNIUM_FSR", "SOMNIUM_CAS", "SOMNIUM_BLOOM",
    "SOMNIUM_LIGHT_SHAFTS", "SOMNIUM_VOLUMETRICS",
    "SOMNIUM_RT_REFLECT", "SOMNIUM_RT_REFRACT",
    "SOMNIUM_RESTIR_GI", "SOMNIUM_SPECULAR_GI",
    "SOMNIUM_PROBES", "SOMNIUM_PATH_TRACER",
    "SOMNIUM_TERRAIN_FORCE_RGBA8", "SOMNIUM_CPU_FRUSTUM",
    "SOMNIUM_CASCADE_CULL", "SOMNIUM_SUN_ELEVATION",
    "SOMNIUM_SUN_AZIMUTH", "SOMNIUM_CAPTURE_AFTER_WATER",
    "SOMNIUM_CAPTURE_AFTER_TAA", "SOMNIUM_CAPTURE"
)

function Invoke-AuditCapture {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [int]$Frame = 48,
        [hashtable]$Environment = @{},
        [switch]$WriteHdr
    )

    foreach ($key in $controlled) {
        [Environment]::SetEnvironmentVariable($key, $null, "Process")
    }
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_DISPLAY_PNG", (Join-Path $evidence "$Name.png"), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_AUDIT_LOG", (Join-Path $evidence "$Name.audit.txt"), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_FRAME", $Frame.ToString(), "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE_QUIT", "1", "Process")
    [Environment]::SetEnvironmentVariable("SOMNIUM_MAXIMIZE", "1", "Process")
    if ($WriteHdr) {
        [Environment]::SetEnvironmentVariable("SOMNIUM_CAPTURE", (Join-Path $evidence "$Name.somcap"), "Process")
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, "Process")
    }

    Write-Host "AUDIT-RUN $Name frame=$Frame"
    & $exe
    if ($LASTEXITCODE -ne 0) {
        throw "$Name exited with code $LASTEXITCODE"
    }
}

# One controlled neutral is shared by the bloom, CAS, and lighting-extra A/Bs.
$neutral = @{
    SOMNIUM_FSR = "0"; SOMNIUM_CAS = "0"; SOMNIUM_BLOOM = "0";
    SOMNIUM_RT_REFLECT = "0"; SOMNIUM_RESTIR_GI = "0";
    SOMNIUM_CAPTURE_AFTER_TAA = "1"
}
Invoke-AuditCapture -Name "post_neutral" -Environment $neutral

$bloom = $neutral.Clone(); $bloom.SOMNIUM_BLOOM = "1"
Invoke-AuditCapture -Name "post_bloom_on" -Environment $bloom

$cas = $neutral.Clone(); $cas.SOMNIUM_CAS = "1"
Invoke-AuditCapture -Name "post_cas_on" -Environment $cas

$shaftsOn = $neutral.Clone(); $shaftsOn.SOMNIUM_VOLUMETRICS = "1"; $shaftsOn.SOMNIUM_LIGHT_SHAFTS = "1"; $shaftsOn.SOMNIUM_SUN_ELEVATION = "8"; $shaftsOn.SOMNIUM_SUN_AZIMUTH = "210"
Invoke-AuditCapture -Name "shafts_on" -Frame 64 -Environment $shaftsOn
$shaftsOff = $shaftsOn.Clone(); $shaftsOff.SOMNIUM_LIGHT_SHAFTS = "0"
Invoke-AuditCapture -Name "shafts_off" -Frame 64 -Environment $shaftsOff

$waterOn = $neutral.Clone(); $waterOn.SOMNIUM_RT_REFLECT = "1"; $waterOn.SOMNIUM_CAPTURE_AFTER_WATER = "1"
Invoke-AuditCapture -Name "water_rt_on" -Frame 96 -Environment $waterOn -WriteHdr
$waterOff = $waterOn.Clone(); $waterOff.SOMNIUM_RT_REFLECT = "0"
Invoke-AuditCapture -Name "water_rt_off" -Frame 96 -Environment $waterOff -WriteHdr

$terrainBc = $neutral.Clone()
Invoke-AuditCapture -Name "terrain_bc7" -Environment $terrainBc -WriteHdr
$terrainRgba = $neutral.Clone(); $terrainRgba.SOMNIUM_TERRAIN_FORCE_RGBA8 = "1"
Invoke-AuditCapture -Name "terrain_rgba8" -Environment $terrainRgba -WriteHdr

$crOn = $neutral.Clone()
Invoke-AuditCapture -Name "cr_culling_on" -Environment $crOn -WriteHdr
$crOff = $neutral.Clone(); $crOff.SOMNIUM_CPU_FRUSTUM = "0"; $crOff.SOMNIUM_CASCADE_CULL = "0"
Invoke-AuditCapture -Name "cr_culling_off" -Environment $crOff -WriteHdr

$spec = $neutral.Clone(); $spec.SOMNIUM_SPECULAR_GI = "1"
Invoke-AuditCapture -Name "rt_specular_on" -Frame 64 -Environment $spec
$probes = $neutral.Clone(); $probes.SOMNIUM_PROBES = "1"
Invoke-AuditCapture -Name "probes_on" -Frame 64 -Environment $probes
$path = $neutral.Clone(); $path.SOMNIUM_PATH_TRACER = "1"
Invoke-AuditCapture -Name "path_tracer" -Frame 48 -Environment $path

Write-Host "AUDIT-RUN complete"
