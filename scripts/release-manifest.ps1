param(
    [string]$AssetDirectory = "dist",
    [string]$OutputPath = "",
    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"

function Read-Checksum([string]$Path, [string]$ExpectedName) {
    $content = (Get-Content -Raw -LiteralPath $Path).Trim()
    if ($content -notmatch '^(?<hash>[0-9a-fA-F]{64})  (?<name>[^\r\n]+)$') {
        throw "Checksum file has an invalid SHA-256 format: $Path"
    }
    if ($Matches.name -ne $ExpectedName) {
        throw "Checksum file $Path names '$($Matches.name)', expected '$ExpectedName'."
    }
    return $Matches.hash.ToLowerInvariant()
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}
$assetRoot = [IO.Path]::GetFullPath((Join-Path $root $AssetDirectory))
if (-not (Test-Path -LiteralPath $assetRoot -PathType Container)) {
    throw "Release asset directory does not exist: $assetRoot"
}
$assets = Get-ChildItem -LiteralPath $assetRoot -File |
    Where-Object { $_.Extension -in ".zip", ".exe" } |
    Sort-Object Name
if ($assets.Count -eq 0) {
    throw "No .zip or .exe release assets found in $assetRoot."
}

$records = foreach ($asset in $assets) {
    $version = [regex]::Escape($package.version)
    $portablePattern = "^FlashShot-$version-windows-[A-Za-z0-9_]+\.zip$"
    $installerPattern = "^FlashShot-$version-windows-setup\.exe$"
    if ($asset.Name -notmatch $portablePattern -and $asset.Name -notmatch $installerPattern) {
        throw "Release asset does not use a supported Flash Shot artifact name: $($asset.Name)"
    }
    $checksumPath = "$($asset.FullName).sha256"
    if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) {
        throw "Missing SHA-256 file for $($asset.Name)."
    }
    $expected = Read-Checksum $checksumPath $asset.Name
    $actual = (Get-FileHash -LiteralPath $asset.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "SHA-256 mismatch for $($asset.Name)."
    }
    [ordered]@{
        name = $asset.Name
        sha256 = $actual
        size_bytes = $asset.Length
    }
}

$manifest = [ordered]@{
    schema_version = 1
    product = "Flash Shot"
    version = $package.version
    platform = "windows"
    assets = @($records)
}
if ($OutputPath.Length -eq 0) {
    $OutputPath = Join-Path $assetRoot "release-manifest.json"
}
$output = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputPath))
}

if ($VerifyOnly) {
    if (-not (Test-Path -LiteralPath $output -PathType Leaf)) {
        throw "Release manifest does not exist: $output"
    }
    $existing = Get-Content -Raw -LiteralPath $output | ConvertFrom-Json
    $expectedJson = $manifest | ConvertTo-Json -Depth 4
    $existingJson = $existing | ConvertTo-Json -Depth 4
    if ($existingJson -ne $expectedJson) {
        throw "Release manifest does not match the verified asset set."
    }
    Write-Host "Verified release manifest $output"
    return
}

$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $output -Encoding ascii
Write-Host "Created release manifest $output"
