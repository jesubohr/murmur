# Draws the Murmur app icon (rounded square with waveform bars) to a 1024 px PNG.
param([string]$Out = "app-icon.png")
Add-Type -AssemblyName System.Drawing
$s = 1024
$bmp = New-Object System.Drawing.Bitmap $s, $s
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.Clear([System.Drawing.Color]::Transparent)
$r = 220
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$path.AddArc(0, 0, $r, $r, 180, 90)
$path.AddArc($s - $r, 0, $r, $r, 270, 90)
$path.AddArc($s - $r, $s - $r, $r, $r, 0, 90)
$path.AddArc(0, $s - $r, $r, $r, 90, 90)
$path.CloseFigure()
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 0), (New-Object System.Drawing.Point $s, $s), ([System.Drawing.Color]::FromArgb(255, 99, 102, 241)), ([System.Drawing.Color]::FromArgb(255, 168, 85, 247))
$g.FillPath($brush, $path)
$white = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
$heights = 220, 420, 640, 420, 220
$w = 96; $gap = 52
$x0 = ($s - (5 * $w + 4 * $gap)) / 2
for ($i = 0; $i -lt 5; $i++) {
  $h = $heights[$i]; $x = $x0 + $i * ($w + $gap); $y = ($s - $h) / 2
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $p.AddArc($x, $y, $w, $w, 180, 180)
  $p.AddArc($x, $y + $h - $w, $w, $w, 0, 180)
  $p.CloseFigure()
  $g.FillPath($white, $p)
}
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
