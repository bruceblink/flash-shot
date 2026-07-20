$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\verify-github-release-fixture"
$packageRoot = Join-Path $fixture "FlashShot-0.1.0-windows-x86_64"
$archive = Join-Path $fixture "FlashShot-0.1.0-windows-x86_64.zip"
$installer = Join-Path $fixture "FlashShot-0.1.0-windows-setup.exe"
$mockDirectory = Join-Path $fixture "mock-bin"
$mockGh = Join-Path $mockDirectory "gh.cmd"
$verify = Join-Path $PSScriptRoot "verify-github-release.ps1"
$originalPath = $env:PATH
$originalAssets = $env:FLASH_SHOT_TEST_RELEASE_ASSETS
$originalDraft = $env:FLASH_SHOT_TEST_IS_DRAFT

try {
    New-Item -ItemType Directory -Force -Path $packageRoot, $mockDirectory | Out-Null
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

    @'
@echo off
if /I "%1 %2"=="release view" (
  echo {"isDraft":%FLASH_SHOT_TEST_IS_DRAFT%,"assets":[{"name":"FlashShot-0.1.0-windows-x86_64.zip"},{"name":"FlashShot-0.1.0-windows-x86_64.zip.sha256"},{"name":"FlashShot-0.1.0-windows-setup.exe"},{"name":"FlashShot-0.1.0-windows-setup.exe.sha256"},{"name":"release-manifest.json"}]}
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
    $env:FLASH_SHOT_TEST_IS_DRAFT = "true"
    & $verify -Tag "v0.1.0" -Repository "fixture/flash-shot" -RequireDraft -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Draft GitHub release verification fixture was rejected."
    }

    $env:FLASH_SHOT_TEST_IS_DRAFT = "false"
    $failed = $false
    try {
        & $verify -Tag "v0.1.0" -Repository "fixture/flash-shot" -RequireDraft -SkipStartupSmoke
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "GitHub release verification accepted a published release when a draft was required."
    }

    & $verify -Tag "v0.1.0" -Repository "fixture/flash-shot" -SkipStartupSmoke
    if ($LASTEXITCODE -ne 0) {
        throw "Published GitHub release verification was rejected without -RequireDraft."
    }
}
finally {
    $env:PATH = $originalPath
    $env:FLASH_SHOT_TEST_RELEASE_ASSETS = $originalAssets
    $env:FLASH_SHOT_TEST_IS_DRAFT = $originalDraft
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
