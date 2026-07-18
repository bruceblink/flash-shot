param(
    [string]$OutputDirectory = "dist",
    [switch]$SkipBuild,
    [switch]$RequireSignature,
    [switch]$ValidateOnly
)

$ErrorActionPreference = "Stop"

function Get-CommandPath([string]$Name, [string[]]$Candidates) {
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

$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path $manifest | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}
$rustHost = (& cargo -vV | Where-Object { $_ -like "host:*" } | Select-Object -First 1).Replace("host: ", "")
if ($rustHost -notmatch "-pc-windows-msvc$") {
    throw "Windows installer packaging requires an MSVC Windows Rust host; found $rustHost."
}

if ($ValidateOnly) {
    if ((Get-Content -Raw $installer) -notmatch "MyAppVersion") {
        throw "Installer script does not accept a version from Cargo metadata."
    }
    Write-Host "Installer configuration is valid for Flash Shot $($package.version) on $rustHost."
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

$signTool = $null
if ($RequireSignature) {
    $signTool = Get-CommandPath "signtool.exe" @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\x64\signtool.exe",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\x86\signtool.exe"
    )
    if ($null -eq $signTool) {
        throw "-RequireSignature needs signtool.exe and a usable code-signing certificate."
    }
    & $signTool sign /fd SHA256 /tr https://timestamp.digicert.com /td SHA256 /a $executable
    if ($LASTEXITCODE -ne 0) {
        throw "Could not sign $executable."
    }
    & $signTool verify /pa $executable
    if ($LASTEXITCODE -ne 0) {
        throw "Signature verification failed for $executable."
    }
}

$iscc = Get-CommandPath "ISCC.exe" @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)
if ($null -eq $iscc) {
    throw "Inno Setup 6 is required. Install it or make ISCC.exe available on PATH."
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

    & $iscc "/DMyAppVersion=$($package.version)" "/DMySourceDir=$staging" "/O$output" $installer
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
    & $signTool sign /fd SHA256 /tr https://timestamp.digicert.com /td SHA256 /a $setup
    if ($LASTEXITCODE -ne 0) {
        throw "Could not sign $setup."
    }
    & $signTool verify /pa $setup
    if ($LASTEXITCODE -ne 0) {
        throw "Signature verification failed for $setup."
    }
}
$hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([IO.Path]::GetFileName($setup))" | Set-Content -LiteralPath "$setup.sha256" -Encoding ascii
Write-Host "Created $setup"
Write-Host "Created $setup.sha256"
