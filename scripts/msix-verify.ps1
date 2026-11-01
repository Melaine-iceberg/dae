#!/usr/bin/env pwsh
<#
.SYNOPSIS
Installs an MSIX build, checks the three things a package must get right, and
reports what it found.

.DESCRIPTION
The packaging step proves a package can be *built*: `winapp pack` validates the
manifest and `release.yml` asserts the payload contains the Qt runtime. Nothing
in CI proves an installed app starts, receives a `dae://` link, or survives an
upgrade — those need a machine with a real user profile, and installing a
sideloaded package changes that machine (it trusts a certificate and creates a
package registration), so it stays a manual step.

What this script does, in order:

1. Reads the manifest out of the .msix without installing it and asserts the
   pieces that make an installed copy behave: the `dae` URL scheme, the
   `dae.exe` execution alias (`default_manager` registers that path, not the
   versioned one), and a four-part version.
2. With `-Previous`, installs the older package first, then the newer one, and
   verifies Windows replaced rather than duplicated it: the version moved
   forward, the old install directory is gone, the alias still resolves.
3. Launches through the app model, writes a `dae://` link into it, and checks
   the log under `%LOCALAPPDATA%\com.melaine.dae\logs` (a full-trust package is
   not redirected, so the same file the portable build writes is used).

Nothing here is verified from inside CI, so every check prints what it observed
instead of trusting a boolean.

.PARAMETER Package
The .msix to verify. Defaults to the newest `*.msix` in the repository root.

.PARAMETER Previous
An older .msix of the same package, for the upgrade leg. Build one by checking
out the previous tag and running `pwsh scripts/pack-msix.ps1`.

.PARAMETER Certificate
The .pfx that signed the package. Defaults to `devcert.pfx` in the repository
root, then to `release.pfx`. Its subject must match the manifest's Publisher;
sideloading needs the certificate in the current user's TrustedPeople store.

.PARAMETER TrustCertificate
Import the certificate into `Cert:\CurrentUser\TrustedPeople` before installing.
Without this the install fails with 0x800B0109 (untrusted root) on a machine
that has never trusted the publisher.

.PARAMETER Uninstall
Remove the package when the checks are done.

.PARAMETER InspectOnly
Stop after reading the manifest. Changes nothing on the machine.

.EXAMPLE
pwsh scripts/msix-verify.ps1 -InspectOnly

.EXAMPLE
pwsh scripts/msix-verify.ps1 -Previous .\dae_0.4.0.0_x64.msix -TrustCertificate
#>
[CmdletBinding()]
param(
  [string]$Package,
  [string]$Previous,
  [string]$Certificate,
  [switch]$TrustCertificate,
  [switch]$Uninstall,
  [switch]$InspectOnly
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$failures = 0

function Write-Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) {
    Write-Host ("  PASS  {0}: {1}" -f $name, $detail) -ForegroundColor Green
  } else {
    Write-Host ("  FAIL  {0}: {1}" -f $name, $detail) -ForegroundColor Red
    $script:failures++
  }
}

function Resolve-Package([string]$path, [string]$label) {
  if (-not $path) {
    $candidates = Get-ChildItem $root -Filter '*.msix' |
      Sort-Object LastWriteTime -Descending
    if (-not $candidates) {
      throw "No .msix in $root. Build one with: pwsh scripts/pack-msix.ps1"
    }
    $path = $candidates[0].FullName
  } elseif (-not (Test-Path $path)) {
    throw "$label package not found: $path"
  }
  Write-Host "$label package: $path"
  return (Resolve-Path $path).Path
}

<#
  Reads AppxManifest.xml straight out of the package. `Get-AppxPackageManifest`
  only works after installing, and the point of this first pass is to catch a
  bad package before it touches the machine.
#>
function Read-PackageManifest([string]$path) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $archive = [IO.Compression.ZipFile]::OpenRead($path)
  try {
    $entry = $archive.GetEntry('AppxManifest.xml')
    if (-not $entry) { throw "$path has no AppxManifest.xml" }
    $reader = New-Object IO.StreamReader($entry.Open())
    try { $document = [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
  } finally {
    $archive.Dispose()
  }

  # Local-name lookups: the manifest mixes default, uap, uap3 and desktop
  # namespaces, and the extension that matters is declared in uap3.
  $identity = $document.SelectSingleNode("//*[local-name()='Identity']")
  $alias = $document.SelectSingleNode(
    "//*[local-name()='Extension'][@Category='windows.appExecutionAlias']//*[local-name()='ExecutionAlias']/@Alias")
  $protocol = $document.SelectSingleNode(
    "//*[local-name()='Extension'][@Category='windows.protocol']//*[local-name()='Protocol']/@Name")
  $application = $document.SelectSingleNode("//*[local-name()='Application']")

  [pscustomobject]@{
    Name        = $identity.Name
    Version     = $identity.Version
    Publisher   = $identity.Publisher
    AppId       = $application.Id
    Scheme      = if ($protocol) { $protocol.Value } else { $null }
    Alias       = if ($alias) { $alias.Value } else { $null }
  }
}

function Assert-Manifest($manifest) {
  # A package that installed but does not receive links or cannot be registered
  # as the folder handler is the failure this whole item is about, so each of
  # these is a hard failure.
  Write-Check 'identity version is four-part' `
    ($manifest.Version -match '^\d+\.\d+\.\d+\.\d+$') $manifest.Version
  Write-Check 'dae:// is declared by the manifest' `
    ($manifest.Scheme -eq 'dae') "protocol=$($manifest.Scheme)"
  Write-Check 'execution alias resolves across upgrades' `
    ($manifest.Alias -eq 'dae.exe') "alias=$($manifest.Alias)"
  Write-Check 'application id is the one the checks launch' `
    ($manifest.AppId -eq 'dae') "id=$($manifest.AppId)"
}

function Stop-RunningInstance {
  # An update cannot replace a running app: the package is in use and the
  # install fails with 0x80073D02.
  Get-Process -Name dae -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 500
}

function Install-Package([string]$path, [string]$certificate) {
  if ($certificate) {
    Write-Host "Importing $certificate into Cert:\CurrentUser\TrustedPeople"
    Import-Certificate -FilePath $certificate `
      -CertStoreLocation 'Cert:\CurrentUser\TrustedPeople' | Out-Null
  }
  Stop-RunningInstance
  Add-AppxPackage -Path $path
}

function Get-Installed([string]$name) {
  Get-AppxPackage -Name $name | Select-Object -First 1
}

function Test-Alias([string]$alias) {
  $path = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\$alias"
  Write-Check 'execution alias exists on PATH' (Test-Path $path) $path
  return $path
}

function Test-InstallTree($installed) {
  $location = $installed.InstallLocation
  Write-Host "  install location: $location"
  foreach ($required in @('dae.exe', 'platforms\qwindows.dll', 'qml\QtQuick',
      'qml\QtMultimedia', 'multimedia')) {
    $present = Test-Path (Join-Path $location $required)
    Write-Check "payload contains $required" $present (Join-Path $location $required)
  }
}

function Start-PackagedApp([string]$packageFamilyName, [string]$appId) {
  $aumid = "$packageFamilyName!$appId"
  Write-Host "Launching $aumid"
  Start-Process 'explorer.exe' "shell:AppsFolder\$aumid"
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    $process = Get-Process -Name dae -ErrorAction SilentlyContinue |
      Select-Object -First 1
    if ($process) { return $process }
    Start-Sleep -Milliseconds 250
  }
  throw "dae did not start within 30 s of the app-model launch"
}

function Wait-ForLog([string]$path, [datetime]$since) {
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    if (Test-Path $path) {
      $stamp = (Get-Item $path).LastWriteTime
      if ($stamp -gt $since) { return $stamp }
    }
    Start-Sleep -Milliseconds 250
  }
  return $null
}

# ---------------------------------------------------------------------------
# 1. Manifest, without installing.
# ---------------------------------------------------------------------------
$packagePath = Resolve-Package $Package 'Package'
$manifest = Read-PackageManifest $packagePath
Write-Host ''
Write-Host 'Manifest'
Write-Host ("  {0} {1} ({2})" -f $manifest.Name, $manifest.Version, $manifest.Publisher)
Assert-Manifest $manifest

$previousManifest = $null
if ($Previous) {
  $previousPath = Resolve-Package $Previous 'Previous'
  $previousManifest = Read-PackageManifest $previousPath
  Write-Host ''
  Write-Host 'Upgrade order'
  Write-Check 'the newer package has the higher version' `
    ([version]$manifest.Version -gt [version]$previousManifest.Version) `
    ("{0} > {1}" -f $manifest.Version, $previousManifest.Version)
  Write-Check 'both packages share one identity' `
    ($manifest.Name -eq $previousManifest.Name -and
      $manifest.Publisher -eq $previousManifest.Publisher) `
    ("{0} / {1}" -f $manifest.Name, $manifest.Publisher)
}

if ($failures -gt 0) {
  Write-Host ''
  Write-Host "$failures manifest check(s) failed; nothing was installed." -ForegroundColor Red
  exit 1
}
if ($InspectOnly) {
  Write-Host ''
  Write-Host 'Inspect only: the manifest is consistent. Run without -InspectOnly to install.'
  exit 0
}

# ---------------------------------------------------------------------------
# 2. Install (and the upgrade leg).
# ---------------------------------------------------------------------------
if (-not $Certificate) {
  foreach ($candidate in @('devcert.pfx', 'release.pfx')) {
    $path = Join-Path $root $candidate
    if (Test-Path $path) { $Certificate = $path; break }
  }
}
# Only import when asked: trusting a publisher is a machine change, and a
# machine that already trusts it needs nothing here.
$trustedCertificate = if ($TrustCertificate) { $Certificate } else { $null }

Write-Host ''
Write-Host 'Install'
$existing = Get-Installed $manifest.Name
if ($existing) {
  Write-Host ("  already installed: {0} at {1}" -f $existing.Version, $existing.InstallLocation)
}

$previousLocation = $null
if ($previousPath) {
  Write-Host "  installing the previous build first: $previousPath"
  Install-Package $previousPath $trustedCertificate
  $before = Get-Installed $previousManifest.Name
  if (-not $before) { throw 'the previous package did not install' }
  $previousLocation = $before.InstallLocation
  Write-Check 'previous version is installed' `
    ($before.Version -eq $previousManifest.Version) `
    ("{0} at {1}" -f $before.Version, $before.InstallLocation)
}

Install-Package $packagePath $trustedCertificate
$installed = Get-Installed $manifest.Name
if (-not $installed) { throw 'the package is not registered after Add-AppxPackage' }

Write-Check 'installed version matches the package' `
  ($installed.Version -eq $manifest.Version) `
  ("{0} (package {1})" -f $installed.Version, $manifest.Version)
Test-InstallTree $installed
$aliasPath = Test-Alias $manifest.Alias

if ($previousLocation) {
  # An upgrade installs a new directory and deletes the old one; a leftover
  # directory means Windows side-by-side registered two packages instead.
  Write-Check 'the upgrade removed the previous install directory' `
    (-not (Test-Path $previousLocation)) $previousLocation
  Write-Check 'only one version is registered' `
    (@(Get-AppxPackage -Name $manifest.Name).Count -eq 1) `
    ((Get-AppxPackage -Name $manifest.Name).Version -join ', ')
}

# ---------------------------------------------------------------------------
# 3. Launch, deep link, log.
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'Launch'
$logDirectory = Join-Path $env:LOCALAPPDATA 'com.melaine.dae\logs'
$log = Join-Path $logDirectory 'dae.log'
$started = Get-Date
$process = Start-PackagedApp $installed.PackageFamilyName $manifest.AppId
Start-Sleep -Seconds 3
Write-Check 'the packaged app is running' (-not $process.HasExited) `
  ("pid {0}" -f $process.Id)

$stamp = Wait-ForLog $log $started
Write-Check 'the packaged app logs where the portable build does' `
  ($null -ne $stamp) $log
if ($stamp) {
  Write-Host '  last log lines:'
  Get-Content $log -Tail 5 | ForEach-Object { Write-Host "    $_" }
}

Write-Host ''
Write-Host 'Deep link'
$target = Join-Path $env:USERPROFILE 'Documents'
$link = 'dae://?path=' + [uri]::EscapeDataString($target)
Write-Host "  opening $link"
Start-Process $link
Start-Sleep -Seconds 3
$instances = @(Get-Process -Name dae -ErrorAction SilentlyContinue)
# A second process would mean the forwarded launch did not reach the owner, and
# two file managers would be fighting over the same window.
Write-Check 'the deep link went to the running instance' `
  ($instances.Count -eq 1) ("{0} process(es)" -f $instances.Count)

$command = $null
try {
  $command = (Get-ItemProperty -Path 'HKCU:\Software\Classes\dae.folder\shell\open\command' `
      -Name '(default)' -ErrorAction Stop).'(default)'
} catch {
  # Not an error: registering dae as the folder handler is a settings action,
  # and the alias is what makes it survive an upgrade.
}
if ($command) {
  Write-Host ''
  Write-Host 'Folder handler registration'
  Write-Check 'the registration points at the stable alias' `
    ($command -like "*WindowsApps\dae.exe*") $command
}

if ($Uninstall) {
  Write-Host ''
  Write-Host 'Uninstall'
  Stop-RunningInstance
  Remove-AppxPackage -Package $installed.PackageFullName
  Write-Check 'the package is gone' `
    ($null -eq (Get-Installed $manifest.Name)) $manifest.Name
}

Write-Host ''
if ($failures -gt 0) {
  Write-Host "$failures check(s) failed." -ForegroundColor Red
  exit 1
}
Write-Host 'All checks passed.' -ForegroundColor Green
