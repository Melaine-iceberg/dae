#!/usr/bin/env bash
# Builds the vendored libghostty-vt static library for dae (Linux/macOS).
#
# Zig's dependency fetcher does not work reliably on every host, so this
# script sidesteps it: every dependency the pinned source tree names is
# downloaded with curl and extracted under local-deps/, and the build.zig.zon
# files are rewritten to point at those directories. The URLs below are part
# of the pin — when ghostty-vt/VERSION moves, update them and the version
# check will tell you if you forgot.
#
# Usage:
#   scripts/build-ghostty-vt.sh
#   ZIG=/opt/zig/zig scripts/build-ghostty-vt.sh
#   RUST_TARGET=aarch64-unknown-linux-gnu ZIG_TARGET=aarch64-linux-gnu scripts/build-ghostty-vt.sh

set -euo pipefail

repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
vendor="$repo/crates/dae-core/ghostty-vt"
version=$(tr -d '[:space:]' < "$vendor/VERSION")
work=${WORK:-${TMPDIR:-/tmp}/ghostty-vt}
zig=${ZIG:-zig}

rust_target=${RUST_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
case "$rust_target" in
    x86_64-unknown-linux-gnu)    zig_target=${ZIG_TARGET:-x86_64-linux-gnu} ;;
    aarch64-unknown-linux-gnu)   zig_target=${ZIG_TARGET:-aarch64-linux-gnu} ;;
    x86_64-apple-darwin)         zig_target=${ZIG_TARGET:-x86_64-macos} ;;
    aarch64-apple-darwin)        zig_target=${ZIG_TARGET:-aarch64-macos} ;;
    *) zig_target=${ZIG_TARGET:-} ;;
esac

# `tip` is a rolling release; the LIB_VERSION check below is what keeps the
# artifacts and the dependency table honest.
source_url="https://github.com/ghostty-org/ghostty/releases/download/tip/libghostty-vt-source.tar.gz"

# The dependency set of ghostty 1.1.0-dev+711702d, exactly as its zon files
# name them. Each archive holds one top-level directory and is extracted to
# local-deps/<name>.
dep_names=(aro highway pixels translate_c uucode wuffs)
aro_url="https://deps.files.ghostty.org/aro-d0c8c4d9c55daa7ef6e40cf0f630a5b5e900989b.tar.gz"
highway_url="https://deps.files.ghostty.org/highway-66486a10623fa0d72fe91260f96c892e41aceb06.tar.gz"
pixels_url="https://deps.files.ghostty.org/pixels-12207ff340169c7d40c570b4b6a97db614fe47e0d83b5801a932dcd44917424c8806.tar.gz"
translate_c_url="https://deps.files.ghostty.org/translate_c-02ff0c523fb92a38939b04d0145c4a9602a74d56.tar.gz"
uucode_url="https://deps.files.ghostty.org/uucode-1fb73433bba5d93366c57f23ff2e9d7939746500.tar.gz"
wuffs_url="https://deps.files.ghostty.org/wuffs-7411f488fe2e2c205c3d3b3d28638b7356522930.tar.gz"
declare -A dep_urls=(
    [aro]=$aro_url [highway]=$highway_url [pixels]=$pixels_url
    [translate_c]=$translate_c_url [uucode]=$uucode_url [wuffs]=$wuffs_url
)

fetch() { # url out
    [ -f "$2" ] && return
    echo "download $1"
    curl -fsSL --retry 3 -o "$2" "$1"
}

# Replaces the .url line and the .hash line below it with a single .path line,
# keeping the .url line's indentation. Fails loudly when the pinned source no
# longer carries that URL, so a version bump cannot silently produce a build
# that still fetches from the network.
patch_zon() { # file url path
    URL=$2 DEST=$3 perl -0pi -e '
        my $url = $ENV{URL};
        my $dest = $ENV{DEST};
        my $re = qr/([ \t]*)\.url\s*=\s*"\Q$url\E"\s*,?\s*\r?\n\s*\.hash\s*=\s*"[^"]*"\s*,?/;
        my $count = s/$re/$1.path = "$dest",/;
        die "pin is stale in $ARGV: $url\n" unless $count;
    ' "$1"
}

# Same, for a dependency fetched from git — translate_c names aro that way.
patch_zon_git() { # file entry path
    ENTRY=$2 DEST=$3 perl -0pi -e '
        my $entry = $ENV{ENTRY};
        my $dest = $ENV{DEST};
        my $re = qr/(\.\Q$entry\E\s*=\s*\.\{)(\s*)\.url\s*=\s*"git\+[^"]*"\s*,?\s*\.hash\s*=\s*"[^"]*"\s*,?/;
        my $count = s/$re/$1$2.path = "$dest",/;
        die "pin is stale in $ARGV: git dependency $entry\n" unless $count;
    ' "$1"
}

echo "libghostty-vt $version -> $rust_target"
mkdir -p "$work"
fetch "$source_url" "$work/libghostty-vt-source.tar.gz"

src="$work/src"
rm -rf "$src"
mkdir -p "$src"
tar -xzf "$work/libghostty-vt-source.tar.gz" -C "$src" --strip-components=1

built=$(tr -d '[:space:]' < "$src/LIB_VERSION")
if [ "$built" != "$version" ]; then
    echo "the source tarball is $built but ghostty-vt/VERSION pins $version; 'tip' moved." >&2
    echo "Update VERSION and the dependency table together." >&2
    exit 1
fi

for name in "${dep_names[@]}"; do
    fetch "${dep_urls[$name]}" "$work/$name.tar.gz"
    mkdir -p "$src/local-deps/$name"
    tar -xzf "$work/$name.tar.gz" -C "$src/local-deps/$name" --strip-components=1
done

patch_zon "$src/build.zig.zon" "$aro_url" "./local-deps/aro"
patch_zon "$src/build.zig.zon" "$uucode_url" "./local-deps/uucode"
patch_zon "$src/pkg/translate-c/build.zig.zon" "$translate_c_url" "../../local-deps/translate_c"
patch_zon "$src/pkg/highway/build.zig.zon" "$highway_url" "../../local-deps/highway"
patch_zon "$src/pkg/wuffs/build.zig.zon" "$wuffs_url" "../../local-deps/wuffs"
patch_zon "$src/pkg/wuffs/build.zig.zon" "$pixels_url" "../../local-deps/pixels"
patch_zon_git "$src/local-deps/translate_c/build.zig.zon" aro "../aro"

# -Dsimd=false drops the vendored SIMD runtime so the archive only needs libc.
build_args=(-Demit-lib-vt -Dsimd=false -Doptimize=ReleaseFast)
if [ -n "$zig_target" ]; then
    build_args+=("-Dtarget=$zig_target")
fi
(cd "$src" && "$zig" build "${build_args[@]}")

mkdir -p "$vendor/lib/$rust_target"
cp "$src/zig-out/lib/libghostty-vt.a" "$vendor/lib/$rust_target/"
cp "$src/LICENSE" "$vendor/LICENSE"
rm -rf "$vendor/include/ghostty"
cp -R "$src/zig-out/include/ghostty" "$vendor/include/ghostty"

echo "vendored $vendor/lib/$rust_target/libghostty-vt.a ($(du -h "$vendor/lib/$rust_target/libghostty-vt.a" | cut -f1))"
