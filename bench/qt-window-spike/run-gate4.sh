#!/usr/bin/env bash
# Build and run gate 4: tear a tab into a second window, measure its cost.
set -euo pipefail

cd "$(dirname "$0")"

QT_ROOT="${QT_ROOT:-C:/Qt/6.12.0/msvc2022_64}"
export QMAKE="$QT_ROOT/bin/qmake.exe"
PATH="$QT_ROOT/bin:$PATH" cargo build --release

PATH="$QT_ROOT/bin:$PATH" ./target/release/qt-window-spike.exe \
  --gate4 --cycles "${CYCLES:-6}" \
  --path "${DATASET:-K:/dae-listing-bench/huge}" \
  --out results/win11-gate4.json --shots results
