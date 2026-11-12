# Vendors the official libghostty-vt Windows build for dae.
#
# Windows links the official prebuilt DLL through its import library, so this
# script downloads the release zip and copies three things out of it: the DLL,
# the import library `ghostty-vt.lib`, and the headers. Everything else (the
# static library, the PDB) stays out of the tree. There is no Zig build on
# Windows.
#
# The pin is the release named in ghostty-vt/VERSION. The zip checksum below is
# part of the pin: a moved release file, or a proxy serving something else,
# fails the script instead of being vendored.
#
# Usage:
#   scripts/build-ghostty-vt.ps1
#   scripts/build-ghostty-vt.ps1 -Target aarch64-pc-windows-msvc
#   scripts/build-ghostty-vt.ps1 -Work D:\build\ghostty-vt

param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$Work = (Join-Path $env:TEMP "ghostty-vt")
)

$ErrorActionPreference = "Stop"

# The zip the official release publishes per architecture, and its SHA256SUMS
# line. Both are part of the pin: update them together with VERSION.
$releases = @{
    "x86_64-pc-windows-msvc"  = @{
        Zip  = "x86_64-windows-gnu"
        Sha  = "8e2eb9e1822e85d786b456c8beb86f49d09c318c91f0c2913f91e7ffa12cda29"
    }
    "aarch64-pc-windows-msvc" = @{
        Zip  = "aarch64-windows-gnu"
        Sha  = "cc61da5f968ea0b08639bdf019c6b7da46bad1d985774fd8a2de57af32862892"
    }
}
$release = $releases[$Target]
if (-not $release) {
    throw "no official Windows build for $Target; the release publishes x86_64 and aarch64"
}

# Git for Windows puts GNU tar on PATH ahead of the system bsdtar, and GNU tar
# reads a C:\... path as a remote host. Name the Windows tools explicitly.
$curl = Join-Path $env:SystemRoot "System32\curl.exe"
$tar = Join-Path $env:SystemRoot "System32\tar.exe"
foreach ($tool in @($curl, $tar)) {
    if (-not (Test-Path $tool)) { throw "missing Windows tool: $tool (Windows 10 1803+ required)" }
}

$repo = Split-Path -Parent $PSScriptRoot
$vendor = Join-Path $repo "crates\dae-core\ghostty-vt"
$version = (Get-Content (Join-Path $vendor "VERSION") -Raw).Trim()

$name = "ghostty-vt-$version-$($release.Zip)"
$url = "https://release.files.ghostty.org/libghostty-vt/$version/$name.zip"
$sha256 = $release.Sha

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$archive = Join-Path $Work "$name.zip"
if ((Test-Path $archive) -and (Get-Sha256 $archive) -ne $sha256) {
    Write-Host "stale $archive, refetching"
    Remove-Item $archive -Force
}
if (-not (Test-Path $archive)) {
    New-Item -ItemType Directory -Force $Work | Out-Null
    Write-Host "download $url"
    # Download aside and move into place: a connection dropped mid-transfer
    # must not leave a partial zip that the next run treats as complete.
    & $curl -fsSL --retry 3 --retry-all-errors -o "$archive.part" $url
    if ($LASTEXITCODE -ne 0) { throw "download failed: $url" }
    $actual = Get-Sha256 "$archive.part"
    if ($actual -ne $sha256) {
        Remove-Item "$archive.part" -Force
        throw "checksum mismatch for ${url}: expected $sha256, got $actual"
    }
    Move-Item "$archive.part" $archive -Force
}

$tree = Join-Path $Work $name
if (Test-Path $tree) { Remove-Item -Recurse -Force $tree }
& $tar -xzf $archive -C $Work
if ($LASTEXITCODE -ne 0) { throw "extract failed: $archive" }
if (-not (Test-Path (Join-Path $tree "bin\ghostty-vt.dll"))) {
    throw "$tree does not look like a libghostty-vt Windows release (no bin\ghostty-vt.dll)"
}

$libDir = Join-Path $vendor "lib\$Target"
New-Item -ItemType Directory -Force $libDir | Out-Null
Copy-Item (Join-Path $tree "bin\ghostty-vt.dll") $libDir -Force
Copy-Item (Join-Path $tree "lib\ghostty-vt.lib") $libDir -Force
# Earlier pins vendored the static library here. Leaving it behind would be a
# 10 MB file nothing links.
Remove-Item (Join-Path $libDir "ghostty-vt-static.lib") -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $tree "LICENSE") (Join-Path $vendor "LICENSE") -Force
$headers = Join-Path $vendor "include\ghostty"
if (Test-Path $headers) { Remove-Item -Recurse -Force $headers }
Copy-Item (Join-Path $tree "include\ghostty") $headers -Recurse -Force

$dll = (Get-Item (Join-Path $libDir "ghostty-vt.dll")).Length
Write-Host "vendored $libDir\ghostty-vt.dll ($([math]::Round($dll / 1MB, 2)) MiB) and ghostty-vt.lib"
