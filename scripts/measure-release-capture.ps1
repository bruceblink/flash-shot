param(
    [ValidateRange(1, 100)]
    [int]$Iterations = 10,
    [ValidateRange(500, 10000)]
    [int]$SampleReadyWaitMilliseconds = 1000,
    [string]$OutputPath = "target\\release-capture-performance.json",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Get-UnixTimestampMilliseconds {
    return [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
}

function Get-CaptureSamplesSince([string]$Path, [Int64]$StartedAtMs) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return @()
    }
    return @(Get-Content -LiteralPath $Path | ForEach-Object {
        try { $_ | ConvertFrom-Json } catch { $null }
    } | Where-Object {
        $null -ne $_ -and $_.schema_version -eq 2 -and $_.build_profile -eq "release" -and
        $_.type -eq "capture_pipeline" -and $_.timestamp_ms -ge $StartedAtMs
    })
}

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root "Cargo.toml") | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "flash-shot" } | Select-Object -First 1
if ($null -eq $package) {
    throw "Cargo metadata did not contain the flash-shot package."
}
$releaseDirectory = Join-Path $metadata.target_directory "release"
$application = Join-Path $releaseDirectory "flash-shot.exe"
$reporter = Join-Path $releaseDirectory "performance-report.exe"
if (-not $SkipBuild) {
    & cargo build --release --bin flash-shot --bin performance-report --manifest-path $package.manifest_path
    if ($LASTEXITCODE -ne 0) {
        throw "Release performance binaries failed to build with exit code $LASTEXITCODE."
    }
}
foreach ($path in @($application, $reporter)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Release performance binary was not found: $path"
    }
}
$protocol = & $reporter --protocol-version
if ($LASTEXITCODE -ne 0 -or $protocol -ne "performance-report-v3") {
    throw "Release performance-report binary does not support capture sampling. Run without -SkipBuild after source changes."
}

$existing = @(Get-Process -Name "flash-shot" -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "A Flash Shot process is already running. Close it before measuring release capture latency."
}

$metrics = Join-Path $env:APPDATA "BruceBlink\\Flash Shot\\data\\metrics\\performance.jsonl"
$captureShortcut = "Ctrl+Alt+F12"
$startedAtMs = Get-UnixTimestampMilliseconds
$previousShortcut = [Environment]::GetEnvironmentVariable("FLASH_SHOT_CAPTURE_HOTKEY", "Process")
[Environment]::SetEnvironmentVariable("FLASH_SHOT_CAPTURE_HOTKEY", $captureShortcut, "Process")
$process = $null
try {
    $process = Start-Process -FilePath $application -WorkingDirectory $releaseDirectory -PassThru
    Start-Sleep -Seconds 2
    $process.Refresh()
    if ($process.HasExited) {
        throw "Release Flash Shot exited during capture measurement startup with exit code $($process.ExitCode)."
    }

    for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
        $before = @(Get-CaptureSamplesSince $metrics $startedAtMs).Count
        $shell = New-Object -ComObject WScript.Shell
        $shell.SendKeys("^%{F12}")
        # Avoid repeatedly opening the JSONL during the application's atomic
        # replacement write: Windows does not allow that rename while a reader
        # without delete sharing is active.
        Start-Sleep -Milliseconds $SampleReadyWaitMilliseconds
        $samples = @(Get-CaptureSamplesSince $metrics $startedAtMs)
        if ($samples.Count -le $before) {
            throw "Release capture iteration $iteration did not append a current capture pipeline sample after $captureShortcut."
        }
        $sample = $samples[-1]
        if ($sample.latency_ms.shortcut_to_frame_ready -lt 0 -or $sample.latency_ms.shortcut_to_overlay_frame -lt 0) {
            throw "Release capture iteration $iteration wrote invalid latency values."
        }
        if (-not $shell.AppActivate($process.Id)) {
            throw "Could not activate Release Flash Shot to close capture iteration $iteration."
        }
        $shell.SendKeys("{ESC}")
        Start-Sleep -Milliseconds 1000
    }
}
finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_CAPTURE_HOTKEY", $previousShortcut, "Process")
}

$output = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputPath))
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $output) | Out-Null
& $reporter --input $metrics --since-ms $startedAtMs --minimum-samples $Iterations --capture-only --output $output
if ($LASTEXITCODE -eq 1) {
    throw "Release capture performance report was malformed or did not include all current samples."
}
if ($LASTEXITCODE -eq 2) {
    throw "Release capture p95 exceeded its configured limit."
}
if ($LASTEXITCODE -ne 0) {
    throw "Release capture performance report failed with exit code $LASTEXITCODE."
}

Write-Host "Measured $Iterations Release capture samples with $captureShortcut and wrote $output"
