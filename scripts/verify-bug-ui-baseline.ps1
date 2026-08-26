[CmdletBinding()]
param(
    [string]$OutputPath = "target\bug-ui-baseline\bug-ui-baseline.json"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$output = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputPath))
}
$repositoryPrefix = $repositoryRoot.TrimEnd("\") + "\"
if (-not $output.StartsWith($repositoryPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputPath must stay inside the repository."
}

function Get-RepositoryRelativePath([string]$path) {
    if (-not $path.StartsWith($repositoryPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Source path is outside the repository: $path"
    }
    return $path.Substring($repositoryPrefix.Length).Replace("\", "/")
}

$scenarioIds = @(
    "baseline-capture-core-single-100",
    "baseline-annotation-text-watermark",
    "baseline-annotation-two-arrows",
    "baseline-annotation-export-4k",
    "baseline-ocr-optional-retry",
    "baseline-repeated-capture-cleanup",
    "baseline-visible-status-inventory"
)
$baselineDocument = Join-Path $repositoryRoot "docs\bug-ui-baseline.md"
if (-not (Test-Path -LiteralPath $baselineDocument -PathType Leaf)) {
    throw "Baseline document is missing: $baselineDocument"
}
$documentText = Get-Content -LiteralPath $baselineDocument -Raw
foreach ($scenarioId in $scenarioIds) {
    $scenarioMarker = '`' + $scenarioId + '`'
    if (-not $documentText.Contains($scenarioMarker)) {
        throw "Baseline document is missing scenario ID: $scenarioId"
    }
}

$runner = Join-Path $repositoryRoot "scripts\run-dev-tool.ps1"
$runnerText = Get-Content -LiteralPath $runner -Raw
$requiredTools = @("annotation-stress", "overlay-interaction-acceptance", "recognition-acceptance")
foreach ($tool in $requiredTools) {
    $toolMarker = '"' + $tool + '"'
    if (-not $runnerText.Contains($toolMarker)) {
        throw "Development runner no longer exposes required tool: $tool"
    }
}

# Collect stable file/line references so U1 can migrate status sources without hand-counting them.
$sourceFiles = @(
    (Join-Path $repositoryRoot "crates\flash-shot-app\src\app\workflow.rs")
)
$sourceFiles += Get-ChildItem (Join-Path $repositoryRoot "crates\flash-shot-app\src\app\workflow") -Filter "*.rs" |
    Sort-Object FullName |
    Select-Object -ExpandProperty FullName
$directStatusAssignments = @()
$catalogReferences = @()
$hardcodedStatusAssignments = @()
foreach ($sourceFile in $sourceFiles) {
    $lines = @(Get-Content -LiteralPath $sourceFile)
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $line = [string]$lines[$index]
        $relativePath = Get-RepositoryRelativePath $sourceFile
        if ($line -match "self\.status\s*=") {
            $entry = [ordered]@{
                path = $relativePath
                line = $index + 1
                source = $line.Trim()
            }
            $directStatusAssignments += [pscustomobject]$entry
            if ($line -match 'self\.status\s*=\s*(?:format!\s*\(\s*)?\x22') {
                $hardcodedStatusAssignments += [pscustomobject]$entry
            }
        }
        if ($line -match "UiText::") {
            $catalogReferences += [pscustomobject][ordered]@{
                path = $relativePath
                line = $index + 1
                source = $line.Trim()
            }
        }
    }
}

$commit = (& git -C $repositoryRoot rev-parse HEAD).Trim()
$versionLine = Select-String -LiteralPath (Join-Path $repositoryRoot "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1
$version = if ($null -ne $versionLine) { $versionLine.Matches[0].Groups[1].Value } else { "unknown" }
$report = [ordered]@{
    schema_version = 1
    baseline_id = "bug-ui-baseline-v1"
    generated_at_utc = [DateTimeOffset]::UtcNow.ToString("O")
    repository_commit = $commit
    application_version = $version
    source_root = "crates/flash-shot-app/src/app/workflow"
    scenario_ids = $scenarioIds
    static_scan = [ordered]@{
        direct_status_assignment_count = $directStatusAssignments.Count
        hardcoded_status_assignment_count = $hardcodedStatusAssignments.Count
        catalog_reference_count = $catalogReferences.Count
        direct_status_assignments = $directStatusAssignments
        hardcoded_status_assignments = $hardcodedStatusAssignments
        catalog_references = $catalogReferences
    }
}

$outputDirectory = Split-Path -Parent $output
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $output -Encoding utf8
Write-Host "Bug/UI baseline verified: $output"
Write-Host "Scenarios: $($scenarioIds.Count); direct status assignments: $($directStatusAssignments.Count); catalog references: $($catalogReferences.Count)"
