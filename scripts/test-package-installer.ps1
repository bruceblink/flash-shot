$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\package-installer-fixture"
$fakeSignTool = Join-Path $fixture "signtool.exe"
$missingSignTool = Join-Path $fixture "missing-signtool.exe"
$packageInstaller = Join-Path $PSScriptRoot "package-installer.ps1"

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

    & $packageInstaller -ValidateOnly

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
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
