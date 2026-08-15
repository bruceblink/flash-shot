param(
    [string]$OutputDirectory = "dist",
    [switch]$ValidateOnly,
    [switch]$SkipBuild,
    [switch]$RequireSignature,
    [string]$SignToolPath = "",
    [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    throw "-OutputDirectory must be a repository-relative path."
}
$assetRoot = [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
$rootPrefix = $root.TrimEnd('\') + [IO.Path]::DirectorySeparatorChar
if (-not $assetRoot.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "-OutputDirectory must stay inside the repository: $OutputDirectory"
}

if (-not $RequireSignature -and ($SignToolPath.Length -gt 0 -or $PSBoundParameters.ContainsKey("TimestampUrl"))) {
    throw "-SignToolPath and -TimestampUrl require -RequireSignature."
}

$pfxPath = $null
$existingThumbprints = @{}
$newThumbprints = @()
$installerParameters = @{
    OutputDirectory = $OutputDirectory
}

try {
    if ($RequireSignature) {
        $certificateBase64 = [Environment]::GetEnvironmentVariable(
            "WINDOWS_SIGNING_CERTIFICATE_BASE64",
            "Process"
        )
        $certificatePassword = [Environment]::GetEnvironmentVariable(
            "WINDOWS_SIGNING_CERTIFICATE_PASSWORD",
            "Process"
        )
        if ([string]::IsNullOrWhiteSpace($certificateBase64)) {
            throw "WINDOWS_SIGNING_CERTIFICATE_BASE64 must contain a base64-encoded production PFX."
        }
        if ([string]::IsNullOrWhiteSpace($certificatePassword)) {
            throw "WINDOWS_SIGNING_CERTIFICATE_PASSWORD must contain the PFX password."
        }

        try {
            $certificateBytes = [Convert]::FromBase64String($certificateBase64)
        }
        catch {
            throw "WINDOWS_SIGNING_CERTIFICATE_BASE64 is not valid base64."
        }
        if ($certificateBytes.Length -eq 0) {
            throw "WINDOWS_SIGNING_CERTIFICATE_BASE64 decoded to an empty PFX."
        }

        $pfxPath = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-signing-" + [guid]::NewGuid() + ".pfx")
        Get-ChildItem -Path Cert:\CurrentUser\My -ErrorAction SilentlyContinue | ForEach-Object {
            $existingThumbprints[$_.Thumbprint.ToUpperInvariant()] = $true
        }
        [IO.File]::WriteAllBytes($pfxPath, $certificateBytes)
        $securePassword = ConvertTo-SecureString -String $certificatePassword -AsPlainText -Force
        $importedCertificates = @(Import-PfxCertificate -FilePath $pfxPath `
            -CertStoreLocation Cert:\CurrentUser\My -Password $securePassword)
        $newThumbprints = @($importedCertificates |
            ForEach-Object { $_.Thumbprint.ToUpperInvariant() } |
            Sort-Object -Unique |
            Where-Object { -not $existingThumbprints.ContainsKey($_) })

        $now = Get-Date
        $codeSigningOid = "1.3.6.1.5.5.7.3.3"
        $signingCertificate = $importedCertificates |
            Where-Object {
                $usages = @($_.EnhancedKeyUsageList | ForEach-Object { [string]$_.ObjectId })
                $_.HasPrivateKey -and $_.NotBefore -le $now -and $_.NotAfter -gt $now -and
                    $usages -contains $codeSigningOid
            } |
            Sort-Object NotAfter -Descending |
            Select-Object -First 1
        if ($null -eq $signingCertificate) {
            throw "The imported PFX has no valid private certificate with the Code Signing usage."
        }

        $installerParameters.RequireSignature = $true
        $installerParameters.CertificateThumbprint = $signingCertificate.Thumbprint
        $installerParameters.TimestampUrl = $TimestampUrl
        if ($SignToolPath.Length -gt 0) {
            $installerParameters.SignToolPath = $SignToolPath
        }
    }
    if ($SkipBuild) {
        $installerParameters.SkipBuild = $true
    }
    if ($ValidateOnly) {
        $installerParameters.ValidateOnly = $true
        & (Join-Path $PSScriptRoot "package-installer.ps1") @installerParameters
        $mode = if ($RequireSignature) { "signed" } else { "unsigned" }
        Write-Host "GitHub release packaging prerequisites are valid for $mode assets."
        return
    }

    if (Test-Path -LiteralPath $assetRoot -PathType Container) {
        $existingAssets = @(Get-ChildItem -LiteralPath $assetRoot -Force)
        if ($existingAssets.Count -gt 0) {
            throw "GitHub release asset directory must be empty: $assetRoot"
        }
    }

    $languageOutputs = @(& (Join-Path $PSScriptRoot "fetch-inno-language.ps1"))
    $chineseMessages = $languageOutputs |
        Where-Object { $_ -is [string] -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
        Select-Object -Last 1
    if ($null -eq $chineseMessages) {
        throw "Could not prepare the pinned Inno Setup Simplified Chinese messages."
    }
    $installerParameters.ChineseMessagesFile = [IO.Path]::GetFullPath($chineseMessages)

    # The installer packager signs the release EXE first; the portable archive must reuse that EXE.
    & (Join-Path $PSScriptRoot "package-installer.ps1") @installerParameters
    & (Join-Path $PSScriptRoot "package-portable.ps1") `
        -OutputDirectory $OutputDirectory -SkipBuild
}
finally {
    foreach ($thumbprint in $newThumbprints) {
        $certificatePath = "Cert:\CurrentUser\My\$thumbprint"
        if (Test-Path -LiteralPath $certificatePath) {
            Remove-Item -LiteralPath $certificatePath -Force
        }
    }
    if ($null -ne $pfxPath -and (Test-Path -LiteralPath $pfxPath -PathType Leaf)) {
        [IO.File]::Delete($pfxPath)
    }
}
