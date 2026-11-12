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

# The source is the immutable commit archive named by VERSION's `+<hash>`
# suffix. The `tip` release tarball used to be fetched here, but `tip` is
# regenerated on every ghostty main commit and has moved past this pin;
# only the commit archive stays buildable against a fixed VERSION.
commit=${version##*+}
source_url="https://github.com/ghostty-org/ghostty/archive/$commit.tar.gz"

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
    # Download aside and move into place: a connection dropped mid-transfer
    # must not leave a partial tarball that the next run treats as complete.
    # --retry-all-errors covers receive failures (curl 56), which plain
    # --retry does not consider transient.
    curl -fsSL --retry 3 --retry-all-errors -o "$2.part" "$1"
    mv "$2.part" "$2"
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
# The archive name carries the commit so a stale tarball from an older pin
# (or from the rolling `tip` this script used to fetch) is never reused.
fetch "$source_url" "$work/ghostty-$commit.tar.gz"

src="$work/src"
rm -rf "$src"
mkdir -p "$src"
tar -xzf "$work/ghostty-$commit.tar.gz" -C "$src" --strip-components=1

# Git archives carry no LIB_VERSION (the release tarballs generate it, and
# build.zig falls back to its own lib_version constant without it). The
# archive's top-level directory is named after the commit instead, and that
# is what keeps the source honest against the VERSION pin. `sed -n 1p`
# rather than `head -n 1`: head would SIGPIPE tar under `set -o pipefail`.
top=$(tar -tzf "$work/ghostty-$commit.tar.gz" | sed -n '1p')
case "$top" in
    "ghostty-$commit"*) ;;
    *)
        echo "the source archive is $top but ghostty-vt/VERSION pins $version." >&2
        echo "Update VERSION and the dependency table together." >&2
        exit 1
        ;;
esac

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

# Upstream links libc into wuffs on Windows only. Everywhere else the
# libc-less build exports weak `calloc`/`free` stubs that return NULL
# (pkg/wuffs/src/main.zig), and in a static ELF link those interpose glibc:
# every calloc in the host process fails. dae always links libc, so build
# wuffs with it on every platform — the same configuration Windows uses.
perl -0pi -e '
    my $count = s/\.link_libc = windows,/.link_libc = true,/;
    die "wuffs link_libc pin is stale in $ARGV\n" unless $count;
' "$src/pkg/wuffs/build.zig"

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
