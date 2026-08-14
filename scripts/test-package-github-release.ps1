$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\package-github-release-fixture"
$pfxPath = Join-Path $fixture "signing-fixture.pfx"
$packageGithubRelease = Join-Path $PSScriptRoot "package-github-release.ps1"
$originalCertificate = $env:WINDOWS_SIGNING_CERTIFICATE_BASE64
$originalPassword = $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD
$passwordText = "FlashShot-" + [guid]::NewGuid()
$certificate = $null
$fixtureThumbprint = $null

# Requires GitHub secret validation to reject malformed credentials before packaging starts.
function Assert-GithubPackagingFails([string]$ExpectedMessage, [hashtable]$Parameters = @{}) {
    $failed = $false
    $failureMessage = ""
    try {
        & $packageGithubRelease -ValidateOnly @Parameters
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed) {
        throw "GitHub release packaging unexpectedly accepted invalid signing credentials."
    }
    if (-not $failureMessage.Contains($ExpectedMessage)) {
        throw "GitHub release packaging failed for an unexpected reason: $failureMessage"
    }
}

try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null

    $env:WINDOWS_SIGNING_CERTIFICATE_BASE64 = $null
    $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD = $null
    Assert-GithubPackagingFails "WINDOWS_SIGNING_CERTIFICATE_BASE64"

    $env:WINDOWS_SIGNING_CERTIFICATE_BASE64 = "not-base64"
    $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD = $passwordText
    Assert-GithubPackagingFails "not valid base64"

    $env:WINDOWS_SIGNING_CERTIFICATE_BASE64 = [Convert]::ToBase64String(
        [Text.Encoding]::ASCII.GetBytes("fixture")
    )
    $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD = $null
    Assert-GithubPackagingFails "WINDOWS_SIGNING_CERTIFICATE_PASSWORD"

    $certificate = New-SelfSignedCertificate `
        -Subject "CN=Flash Shot GitHub release fixture" `
        -Type CodeSigningCert `
        -CertStoreLocation Cert:\CurrentUser\My `
        -KeyExportPolicy Exportable `
        -NotAfter (Get-Date).AddDays(1)
    $securePassword = ConvertTo-SecureString -String $passwordText -AsPlainText -Force
    Export-PfxCertificate -Cert $certificate -FilePath $pfxPath -Password $securePassword | Out-Null
    $fixtureThumbprint = $certificate.Thumbprint.ToUpperInvariant()
    Remove-Item -LiteralPath "Cert:\CurrentUser\My\$fixtureThumbprint" -Force
    $certificate = $null

    $env:WINDOWS_SIGNING_CERTIFICATE_BASE64 = [Convert]::ToBase64String(
        [IO.File]::ReadAllBytes($pfxPath)
    )
    $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD = $passwordText
    Assert-GithubPackagingFails "must stay inside the repository" @{
        OutputDirectory = "..\outside-release"
    }
    & $packageGithubRelease -ValidateOnly

    if (Test-Path -LiteralPath "Cert:\CurrentUser\My\$fixtureThumbprint") {
        throw "GitHub release packaging left the imported signing certificate in CurrentUser storage."
    }
}
finally {
    $env:WINDOWS_SIGNING_CERTIFICATE_BASE64 = $originalCertificate
    $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD = $originalPassword
    if ($null -ne $certificate -and
        (Test-Path -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)")) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -Force
    }
    if ($null -ne $fixtureThumbprint -and
        (Test-Path -LiteralPath "Cert:\CurrentUser\My\$fixtureThumbprint")) {
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$fixtureThumbprint" -Force
    }
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
