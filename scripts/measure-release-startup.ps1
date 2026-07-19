param(
    [ValidateRange(10, 100)]
    [int]$Iterations = 10,
    [ValidateRange(1, 30)]
    [int]$StartupWaitSeconds = 1,
    [string]$OutputPath = "target\\release-startup-performance.json",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Get-UnixTimestampMilliseconds {
    return [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
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
if ($SkipBuild) {
    $protocol = & $reporter --protocol-version
    if ($LASTEXITCODE -ne 0 -or $protocol -ne "performance-report-v3") {
        throw "-SkipBuild requires a current Release performance-report binary. Run without -SkipBuild after source changes."
    }
}

$existing = @(Get-Process -Name "flash-shot" -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "A Flash Shot process is already running. Close it before measuring release startup."
}

$metrics = Join-Path $env:APPDATA "BruceBlink\\Flash Shot\\data\\metrics\\performance.jsonl"
$startedAtMs = Get-UnixTimestampMilliseconds
$recorded = 0
for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
    $process = Start-Process -FilePath $application -WorkingDirectory $releaseDirectory -PassThru
    try {
        Start-Sleep -Seconds $StartupWaitSeconds
        $process.Refresh()
        if ($process.HasExited) {
            throw "Release Flash Shot exited during startup iteration $iteration with exit code $($process.ExitCode)."
        }
    }
    finally {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }

    if (-not (Test-Path -LiteralPath $metrics -PathType Leaf)) {
        throw "Release startup iteration $iteration did not create the performance metrics file."
    }
    $sample = Get-Content -LiteralPath $metrics | Select-Object -Last 1 | ConvertFrom-Json
    if ($sample.schema_version -ne 2 -or $sample.build_profile -ne "release" -or
        $sample.type -ne "duration" -or $sample.metric -ne "startup_to_first_frame" -or
        $sample.timestamp_ms -lt $startedAtMs) {
        throw "Release startup iteration $iteration did not append a current Release startup sample."
    }
    $recorded++
}

$output = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputPath))
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $output) | Out-Null
& $reporter --input $metrics --since-ms $startedAtMs --minimum-samples $Iterations --startup-only --output $output
if ($LASTEXITCODE -eq 1) {
    throw "Release startup performance report was malformed or did not include all current samples."
}
if ($LASTEXITCODE -eq 2) {
    throw "Release startup performance p95 exceeded its configured limit."
}
if ($LASTEXITCODE -ne 0) {
    throw "Release startup performance report failed with exit code $LASTEXITCODE."
}

Write-Host "Measured $recorded Release startup samples and wrote $output"
