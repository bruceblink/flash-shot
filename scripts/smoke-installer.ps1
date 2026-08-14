param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [ValidateRange(1, 30)]
    [int]$StartupSeconds = 5,
    [ValidateRange(10, 300)]
    [int]$OperationTimeoutSeconds = 120,
    [switch]$RequireSignature
)

$ErrorActionPreference = "Stop"

# Waits for a native setup process without allowing a broken installer to hang the release job.
function Wait-ReleaseProcess([Diagnostics.Process]$Process, [string]$Operation, [int]$TimeoutSeconds) {
    if (-not $Process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        $Process.WaitForExit()
        throw "$Operation timed out after $TimeoutSeconds seconds."
    }
    $Process.Refresh()
    if ($Process.ExitCode -ne 0) {
        throw "$Operation failed with exit code $($Process.ExitCode)."
    }
}

# Retries removal because Inno's temporary uninstaller can retain its log briefly after exit.
function Remove-SmokeStagingDirectory([string]$Path) {
    $lastError = $null
    for ($attempt = 1; $attempt -le 20; $attempt++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        }
        catch {
            $lastError = $_.Exception
            if ($attempt -lt 20) {
                Start-Sleep -Milliseconds 250
            }
        }
    }
    throw "Could not remove installer smoke directory '$Path': $($lastError.Message)"
}

# Waits for the temporary Inno uninstaller child to remove files, shortcuts, and registration.
function Wait-UninstallCleanup(
    [string]$InstallPath,
    [string]$ShortcutPath,
    [string]$RegistryPath,
    [int]$TimeoutSeconds
) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $remainingFiles = if (Test-Path -LiteralPath $InstallPath -PathType Container) {
            @(Get-ChildItem -LiteralPath $InstallPath -Force)
        }
        else {
            @()
        }
        if ($remainingFiles.Count -eq 0 -and
            -not (Test-Path -LiteralPath $ShortcutPath) -and
            -not (Test-Path -LiteralPath $RegistryPath)) {
            return
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)

    $remainingNames = @($remainingFiles | ForEach-Object { $_.Name })
    throw "Flash Shot uninstall left artifacts: files=[$($remainingNames -join ', ')], shortcut=$((Test-Path -LiteralPath $ShortcutPath)), registration=$((Test-Path -LiteralPath $RegistryPath))."
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}

$installer = if ([IO.Path]::IsPathRooted($InstallerPath)) {
    [IO.Path]::GetFullPath($InstallerPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path (Get-Location) $InstallerPath))
}
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw "Flash Shot installer does not exist: $installer"
}
$expectedInstallerName = "FlashShot-$($package.version)-windows-setup.exe"
if ([IO.Path]::GetFileName($installer) -cne $expectedInstallerName) {
    throw "Expected installer '$expectedInstallerName', found '$([IO.Path]::GetFileName($installer))'."
}
if ($RequireSignature) {
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne [Management.Automation.SignatureStatus]::Valid) {
        throw "Installer Authenticode signature is not valid: $($signature.Status)."
    }
}
if (Get-Process -Name "flash-shot" -ErrorAction SilentlyContinue) {
    throw "A Flash Shot process is already running. Close it before running the installer smoke test."
}
$uninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{BF3C499B-7D1B-4E5D-9E9B-7BF1A1E9297D}_is1"
$startMenuShortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Flash Shot.lnk"
foreach ($existingArtifact in @($uninstallRegistryPath, $startMenuShortcut)) {
    if (Test-Path -LiteralPath $existingArtifact) {
        throw "Refusing to disturb an existing Flash Shot installation artifact: $existingArtifact"
    }
}

$staging = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-installer-smoke-" + [guid]::NewGuid())
$installDirectory = Join-Path $staging "installed"
$profileDirectory = Join-Path $staging "profile"
$setupLog = Join-Path $staging "setup.log"
$uninstallLog = Join-Path $staging "uninstall.log"
$previousProfileDirectory = [Environment]::GetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", "Process")
$appProcess = $null
$setupProcess = $null
$uninstallProcess = $null
$uninstaller = Join-Path $installDirectory "unins000.exe"
$installed = $false
$uninstalled = $false

try {
    New-Item -ItemType Directory -Force -Path $staging, $profileDirectory | Out-Null
    $setupArguments = @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/SP-",
        "/CURRENTUSER",
        "/DIR=`"$installDirectory`"",
        "/LOG=`"$setupLog`""
    )
    $setupProcess = Start-Process -FilePath $installer -ArgumentList $setupArguments -PassThru
    Wait-ReleaseProcess $setupProcess "Installer smoke setup" $OperationTimeoutSeconds
    $installed = $true

    $executable = Join-Path $installDirectory "flash-shot.exe"
    foreach ($requiredFile in @($executable, $uninstaller, (Join-Path $installDirectory "LICENSE.txt"))) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            throw "Installer did not create required file: $requiredFile"
        }
    }
    if (-not (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf)) {
        throw "Installer did not create the current-user Start menu shortcut: $startMenuShortcut"
    }
    if (-not (Test-Path -LiteralPath $uninstallRegistryPath)) {
        throw "Installer did not create the current-user uninstall registration: $uninstallRegistryPath"
    }
    $uninstallRegistration = Get-ItemProperty -LiteralPath $uninstallRegistryPath
    if ([string]$uninstallRegistration.DisplayVersion -ne [string]$package.version) {
        throw "Installed version '$($uninstallRegistration.DisplayVersion)' did not match Cargo version '$($package.version)'."
    }
    if ($RequireSignature) {
        $signature = Get-AuthenticodeSignature -LiteralPath $executable
        if ($signature.Status -ne [Management.Automation.SignatureStatus]::Valid) {
            throw "Installed executable Authenticode signature is not valid: $($signature.Status)."
        }
    }

    [Environment]::SetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", $profileDirectory, "Process")
    $appProcess = Start-Process -FilePath $executable -WorkingDirectory $installDirectory -PassThru
    Start-Sleep -Seconds $StartupSeconds
    $appProcess.Refresh()
    if ($appProcess.HasExited) {
        throw "Installed Flash Shot exited during startup with exit code $($appProcess.ExitCode)."
    }
    foreach ($requiredDirectory in @("config", "data", "cache", "history")) {
        if (-not (Test-Path -LiteralPath (Join-Path $profileDirectory $requiredDirectory) -PathType Container)) {
            throw "Installed Flash Shot did not initialize isolated profile directory '$requiredDirectory'."
        }
    }

    Stop-Process -Id $appProcess.Id -Force
    $appProcess.WaitForExit()
    $appProcess = $null

    $uninstallArguments = @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/LOG=`"$uninstallLog`""
    )
    $uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList $uninstallArguments -PassThru
    Wait-ReleaseProcess $uninstallProcess "Installer smoke uninstall" $OperationTimeoutSeconds
    $uninstalled = $true

    if (Test-Path -LiteralPath $executable -PathType Leaf) {
        throw "Flash Shot executable remained after uninstall: $executable"
    }
    Wait-UninstallCleanup $installDirectory $startMenuShortcut $uninstallRegistryPath $OperationTimeoutSeconds

    Write-Host "Installer smoke test installed, started, and uninstalled Flash Shot $($package.version)."
}
finally {
    foreach ($process in @($appProcess, $setupProcess, $uninstallProcess)) {
        if ($null -ne $process) {
            $process.Refresh()
            if (-not $process.HasExited) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                $process.WaitForExit()
            }
        }
    }
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_PROFILE_DIR", $previousProfileDirectory, "Process")

    if ($installed -and -not $uninstalled -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        try {
            $cleanupProcess = Start-Process -FilePath $uninstaller -ArgumentList @(
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART"
            ) -PassThru
            Wait-ReleaseProcess $cleanupProcess "Installer smoke cleanup uninstall" $OperationTimeoutSeconds
        }
        catch {
            Write-Warning "Could not run cleanup uninstaller: $($_.Exception.Message)"
        }
    }

    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
    $resolvedStaging = [IO.Path]::GetFullPath($staging)
    $expectedPrefix = $tempRoot + [IO.Path]::DirectorySeparatorChar + "flash-shot-installer-smoke-"
    if (-not $resolvedStaging.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove unexpected installer smoke path: $resolvedStaging"
    }
    if (Test-Path -LiteralPath $resolvedStaging) {
        Remove-SmokeStagingDirectory $resolvedStaging
    }
}
