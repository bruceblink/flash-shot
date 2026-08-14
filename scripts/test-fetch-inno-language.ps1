$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\fetch-inno-language-fixture"
$language = Join-Path $fixture "ChineseSimplified.isl"
$fixtureRepository = Join-Path $fixture "source"
$sourceLanguage = Join-Path $fixtureRepository "Files\Languages\ChineseSimplified.isl"
$downloadedLanguage = Join-Path $fixture "downloaded\ChineseSimplified.isl"
$fetchScript = Join-Path $PSScriptRoot "fetch-inno-language.ps1"

# Runs one invalid language-file check and requires the intended validation error.
function Assert-LanguageValidationFails([string]$Path, [string]$ExpectedSha256, [string]$ExpectedMessage) {
    $failed = $false
    $failureMessage = ""
    try {
        Assert-InnoSimplifiedChineseLanguage $Path $ExpectedSha256
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "Inno Setup language validation unexpectedly accepted invalid input."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "Inno Setup language validation failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null
    . $fetchScript

    @(
        "[LangOptions]",
        'LanguageID=$0804',
        "[Messages]",
        "SetupAppTitle=Install"
    ) | Set-Content -LiteralPath $language -Encoding UTF8
    $expected = (Get-FileHash -LiteralPath $language -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-InnoSimplifiedChineseLanguage $language $expected

    Assert-LanguageValidationFails $language ("0" * 64) "SHA-256 mismatch"
    Assert-LanguageValidationFails $language "not-a-hash" "exactly 64 hexadecimal"

    "not an Inno Setup language file" | Set-Content -LiteralPath $language -Encoding UTF8
    $invalidContentHash = (Get-FileHash -LiteralPath $language -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-LanguageValidationFails $language $invalidContentHash "not a Simplified Chinese messages file"

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $sourceLanguage) | Out-Null
    @(
        "[LangOptions]",
        'LanguageID=$0804',
        "[Messages]",
        "SetupAppTitle=Install from Git"
    ) | Set-Content -LiteralPath $sourceLanguage -Encoding UTF8
    & git init --quiet $fixtureRepository
    & git -C $fixtureRepository config user.name "Flash Shot fixture"
    & git -C $fixtureRepository config user.email "fixture@example.test"
    & git -C $fixtureRepository config core.autocrlf false
    & git -C $fixtureRepository add Files/Languages/ChineseSimplified.isl
    & git -C $fixtureRepository commit --quiet -m "fixture"
    if ($LASTEXITCODE -ne 0) {
        throw "Could not create the local Inno Setup language fixture repository."
    }
    $fixtureCommit = (& git -C $fixtureRepository rev-parse HEAD).Trim()
    $sourceHash = (Get-FileHash -LiteralPath $sourceLanguage -Algorithm SHA256).Hash.ToLowerInvariant()
    Get-InnoSimplifiedChineseLanguage `
        $downloadedLanguage $fixtureRepository $fixtureCommit `
        "Files/Languages/ChineseSimplified.isl" $sourceHash
    Assert-InnoSimplifiedChineseLanguage $downloadedLanguage $sourceHash

    "corrupt cache" | Set-Content -LiteralPath $downloadedLanguage -Encoding UTF8
    Get-InnoSimplifiedChineseLanguage `
        $downloadedLanguage $fixtureRepository $fixtureCommit `
        "Files/Languages/ChineseSimplified.isl" $sourceHash
    Assert-InnoSimplifiedChineseLanguage $downloadedLanguage $sourceHash
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
