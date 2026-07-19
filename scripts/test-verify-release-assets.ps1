$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\verify-release-assets-fixture"
$packageRoot = Join-Path $fixture "FlashShot-0.1.0-windows-x86_64"
$archive = Join-Path $fixture "FlashShot-0.1.0-windows-x86_64.zip"
$installer = Join-Path $fixture "FlashShot-0.1.0-windows-setup.exe"
$verify = Join-Path $PSScriptRoot "verify-release-assets.ps1"
try {
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    [IO.File]::WriteAllText((Join-Path $packageRoot "flash-shot.exe"), "fixture executable")
    [IO.File]::WriteAllText((Join-Path $packageRoot "LICENSE.txt"), "fixture license")
    [IO.File]::WriteAllText((Join-Path $packageRoot "README.md"), "fixture readme")
    [IO.File]::WriteAllText((Join-Path $packageRoot "PORTABLE.txt"), "Version: 0.1.0")
    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive
    [IO.File]::WriteAllText($installer, "fixture installer")

    $records = @()
    foreach ($asset in @($archive, $installer)) {
        $file = Get-Item -LiteralPath $asset
        $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $($file.Name)" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
        $records += [ordered]@{ name = $file.Name; sha256 = $hash; size_bytes = $file.Length }
    }
    [ordered]@{
        schema_version = 1
        product = "Flash Shot"
        version = "0.1.0"
        platform = "windows"
        assets = $records
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $fixture "release-manifest.json") -Encoding ascii

    & $verify -AssetDirectory "target\verify-release-assets-fixture" -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Valid downloaded release fixture was rejected."
    }
    & $verify -AssetDirectory $fixture -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Valid downloaded release fixture was rejected by an absolute asset directory."
    }

    $manifestPath = Join-Path $fixture "release-manifest.json"
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $manifest.assets[0].size_bytes++
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding ascii
    $failed = $false
    try {
        & $verify -AssetDirectory "target\verify-release-assets-fixture" -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Release asset verification accepted a mismatched manifest size."
    }

    $manifest.assets[0].size_bytes--
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding ascii
    [IO.File]::WriteAllText((Join-Path $fixture "unexpected.txt"), "unexpected")
    $failed = $false
    try {
        & $verify -AssetDirectory "target\verify-release-assets-fixture" -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Release asset verification accepted an unexpected downloaded file."
    }
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
