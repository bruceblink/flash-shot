param(
    [string]$OutputDirectory = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}

$rustHost = (& cargo -vV | Where-Object { $_ -like "host:*" } | Select-Object -First 1).Replace("host: ", "")
if ($rustHost -notmatch "-pc-windows-msvc$") {
    throw "Portable Windows packaging requires an MSVC Windows Rust host; found $rustHost."
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

$output = [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
New-Item -ItemType Directory -Force -Path $output | Out-Null
$name = "FlashShot-$($package.version)-windows-$($rustHost.Split('-')[0])"
$archive = Join-Path $output "$name.zip"
$checksum = "$archive.sha256"
$staging = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-package-" + [guid]::NewGuid())
$packageRoot = Join-Path $staging $name

try {
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $packageRoot "flash-shot.exe")
    Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $packageRoot "LICENSE.txt")
    Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $packageRoot "README.md")

    @(
        "Flash Shot portable package",
        "",
        "Run flash-shot.exe directly. No installer or elevated privileges are required.",
        "FFmpeg is intentionally not bundled. Install a compatible FFmpeg build or set FLASH_SHOT_FFMPEG to its executable path before recording.",
        "Version: $($package.version)",
        "Target: $rustHost"
    ) | Set-Content -LiteralPath (Join-Path $packageRoot "PORTABLE.txt") -Encoding ascii

    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive -Force
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath $checksum -Encoding ascii
    & (Join-Path $PSScriptRoot "verify-portable-package.ps1") -ArchivePath $archive
    if ($LASTEXITCODE -ne 0) {
        throw "Portable package verification failed with exit code $LASTEXITCODE."
    }
}
finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

Write-Host "Created $archive"
Write-Host "Created $checksum"
