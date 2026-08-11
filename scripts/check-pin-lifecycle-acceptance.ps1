param(
    [string]$OutputDirectory = "target\pin-lifecycle-acceptance",
    [ValidateRange(3000, 60000)]
    [int]$TimeoutMilliseconds = 20000,
    [ValidateRange(100, 3000)]
    [int]$SettleMilliseconds = 700,
    [switch]$DebugBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$outputPath = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputDirectory))
}
$startedAt = (Get-Date).AddSeconds(-1)
$cargoArguments = @("run")
if (-not $DebugBuild) {
    $cargoArguments += "--release"
}
$cargoArguments += @(
    "--bin", "pin-lifecycle-acceptance", "--",
    "--output-dir", $outputPath,
    "--timeout-ms", $TimeoutMilliseconds,
    "--settle-ms", $SettleMilliseconds
)

Push-Location $repositoryRoot
try {
    & cargo @cargoArguments
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
if ($report.screenshots.Count -ne 2) {
    throw "Pin lifecycle acceptance did not report both native screenshots"
}
foreach ($relativeScreenshot in $report.screenshots) {
    $screenshotPath = Join-Path $session.FullName ([string]$relativeScreenshot)
    if (-not (Test-Path -LiteralPath $screenshotPath -PathType Leaf)) {
        throw "Pin lifecycle screenshot is missing: $screenshotPath"
    }
}

Write-Host "Pin lifecycle acceptance passed: $reportPath"
