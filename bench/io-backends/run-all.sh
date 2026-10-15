#!/usr/bin/env bash
# The full sweep, in the order the report's tables are in.
#
# Order matters: the read-only workloads run first on a quiet volume, then the
# two that write (copy, write), then the two cold ones — whose windows rotate,
# so their position in the sequence does not matter.
#
#   ./run-all.sh [root]        # default root: K:/dae-io-bench
#
# Every scenario's TSV lands in ./results/<scenario>[-readwrite].tsv.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/target/release/io-backends"
if [ -x "$bin.exe" ]; then bin="$bin.exe"; fi
root="${1:-K:/dae-io-bench}"
out="$here/results"
mkdir -p "$out"

# `prepare` is idempotent and skips datasets that are already the right size, so
# the sweep is one command from a fresh clone rather than a documented two-step.
"$bin" prepare all --root "$root"

run() {
  local scenario="$1"; shift
  local file="$1"; shift
  echo "### $scenario -> $file"
  "$bin" run "$scenario" --root "$root" "$@" | tee "$out/$file"
}

run dispatch     dispatch.tsv      --threads 1,16    --passes 3 --warmup 1
run perop        perop.tsv         --threads 1,16    --passes 3 --warmup 1
run enumerate    enumerate.tsv     --threads 1,16    --passes 3 --warmup 1
run smallfiles   smallfiles.tsv    --threads 1,16    --passes 3 --warmup 1
run seqread      seqread.tsv       --threads 1,16    --passes 3 --warmup 1
run randread     randread.tsv      --threads 1,16,64 --passes 3 --warmup 1
run randread-multi randread-multi.tsv --threads 1,16,64 --passes 3 --warmup 1

run write        write.tsv         --threads 1,16    --passes 2 --warmup 1

DAE_BENCH_COPY_MODE=kernel    run copy copy-kernel.tsv    --threads 1,16 --passes 2 --warmup 1
DAE_BENCH_COPY_MODE=readwrite run copy copy-readwrite.tsv --threads 1,16 --passes 2 --warmup 1

run randread-big randread-big.tsv  --threads 1,16,64 --passes 2 --warmup 0
run randread-big-multi randread-big-multi.tsv --threads 1,16,64 --passes 2 --warmup 0
run seqread-big  seqread-big.tsv   --threads 1,16    --passes 2 --warmup 0

echo "### done"
