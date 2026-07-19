param(
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [string]$Repository = "bruceblink/flash-shot",
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"

function Test-AssetName([string]$Name, [string]$Version) {
    $escapedVersion = [regex]::Escape($Version)
    return $Name -match "^FlashShot-$escapedVersion-windows-[A-Za-z0-9_]+\.zip$" -or
        $Name -match "^FlashShot-$escapedVersion-windows-setup\.exe$"
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$verifyTag = Join-Path $PSScriptRoot "verify-release-tag.ps1"
& $verifyTag -Tag $Tag
if ($LASTEXITCODE -ne 0) {
    throw "Release tag verification failed with exit code $LASTEXITCODE."
}
if ($null -eq (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI is required to download release assets."
}

$release = & gh release view $Tag --repo $Repository --json assets | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw "Could not inspect GitHub release assets for $Tag."
}
$assetNames = @($release.assets | ForEach-Object name)
$uniqueAssetNames = @($assetNames | Sort-Object -Unique)
if ($uniqueAssetNames.Count -ne $assetNames.Count) {
    throw "GitHub release $Tag has duplicate asset names."
}
$principalAssets = @($assetNames | Where-Object { Test-AssetName $_ $Tag.Substring(1) })
if ($principalAssets.Count -eq 0 -or -not ($principalAssets | Where-Object { $_.EndsWith(".zip") })) {
    throw "GitHub release $Tag has no portable Flash Shot asset."
}
$expectedAssets = @("release-manifest.json")
foreach ($asset in $principalAssets) {
    $expectedAssets += $asset, "$asset.sha256"
}
if (($uniqueAssetNames -join "`n") -ne (($expectedAssets | Sort-Object) -join "`n")) {
    throw "GitHub release $Tag has missing or unsupported assets."
}

$temporary = $OutputDirectory.Length -eq 0
if ($temporary) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-release-$Tag-" + [guid]::NewGuid())
}
$destination = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    [IO.Path]::GetFullPath($OutputDirectory)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
}
New-Item -ItemType Directory -Force -Path $destination | Out-Null
try {
    & gh release download $Tag --repo $Repository --dir $destination --clobber `
        --pattern "FlashShot-*.zip" --pattern "FlashShot-*.zip.sha256" `
        --pattern "FlashShot-*.exe" --pattern "FlashShot-*.exe.sha256" `
        --pattern "release-manifest.json"
    if ($LASTEXITCODE -ne 0) {
        throw "Could not download GitHub release assets for $Tag."
    }

    & (Join-Path $PSScriptRoot "verify-release-assets.ps1") -AssetDirectory $destination
    if ($LASTEXITCODE -ne 0) {
        throw "Downloaded GitHub release asset verification failed with exit code $LASTEXITCODE."
    }
}
finally {
    if ($temporary -and (Test-Path -LiteralPath $destination)) {
        Remove-Item -LiteralPath $destination -Recurse -Force
    }
}
