# What is on screen while the window boots? Screenshots at fixed offsets from
# the launch, cropped to the app window.
#
# Diagnostic only — it is NOT the A/B: screen capture perturbs the timings, so
# the numbers in `docs/decisions/boot-screen-prerender.md` come from `run.ts`.
# This answers the one question the timeline cannot: is the boot screen actually
# visible, and when does the real UI take over. Screenshots land in `.perf/`.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File bench/cold-start/bootshot.ps1
#   powershell ... -File bench/cold-start/bootshot.ps1 -Skeleton 1
#   $env:DAE_BOOT_YIELD="1"; powershell ... -File bench/cold-start/bootshot.ps1 -Skeleton 1
#
# Requires a built `src-tauri/target/release/dae.exe` and no other instance
# running (the app is single-instance). The window rect is read once, right
# after the handle appears; Windows may then place the window elsewhere, in
# which case the crops are offset but still show the screen.
param(
  [string]$Skeleton = "0",
  [string]$Out = ".perf/shots"
)

Add-Type -AssemblyName System.Drawing

$sig = @'
using System;
using System.Runtime.InteropServices;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
}
'@
if (-not ("Win" -as [type])) { Add-Type -TypeDefinition $sig }

New-Item -ItemType Directory -Force -Path $Out | Out-Null
Get-ChildItem $Out -Filter *.png -ErrorAction SilentlyContinue | Remove-Item -Force

$env:DAE_BOOT_SKELETON = $Skeleton
$p = Start-Process -FilePath "src-tauri/target/release/dae.exe" -PassThru
$sw = [Diagnostics.Stopwatch]::StartNew()

# The window is 800x600; wait for the handle, then pin the rect.
$rect = $null
while ($sw.ElapsedMilliseconds -lt 2000) {
  $p.Refresh()
  if ($p.MainWindowHandle -ne [IntPtr]::Zero) {
    $r = New-Object Win+RECT
    if ([Win]::GetWindowRect($p.MainWindowHandle, [ref]$r)) {
      if (($r.R - $r.L) -gt 100) { $rect = $r; break }
    }
  }
  Start-Sleep -Milliseconds 5
}
if (-not $rect) { Write-Error "no window"; exit 1 }
Write-Host "window rect: $($rect.L),$($rect.T) - $($rect.R),$($rect.B) at $($sw.ElapsedMilliseconds) ms"

$width = $rect.R - $rect.L
$height = $rect.B - $rect.T
# The offsets span the interesting part of the waterfall: the window exists at
# ~300 ms, React commits at ~420-490 ms.
foreach ($t in 320, 360, 400, 440, 480, 520, 560, 640, 800, 1200) {
  while ($sw.ElapsedMilliseconds -lt $t) { Start-Sleep -Milliseconds 1 }
  $bmp = New-Object Drawing.Bitmap $width, $height
  $g = [Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.L, $rect.T, 0, 0, (New-Object Drawing.Size $width, $height))
  $file = Join-Path $Out ("s{0}-{1:d4}.png" -f $Skeleton, $t)
  $bmp.Save($file)
  $g.Dispose(); $bmp.Dispose()
  Write-Host "shot $t ms at $($sw.ElapsedMilliseconds) ms -> $file"
}

taskkill /PID $p.Id /T /F | Out-Null
