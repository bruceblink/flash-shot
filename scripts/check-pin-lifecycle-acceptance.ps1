param(
    [string]$OutputDirectory = "target\pin-lifecycle-acceptance",
    [ValidateSet("en", "zh-CN")]
    [string]$Locale = "en",
    [ValidateSet("dark", "light")]
    [string]$Theme = "dark",
    [ValidateRange(3000, 900000)]
    [int]$TimeoutMilliseconds = 20000,
    [ValidateRange(100, 3000)]
    [int]$SettleMilliseconds = 700,
    [ValidateRange(0, 600000)]
    [int]$SoakMilliseconds = 0,
    [switch]$DebugBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($SoakMilliseconds -gt 0 -and $SoakMilliseconds -lt 10000) {
    throw "SoakMilliseconds must be zero or at least 10000"
}
if ($SoakMilliseconds -gt 0 -and $TimeoutMilliseconds -lt ($SoakMilliseconds + 10000)) {
    throw "TimeoutMilliseconds must allow at least 10000 ms beyond the soak duration"
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputPath = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputDirectory))
}
$startedAt = (Get-Date).AddSeconds(-1)
$runnerArguments = @{
    Tool = "pin-lifecycle-acceptance"
    ToolArguments = @(
        "--output-dir", $outputPath,
        "--locale", $Locale,
        "--theme", $Theme,
        "--timeout-ms", $TimeoutMilliseconds,
        "--settle-ms", $SettleMilliseconds
    )
}
if (-not $DebugBuild) {
    $runnerArguments["Release"] = $true
}
if ($SoakMilliseconds -gt 0) {
    $runnerArguments["ToolArguments"] += @("--soak-ms", $SoakMilliseconds)
}

Push-Location $repositoryRoot
try {
    & (Join-Path $PSScriptRoot "run-dev-tool.ps1") @runnerArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Pin lifecycle acceptance exited with code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$session = Get-ChildItem -LiteralPath $outputPath -Directory |
    Where-Object { $_.LastWriteTime -ge $startedAt } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if ($null -eq $session) {
    throw "Pin lifecycle acceptance did not create a current evidence session"
}

$reportPath = Join-Path $session.FullName "report.json"
if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
    throw "Pin lifecycle acceptance report is missing: $reportPath"
}
$report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
if ($report.status -ne "passed" -or $null -ne $report.error) {
    throw "Pin lifecycle acceptance report did not pass: $($report.error)"
}
if ($report.schema_version -ne 5 -or $report.locale -ne $Locale -or $report.theme -ne $Theme) {
    throw "Pin lifecycle acceptance report does not match locale/theme $Locale/$Theme"
}
if (-not $report.system_services_disabled -or $report.windows.Count -ne 3) {
    throw "Pin lifecycle acceptance did not preserve its isolated three-window boundary"
}
if ($report.zoom.after.right - $report.zoom.after.left -le $report.zoom.before.right - $report.zoom.before.left -or
    $report.zoom.after.bottom - $report.zoom.after.top -le $report.zoom.before.bottom - $report.zoom.before.top) {
    throw "Pin lifecycle acceptance did not observe a larger native window after zoom"
}
if ($report.opacity.expected_alpha -ne 191 -or $report.opacity.observed_alpha -ne 191) {
    throw "Pin lifecycle acceptance did not observe the expected 75 percent opacity"
}
if ($report.copy.calls -ne 1 -or -not $report.copy.complete_frame_equal) {
    throw "Pin lifecycle acceptance did not preserve the complete in-memory copied frame"
}
if ($report.save.source -ne "pinned" -or -not (Test-Path -LiteralPath ([string]$report.save.path) -PathType Leaf)) {
    throw "Pin lifecycle acceptance did not create its isolated pinned-history PNG"
}
if ($report.solo.visible_count -ne 1 -or $report.show_all.visible_count -ne 3 -or
    -not $report.show_all_preserved_focus -or
    $report.registered_windows_after_solo -ne 3 -or $report.registered_windows_after_show_all -ne 3) {
    throw "Pin lifecycle acceptance damaged focus or the registry during Solo or Show all"
}
if ($report.live_windows_after_close -ne 2 -or -not $report.capture_preflight_ready) {
    throw "Pin lifecycle acceptance did not keep two Pins and capture preflight ready after Close"
}
$expectedScreenshotCount = if ($SoakMilliseconds -gt 0) { 3 } else { 2 }
if ($report.screenshots.Count -ne $expectedScreenshotCount) {
    throw "Pin lifecycle acceptance reported $($report.screenshots.Count) screenshots, expected $expectedScreenshotCount"
}
foreach ($relativeScreenshot in $report.screenshots) {
    $screenshotPath = Join-Path $session.FullName ([string]$relativeScreenshot)
    if (-not (Test-Path -LiteralPath $screenshotPath -PathType Leaf)) {
        throw "Pin lifecycle screenshot is missing: $screenshotPath"
    }
}

if ($SoakMilliseconds -gt 0) {
    if ($null -eq $report.soak) {
        throw "Pin lifecycle acceptance did not report the requested soak phase"
    }
    if ($report.soak.requested_duration_ms -ne $SoakMilliseconds -or
        $report.soak.elapsed_ms -lt $SoakMilliseconds -or
        $report.soak.cycles_completed -lt 1 -or
        $report.soak.samples.Count -ne $report.soak.cycles_completed) {
        throw "Pin lifecycle soak duration or sample count is incomplete"
    }
    if ($report.soak.registry_min -ne 3 -or $report.soak.registry_max -ne 3 -or
        $report.soak.frame_checks -ne (3 * $report.soak.cycles_completed) -or
        $report.soak.preflight_checks -ne $report.soak.cycles_completed -or
        -not $report.soak.focus_preserved) {
        throw "Pin lifecycle soak did not preserve the registry, source frames, focus, or Capture preflight"
    }
    if ($report.soak.working_set_start_bytes -le 0 -or
        $report.soak.working_set_end_bytes -le 0 -or
        $report.soak.working_set_peak_bytes -lt $report.soak.working_set_start_bytes -or
        $report.soak.working_set_peak_bytes -lt $report.soak.working_set_end_bytes -or
        $report.soak.private_commit_start_bytes -le 0 -or
        $report.soak.private_commit_end_bytes -le 0 -or
        $report.soak.private_commit_peak_bytes -lt $report.soak.private_commit_start_bytes -or
        $report.soak.private_commit_peak_bytes -lt $report.soak.private_commit_end_bytes) {
        throw "Pin lifecycle soak did not report valid process resource samples"
    }
    foreach ($sample in $report.soak.samples) {
        if ($sample.registered_windows -ne 3 -or $sample.solo_visible_count -ne 1 -or
            $sample.show_all_visible_count -ne 3 -or -not $sample.source_frames_equal -or
            -not $sample.bounds_valid -or -not $sample.capture_preflight_ready -or
            -not $sample.focus_preserved -or $sample.native_bounds.Count -ne 3 -or
            $sample.working_set_bytes -le 0 -or $sample.private_commit_bytes -le 0) {
            throw "Pin lifecycle soak sample $($sample.cycle) failed a lifecycle invariant"
        }
    }
} elseif ($null -ne $report.soak) {
    throw "Pin lifecycle acceptance unexpectedly ran a soak phase"
}

Write-Host "Pin lifecycle acceptance passed: $reportPath"
