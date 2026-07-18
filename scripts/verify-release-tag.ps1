param(
    [Parameter(Mandatory = $true)]
    [string]$Tag
)

$ErrorActionPreference = "Stop"

if ($Tag -notmatch '^v(?<version>\d+\.\d+\.\d+)$') {
    throw "Release tag must use v<major>.<minor>.<patch>; received '$Tag'."
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}

if ($Matches.version -ne $package.version) {
    throw "Release tag $Tag does not match Cargo version $($package.version)."
}

Write-Host "Verified release tag $Tag for Flash Shot $($package.version)."
