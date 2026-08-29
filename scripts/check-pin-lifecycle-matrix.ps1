[CmdletBinding()]
param(
    [string]$OutputDirectory = "target\pin-lifecycle-matrix",
    [ValidateRange(3000, 900000)]
    [int]$TimeoutMilliseconds = 30000,
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

$combinations = @(
    [ordered]@{ Name = "en-dark"; Locale = "en"; Theme = "dark" },
    [ordered]@{ Name = "en-light"; Locale = "en"; Theme = "light" },
    [ordered]@{ Name = "zh-CN-dark"; Locale = "zh-CN"; Theme = "dark" },
    [ordered]@{ Name = "zh-CN-light"; Locale = "zh-CN"; Theme = "light" }
)
$entries = [System.Collections.Generic.List[object]]::new()

# Each combination gets a separate disposable profile and report directory.
foreach ($combination in $combinations) {
    $combinationOutput = Join-Path $outputPath $combination.Name
    $runnerParameters = @{
        OutputDirectory = $combinationOutput
        Locale = $combination.Locale
        Theme = $combination.Theme
        TimeoutMilliseconds = $TimeoutMilliseconds
        SettleMilliseconds = $SettleMilliseconds
    }
    if ($DebugBuild) {
        $runnerParameters["DebugBuild"] = $true
    }

    & (Join-Path $PSScriptRoot "check-pin-lifecycle-acceptance.ps1") @runnerParameters
    if ($LASTEXITCODE -ne 0) {
        throw "Pin lifecycle matrix failed for $($combination.Name) with exit code $LASTEXITCODE"
    }

    $session = Get-ChildItem -LiteralPath $combinationOutput -Directory |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $session) {
        throw "Pin lifecycle matrix did not create a session for $($combination.Name)"
    }
    $reportPath = Join-Path $session.FullName "report.json"
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw "Pin lifecycle matrix report is missing for $($combination.Name): $reportPath"
    }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    [void]$entries.Add([ordered]@{
            id = $combination.Name
            locale = [string]$report.locale
            theme = [string]$report.theme
            status = [string]$report.status
            report = $reportPath
            screenshots = @($report.screenshots | ForEach-Object {
                    Join-Path $session.FullName ([string]$_)
                })
        })
}

$matrixReportPath = Join-Path $outputPath "matrix-report.json"
[ordered]@{
    schema_version = 1
    test = "pin_lifecycle_matrix"
    status = "passed"
    debug_build = [bool]$DebugBuild
    combinations = $entries
} |
    ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $matrixReportPath -Encoding utf8

Write-Host "Pin lifecycle matrix passed: $matrixReportPath"
