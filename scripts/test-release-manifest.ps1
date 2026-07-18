$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\release-manifest-fixture"
$asset = Join-Path $fixture "FlashShot-0.1.0-windows-x86_64.zip"
try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null
    [IO.File]::WriteAllText($asset, "release-manifest-fixture")
    $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($asset))" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
    & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture"
    & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture" -VerifyOnly
    if ($LASTEXITCODE -ne 0) {
        throw "Release manifest fixture verification failed."
    }
    "$("0" * 64)  $([IO.Path]::GetFileName($asset))" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
    $failed = $false
    try {
        & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture" -VerifyOnly
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Release manifest verification accepted a changed checksum."
    }
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
