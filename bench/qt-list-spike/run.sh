#!/usr/bin/env bash
# Build and run gate 1 twice: unthrottled frame time, then display cadence.
set -euo pipefail

cd "$(dirname "$0")"

QT_ROOT="${QT_ROOT:-C:/Qt/6.12.0/msvc2022_64}"
export QMAKE="$QT_ROOT/bin/qmake.exe"
export DAE_QT_VERSION="$("$QMAKE" -query QT_VERSION 2>/dev/null || echo unknown)"
PATH="$QT_ROOT/bin:$PATH" cargo build --release

DATASET="${DATASET:-K:/dae-listing-bench/huge}"
SECONDS_TO_RUN="${SECONDS_TO_RUN:-12}"
mkdir -p results

PATH="$QT_ROOT/bin:$PATH" ./target/release/qt-list-spike.exe \
  --path "$DATASET" --seconds "$SECONDS_TO_RUN" --vsync off \
  --out results/qt-list-spike-vsync-off.json

PATH="$QT_ROOT/bin:$PATH" ./target/release/qt-list-spike.exe \
  --path "$DATASET" --seconds "$SECONDS_TO_RUN" --vsync on \
  --out results/qt-list-spike-vsync-on.json
