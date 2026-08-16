$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\smoke-installer-fixture"
$smokeInstaller = Join-Path $PSScriptRoot "smoke-installer.ps1"
$expectedInstaller = Join-Path $fixture "FlashShot-0.1.2-windows-setup.exe"
$wrongNameInstaller = Join-Path $fixture "unexpected-setup.exe"

# Requires a pre-install validation case to fail for the exact release-safety reason under test.
function Assert-SmokeValidationFails([hashtable]$Parameters, [string]$ExpectedMessage) {
    $failed = $false
    $failureMessage = ""
    try {
        & $smokeInstaller @Parameters
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "Installer smoke validation unexpectedly accepted invalid input."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "Installer smoke validation failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null

    Assert-SmokeValidationFails @{
        InstallerPath = (Join-Path $fixture "missing-setup.exe")
    } "installer does not exist"

    [IO.File]::WriteAllText($wrongNameInstaller, "fixture installer")
    Assert-SmokeValidationFails @{
        InstallerPath = $wrongNameInstaller
    } "Expected installer 'FlashShot-0.1.2-windows-setup.exe'"

    [IO.File]::WriteAllText($expectedInstaller, "unsigned fixture installer")
    Assert-SmokeValidationFails @{
        InstallerPath = $expectedInstaller
        RequireSignature = $true
    } "Authenticode signature is not valid"
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
