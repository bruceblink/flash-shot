param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath
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

$archive = [IO.Path]::GetFullPath($ArchivePath)
if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
    throw "Portable archive does not exist: $archive"
}
$expectedName = "FlashShot-$($package.version)-windows-(?<architecture>[^.]+)\.zip"
if ([IO.Path]::GetFileName($archive) -notmatch "^$expectedName$") {
    throw "Portable archive name does not include Cargo version $($package.version): $archive"
}
$packageRoot = [IO.Path]::GetFileNameWithoutExtension($archive)
$checksum = "$archive.sha256"
if (-not (Test-Path -LiteralPath $checksum -PathType Leaf)) {
    throw "Missing SHA-256 file for $archive."
}
$expectedHash = Read-Checksum $checksum ([IO.Path]::GetFileName($archive))
$actualHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) {
    throw "SHA-256 mismatch for $archive."
}

$staging = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-portable-verify-" + [guid]::NewGuid())
try {
    Expand-Archive -LiteralPath $archive -DestinationPath $staging
    $expectedFiles = @("flash-shot.exe", "LICENSE.txt", "README.md", "PORTABLE.txt")
    $actualFiles = Get-ChildItem -LiteralPath (Join-Path $staging $packageRoot) -File |
        ForEach-Object Name |
        Sort-Object
    if (($actualFiles -join "`n") -ne (($expectedFiles | Sort-Object) -join "`n")) {
        throw "Portable archive has an unexpected file layout."
    }
    $rootEntries = Get-ChildItem -LiteralPath $staging
    if ($rootEntries.Count -ne 1 -or $rootEntries[0].Name -ne $packageRoot -or -not $rootEntries[0].PSIsContainer) {
        throw "Portable archive must contain exactly one root directory named $packageRoot."
    }
    $portable = Get-Content -Raw -LiteralPath (Join-Path (Join-Path $staging $packageRoot) "PORTABLE.txt")
    if ($portable -notmatch [regex]::Escape("Version: $($package.version)")) {
        throw "Portable package metadata does not identify Flash Shot $($package.version)."
    }
}
finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

Write-Host "Verified portable package $archive"
