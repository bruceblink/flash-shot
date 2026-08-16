$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\verify-github-release-fixture"
$packageRoot = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64"
$archive = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64.zip"
$installer = Join-Path $fixture "FlashShot-0.1.1-windows-setup.exe"
$mockDirectory = Join-Path $fixture "mock-bin"
$mockGh = Join-Path $mockDirectory "gh.cmd"
$verify = Join-Path $PSScriptRoot "verify-github-release.ps1"
$originalPath = $env:PATH
$originalAssets = $env:FLASH_SHOT_TEST_RELEASE_ASSETS
$originalReleaseView = $env:FLASH_SHOT_TEST_RELEASE_VIEW

# Builds the compact release metadata returned by the mocked GitHub CLI.
function New-ReleaseViewJson([bool]$IsDraft, [string[]]$AssetNames) {
    $assets = @($AssetNames | ForEach-Object { [ordered]@{ name = $_ } })
    return [ordered]@{ isDraft = $IsDraft; assets = $assets } |
        ConvertTo-Json -Depth 3 -Compress
}

# Requires mocked GitHub metadata to fail for the exact asset-count gate under test.
function Assert-GithubVerificationFails([string[]]$AssetNames, [string]$ExpectedMessage) {
    $env:FLASH_SHOT_TEST_RELEASE_VIEW = New-ReleaseViewJson $true $AssetNames
    $failed = $false
    $failureMessage = ""
    try {
        & $verify -Tag "v0.1.1" -Repository "fixture/flash-shot" -RequireDraft -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "GitHub release verification unexpectedly accepted invalid asset metadata."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "GitHub release verification failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $packageRoot, $mockDirectory | Out-Null
    [IO.File]::WriteAllText((Join-Path $packageRoot "flash-shot.exe"), "fixture executable")
    [IO.File]::WriteAllText((Join-Path $packageRoot "LICENSE.txt"), "fixture license")
    [IO.File]::WriteAllText((Join-Path $packageRoot "README.md"), "fixture readme")
    [IO.File]::WriteAllText((Join-Path $packageRoot "README_EN.md"), "fixture English readme")
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
    [ordered]@{
        schema_version = 1
        product = "Flash Shot"
        version = "0.1.1"
        platform = "windows"
        assets = $records
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $fixture "release-manifest.json") -Encoding ascii

@'
@echo off
if /I "%1 %2"=="release view" (
  echo %FLASH_SHOT_TEST_RELEASE_VIEW%
  exit /b 0
)
if /I "%1 %2"=="release download" (
  set "destination="
:arguments
  if "%~1"=="" goto copy_assets
  if /I "%~1"=="--dir" (
    set "destination=%~2"
    shift
  )
  shift
  goto arguments
:copy_assets
  copy /y "%FLASH_SHOT_TEST_RELEASE_ASSETS%\*" "%destination%\" >nul
  exit /b 0
)
exit /b 1
'@ | Set-Content -LiteralPath $mockGh -Encoding ascii

    $env:PATH = "$mockDirectory;$originalPath"
    $env:FLASH_SHOT_TEST_RELEASE_ASSETS = $fixture
    $portableName = "FlashShot-0.1.1-windows-x86_64.zip"
    $installerName = "FlashShot-0.1.1-windows-setup.exe"
    $manifestName = "release-manifest.json"
    $validAssets = @(
        $portableName,
        "$portableName.sha256",
        $installerName,
        "$installerName.sha256",
        $manifestName
    )
    $env:FLASH_SHOT_TEST_RELEASE_VIEW = New-ReleaseViewJson $true $validAssets
    & $verify -Tag "v0.1.1" -Repository "fixture/flash-shot" -RequireDraft -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Draft GitHub release verification fixture was rejected."
    }

    $failed = $false
    $failureMessage = ""
    try {
        & $verify -Tag "v0.1.1" -Repository "fixture/flash-shot" `
            -RequireDraft -SkipStartupSmoke -RequireSignature
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed -or -not $failureMessage.Contains("Setup Authenticode signature is not valid")) {
        throw "GitHub release verification did not forward the signature requirement."
    }

    Assert-GithubVerificationFails @(
        $portableName,
        "$portableName.sha256",
        $manifestName
    ) "exactly one setup EXE asset; found 0"
    Assert-GithubVerificationFails @(
        $installerName,
        "$installerName.sha256",
        $manifestName
    ) "exactly one portable ZIP asset; found 0"
    Assert-GithubVerificationFails @(
        $portableName,
        "$portableName.sha256",
        "FlashShot-0.1.1-windows-arm64.zip",
        "FlashShot-0.1.1-windows-arm64.zip.sha256",
        $installerName,
        "$installerName.sha256",
        $manifestName
    ) "exactly one portable ZIP asset; found 2"

    $env:FLASH_SHOT_TEST_RELEASE_VIEW = New-ReleaseViewJson $false $validAssets
    $failed = $false
    try {
        & $verify -Tag "v0.1.1" -Repository "fixture/flash-shot" -RequireDraft -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "GitHub release verification accepted a published release when a draft was required."
    }

    & $verify -Tag "v0.1.1" -Repository "fixture/flash-shot" -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Published GitHub release verification was rejected without -RequireDraft."
    }
}
finally {
    $env:PATH = $originalPath
    $env:FLASH_SHOT_TEST_RELEASE_ASSETS = $originalAssets
    $env:FLASH_SHOT_TEST_RELEASE_VIEW = $originalReleaseView
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
