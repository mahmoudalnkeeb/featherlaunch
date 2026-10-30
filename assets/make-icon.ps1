# Renders the FeatherLaunch icon (24-unit design grid) to PNGs and a multi-size ICO.
param([string]$outDir)
Add-Type -AssemblyName System.Drawing
$accent = [System.Drawing.Color]::FromArgb(255, 0x74, 0xDD, 0xE8)
$ink    = [System.Drawing.Color]::FromArgb(255, 0x0B, 0x12, 0x14)
function Render([int]$size) {
  $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'; $g.Clear([System.Drawing.Color]::Transparent)
  $s = $size / 24.0
  # Rounded tile.
  $r = 5.5 * $s; $d = 2 * $r; $w = $size - 0.5
  $tile = New-Object System.Drawing.Drawing2D.GraphicsPath
  $tile.AddArc(0, 0, $d, $d, 180, 90); $tile.AddArc($w - $d, 0, $d, $d, 270, 90)
  $tile.AddArc($w - $d, $w - $d, $d, $d, 0, 90); $tile.AddArc(0, $w - $d, $d, $d, 90, 90); $tile.CloseFigure()
  $g.FillPath((New-Object System.Drawing.SolidBrush $accent), $tile)
  $P = { param($x, $y) New-Object System.Drawing.PointF ([single]($x * $s)), ([single]($y * $s)) }
  # Feather vane: two cubic curves from the base (7,17) to the tip (19,5).
  $vane = New-Object System.Drawing.Drawing2D.GraphicsPath
  $vane.AddBezier((& $P 7 17), (& $P 6.5 11), (& $P 12 5.5), (& $P 19.5 4.5))
  $vane.AddBezier((& $P 19.5 4.5), (& $P 18.5 11.5), (& $P 13 17.5), (& $P 7 17))
  $vane.CloseFigure()
  $g.FillPath((New-Object System.Drawing.SolidBrush $ink), $vane)
  $cap = [System.Drawing.Drawing2D.LineCap]::Round
  # Shaft through the vane in the tile color, stem below it in ink.
  $shaft = New-Object System.Drawing.Pen $accent, ([single][Math]::Max(1.0, 1.3 * $s)); $shaft.StartCap = $cap; $shaft.EndCap = $cap
  $g.DrawLine($shaft, (& $P 8.6 15.4), (& $P 16.5 7.5))
  # Two barb splits on the lower edge.
  $split = New-Object System.Drawing.Pen $accent, ([single][Math]::Max(1.0, 1.1 * $s)); $split.StartCap = $cap; $split.EndCap = $cap
  $g.DrawLine($split, (& $P 13.6 10.4), (& $P 17.8 11.6))
  $g.DrawLine($split, (& $P 11 13), (& $P 14.6 15.4))
  $stem = New-Object System.Drawing.Pen $ink, ([single][Math]::Max(1.0, 1.6 * $s)); $stem.StartCap = $cap; $stem.EndCap = $cap
  $g.DrawLine($stem, (& $P 5 19), (& $P 8.2 15.8))
  $g.Dispose(); return $bmp
}
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$pngs = @{}
foreach ($sz in $sizes) {
  $b = Render $sz; $ms = New-Object System.IO.MemoryStream; $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $pngs[$sz] = $ms.ToArray()
  if ($sz -in 64, 256) { [IO.File]::WriteAllBytes((Join-Path $outDir "icon-$sz.png"), $pngs[$sz]) }
}
# ICO with PNG-compressed entries (supported since Windows Vista).
$fs = New-Object System.IO.MemoryStream; $bw = New-Object System.IO.BinaryWriter $fs
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
foreach ($sz in $sizes) {
  $dim = if ($sz -ge 256) { 0 } else { $sz }
  $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
  $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]$pngs[$sz].Length); $bw.Write([uint32]$offset)
  $offset += $pngs[$sz].Length
}
foreach ($sz in $sizes) { $bw.Write($pngs[$sz]) }
$bw.Flush(); [IO.File]::WriteAllBytes((Join-Path $outDir "icon.ico"), $fs.ToArray())
Get-ChildItem $outDir | ForEach-Object { "{0} {1} bytes" -f $_.Name, $_.Length }
