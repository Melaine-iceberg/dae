#!/usr/bin/env bash
# The same harness on Linux, where compio binds to io_uring instead of IOCP.
#
#   bash run-linux.sh /mnt/k/code/dae/bench/io-backends
#
# The crate is copied out of the mounted Windows drive first: building over
# 9p/drvfs is slow, and the datasets want a native filesystem to be worth
# measuring at all. The numbers are therefore not comparable to the Windows
# tables — they are a comparison of *mechanisms on Linux*, and the driver compio
# picked is printed at the top of every scenario so a `polling` fallback cannot
# pass for io_uring.
set -euo pipefail

source_dir="${1:?usage: run-linux.sh <crate dir>}"
work="$HOME/io-backends"
root="$HOME/dae-io-bench"

export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null; then
  echo "### installing rust"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
  sh /tmp/rustup.sh -y --profile minimal --default-toolchain stable
fi

mkdir -p "$work"
cp "$source_dir/Cargo.toml" "$source_dir/Cargo.lock" "$work/" 2>/dev/null || cp "$source_dir/Cargo.toml" "$work/"
cp -r "$source_dir/src" "$work/"
cp "$source_dir/summarize.py" "$work/"

cd "$work"
cargo build --release

bin="$work/target/release/io-backends"

echo "### prepare"
# 4 GiB of blob: this VM has ~19 GiB of RAM, so this one is the cache-resident
# one, and a 96 GiB cache-defeating blob would not fit on the virtual disk.
"$bin" prepare all --root "$root" --blob-mib 4096 --big-mib 4096

mkdir -p "$work/results"
run() {
  local scenario="$1"; shift
  local file="$1"; shift
  echo "### $scenario -> $file"
  "$bin" run "$scenario" --root "$root" --blob-mib 4096 --big-mib 4096 "$@" | tee "$work/results/$file"
}

run dispatch        dispatch.tsv        --threads 1,16    --passes 3 --warmup 1
run perop           perop.tsv           --threads 1,16    --passes 3 --warmup 1
run enumerate       enumerate.tsv       --threads 1,16    --passes 3 --warmup 1
run smallfiles      smallfiles.tsv      --threads 1,16    --passes 3 --warmup 1
run seqread         seqread.tsv         --threads 1,16    --passes 3 --warmup 1
run randread        randread.tsv        --threads 1,16,64 --passes 3 --warmup 1
run randread-multi  randread-multi.tsv  --threads 1,16,64 --passes 3 --warmup 1
run copy            copy-kernel.tsv     --threads 1,16    --passes 3 --warmup 1

python3 "$work/summarize.py" "$work/results" > "$work/results/summary.md"
cat "$work/results/summary.md"

# The per-pass TSVs land next to the Windows ones, so both platforms' raw
# numbers can be re-derived from the repository.
if [ -d /mnt/k/code/dae/bench/io-backends ]; then
  mkdir -p /mnt/k/code/dae/bench/io-backends/results-linux
  cp "$work"/results/*.tsv "$work"/results/summary.md /mnt/k/code/dae/bench/io-backends/results-linux/
fi
echo "### done"
