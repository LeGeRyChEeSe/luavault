# Generates a 1024x1024 app icon (azure gradient rounded square + "LV"),
# then regenerates all Tauri icon sizes from it.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path $PSScriptRoot -Parent
$source = Join-Path $root 'icon-source.png'

$bmp = New-Object System.Drawing.Bitmap(1024, 1024)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$g.Clear([System.Drawing.Color]::Transparent)

# Rounded square
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$x = 40; $y = 40; $w = 944; $h = 944; $r = 230
$path.AddArc($x, $y, $r, $r, 180, 90)
$path.AddArc($x + $w - $r, $y, $r, $r, 270, 90)
$path.AddArc($x + $w - $r, $y + $h - $r, $r, $r, 0, 90)
$path.AddArc($x, $y + $h - $r, $r, $r, 90, 90)
$path.CloseFigure()

$topLeft = New-Object System.Drawing.Point($x, $y)
$bottomRight = New-Object System.Drawing.Point(($x + $w), ($y + $h))
$from = [System.Drawing.Color]::FromArgb(255, 54, 169, 255)   # azure-400
$to = [System.Drawing.Color]::FromArgb(255, 1, 89, 163)       # azure-700
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush($topLeft, $bottomRight, $from, $to)
$g.FillPath($brush, $path)

# Subtle inner highlight
$highlight = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(40, 255, 255, 255))
$g.FillEllipse($highlight, 120, 90, 620, 330)

# "LV" label
$font = New-Object System.Drawing.Font('Segoe UI', 280, [System.Drawing.FontStyle]::Bold)
$textBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$sf.LineAlignment = [System.Drawing.StringAlignment]::Center
$rectF = New-Object System.Drawing.RectangleF($x, ([single]$y + 30), ([single]$w), ([single]$h))
$g.DrawString('LV', $font, $textBrush, $rectF, $sf)

$bmp.Save($source, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()

Write-Host "Source icon written: $source"

Push-Location $root
try {
    npx tauri icon $source
} finally {
    Pop-Location
}
Write-Host "Tauri icons regenerated in src-tauri/icons"
