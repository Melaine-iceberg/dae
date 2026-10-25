#!/usr/bin/env bash
# Build and run gate 2: Chinese IME in a custom-styled QML TextInput.
set -euo pipefail

cd "$(dirname "$0")"

QT_ROOT="${QT_ROOT:-C:/Qt/6.12.0/msvc2022_64}"
export QMAKE="$QT_ROOT/bin/qmake.exe"
export QT_ASSUME_STDERR_HAS_CONSOLE=1
PATH="$QT_ROOT/bin:$PATH" cargo build --release

mkdir -p results/shots
PATH="$QT_ROOT/bin:$PATH" ./target/release/qt-ime-spike.exe \
  --out results/win11-ime.json \
  --shots results/shots
