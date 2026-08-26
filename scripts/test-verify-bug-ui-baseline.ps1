$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$verify = Join-Path $PSScriptRoot "verify-bug-ui-baseline.ps1"
$fixture = Join-Path $root "target\bug-ui-baseline-test"
$output = Join-Path $fixture "first.json"
$repeatOutput = Join-Path $fixture "repeat.json"
$outsideRoot = Join-Path ([IO.Path]::GetTempPath()) ("flash-shot-baseline-test-" + [guid]::NewGuid())
$outsideOutput = Join-Path $outsideRoot "outside.json"

try {
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null

    & $verify -OutputPath $output
    if ($LASTEXITCODE -ne 0) {
        throw "Bug/UI baseline verifier rejected a valid repository output path."
    }
    & $verify -OutputPath $repeatOutput
    if ($LASTEXITCODE -ne 0) {
        throw "Bug/UI baseline verifier was not repeatable."
    }

    $first = Get-Content -LiteralPath $output -Raw | ConvertFrom-Json
    $repeat = Get-Content -LiteralPath $repeatOutput -Raw | ConvertFrom-Json
    if ($first.schema_version -ne 1 -or $first.baseline_id -ne "bug-ui-baseline-v1" -or
        $first.scenario_ids.Count -ne 7 -or $first.static_scan.direct_status_assignment_count -le 0 -or
        $first.static_scan.catalog_reference_count -le 0) {
        throw "Bug/UI baseline report did not contain the expected schema and inventory."
    }
    if ($first.repository_commit -ne $repeat.repository_commit -or
        $first.static_scan.direct_status_assignment_count -ne $repeat.static_scan.direct_status_assignment_count -or
        $first.static_scan.catalog_reference_count -ne $repeat.static_scan.catalog_reference_count) {
        throw "Bug/UI baseline report changed stable values across repeated runs."
    }

    New-Item -ItemType Directory -Force -Path $outsideRoot | Out-Null
    $failed = $false
    try {
        & $verify -OutputPath $outsideOutput
        $failed = $LASTEXITCODE -ne 0
    }
    catch {
        $failed = $true
    }
    if (-not $failed -or (Test-Path -LiteralPath $outsideOutput)) {
        throw "Bug/UI baseline verifier accepted or wrote outside-repository output."
    }
    Write-Host "Bug/UI baseline verifier tests passed"
}
finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
    if (Test-Path -LiteralPath $outsideRoot) {
        Remove-Item -LiteralPath $outsideRoot -Recurse -Force
    }
}
