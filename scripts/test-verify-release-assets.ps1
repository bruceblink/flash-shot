$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\verify-release-assets-fixture"
$packageRoot = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64"
$archive = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64.zip"
$installer = Join-Path $fixture "FlashShot-0.1.1-windows-setup.exe"
$verify = Join-Path $PSScriptRoot "verify-release-assets.ps1"

# Rebuilds the fixture manifest so each negative case changes only the asset inventory under test.
function Write-FixtureManifest([object[]]$Records) {
    [ordered]@{
        schema_version = 1
        product = "Flash Shot"
        version = "0.1.1"
        platform = "windows"
        assets = @($Records)
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $fixture "release-manifest.json") -Encoding ascii
}

# Requires the verifier to reject a fixture for the expected release-gate reason.
function Assert-AssetVerificationFails([string]$ExpectedMessage) {
    $failed = $false
    $failureMessage = ""
    try {
        & $verify -AssetDirectory $fixture -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "Release asset verification unexpectedly accepted an invalid fixture."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "Release asset verification failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    [IO.File]::WriteAllText((Join-Path $packageRoot "flash-shot.exe"), "fixture executable")
    [IO.File]::WriteAllText((Join-Path $packageRoot "LICENSE.txt"), "fixture license")
    [IO.File]::WriteAllText((Join-Path $packageRoot "README.md"), "fixture readme")
    [IO.File]::WriteAllText((Join-Path $packageRoot "PORTABLE.txt"), "Version: 0.1.1")
    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive
    [IO.File]::WriteAllText($installer, "fixture installer")

    $records = @()
    foreach ($asset in @($archive, $installer)) {
        $file = Get-Item -LiteralPath $asset
        $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $($file.Name)" | Set-Content -LiteralPath "$asset.sha256" -Encoding ascii
        $records += [ordered]@{ name = $file.Name; sha256 = $hash; size_bytes = $file.Length }
    }
    Write-FixtureManifest -Records $records

    & $verify -AssetDirectory "target\verify-release-assets-fixture" -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Valid downloaded release fixture was rejected."
    }
    & $verify -AssetDirectory $fixture -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Valid downloaded release fixture was rejected by an absolute asset directory."
    }

    $failed = $false
    $failureMessage = ""
    try {
        & $verify -AssetDirectory $fixture -SkipStartupSmoke -RequireSignature
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed -or -not $failureMessage.Contains("Setup Authenticode signature is not valid")) {
        throw "Release asset verification did not reject an unsigned setup for the expected reason."
    }

    Write-FixtureManifest -Records @($records[0])
    Assert-AssetVerificationFails "exactly one setup EXE asset; found 0"

    Write-FixtureManifest -Records @($records[1])
    Assert-AssetVerificationFails "exactly one portable ZIP asset; found 0"

    $duplicateArchive = Join-Path $fixture "FlashShot-0.1.1-windows-arm64.zip"
    Copy-Item -LiteralPath $archive -Destination $duplicateArchive
    $duplicateHash = (Get-FileHash -LiteralPath $duplicateArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$duplicateHash  $([IO.Path]::GetFileName($duplicateArchive))" |
        Set-Content -LiteralPath "$duplicateArchive.sha256" -Encoding ascii
    $duplicateRecord = [ordered]@{
        name = [IO.Path]::GetFileName($duplicateArchive)
        sha256 = $duplicateHash
        size_bytes = (Get-Item -LiteralPath $duplicateArchive).Length
    }
    Write-FixtureManifest -Records @($records[0], $duplicateRecord, $records[1])
    Assert-AssetVerificationFails "exactly one portable ZIP asset; found 2"
    Remove-Item -LiteralPath $duplicateArchive, "$duplicateArchive.sha256"

    Write-FixtureManifest -Records $records

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
