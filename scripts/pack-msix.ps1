#!/usr/bin/env pwsh
<#
.SYNOPSIS
Packs the Qt shell into an MSIX.

.DESCRIPTION
The package needs a deployed Qt tree, not just `dae.exe`: the shell draws with
Qt Quick and plays previews through QtMultimedia, and neither is statically
linked. This stages what `windeployqt` produces into `dist/` — the same layout
the release workflow ships as a portable zip — and hands that folder to
`winapp pack`. The package manifest and Assets/ stay at the repository root,
where `winapp` finds them.

`dist/` is emptied first. A package is a snapshot of everything in the folder,
so a file left over from a previous run would ship inside it.

.PARAMETER SkipBuild
Reuse `target\release\dae.exe` instead of rebuilding it.

.PARAMETER QtRoot
Qt installation to deploy from. Defaults to `$env:QT_ROOT_DIR`, then to the
prefix `qmake` on PATH reports.

.EXAMPLE
pwsh scripts/pack-msix.ps1
#>
[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [string]$QtRoot = $env:QT_ROOT_DIR
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $IsWindows -and $PSVersionTable.PSVersion.Major -ge 6) {
  throw 'MSIX packing is Windows-only.'
}

if (-not (Get-Command winapp -ErrorAction SilentlyContinue)) {
  throw 'winapp is not on PATH. Install it with: winget install Microsoft.WinAppCli (or `dotnet tool install -g Microsoft.WinAppCli`).'
}

# `windeployqt` is the only tool here that knows which Qt modules the QML tree
# imports, so the script refuses to guess a path for it.
if (-not $QtRoot) {
  $qmake = Get-Command qmake6, qmake -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if ($qmake) {
    $QtRoot = (& $qmake.Source -query QT_INSTALL_PREFIX).Trim()
  }
}
$windeployqt = if ($QtRoot) { Join-Path $QtRoot 'bin\windeployqt.exe' } else { $null }
if (-not $windeployqt -or -not (Test-Path $windeployqt)) {
  throw 'windeployqt was not found. Set QT_ROOT_DIR to the Qt installation (for example K:\Qt\6.12.0\msvc2022_64).'
}
Write-Host "Qt: $QtRoot"

# Keeps the manifest's four-part version in step with package.json; packing the
# same version twice bumps the revision, or Windows refuses the install.
node scripts/sync-msix-version.mjs
if ($LASTEXITCODE -ne 0) { throw 'sync-msix-version failed' }

if (-not $SkipBuild) {
  cargo build --release --package dae-app
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
}
$exe = Join-Path $root 'target\release\dae.exe'
if (-not (Test-Path $exe)) {
  throw "$exe is missing. Run without -SkipBuild, or build it with: cargo build --release --package dae-app"
}

if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory dist | Out-Null
Copy-Item $exe (Join-Path $root 'dist\dae.exe')

& $windeployqt --release --no-translations --compiler-runtime --qmldir qml `
  (Join-Path $root 'dist\dae.exe')
if ($LASTEXITCODE -ne 0) { throw "windeployqt failed with exit code $LASTEXITCODE" }

# The same set the release workflow asserts before it packs: a payload without
# these starts and then fails to open a window, which is far harder to read
# from a bug report than a failed build.
foreach ($required in @('dae.exe', 'platforms\qwindows.dll', 'qml\QtQuick',
    'qml\QtMultimedia', 'multimedia')) {
  if (-not (Test-Path (Join-Path $root "dist\$required"))) {
    throw "windeployqt did not produce $required"
  }
}
$bytes = (Get-ChildItem dist -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host ("Payload: {0:N1} MB" -f ($bytes / 1MB))

# Same certificate handling as the release workflow: a real .pfx from the
# environment when one is configured, otherwise the throwaway development
# certificate. Either way its subject has to match the manifest's Publisher.
if ($env:MSIX_CERTIFICATE) {
  [IO.File]::WriteAllBytes(
    (Join-Path $root 'release.pfx'),
    [Convert]::FromBase64String($env:MSIX_CERTIFICATE))
  winapp pack .\dist --cert .\release.pfx `
    --cert-password "$env:MSIX_CERTIFICATE_PASSWORD"
  if ($LASTEXITCODE -ne 0) { throw "winapp pack failed with exit code $LASTEXITCODE" }
  Remove-Item (Join-Path $root 'release.pfx') -Force -ErrorAction SilentlyContinue
} else {
  winapp cert generate --if-exists skip
  winapp pack .\dist --cert .\devcert.pfx
  if ($LASTEXITCODE -ne 0) { throw "winapp pack failed with exit code $LASTEXITCODE" }
}

$msix = Get-ChildItem $root -Filter '*.msix' | Sort-Object LastWriteTime |
  Select-Object -Last 1
if (-not $msix) { throw 'winapp produced no .msix' }
Write-Host ("Package: {0} ({1:N1} MB)" -f $msix.Name, ($msix.Length / 1MB))
