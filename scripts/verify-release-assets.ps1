param(
    [Parameter(Mandatory = $true)]
    [string]$AssetDirectory,
    [switch]$SkipStartupSmoke
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

function Test-AssetName([string]$Name, [string]$Version) {
    $escapedVersion = [regex]::Escape($Version)
    return $Name -match "^FlashShot-$escapedVersion-windows-[A-Za-z0-9_]+\.zip$" -or
        $Name -match "^FlashShot-$escapedVersion-windows-setup\.exe$"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}

$assetRoot = if ([IO.Path]::IsPathRooted($AssetDirectory)) {
    [IO.Path]::GetFullPath($AssetDirectory)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $AssetDirectory))
}
if (-not (Test-Path -LiteralPath $assetRoot -PathType Container)) {
    throw "Release asset directory does not exist: $assetRoot"
}
$manifestPath = Join-Path $assetRoot "release-manifest.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Release manifest does not exist: $manifestPath"
}
$manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
if ($manifest.schema_version -ne 1 -or $manifest.product -ne "Flash Shot" -or
    $manifest.platform -ne "windows" -or $manifest.version -ne $package.version) {
    throw "Release manifest does not describe Flash Shot $($package.version) for Windows."
}
$records = @($manifest.assets)
if ($records.Count -eq 0) {
    throw "Release manifest has no assets."
}

$names = @{}
foreach ($record in $records) {
    if ($record.name -isnot [string] -or -not (Test-AssetName $record.name $package.version)) {
        throw "Release manifest contains an unsupported asset name."
    }
    if ($names.ContainsKey($record.name)) {
        throw "Release manifest contains duplicate asset $($record.name)."
    }
    $names[$record.name] = $true
    if ($record.sha256 -isnot [string] -or $record.sha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Release manifest contains an invalid SHA-256 for $($record.name)."
    }
    $sizeBytes = 0L
    if (-not [Int64]::TryParse([string]$record.size_bytes, [ref]$sizeBytes) -or $sizeBytes -le 0) {
        throw "Release manifest contains an invalid size for $($record.name)."
    }

    $asset = Join-Path $assetRoot $record.name
    $checksum = "$asset.sha256"
    if (-not (Test-Path -LiteralPath $asset -PathType Leaf) -or -not (Test-Path -LiteralPath $checksum -PathType Leaf)) {
        throw "Downloaded release is missing $($record.name) or its SHA-256 sidecar."
    }
    $file = Get-Item -LiteralPath $asset
    if ($file.Length -ne $sizeBytes) {
        throw "Downloaded release asset size does not match its manifest: $($record.name)."
    }
    $actualHash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $record.sha256.ToLowerInvariant()) {
        throw "Downloaded release asset SHA-256 does not match its manifest: $($record.name)."
    }
    if ((Read-Checksum $checksum $record.name) -ne $actualHash) {
        throw "Downloaded release asset SHA-256 sidecar does not match: $($record.name)."
    }
}

$downloadedAssets = Get-ChildItem -LiteralPath $assetRoot -File |
    ForEach-Object Name |
    Sort-Object
$expectedFiles = @("release-manifest.json")
foreach ($name in $names.Keys) {
    $expectedFiles += $name, "$name.sha256"
}
$expectedFiles = $expectedFiles | Sort-Object
if (($downloadedAssets -join "`n") -ne ($expectedFiles -join "`n")) {
    throw "Downloaded release files do not exactly match the release manifest and SHA-256 sidecars."
}

$portableRecords = @($records | Where-Object { $_.name.EndsWith(".zip") })
if ($portableRecords.Count -eq 0) {
    throw "Release manifest has no portable ZIP asset."
}

$verifyPortable = Join-Path $PSScriptRoot "verify-portable-package.ps1"
$smokePortable = Join-Path $PSScriptRoot "smoke-portable-startup.ps1"
foreach ($record in $portableRecords) {
    $archive = Join-Path $assetRoot $record.name
    & $verifyPortable -ArchivePath $archive
    if ($LASTEXITCODE -ne 0) {
        throw "Portable package verification failed with exit code $LASTEXITCODE."
    }
    if (-not $SkipStartupSmoke) {
        & $smokePortable -ArchivePath $archive
        if ($LASTEXITCODE -ne 0) {
            throw "Portable startup smoke test failed with exit code $LASTEXITCODE."
        }
    }
}

Write-Host "Verified downloaded release assets in $assetRoot"
