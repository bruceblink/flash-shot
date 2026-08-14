param(
    [string]$OutputDirectory = "target\inno-languages"
)

$ErrorActionPreference = "Stop"

$sourceCommit = "3cfb0e5632828e0dd9b49400a185834e8f1ab570"
$sourceRepository = "https://github.com/jrsoftware/issrc.git"
$sourcePath = "Files/Languages/ChineseSimplified.isl"
$expectedSha256 = "e0b0b350e2245f3c5e65586dfe43d574f6e7f06f2261149aba284954b3fc9a8d"

# Verifies both the pinned bytes and the Simplified Chinese language identity before packaging.
function Assert-InnoSimplifiedChineseLanguage([string]$Path, [string]$ExpectedSha256) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Inno Setup Simplified Chinese language file is missing: $Path"
    }
    if ($ExpectedSha256 -notmatch "^[0-9a-fA-F]{64}$") {
        throw "Expected Inno Setup language SHA-256 must contain exactly 64 hexadecimal characters."
    }
    $actualSha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $ExpectedSha256.ToLowerInvariant()) {
        throw "Inno Setup Simplified Chinese language SHA-256 mismatch: expected $ExpectedSha256, found $actualSha256."
    }
    $content = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    if ($content -notmatch '(?m)^LanguageID=\$0804\s*$' -or
        $content -notmatch '(?m)^\[Messages\]\s*$' -or
        $content -notmatch '(?m)^SetupAppTitle=') {
        throw "Downloaded Inno Setup language file is not a Simplified Chinese messages file."
    }
}

# Fetches one pinned Git blob and copies its raw bytes without PowerShell text re-encoding.
function Export-PinnedGitBlob(
    [string]$Repository,
    [string]$Commit,
    [string]$Path,
    [string]$Destination
) {
    $git = Get-Command git.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $git) {
        throw "git.exe is required to fetch the pinned Inno Setup language file."
    }

    $cache = Join-Path (Split-Path -Parent $Destination) ".issrc-language.git"
    if (-not (Test-Path -LiteralPath $cache -PathType Container)) {
        & $git.Source init --bare --quiet $cache
        if ($LASTEXITCODE -ne 0) {
            throw "Could not initialize the Inno Setup language Git cache."
        }
    }
    $remotes = @(& $git.Source -C $cache remote)
    if ($remotes -notcontains "origin") {
        & $git.Source -C $cache remote add origin $Repository
    }
    else {
        $origin = (& $git.Source -C $cache remote get-url origin).Trim()
        if ($origin -ne $Repository) {
            & $git.Source -C $cache remote set-url origin $Repository
        }
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Could not configure the Inno Setup language Git source."
    }

    & $git.Source -C $cache -c http.lowSpeedLimit=1 -c http.lowSpeedTime=30 `
        fetch --quiet --depth=1 origin $Commit
    if ($LASTEXITCODE -ne 0) {
        throw "Could not fetch pinned Inno Setup source commit $Commit."
    }
    $blob = (& $git.Source -C $cache rev-parse "$Commit`:$Path").Trim()
    if ($LASTEXITCODE -ne 0 -or $blob -notmatch "^[0-9a-fA-F]{40,64}$") {
        throw "Pinned Inno Setup source does not contain $Path."
    }

    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $git.Source
    $info.Arguments = "cat-file blob $blob"
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.EnvironmentVariables["GIT_DIR"] = $cache
    $process = [Diagnostics.Process]::Start($info)
    try {
        $output = [IO.File]::Create($Destination)
        try {
            $process.StandardOutput.BaseStream.CopyTo($output)
        }
        finally {
            $output.Dispose()
        }
        $errorText = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "Could not export pinned Inno Setup language blob: $errorText"
        }
    }
    finally {
        $process.Dispose()
    }
}

# Downloads to a temporary neighbor, validates it, and only then replaces the reusable cache.
function Get-InnoSimplifiedChineseLanguage(
    [string]$Destination,
    [string]$Repository,
    [string]$Commit,
    [string]$SourcePath,
    [string]$ExpectedSha256
) {
    if (Test-Path -LiteralPath $Destination -PathType Leaf) {
        try {
            Assert-InnoSimplifiedChineseLanguage $Destination $ExpectedSha256
            return
        }
        catch {
            Write-Host "Refreshing invalid cached Inno Setup language file: $($_.Exception.Message)"
        }
    }

    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $temporary = Join-Path $parent (".ChineseSimplified-" + [guid]::NewGuid() + ".isl")
    try {
        Export-PinnedGitBlob $Repository $Commit $SourcePath $temporary
        Assert-InnoSimplifiedChineseLanguage $temporary $ExpectedSha256
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

if ($MyInvocation.InvocationName -ne ".") {
    $root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $output = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
        [IO.Path]::GetFullPath($OutputDirectory)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
    }
    $destination = Join-Path $output "ChineseSimplified.isl"
    Get-InnoSimplifiedChineseLanguage `
        $destination $sourceRepository $sourceCommit $sourcePath $expectedSha256
    Write-Host "Prepared official Inno Setup Simplified Chinese messages at $destination"
    Write-Output $destination
}
