#!/usr/bin/env python3
"""Fail when tracked active files expose internal roadmap-number names."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORD = "pro" + "mpt"
PATTERNS = [
    ("combined_roadmap_word", re.compile(rf"\bcombined[ _-]*{WORD}\b", re.IGNORECASE)),
    ("roadmap_word_number_spaced", re.compile(rf"\b{WORD}[ _-]*\d+[a-z]?\b", re.IGNORECASE)),
    ("roadmap_word_number_compact", re.compile(rf"\b{WORD}\d+[a-z]?\b", re.IGNORECASE)),
    ("roadmap_upper_number", re.compile(rf"{WORD.upper()}\d+")),
    ("roadmap_snake_number", re.compile(rf"{WORD}_\d+")),
    ("roadmap_dash_number", re.compile(rf"{WORD}-\d+")),
]

SKIP_PREFIXES = (
    ".git/",
    "target/",
    ".venv-public-benchmark/",
)
SKIP_PARTS = {
    ".git",
    "target",
    ".venv-public-benchmark",
}
TEXT_EXTENSIONS = {
    ".adoc",
    ".bat",
    ".c",
    ".cmd",
    ".cs",
    ".css",
    ".csv",
    ".gradle",
    ".h",
    ".html",
    ".java",
    ".js",
    ".json",
    ".jsonl",
    ".md",
    ".mjs",
    ".ps1",
    ".py",
    ".rs",
    ".sh",
    ".svg",
    ".toml",
    ".ts",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
}


def git_files() -> list[str]:
    completed = subprocess.run(
        ["git", "ls-files"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(completed.stderr.strip() or "git ls-files failed")
    return [line for line in completed.stdout.splitlines() if line]


def should_scan_path(path: str) -> bool:
    normalized = path.replace("\\", "/")
    if normalized.startswith(SKIP_PREFIXES):
        return False
    if any(part in SKIP_PARTS for part in Path(normalized).parts):
        return False
    return True


def is_text_file(path: Path) -> bool:
    if path.suffix.lower() not in TEXT_EXTENSIONS and path.name not in {
        "Dockerfile",
        "LICENSE",
        "NOTICE",
        "README",
    }:
        return False
    try:
        sample = path.read_bytes()[:4096]
    except OSError:
        return False
    return b"\x00" not in sample


def scan_text(path: Path) -> list[dict[str, object]]:
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return []
    findings: list[dict[str, object]] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        for family, pattern in PATTERNS:
            if pattern.search(line):
                findings.append(
                    {
                        "family": family,
                        "line": line_number,
                    }
                )
                break
    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=False)
    args = parser.parse_args()

    findings: list[dict[str, object]] = []
    for rel in git_files():
        if not should_scan_path(rel):
            continue
        if any(pattern.search(rel) for _, pattern in PATTERNS):
            findings.append({"path": rel, "kind": "path", "matches": [{"family": "path"}]})
            continue
        path = ROOT / rel
        if path.exists() and path.is_file() and is_text_file(path):
            matches = scan_text(path)
            if matches:
                findings.append({"path": rel, "kind": "content", "matches": matches})

    report = {
        "verdict": "zero_internal_roadmap_names" if not findings else "internal_roadmap_names_found",
        "finding_count": len(findings),
        "findings": findings,
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    else:
        print(json.dumps(report, indent=2))
    return 0 if not findings else 1


if __name__ == "__main__":
    sys.exit(main())
