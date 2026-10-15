#!/usr/bin/env python3
"""Turns ./results/*.tsv into the markdown tables used by the report.

    python summarize.py [results-dir]
"""
import csv
import pathlib
import sys

BACKENDS = ["tokio", "tokio-chunked", "rayon", "compio"]


def load(path):
    rows = []
    with open(path, newline="", encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("#") or not line.strip():
                continue
            rows.append(line.rstrip("\n").split("\t"))
    header, rows = rows[0], rows[1:]
    return [dict(zip(header, row)) for row in rows]


def main():
    directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "results")
    for path in sorted(directory.glob("*.tsv")):
        rows = [row for row in load(path) if row["pass"] == "median"]
        if not rows:
            continue
        unit = {
            "enumerate": "entries/s",
            "dispatch": "kops/s",
            "perop": "kops/s",
        }.get(path.stem, "MiB/s")

        threads = sorted({int(row["threads"]) for row in rows})
        print(f"\n## {path.stem} ({unit})\n")
        header = ["threads"] + BACKENDS + ["tokio/compio"]
        print("| " + " | ".join(header) + " |")
        print("|" + "---|" * len(header))

        for thread in threads:
            cells = [str(thread)]
            values = {}
            for backend in BACKENDS:
                match = [
                    row
                    for row in rows
                    if int(row["threads"]) == thread and row["backend"] == backend
                ]
                if match:
                    values[backend] = float(match[0]["unit_per_s"])
                    cpu = match[0]["cpu_ms"]
                    suffix = "" if cpu == "-" else f" ({float(cpu) / 1000:.1f}s cpu)"
                    cells.append(f"{values[backend]:.0f}{suffix}")
                else:
                    cells.append("-")
            if "tokio" in values and "compio" in values and values["compio"]:
                cells.append(f"{values['tokio'] / values['compio']:.2f}")
            else:
                cells.append("-")
            print("| " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
