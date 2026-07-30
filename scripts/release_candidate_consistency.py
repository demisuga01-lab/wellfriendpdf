#!/usr/bin/env python3
"""Validate release-candidate README benchmark figures against summary.json."""

from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
README = ROOT / "README.md"
SUMMARY = ROOT / "benchmarks" / "results" / "release-candidate" / "summary.json"


def median(summary: dict, task: str, tool: str) -> str:
    for row in summary["rows"]:
        if row["task"] == task and row["tool"] == tool:
            value = row.get("median_sec")
            return "Not measured" if value is None else f"{value * 1000:.2f} ms"
    return "Not measured"


def main() -> int:
    summary = json.loads(SUMMARY.read_text(encoding="utf-8"))
    readme = README.read_text(encoding="utf-8")
    corpus = summary["corpus"]
    expected = [
        str(corpus["file_count"]),
        str(corpus["page_count"]),
        str(corpus["bytes"]),
        median(summary, "render_72", "wellfriend"),
        median(summary, "render_150", "wellfriend"),
        median(summary, "render_300", "wellfriend"),
        median(summary, "render_72", "poppler"),
        median(summary, "render_150", "poppler"),
        median(summary, "render_300", "poppler"),
        median(summary, "extract_text", "wellfriend"),
        median(summary, "extract_text", "poppler"),
        median(summary, "source_linked_text_replace", "wellfriend"),
    ]
    missing = [value for value in expected if value not in readme]
    forbidden = [value for value in ["35.185.176.47", "b741608"] if value in readme]
    if missing or forbidden:
        print(
            json.dumps(
                {"status": "fail", "missing": missing, "forbidden": forbidden},
                indent=2,
            )
        )
        return 1
    print(json.dumps({"status": "pass", "checked_values": len(expected)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
