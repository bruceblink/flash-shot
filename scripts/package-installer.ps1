param(
    [string]$OutputDirectory = "dist",
    [switch]$SkipBuild,
    [switch]$RequireSignature,
    [switch]$ValidateOnly,
    [string]$SignToolPath = "",
    [string]$CertificateThumbprint = "",
    [string]$TimestampUrl = "http://timestamp.digicert.com",
    [string]$ChineseMessagesFile = ""
)

$ErrorActionPreference = "Stop"

# Resolves an explicit absolute tool path first, then PATH and known installation locations.
function Get-CommandPath([string]$Name, [string[]]$Candidates, [string]$ExplicitPath = "") {
    if ($ExplicitPath.Length -gt 0) {
        if (-not [IO.Path]::IsPathRooted($ExplicitPath)) {
            throw "Explicit $Name path must be absolute."
        }
        $resolved = [IO.Path]::GetFullPath($ExplicitPath)
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw "$Name path does not exist: $resolved"
        }
        return $resolved
    }
    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $command) {
        return $command.Source
    }
    foreach ($candidate in $Candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return $candidate
        }
    }
    return $null
}

# Selects the exact valid private certificate that SignTool will use from the current-user store.
function Get-CodeSigningCertificate([string]$Thumbprint) {
    $normalized = ($Thumbprint -replace '\s', '').ToUpperInvariant()
    if ($normalized.Length -gt 0 -and $normalized -notmatch '^[0-9A-F]{40}$') {
        throw "-CertificateThumbprint must contain exactly 40 hexadecimal characters."
    }

    $now = Get-Date
    $codeSigningOid = "1.3.6.1.5.5.7.3.3"
    $certificates = @(Get-ChildItem -Path Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
        Where-Object {
            $usages = @($_.EnhancedKeyUsageList | ForEach-Object { [string]$_.ObjectId })
            $_.HasPrivateKey -and $_.NotBefore -le $now -and $_.NotAfter -gt $now -and
                $usages -contains $codeSigningOid -and
                ($normalized.Length -eq 0 -or $_.Thumbprint.ToUpperInvariant() -eq $normalized)
        } |
        Sort-Object NotAfter -Descending)
    return $certificates | Select-Object -First 1
}

# Validates the RFC 3161 endpoint before packaging so signing fails early with an actionable error.
function Get-TimestampEndpoint([string]$Value) {
    try {
        $endpoint = [Uri]$Value
    }
    catch {
        throw "-TimestampUrl must be an absolute HTTP or HTTPS URL."
    }
    if (-not $endpoint.IsAbsoluteUri -or $endpoint.Scheme -notin @("http", "https")) {
        throw "-TimestampUrl must be an absolute HTTP or HTTPS URL."
    }
    return $endpoint.AbsoluteUri
}

# Validates an operator-supplied language file early so ISCC never sees an ambiguous path or locale.
function Get-ChineseMessagesFile([string]$Path) {
    if (-not [IO.Path]::IsPathRooted($Path)) {
        throw "-ChineseMessagesFile must be an absolute path."
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Inno Setup Simplified Chinese messages file does not exist: $resolved"
    }
    $content = Get-Content -LiteralPath $resolved -Raw -Encoding UTF8
    if ($content -notmatch '(?m)^LanguageID=\$0804\s*$' -or
        $content -notmatch '(?m)^\[Messages\]\s*$' -or
        $content -notmatch '(?m)^SetupAppTitle=') {
        throw "-ChineseMessagesFile is not a Simplified Chinese Inno Setup messages file."
    }
    return $resolved
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifest = Join-Path $root "Cargo.toml"
$installer = Join-Path $root "installer\flash-shot.iss"
$icon = Join-Path $root "resources\icons\icon.ico"
$license = Join-Path $root "LICENSE"
$readme = Join-Path $root "README.md"
$required = @($manifest, $installer, $icon, $license, $readme)
foreach ($path in $required) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required installer input is missing: $path"
    }
}
$resolvedChineseMessagesFile = if ($ChineseMessagesFile.Length -gt 0) {
    Get-ChineseMessagesFile $ChineseMessagesFile
}
else {
    $null
}

$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path $manifest | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}
$rustHost = (& cargo -vV | Where-Object { $_ -like "host:*" } | Select-Object -First 1).Replace("host: ", "")
if ($rustHost -notmatch "-pc-windows-msvc$") {
    throw "Windows installer packaging requires an MSVC Windows Rust host; found $rustHost."
}

$signTool = $null
$signingCertificate = $null
$timestampEndpoint = $null
if (-not $RequireSignature -and (
        $SignToolPath.Length -gt 0 -or
        $CertificateThumbprint.Length -gt 0 -or
        $PSBoundParameters.ContainsKey("TimestampUrl")
    )) {
    throw "-SignToolPath, -CertificateThumbprint, and -TimestampUrl require -RequireSignature."
}
if ($RequireSignature) {
    $signToolCandidates = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\x64\signtool.exe",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\x86\signtool.exe"
    )
    $windowsKitsBin = Join-Path "${env:ProgramFiles(x86)}" "Windows Kits\10\bin"
    if (Test-Path -LiteralPath $windowsKitsBin -PathType Container) {
        $signToolCandidates += Get-ChildItem -LiteralPath $windowsKitsBin -Directory |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName "x64\signtool.exe" }
    }
    $signTool = Get-CommandPath "signtool.exe" $signToolCandidates $SignToolPath
    if ($null -eq $signTool) {
        throw "-RequireSignature needs signtool.exe on PATH or an explicit -SignToolPath."
    }
    $timestampEndpoint = Get-TimestampEndpoint $TimestampUrl
    $signingCertificate = Get-CodeSigningCertificate $CertificateThumbprint
    if ($null -eq $signingCertificate) {
        throw "-RequireSignature needs a valid CurrentUser code-signing certificate with a private key."
    }
}

if ($ValidateOnly) {
    $installerDefinition = Get-Content -Raw $installer
    if ($installerDefinition -notmatch "MyAppVersion") {
        throw "Installer script does not accept a version from Cargo metadata."
    }
    if ($installerDefinition -notmatch "ChineseMessagesFile") {
        throw "Installer script does not accept an explicit Simplified Chinese messages file."
    }
    $signingStatus = if ($RequireSignature) {
        " Signing prerequisites are ready for certificate $($signingCertificate.Thumbprint)."
    }
    else {
        ""
    }
    Write-Host "Installer configuration is valid for Flash Shot $($package.version) on $rustHost.$signingStatus"
    return
}

$releaseDirectory = Join-Path $metadata.target_directory (Join-Path $rustHost "release")
$executable = Join-Path $releaseDirectory "flash-shot.exe"
if (-not $SkipBuild) {
    & cargo build --release --bin flash-shot --target $rustHost --manifest-path $package.manifest_path
    if ($LASTEXITCODE -ne 0) {
        throw "Release build failed with exit code $LASTEXITCODE."
    }
}
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Release executable not found at $executable. Run without -SkipBuild or build the $rustHost target."
}

$iscc = Get-CommandPath "ISCC.exe" @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
    "${env:LOCALAPPDATA}\Programs\Inno Setup 6\ISCC.exe"
)
if ($null -eq $iscc) {
    throw "Inno Setup 6 is required. Install it or make ISCC.exe available on PATH."
}

# Checks every compiler message file before signing so a failed installer build leaves no signed partial artifact.
$isccRoot = Split-Path -Parent $iscc
$compilerLanguageFiles = @(
    [regex]::Matches((Get-Content -Raw $installer), 'MessagesFile:\s*"compiler:([^" ]+)"') |
        ForEach-Object { Join-Path $isccRoot $_.Groups[1].Value.Replace('\', [IO.Path]::DirectorySeparatorChar) }
)
$defaultChineseMessagesFile = Join-Path $isccRoot "Languages\ChineseSimplified.isl"
if ($null -eq $resolvedChineseMessagesFile) {
    if (Test-Path -LiteralPath $defaultChineseMessagesFile -PathType Leaf) {
        $resolvedChineseMessagesFile = Get-ChineseMessagesFile $defaultChineseMessagesFile
    }
    else {
        throw "Inno Setup Simplified Chinese messages are missing. Run scripts\fetch-inno-language.ps1 and pass its absolute output with -ChineseMessagesFile."
    }
}
$missingLanguageFiles = @($compilerLanguageFiles | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
if ($missingLanguageFiles.Count -gt 0) {
    throw "Inno Setup compiler message files are missing: $($missingLanguageFiles -join ', '). Install the full Inno Setup language pack."
}

if ($RequireSignature) {
    & $signTool sign /fd SHA256 /tr $timestampEndpoint /td SHA256 `
        /sha1 $signingCertificate.Thumbprint $executable
    if ($LASTEXITCODE -ne 0) {
        throw "Could not sign $executable."
    }
    & $signTool verify /pa /tw $executable
    if ($LASTEXITCODE -ne 0) {
        throw "Signature verification failed for $executable."
    }
}

$output = [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
New-Item -ItemType Directory -Force -Path $output | Out-Null
$staging = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-installer-" + [guid]::NewGuid())
try {
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $staging "flash-shot.exe")
    Copy-Item -LiteralPath $license -Destination (Join-Path $staging "LICENSE.txt")
    Copy-Item -LiteralPath $readme -Destination (Join-Path $staging "README.md")
    @(
        "Flash Shot installer package",
        "FFmpeg is intentionally not bundled. Install a compatible build or set FLASH_SHOT_FFMPEG before recording.",
        "Version: $($package.version)",
        "Target: $rustHost"
    ) | Set-Content -LiteralPath (Join-Path $staging "PORTABLE.txt") -Encoding ascii

    & $iscc "/DMyAppVersion=$($package.version)" "/DMySourceDir=$staging" `
        "/DChineseMessagesFile=$resolvedChineseMessagesFile" "/O$output" $installer
    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compilation failed with exit code $LASTEXITCODE."
    }
}
finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

$setup = Join-Path $output "FlashShot-$($package.version)-windows-setup.exe"
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) {
    throw "Inno Setup did not create the expected installer at $setup."
}
if ($RequireSignature) {
    & $signTool sign /fd SHA256 /tr $timestampEndpoint /td SHA256 `
        /sha1 $signingCertificate.Thumbprint $setup
    if ($LASTEXITCODE -ne 0) {
        throw "Could not sign $setup."
    }
    & $signTool verify /pa /tw $setup
    if ($LASTEXITCODE -ne 0) {
        throw "Signature verification failed for $setup."
    }
}
$hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([IO.Path]::GetFileName($setup))" | Set-Content -LiteralPath "$setup.sha256" -Encoding ascii
Write-Host "Created $setup"
Write-Host "Created $setup.sha256"
