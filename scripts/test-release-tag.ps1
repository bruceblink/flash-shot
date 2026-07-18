$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$script = Join-Path $PSScriptRoot "verify-release-tag.ps1"
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}
$versionParts = $package.version.Split('.')
$mismatchedPatch = ([int]$versionParts[2]) + 1
$mismatchedVersion = "$($versionParts[0]).$($versionParts[1]).$mismatchedPatch"

& $script -Tag "v$($package.version)"
if ($LASTEXITCODE -ne 0) {
    throw "Matching release tag was rejected."
}

$failed = $false
try {
    & $script -Tag "v$mismatchedVersion"
    $failed = $LASTEXITCODE -ne 0
}
catch {
    $failed = $true
}
if (-not $failed) {
    throw "Version-mismatched release tag was accepted."
}

$failed = $false
try {
    & $script -Tag "release-$($package.version)"
    $failed = $LASTEXITCODE -ne 0
}
catch {
    $failed = $true
}
if (-not $failed) {
    throw "Malformed release tag was accepted."
}
