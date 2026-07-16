param(
    [string]$Source = "resources/branding/logo-mark.svg"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$sourcePath = Join-Path $root $Source
$iconsDir = Join-Path $root "resources/icons"
$edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"

if (-not (Test-Path -LiteralPath $edge)) {
    throw "Microsoft Edge is required to rasterize the SVG source."
}

New-Item -ItemType Directory -Path $iconsDir -Force | Out-Null
$sourcePng = Join-Path $iconsDir "icon-512.png"
$sourceUri = [Uri]::new($sourcePath).AbsoluteUri

& $edge --headless --disable-gpu --screenshot=$sourcePng --window-size=512,512 $sourceUri | Out-Null

Add-Type -AssemblyName System.Drawing
$sourceBitmap = [System.Drawing.Bitmap]::FromFile($sourcePng)
try {
    foreach ($size in 16, 24, 32, 48, 64, 128, 256) {
        $target = New-Object System.Drawing.Bitmap($size, $size)
        $graphics = [System.Drawing.Graphics]::FromImage($target)
        try {
            $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $graphics.DrawImage($sourceBitmap, 0, 0, $size, $size)
            $target.Save(
                (Join-Path $iconsDir "icon-$size.png"),
                [System.Drawing.Imaging.ImageFormat]::Png
            )
        }
        finally {
            $graphics.Dispose()
            $target.Dispose()
        }
    }
}
finally {
    $sourceBitmap.Dispose()
}

$png = [IO.File]::ReadAllBytes((Join-Path $iconsDir "icon-256.png"))
$stream = [IO.File]::Create((Join-Path $iconsDir "icon.ico"))
$writer = New-Object IO.BinaryWriter($stream)
try {
    $writer.Write([UInt16]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]1)
    $writer.Write([Byte]0)
    $writer.Write([Byte]0)
    $writer.Write([Byte]0)
    $writer.Write([Byte]0)
    $writer.Write([UInt16]1)
    $writer.Write([UInt16]32)
    $writer.Write([UInt32]$png.Length)
    $writer.Write([UInt32]22)
    $writer.Write($png)
}
finally {
    $writer.Dispose()
    $stream.Dispose()
}

Write-Host "Generated Flash Shot icons in $iconsDir"
