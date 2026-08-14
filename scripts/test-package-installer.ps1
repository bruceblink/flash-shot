$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\package-installer-fixture"
$fakeSignTool = Join-Path $fixture "signtool.exe"
$missingSignTool = Join-Path $fixture "missing-signtool.exe"
$fakeLanguage = Join-Path $fixture "ChineseSimplified.isl"
$invalidLanguage = Join-Path $fixture "invalid-language.isl"
$fakeShimDirectory = Join-Path $fixture "chocolatey-bin"
$fakeShim = Join-Path $fakeShimDirectory "ISCC.exe"
$fakeInnoRoot = Join-Path $fixture "Inno Setup 6"
$fakeInnoCompiler = Join-Path $fakeInnoRoot "ISCC.exe"
$packageInstaller = Join-Path $PSScriptRoot "package-installer.ps1"
$timestampValidationCertificate = "0000000000000000000000000000000000000000"

# Runs one invalid preflight and requires the failure to come from the intended signing gate.
function Assert-InstallerValidationFails([hashtable]$Parameters, [string]$ExpectedMessage) {
    $failed = $false
    $failureMessage = ""
    try {
        & $packageInstaller @Parameters
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "Installer validation unexpectedly accepted invalid signing prerequisites."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "Installer validation failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null

    # Load the helper through its normal validation path, then exercise the shim resolution rule directly.
    . $packageInstaller -ValidateOnly
    New-Item -ItemType Directory -Force -Path $fakeShimDirectory, $fakeInnoRoot | Out-Null
    [IO.File]::WriteAllText($fakeShim, "fixture Chocolatey shim")
    [IO.File]::WriteAllText($fakeInnoCompiler, "fixture Inno compiler")
    [IO.File]::WriteAllText((Join-Path $fakeInnoRoot "Default.isl"), "fixture compiler messages")
    $resolvedCompiler = Get-InnoSetupCompilerPath @($fakeInnoCompiler) $fakeShim
    if ($resolvedCompiler -ne [IO.Path]::GetFullPath($fakeInnoCompiler)) {
        throw "Inno compiler resolution did not bypass a package-manager shim."
    }

    & $packageInstaller -ValidateOnly

    @(
        "[LangOptions]",
        'LanguageID=$0804',
        "[Messages]",
        "SetupAppTitle=Install"
    ) | Set-Content -LiteralPath $fakeLanguage -Encoding UTF8
    & $packageInstaller -ValidateOnly -ChineseMessagesFile $fakeLanguage

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        ChineseMessagesFile = "relative\ChineseSimplified.isl"
    } "must be an absolute path"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        ChineseMessagesFile = (Join-Path $fixture "missing-language.isl")
    } "messages file does not exist"

    "invalid language" | Set-Content -LiteralPath $invalidLanguage -Encoding UTF8
    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        ChineseMessagesFile = $invalidLanguage
    } "not a Simplified Chinese"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        RequireSignature = $true
        SignToolPath = $missingSignTool
    } "signtool.exe path does not exist"

    [IO.File]::WriteAllText($fakeSignTool, "fixture SignTool")
    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        RequireSignature = $true
        SignToolPath = $fakeSignTool
        CertificateThumbprint = "not-a-thumbprint"
    } "exactly 40 hexadecimal characters"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        RequireSignature = $true
        SignToolPath = $fakeSignTool
        CertificateThumbprint = "0000000000000000000000000000000000000000"
    } "valid CurrentUser code-signing certificate"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        CertificateThumbprint = "0000000000000000000000000000000000000000"
    } "require -RequireSignature"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        TimestampUrl = "not-a-url"
    } "require -RequireSignature"

    Assert-InstallerValidationFails @{
        ValidateOnly = $true
        RequireSignature = $true
        SignToolPath = $fakeSignTool
        CertificateThumbprint = $timestampValidationCertificate
        TimestampUrl = "ftp://timestamp.example.test"
    } "absolute HTTP or HTTPS URL"
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
