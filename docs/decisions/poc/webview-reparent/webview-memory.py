#!/usr/bin/env python3
"""Measure the memory cost of a dae process tree, per webview.

WebKitGTK gives every webview its own `WebKitWebProcess`, so the number of
webviews decides the footprint. `reparent`-based tab dragging needs one webview
per tab (see PLAN.md), which is why this is worth measuring before committing to
it.

Reports both RSS and PSS:
  * RSS counts shared pages once per process, so summing it over a WebKit tree
    double-counts the shared libraries and exaggerates the cost.
  * PSS (`/proc/<pid>/smaps_rollup`) splits shared pages proportionally, so
    summing PSS is the honest "what would we get back if this went away" number.

Usage:
    python3 scripts/dev/webview-memory.py <pid>
    python3 scripts/dev/webview-memory.py $(pgrep -f '[t]arget/debug/dae' | head -1)

To compare "1 webview" against "N webviews", take a baseline with the app
running normally, then a second reading with extra webviews alive (the
`DAE_POC_REPARENT=1` spike creates some).
"""

from __future__ import annotations

import os
import sys


def read_status(pid: int) -> dict[str, str]:
    out: dict[str, str] = {}
    try:
        with open(f"/proc/{pid}/status") as handle:
            for line in handle:
                key, _, value = line.partition(":")
                out[key.strip()] = value.strip()
    except OSError:
        pass
    return out


def read_pss_kb(pid: int) -> int | None:
    """PSS in kB from smaps_rollup, or None when the kernel does not expose it."""
    try:
        with open(f"/proc/{pid}/smaps_rollup") as handle:
            for line in handle:
                if line.startswith("Pss:"):
                    return int(line.split()[1])
    except OSError:
        return None
    return None


def read_comm(pid: int) -> str:
    try:
        with open(f"/proc/{pid}/comm") as handle:
            return handle.read().strip()
    except OSError:
        return "?"


def ppid(pid: int) -> int | None:
    value = read_status(pid).get("PPid")
    return int(value) if value and value.isdigit() else None


def tree(root: int) -> list[int]:
    children: dict[int, list[int]] = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        pid = int(entry)
        parent = ppid(pid)
        if parent is not None:
            children.setdefault(parent, []).append(pid)

    seen: list[int] = []
    stack = [root]
    while stack:
        pid = stack.pop()
        seen.append(pid)
        stack.extend(children.get(pid, []))
    return seen


def main() -> int:
    if len(sys.argv) != 2 or not sys.argv[1].isdigit():
        print(__doc__)
        return 2
    root = int(sys.argv[1])

    rows = []
    for pid in tree(root):
        status = read_status(pid)
        rss = status.get("VmRSS")
        if not rss:
            continue
        pss = read_pss_kb(pid)
        rows.append((int(rss.split()[0]), pss, pid, read_comm(pid)))

    if not rows:
        print(f"pid {root}: no readable processes (did the app exit?)")
        return 1

    rows.sort(reverse=True)
    total_rss = total_pss = 0
    webviews = 0
    print(f"{'RSS MB':>9} {'PSS MB':>9}  pid      process")
    for rss, pss, pid, comm in rows:
        total_rss += rss
        if pss is not None:
            total_pss += pss
        # /proc/<pid>/comm truncates to 15 chars: "WebKitWebProces".
        if comm.startswith("WebKitWebProces"):
            webviews += 1
        pss_text = f"{pss / 1024:.1f}" if pss is not None else "-"
        print(f"{rss / 1024:9.1f} {pss_text:>9}  {pid:<8} {comm}")

    print()
    print(f"webviews (WebKitWebProcess) = {webviews}")
    print(f"tree RSS total = {total_rss / 1024:.1f} MB (double-counts shared pages)")
    if total_pss:
        print(f"tree PSS total = {total_pss / 1024:.1f} MB (honest, sum over the tree)")
    else:
        print("tree PSS total = unavailable (no smaps_rollup)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
