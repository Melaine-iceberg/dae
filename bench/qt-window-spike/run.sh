#!/usr/bin/env bash
# Build and run gate 3: frameless window, Mica, custom title bar, real input.
set -euo pipefail

cd "$(dirname "$0")"

QT_ROOT="${QT_ROOT:-C:/Qt/6.12.0/msvc2022_64}"
export QMAKE="$QT_ROOT/bin/qmake.exe"
PATH="$QT_ROOT/bin:$PATH" cargo build --release

PATH="$QT_ROOT/bin:$PATH" ./target/release/qt-window-spike.exe \
  --out results/win11-gate3.json --shots results
