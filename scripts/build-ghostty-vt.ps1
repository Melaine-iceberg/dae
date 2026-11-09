# Builds the vendored libghostty-vt static library for dae.
#
# Zig's dependency fetcher fails on some Windows builds ("failed to delete
# temporary directory" while installing a package into the global cache), so
# this script sidesteps it entirely: every dependency the pinned source tree
# names is downloaded with curl and extracted under local-deps/, and the
# build.zig.zon files are rewritten to point at those directories. The URLs
# below are part of the pin — when ghostty-vt/VERSION moves, update them and
# the version check will tell you if you forgot.
#
# Usage:
#   scripts/build-ghostty-vt.ps1
#   scripts/build-ghostty-vt.ps1 -Target aarch64-pc-windows-msvc -ZigTarget aarch64-windows-msvc
#   scripts/build-ghostty-vt.ps1 -Zig C:\tools\zig\zig.exe -Work D:\build\ghostty-vt

param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$ZigTarget = "x86_64-windows-msvc",
    [string]$Zig = "zig",
    [string]$Work = (Join-Path $env:TEMP "ghostty-vt")
)

$ErrorActionPreference = "Stop"

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

# `tip` is a rolling release; the LIB_VERSION check below is what keeps the
# artifacts and the dependency table honest.
$sourceUrl = "https://github.com/ghostty-org/ghostty/releases/download/tip/libghostty-vt-source.tar.gz"

# The dependency set of ghostty 1.1.0-dev+711702d, exactly as its zon files
# name them. Each archive holds one top-level directory and is extracted to
# local-deps/<name>.
$deps = [ordered]@{
    aro         = "https://deps.files.ghostty.org/aro-d0c8c4d9c55daa7ef6e40cf0f630a5b5e900989b.tar.gz"
    highway     = "https://deps.files.ghostty.org/highway-66486a10623fa0d72fe91260f96c892e41aceb06.tar.gz"
    pixels      = "https://deps.files.ghostty.org/pixels-12207ff340169c7d40c570b4b6a97db614fe47e0d83b5801a932dcd44917424c8806.tar.gz"
    translate_c = "https://deps.files.ghostty.org/translate_c-02ff0c523fb92a38939b04d0145c4a9602a74d56.tar.gz"
    uucode      = "https://deps.files.ghostty.org/uucode-1fb73433bba5d93366c57f23ff2e9d7939746500.tar.gz"
    wuffs       = "https://deps.files.ghostty.org/wuffs-7411f488fe2e2c205c3d3b3d28638b7356522930.tar.gz"
}

function Get-Archive([string]$Url, [string]$Out) {
    if (Test-Path $Out) { return }
    Write-Host "download $Url"
    & $curl -fsSL --retry 3 -o $Out $Url
    if ($LASTEXITCODE -ne 0) { throw "download failed: $Url" }
}

function Expand-Tarball([string]$Archive, [string]$Directory) {
    New-Item -ItemType Directory -Force $Directory | Out-Null
    & $tar -xzf $Archive -C $Directory --strip-components=1
    if ($LASTEXITCODE -ne 0) { throw "extract failed: $Archive" }
}

# Replaces the .url line and the .hash line below it with a single .path line,
# keeping the .url line's indentation. Fails loudly when the pinned source no
# longer carries that URL, so a version bump cannot silently produce a build
# that still fetches from the network.
function Set-ZonUrlPath([string]$Zon, [string]$Url, [string]$Path) {
    $text = [IO.File]::ReadAllText($Zon)
    $pattern = '([ \t]*)\.url\s*=\s*"' + [regex]::Escape($Url) + '"\s*,?\s*\r?\n\s*\.hash\s*=\s*"[^"]*"\s*,?'
    $updated = [regex]::Replace($text, $pattern, '${1}.path = "' + $Path + '",')
    if ($updated -eq $text) { throw "pin is stale in ${Zon}: $Url" }
    [IO.File]::WriteAllText($Zon, $updated, [Text.UTF8Encoding]::new($false))
}

# Same, for a dependency fetched from git — translate_c names aro that way.
function Set-ZonGitPath([string]$Zon, [string]$Entry, [string]$Path) {
    $text = [IO.File]::ReadAllText($Zon)
    $pattern = '(\.' + [regex]::Escape($Entry) + '\s*=\s*\.\{)(\s*)\.url\s*=\s*"git\+[^"]*"\s*,?\s*\.hash\s*=\s*"[^"]*"\s*,?'
    $updated = [regex]::Replace($text, $pattern, '${1}${2}.path = "' + $Path + '",')
    if ($updated -eq $text) { throw "pin is stale in ${Zon}: git dependency $Entry" }
    [IO.File]::WriteAllText($Zon, $updated, [Text.UTF8Encoding]::new($false))
}

Write-Host "libghostty-vt $version -> $Target"

$src = Join-Path $Work "src"
$sourceArchive = Join-Path $Work "libghostty-vt-source.tar.gz"
New-Item -ItemType Directory -Force $Work | Out-Null
Get-Archive $sourceUrl $sourceArchive
if (Test-Path $src) { Remove-Item -Recurse -Force $src }
Expand-Tarball $sourceArchive $src

$built = (Get-Content (Join-Path $src "LIB_VERSION") -Raw).Trim()
if ($built -ne $version) {
    throw "the source tarball is $built but ghostty-vt/VERSION pins $version; `tip` moved. Update VERSION and the dependency table together."
}

foreach ($name in $deps.Keys) {
    $archive = Join-Path $Work "$name.tar.gz"
    Get-Archive $deps[$name] $archive
    Expand-Tarball $archive (Join-Path $src "local-deps\$name")
}

Set-ZonUrlPath (Join-Path $src "build.zig.zon") $deps.aro "./local-deps/aro"
Set-ZonUrlPath (Join-Path $src "build.zig.zon") $deps.uucode "./local-deps/uucode"
Set-ZonUrlPath (Join-Path $src "pkg\translate-c\build.zig.zon") $deps.translate_c "../../local-deps/translate_c"
Set-ZonUrlPath (Join-Path $src "pkg\highway\build.zig.zon") $deps.highway "../../local-deps/highway"
Set-ZonUrlPath (Join-Path $src "pkg\wuffs\build.zig.zon") $deps.wuffs "../../local-deps/wuffs"
Set-ZonUrlPath (Join-Path $src "pkg\wuffs\build.zig.zon") $deps.pixels "../../local-deps/pixels"
Set-ZonGitPath (Join-Path $src "local-deps\translate_c\build.zig.zon") "aro" "../aro"

Push-Location $src
try {
    # -Dtarget is explicit because zig defaults to the GNU ABI on Windows and
    # the Rust host links MSVC. -Dsimd=false drops the vendored SIMD runtime so
    # the archive only needs libc (and ntdll, which build.rs adds).
    & $Zig build -Demit-lib-vt -Dsimd=false -Doptimize=ReleaseFast "-Dtarget=$ZigTarget"
    if ($LASTEXITCODE -ne 0) { throw "zig build failed" }
} finally {
    Pop-Location
}

$libDir = Join-Path $vendor "lib\$Target"
New-Item -ItemType Directory -Force $libDir | Out-Null
Copy-Item (Join-Path $src "zig-out\lib\ghostty-vt-static.lib") $libDir -Force
Copy-Item (Join-Path $src "LICENSE") (Join-Path $vendor "LICENSE") -Force
$headers = Join-Path $vendor "include\ghostty"
if (Test-Path $headers) { Remove-Item -Recurse -Force $headers }
Copy-Item (Join-Path $src "zig-out\include\ghostty") $headers -Recurse -Force

$size = (Get-Item (Join-Path $libDir "ghostty-vt-static.lib")).Length
Write-Host "vendored $libDir\ghostty-vt-static.lib ($([math]::Round($size / 1MB, 2)) MiB)"
