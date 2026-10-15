#!/usr/bin/env bash
# The batching curve: how much of the per-file cost is the dispatch itself.
#
# `--batch 1` is what a Tauri command does today (one blocking call per file);
# `--batch 1250` at 16 in flight is one call per thread, which is what
# `tokio-chunked`/rayon already do. Everything between is the interesting part.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/target/release/io-backends.exe"
root="${1:-K:/dae-io-bench}"
out="$here/results"

for scenario in smallfiles perop; do
  for batch in 1 2 4 8 16 32 64 128 256 512 1250; do
    echo "### $scenario batch=$batch"
    "$bin" run "$scenario" --root "$root" --backend tokio --threads 16 \
      --batch "$batch" --passes 3 --warmup 1 > "$out/$scenario-batch-$batch.tsv"
    rg "median" "$out/$scenario-batch-$batch.tsv" | rg -v warm
  done
done

# The same two endpoints with a single call in flight at a time: the shape a
# frontend has when it asks for one preview and waits for it.
for batch in 1 20000; do
  echo "### smallfiles batch=$batch (in flight 1)"
  "$bin" run smallfiles --root "$root" --backend tokio --threads 1 \
    --batch "$batch" --passes 3 --warmup 1 > "$out/smallfiles-batch1-thread1-$batch.tsv"
  rg "median" "$out/smallfiles-batch1-thread1-$batch.tsv" | rg -v warm
done

echo "### batching curve done"
