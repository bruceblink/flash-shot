[CmdletBinding()]
param(
    [switch]$Release,

    [string]$OutputDir,

    [ValidateRange(500, 60000)]
    [int]$SettleMs = 1500,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $repositoryRoot ("target\ui-acceptance\w6-workspace-matrix-{0}" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
}
$outputRoot = [System.IO.Path]::GetFullPath($OutputDir)
$runner = Join-Path $repositoryRoot "scripts\run-dev-tool.ps1"

$themes = @("dark", "light")
$locales = @("en", "zh-CN")
$sizes = @(
    [ordered]@{ name = "420x420"; width = 420; height = 420 },
    [ordered]@{ name = "520x640"; width = 520; height = 640 },
    [ordered]@{ name = "980x760"; width = 980; height = 760 }
)
$surfaces = @(
    "overlay-control",
    "overlay-window",
    "overlay-selection",
    "overlay-selection-more",
    "overlay-selection-bottom-right-more",
    "overlay-marking",
    "overlay-tool-group"
)

# Normalize generated text so reports remain portable across Windows shells.
function Write-LfUtf8 {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [string]$Text
    )

    $normalized = $Text.Replace("`r`n", "`n").Replace("`r", "`n")
    [System.IO.File]::WriteAllText($Path, $normalized, [System.Text.UTF8Encoding]::new($false))
}

# Serialize one matrix report with stable UTF-8/LF output for later review.
function Write-Report {
    param(
        [Parameter(Mandatory)]
        [string]$Path,

        [Parameter(Mandatory)]
        [object]$Report
    )

    Write-LfUtf8 -Path $Path -Text ($Report | ConvertTo-Json -Depth 16)
}

# Expand the first W6 slice into every theme, locale, size, and overlay surface.
$cases = @()
foreach ($theme in $themes) {
    foreach ($locale in $locales) {
        foreach ($size in $sizes) {
            foreach ($surface in $surfaces) {
                $cases += [ordered]@{
                    case_id = "{0}-{1}-{2}-{3}" -f $theme, $locale, $size.name, $surface
                    theme = $theme
                    locale = $locale
                    size = $size.name
                    width = $size.width
                    height = $size.height
                    surface = $surface
                }
            }
        }
    }
}

if ($DryRun) {
    $dryRunReport = [ordered]@{
        schema_version = 1
        workflow = "w6_workspace_state_matrix"
        status = "dry_run"
        case_count = $cases.Count
        cases = $cases
    }
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    Write-Report -Path (Join-Path $outputRoot "matrix-report.json") -Report $dryRunReport
    Write-Output ("W6 workspace matrix dry run: {0} cases -> {1}" -f $cases.Count, (Join-Path $outputRoot "matrix-report.json"))
    exit 0
}

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$results = @()
$failures = @()

foreach ($case in $cases) {
    $screenshotPath = Join-Path $outputRoot ("{0}.png" -f $case.case_id)
    $metadataPath = [System.IO.Path]::ChangeExtension($screenshotPath, ".json")
    $toolArguments = @(
        $case.theme,
        [string]$case.width,
        [string]$case.height,
        $screenshotPath,
        [string]$SettleMs,
        "0",
        "1.0",
        "capture",
        "0",
        "idle",
        "translation-idle",
        "ocr-idle",
        "recording-support-idle",
        "update-idle",
        $case.surface,
        $case.locale
    )

    Write-Output ("[{0}/{1}] {2}" -f ($results.Count + $failures.Count + 1), $cases.Count, $case.case_id)
    try {
        if ($Release) {
            & $runner -Tool "settings-ui-acceptance" -Release @toolArguments
        }
        else {
            & $runner -Tool "settings-ui-acceptance" @toolArguments
        }
        $caseExitCode = $LASTEXITCODE
        if ($caseExitCode -ne 0) {
            throw "settings-ui-acceptance exited with code $caseExitCode"
        }
        if (-not (Test-Path -LiteralPath $screenshotPath)) {
            throw "screenshot was not created: $screenshotPath"
        }
        if (-not (Test-Path -LiteralPath $metadataPath)) {
            throw "screenshot metadata was not created: $metadataPath"
        }
        $metadata = Get-Content -Raw -LiteralPath $metadataPath | ConvertFrom-Json
        if ($metadata.locale -ne $case.locale) {
            throw "metadata locale '$($metadata.locale)' does not match '$($case.locale)'"
        }
        if ($metadata.scale_match -ne $true) {
            throw "metadata scale_match is not true"
        }
        $observedWidth = [int]$metadata.physical_bounds.right - [int]$metadata.physical_bounds.left
        $observedHeight = [int]$metadata.physical_bounds.bottom - [int]$metadata.physical_bounds.top
        if ($observedWidth -le 0 -or $observedHeight -le 0) {
            throw "metadata physical bounds are empty"
        }
        $results += [ordered]@{
            case_id = $case.case_id
            theme = $case.theme
            locale = $case.locale
            size = $case.size
            surface = $case.surface
            screenshot = $screenshotPath
            metadata = $metadataPath
            dpi = [int]$metadata.dpi
            scale_factor = [double]$metadata.scale_factor
            physical_width = $observedWidth
            physical_height = $observedHeight
            status = "passed"
        }
    }
    catch {
        $failures += [ordered]@{
            case_id = $case.case_id
            theme = $case.theme
            locale = $case.locale
            size = $case.size
            surface = $case.surface
            status = "failed"
            error = $_.Exception.Message
        }
    }
}

$report = [ordered]@{
    schema_version = 1
    workflow = "w6_workspace_state_matrix"
    status = if ($failures.Count -eq 0) { "passed" } else { "failed" }
    release = [bool]$Release
    settle_ms = $SettleMs
    case_count = $cases.Count
    passed_count = $results.Count
    failed_count = $failures.Count
    themes = $themes
    locales = $locales
    sizes = $sizes
    surfaces = $surfaces
    cases = $results
    failures = $failures
}
$reportPath = Join-Path $outputRoot "matrix-report.json"
Write-Report -Path $reportPath -Report $report
Write-Output ("W6 workspace matrix {0}: {1} passed, {2} failed -> {3}" -f $report.status, $results.Count, $failures.Count, $reportPath)

if ($failures.Count -ne 0) {
    exit 1
}
exit 0
