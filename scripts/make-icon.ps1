<#
.SYNOPSIS
產生輸入法清單 / 工作列用的圖示 crates/zct-tsf/res/zct.ico（白底黑字「動」，與微軟新注音的「ㄅ」同風格）

.DESCRIPTION
每個尺寸存成 32 位元 BMP（ExtractIconEx 支援最完整）。改字或字型後重新執行即可。
#>
param(
    [string]$Text = '動',
    [string]$Font = 'Microsoft JhengHei'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'crates\zct-tsf\res\zct.ico'
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null

$sizes = 16, 20, 24, 32, 40, 48, 64
$entries = foreach ($size in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::White)
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $face = New-Object System.Drawing.Font $Font, ($size * 0.78), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = [System.Drawing.StringAlignment]::Center
    $fmt.LineAlignment = [System.Drawing.StringAlignment]::Center
    $rect = New-Object System.Drawing.RectangleF 0, 0, $size, ($size + $size * 0.06)
    $g.DrawString($Text, $face, [System.Drawing.Brushes]::Black, $rect, $fmt)
    $g.Dispose()

    # BITMAPINFOHEADER + 由下而上的 BGRA 像素 + 全 0 的 AND 遮罩
    $ms = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $ms
    $w.Write([int]40); $w.Write([int]$size); $w.Write([int]($size * 2))
    $w.Write([int16]1); $w.Write([int16]32); $w.Write([int]0)
    $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0)
    for ($y = $size - 1; $y -ge 0; $y--) {
        for ($x = 0; $x -lt $size; $x++) {
            $c = $bmp.GetPixel($x, $y)
            $w.Write([byte]$c.B); $w.Write([byte]$c.G); $w.Write([byte]$c.R); $w.Write([byte]255)
        }
    }
    $maskRow = [int]([math]::Ceiling($size / 32) * 4)
    $w.Write((New-Object byte[] ($maskRow * $size)))
    $w.Flush()
    $bmp.Dispose()
    [pscustomobject]@{ Size = $size; Data = $ms.ToArray() }
}

$fs = [System.IO.File]::Create($out)
$w = New-Object System.IO.BinaryWriter $fs
$w.Write([int16]0); $w.Write([int16]1); $w.Write([int16]$entries.Count)
$offset = 6 + 16 * $entries.Count
foreach ($e in $entries) {
    $w.Write([byte]($e.Size % 256)); $w.Write([byte]($e.Size % 256))
    $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([int16]1); $w.Write([int16]32)
    $w.Write([int]$e.Data.Length); $w.Write([int]$offset)
    $offset += $e.Data.Length
}
foreach ($e in $entries) { $w.Write($e.Data) }
$w.Close()
Write-Host "完成：$out"
