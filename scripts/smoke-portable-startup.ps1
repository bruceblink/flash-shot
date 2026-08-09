param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath,
    [ValidateRange(1, 30)]
    [int]$StartupSeconds = 5
)

$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$archive = [IO.Path]::GetFullPath($ArchivePath)
$verify = Join-Path $PSScriptRoot "verify-portable-package.ps1"
& $verify -ArchivePath $archive
if ($LASTEXITCODE -ne 0) {
    throw "Portable package verification failed with exit code $LASTEXITCODE."
}

if (Get-Process -Name "flash-shot" -ErrorAction SilentlyContinue) {
    throw "A Flash Shot process is already running. Close it before running the portable startup smoke test."
}

$packageRoot = [IO.Path]::GetFileNameWithoutExtension($archive)
$staging = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-portable-smoke-" + [guid]::NewGuid())
$profileDirectory = Join-Path $staging "profile"
$previousProfileDirectory = [Environment]::GetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", "Process")
$process = $null
try {
    Expand-Archive -LiteralPath $archive -DestinationPath $staging
    $executable = Join-Path (Join-Path $staging $packageRoot) "flash-shot.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Portable archive has no flash-shot executable at the expected path."
    }

    New-Item -ItemType Directory -Force -Path $profileDirectory | Out-Null
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", $profileDirectory, "Process")
    $process = Start-Process -FilePath $executable -WorkingDirectory (Split-Path -Parent $executable) -PassThru
    Start-Sleep -Seconds $StartupSeconds
    $process.Refresh()
    if ($process.HasExited) {
        throw "Portable Flash Shot exited during startup with exit code $($process.ExitCode)."
    }
    foreach ($requiredDirectory in @("config", "data", "cache", "history")) {
        if (-not (Test-Path -LiteralPath (Join-Path $profileDirectory $requiredDirectory) -PathType Container)) {
            throw "Portable Flash Shot did not initialize isolated profile directory '$requiredDirectory'."
        }
    }

    Write-Host "Portable Flash Shot stayed running for $StartupSeconds seconds with an isolated profile."
}
finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", $previousProfileDirectory, "Process")
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}
