#!/usr/bin/env bash
# Builds the vendored libghostty-vt static library for dae (Linux/macOS).
#
# The pin is the official libghostty-vt release named in ghostty-vt/VERSION.
# A release ships two tarballs: the source, and every Zig package the build
# needs, already unpacked in Zig's own cache layout. Passing that directory
# as --system builds without touching the network, so there is no dependency
# table or build.zig.zon rewriting here to drift out of date.
#
# Usage:
#   scripts/build-ghostty-vt.sh
#   ZIG=/opt/zig/zig scripts/build-ghostty-vt.sh
#   RUST_TARGET=aarch64-unknown-linux-gnu ZIG_TARGET=aarch64-linux-gnu scripts/build-ghostty-vt.sh

set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
vendor="$repo/crates/dae-core/ghostty-vt"
version=$(tr -d '[:space:]' < "$vendor/VERSION")
work=${WORK:-${TMPDIR:-/tmp}/ghostty-vt}
zig=${ZIG:-zig}

rust_target=${RUST_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
case "$rust_target" in
    # Windows links the official prebuilt DLL, not a static archive; that is
    # scripts/build-ghostty-vt.ps1's job and needs no Zig.
    *-windows-*) echo "Windows uses the official libghostty-vt DLL: run scripts/build-ghostty-vt.ps1" >&2; exit 1 ;;
    x86_64-unknown-linux-gnu)    zig_target=${ZIG_TARGET:-x86_64-linux-gnu} ;;
    aarch64-unknown-linux-gnu)   zig_target=${ZIG_TARGET:-aarch64-linux-gnu} ;;
    x86_64-apple-darwin)         zig_target=${ZIG_TARGET:-x86_64-macos} ;;
    aarch64-apple-darwin)        zig_target=${ZIG_TARGET:-aarch64-macos} ;;
    *) zig_target=${ZIG_TARGET:-} ;;
esac

base="https://release.files.ghostty.org/libghostty-vt/$version"
source_url="$base/libghostty-vt-$version.tar.gz"
deps_url="$base/libghostty-vt-$version-deps.tar.gz"

# The SHA256SUMS lines for this release. They are part of the pin: a moved
# release file, or a proxy serving something else, fails the build instead
# of being vendored. Update both when VERSION moves.
source_sha256="333dfce97641ef9b536bb9e08907ad86abaaada50e8f7417b70d9284adef37aa"
deps_sha256="91a4e10960a52bf5bf4b15aa2a2cddaff8a89d6cd903d3055bda4fd17de3356d"

sha256() { # file
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        # macOS ships shasum, not sha256sum.
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

fetch() { # url out expected
    local actual
    if [ -f "$2" ]; then
        actual=$(sha256 "$2")
        if [ "$actual" = "$3" ]; then return; fi
        echo "stale $2 (sha256 $actual), refetching" >&2
        rm -f "$2"
    fi
    echo "download $1"
    # Download aside and move into place: a connection dropped mid-transfer
    # must not leave a partial tarball that the next run treats as complete.
    # --retry-all-errors covers receive failures (curl 56), which plain
    # --retry does not consider transient.
    curl -fsSL --retry 3 --retry-all-errors -o "$2.part" "$1"
    actual=$(sha256 "$2.part")
    if [ "$actual" != "$3" ]; then
        rm -f "$2.part"
        echo "checksum mismatch for $1: expected $3, got $actual" >&2
        exit 1
    fi
    mv "$2.part" "$2"
}

echo "libghostty-vt $version -> $rust_target"
mkdir -p "$work"
fetch "$source_url" "$work/libghostty-vt-$version.tar.gz" "$source_sha256"
fetch "$deps_url" "$work/libghostty-vt-$version-deps.tar.gz" "$deps_sha256"

src="$work/libghostty-vt-$version"
deps="$work/libghostty-vt-$version-deps"
rm -rf "$src" "$deps"
tar -xzf "$work/libghostty-vt-$version.tar.gz" -C "$work"
tar -xzf "$work/libghostty-vt-$version-deps.tar.gz" -C "$work"
if [ ! -f "$src/LIB_VERSION" ]; then
    echo "$src is not a libghostty-vt source tarball (no LIB_VERSION)." >&2
    echo "Use the release tarball, not the GitHub archive for a vt-* tag." >&2
    exit 1
fi

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
# -Dcpu=baseline keeps a native target from baking in the build host's CPU.
build_args=(-Demit-lib-vt -Dsimd=false -Doptimize=ReleaseFast -Dcpu=baseline)
if [ -n "$zig_target" ]; then
    build_args+=("-Dtarget=$zig_target")
fi

# A fresh global cache is required: Zig 0.17 can crash ("reached unreachable
# code") when the cache holds an archive of a package that --system also
# provides. An empty one also proves the dependency tarball is complete.
cache="$work/cache"
rm -rf "$cache"
(cd "$src" && ZIG_GLOBAL_CACHE_DIR="$cache" "$zig" build "${build_args[@]}" --system "$deps")

mkdir -p "$vendor/lib/$rust_target"
cp "$src/zig-out/lib/libghostty-vt.a" "$vendor/lib/$rust_target/"
cp "$src/LICENSE" "$vendor/LICENSE"
rm -rf "$vendor/include/ghostty"
cp -R "$src/zig-out/include/ghostty" "$vendor/include/ghostty"

echo "vendored $vendor/lib/$rust_target/libghostty-vt.a ($(du -h "$vendor/lib/$rust_target/libghostty-vt.a" | cut -f1))"
