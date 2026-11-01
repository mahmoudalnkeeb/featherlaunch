# Renders the FeatherLaunch brand set into assets/brand, from the same 24-unit geometry as make-icon.ps1.
Add-Type -AssemblyName System.Drawing
$out = Join-Path $PSScriptRoot 'brand'
function Hex([string]$h) { [System.Drawing.ColorTranslator]::FromHtml($h) }
$accent = Hex '#74DDE8'; $ink = Hex '#0B1214'; $bg = Hex '#0E0F11'; $surface = Hex '#16171A'
$border = Hex '#26282D'; $text = Hex '#ECEDEF'; $muted = Hex '#9095A0'; $white = [System.Drawing.Color]::White
$none = [System.Drawing.Color]::Transparent

function Canvas([int]$w, [int]$h, $fill) {
  $b = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($b)
  $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'; $g.TextRenderingHint = 'AntiAliasGridFit'; $g.Clear($fill)
  , @($b, $g)
}
function Save($c, [string]$name) {
  $path = Join-Path $out $name
  [void][IO.Directory]::CreateDirectory((Split-Path $path))
  $c[1].Dispose(); $c[0].Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $c[0].Dispose()
}
# The feather in a 24-unit box at ($x,$y). $cut draws the shaft and barbs; transparent punches them out.
function Feather($g, [single]$x, [single]$y, [single]$size, $color, $cut) {
  $s = $size / 24.0
  $P = { param($a, $b) New-Object System.Drawing.PointF ([single]($x + $a * $s)), ([single]($y + $b * $s)) }
  $cap = [System.Drawing.Drawing2D.LineCap]::Round
  $vane = New-Object System.Drawing.Drawing2D.GraphicsPath
  $vane.AddBezier((& $P 7 17), (& $P 6.5 11), (& $P 12 5.5), (& $P 19.5 4.5))
  $vane.AddBezier((& $P 19.5 4.5), (& $P 18.5 11.5), (& $P 13 17.5), (& $P 7 17))
  $vane.CloseFigure()
  $g.FillPath((New-Object System.Drawing.SolidBrush $color), $vane)
  # Only a transparent cut needs to replace pixels; copying an opaque one leaves a dark fringe.
  if ($cut.A -eq 0) { $g.CompositingMode = 'SourceCopy' }
  $shaft = New-Object System.Drawing.Pen $cut, ([single](1.3 * $s)); $shaft.StartCap = $cap; $shaft.EndCap = $cap
  $g.DrawLine($shaft, (& $P 8.6 15.4), (& $P 16.5 7.5))
  $split = New-Object System.Drawing.Pen $cut, ([single](1.1 * $s)); $split.StartCap = $cap; $split.EndCap = $cap
  $g.DrawLine($split, (& $P 13.6 10.4), (& $P 17.8 11.6))
  $g.DrawLine($split, (& $P 11 13), (& $P 14.6 15.4))
  $g.CompositingMode = 'SourceOver'
  $stem = New-Object System.Drawing.Pen $color, ([single](1.6 * $s)); $stem.StartCap = $cap; $stem.EndCap = $cap
  $g.DrawLine($stem, (& $P 5 19), (& $P 8.2 15.8))
}
function Tile($g, [single]$x, [single]$y, [single]$size) {
  $d = 11 * $size / 24
  $t = New-Object System.Drawing.Drawing2D.GraphicsPath
  $e = $size - 0.5
  $t.AddArc($x, $y, $d, $d, 180, 90); $t.AddArc($x + $e - $d, $y, $d, $d, 270, 90)
  $t.AddArc($x + $e - $d, $y + $e - $d, $d, $d, 0, 90); $t.AddArc($x, $y + $e - $d, $d, $d, 90, 90); $t.CloseFigure()
  $g.FillPath((New-Object System.Drawing.SolidBrush $accent), $t)
  Feather $g $x $y $size $ink $accent
}
function Label($g, [string]$s, [string]$font, [single]$px, $color, [single]$x, [single]$y) {
  $f = New-Object System.Drawing.Font $font, $px, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
  $g.DrawString($s, $f, (New-Object System.Drawing.SolidBrush $color), $x, $y, [System.Drawing.StringFormat]::GenericTypographic)
}
# Icon and name centred on a dark card, with the tagline under them.
function Card([int]$w, [int]$h, [string]$name, [bool]$tagline = $true) {
  $c = Canvas $w $h $bg; $g = $c[1]
  $u = [Math]::Min($h / 630.0, $w / 1200.0)
  $icon = 150 * $u; $title = 84 * $u
  $f = New-Object System.Drawing.Font 'Segoe UI Semibold', ([single]$title), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
  $tw = $g.MeasureString('FeatherLaunch', $f, [int]$w, [System.Drawing.StringFormat]::GenericTypographic).Width
  $x = ($w - $icon - 40 * $u - $tw) / 2; $block = if ($tagline) { 230 * $u } else { $icon }; $y = ($h - $block) / 2
  Tile $g $x $y $icon
  Label $g 'FeatherLaunch' 'Segoe UI Semibold' $title $text ($x + $icon + 40 * $u) ($y + ($icon - $title * 1.33) / 2)
  if ($tagline) {
    $line = 'A Minecraft: Java Edition launcher for older PCs'
    $sf = New-Object System.Drawing.Font 'Segoe UI', ([single](34 * $u)), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
    $lw = $g.MeasureString($line, $sf, [int]$w, [System.Drawing.StringFormat]::GenericTypographic).Width
    Label $g $line 'Segoe UI' (34 * $u) $muted (($w - $lw) / 2) ($y + $icon + 40 * $u)
  }
  Save $c $name
}

# App icons on the rounded tile, with transparent corners.
foreach ($n in 16, 24, 32, 48, 64, 96, 128, 256, 512, 1024) { $c = Canvas $n $n $none; Tile $c[1] 0 0 $n; Save $c "icons/icon-$n.png" }

# Web: favicons have corners; platforms that round the icon themselves get a full square.
foreach ($n in 16, 32, 48, 192, 512) { $c = Canvas $n $n $none; Tile $c[1] 0 0 $n; Save $c "web/favicon-$n.png" }
$c = Canvas 180 180 $accent; Feather $c[1] 0 0 180 $ink $accent; Save $c 'web/apple-touch-icon-180.png'
# Maskable icons keep the feather inside the central safe zone.
foreach ($n in 192, 512) { $c = Canvas $n $n $accent; Feather $c[1] ($n * 0.1) ($n * 0.1) ($n * 0.8) $ink $accent; Save $c "web/maskable-$n.png" }

# The feather alone, for use over other backgrounds.
foreach ($v in @(@('accent', $accent), @('dark', $ink), @('white', $white))) {
  foreach ($n in 256, 1024) { $c = Canvas $n $n $none; Feather $c[1] 0 0 $n $v[1] $none; Save $c "mark/mark-$($v[0])-$n.png" }
}

# Discord: Rich Presence art (upload this one as "logo"), and the app's icon and banner.
$c = Canvas 1024 1024 $none; Tile $c[1] 0 0 1024; Save $c 'discord/logo-1024.png'
Card 1024 576 'discord/banner-1024x576.png' $false

# Sharing and listings.
Card 1200 630 'social/og-1200x630.png'
Card 1280 640 'social/github-1280x640.png'
Card 1500 500 'social/x-header-1500x500.png' $false
Card 460 215 'store/capsule-460x215.png' $false
Get-ChildItem $out -Recurse -File | Measure-Object | ForEach-Object { "$($_.Count) files" }
