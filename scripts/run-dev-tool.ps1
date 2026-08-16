[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet(
        "annotation-stress",
        "capture-stress",
        "copy-performance",
        "export-stress",
        "history-resource-acceptance",
        "overlay-copy-batch",
        "overlay-interaction-acceptance",
        "performance-report",
        "pin-lifecycle-acceptance",
        "png-stress",
        "recognition-acceptance",
        "recording-acceptance",
        "scroll-acceptance",
        "settings-ui-acceptance",
        "windows-acceptance-probe"
    )]
    [string]$Tool,

    [switch]$Release,

    [Parameter(ValueFromRemainingArguments)]
    [string[]]$ToolArguments
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$targetDirectory = Join-Path $repositoryRoot "target\dev-tools"
$cargoArguments = @(
    "run",
    "--locked",
    "--target-dir", $targetDirectory,
    "--features", "dev-tools"
)
if ($Release) {
    $cargoArguments += "--release"
}
$cargoArguments += "--"
$cargoArguments += $ToolArguments

$previousTool = [Environment]::GetEnvironmentVariable("FLASH_SHOT_DEV_TOOL", "Process")
$exitCode = 1
Push-Location $repositoryRoot
try {
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_DEV_TOOL", $Tool, "Process")
    & cargo @cargoArguments
    $exitCode = $LASTEXITCODE
}
finally {
    [Environment]::SetEnvironmentVariable("FLASH_SHOT_DEV_TOOL", $previousTool, "Process")
    Pop-Location
}

exit $exitCode
