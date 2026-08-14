$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $root "target\portable-package-fixture"
$archive = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64.zip"
$staging = Join-Path $fixture "FlashShot-0.1.1-windows-x86_64"
$verify = Join-Path $PSScriptRoot "verify-portable-package.ps1"
try {
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    [IO.File]::WriteAllText((Join-Path $staging "flash-shot.exe"), "fixture executable")
    [IO.File]::WriteAllText((Join-Path $staging "LICENSE.txt"), "fixture license")
    [IO.File]::WriteAllText((Join-Path $staging "README.md"), "fixture readme")
    [IO.File]::WriteAllText((Join-Path $staging "PORTABLE.txt"), "Version: 0.1.1")
    Compress-Archive -LiteralPath $staging -DestinationPath $archive
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii

    & $verify -ArchivePath $archive
    if ($LASTEXITCODE -ne 0) {
        throw "Valid portable package fixture was rejected."
    }

    $failed = $false
    $failureMessage = ""
    try {
        & $verify -ArchivePath $archive -RequireSignature
    }
    catch {
        $failed = $true
        $failureMessage = $_.Exception.Message
    }
    if (-not $failed -or -not $failureMessage.Contains("Portable executable Authenticode signature is not valid")) {
        throw "Portable package verification did not reject an unsigned executable."
    }

    [IO.File]::WriteAllText((Join-Path $staging "unexpected.txt"), "unexpected")
    Remove-Item -LiteralPath $archive, "$archive.sha256"
    Compress-Archive -LiteralPath $staging -DestinationPath $archive
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii

    $failed = $false
    try {
        & $verify -ArchivePath $archive
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed) {
        throw "Portable package verification accepted an unexpected file."
    }
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
