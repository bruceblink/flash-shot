$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\release-manifest-fixture"
$asset = Join-Path $fixture "FlashShot-0.1.2-windows-x86_64.zip"
$installer = Join-Path $fixture "FlashShot-0.1.2-windows-setup.exe"
try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null
    [IO.File]::WriteAllText($asset, "release-manifest-fixture")
    $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($asset))" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
    [IO.File]::WriteAllText($installer, "release-manifest-installer-fixture")
    $installerHash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    "$installerHash  $([IO.Path]::GetFileName($installer))" | Set-Content -LiteralPath "$installer.sha256" -Encoding ascii
    & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture"
    & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture" -VerifyOnly
    if ($LASTEXITCODE -ne 0) {
        throw "Release manifest fixture verification failed."
    }

    Remove-Item -LiteralPath $installer, "$installer.sha256"
    $failed = $false
    try {
        & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture"
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $_.Exception.Message -like "*exactly one setup EXE asset; found 0*"
    }
    if (-not $failed) {
        throw "Release manifest accepted a fixture without an installer."
    }
    [IO.File]::WriteAllText($installer, "release-manifest-installer-fixture")
    "$installerHash  $([IO.Path]::GetFileName($installer))" | Set-Content -LiteralPath "$installer.sha256" -Encoding ascii

    $duplicate = Join-Path $fixture "FlashShot-0.1.2-windows-arm64.zip"
    [IO.File]::WriteAllText($duplicate, "duplicate portable fixture")
    $duplicateHash = (Get-FileHash -LiteralPath $duplicate -Algorithm SHA256).Hash.ToLowerInvariant()
    "$duplicateHash  $([IO.Path]::GetFileName($duplicate))" | Set-Content -LiteralPath "$duplicate.sha256" -Encoding ascii
    $failed = $false
    try {
        & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture"
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $_.Exception.Message -like "*exactly one portable ZIP asset; found 2*"
    }
    if (-not $failed) {
        throw "Release manifest accepted duplicate portable packages."
    }
    Remove-Item -LiteralPath $duplicate, "$duplicate.sha256"

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

    "$hash  $([IO.Path]::GetFileName($asset))" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
    $unexpected = Join-Path $fixture "FlashShot-0.1.2-windows-x86_64.exe"
    [IO.File]::WriteAllText($unexpected, "unexpected release asset")
    $unexpectedHash = (Get-FileHash -LiteralPath $unexpected -Algorithm SHA256).Hash.ToLowerInvariant()
    "$unexpectedHash  $([IO.Path]::GetFileName($unexpected))" | Set-Content -LiteralPath "$unexpected.sha256" -Encoding ascii
    $failed = $false
    try {
        & (Join-Path $PSScriptRoot "release-manifest.ps1") -AssetDirectory "target\release-manifest-fixture"
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Release manifest accepted an unsupported artifact name."
    }
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
