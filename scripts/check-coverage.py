# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Fail when Rust line coverage falls below the committed baseline.

Reads the JSON summary written by `cargo llvm-cov --json --summary-only`. The
baseline only moves up: raise it in tools/coverage-baseline.json when coverage
improves, and never lower it without maintainer review.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "tools" / "coverage-baseline.json"
# Thread scheduling can move a handful of lines between runs.
TOLERANCE = 0.25


def line_percent(report: Path) -> float:
    data = json.loads(report.read_text())
    percent = data["data"][0]["totals"]["lines"]["percent"]
    if not isinstance(percent, int | float):
        raise ValueError("coverage report has no line percentage")
    return float(percent)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="cargo llvm-cov JSON summary")
    args = parser.parse_args()
    baseline = float(json.loads(BASELINE.read_text())["rust_line_percent"])
    actual = line_percent(args.report)
    print(f"Rust line coverage: {actual:.2f}% (baseline {baseline:.2f}%)")
    if actual + TOLERANCE < baseline:
        raise SystemExit(
            f"Coverage fell {baseline - actual:.2f} points below the baseline. "
            "Add tests for the changed code."
        )
    if actual >= baseline + 1:
        print(f"Coverage improved; raise rust_line_percent in {BASELINE.relative_to(ROOT)}.")


if __name__ == "__main__":
    main()
