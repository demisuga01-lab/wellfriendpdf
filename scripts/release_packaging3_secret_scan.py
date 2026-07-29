#!/usr/bin/env python3
"""text reflow secret scan with no source-line disclosure in output.

The scanner records file, line, pattern family, and classification only. It
does not copy matched source text into retained artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


STRONG_PATTERNS = [
    ("private_key_material", re.compile(r"^\s*-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----")),
    ("aws_access_key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("github_token", re.compile(r"ghp_[A-Za-z0-9_]{20,}")),
]

KEYWORD_PATTERNS = [
    ("private_key_keyword", re.compile(r"PRIVATE KEY|BEGIN RSA PRIVATE KEY|BEGIN EC PRIVATE KEY|OPENSSH PRIVATE KEY")),
    ("password_keyword", re.compile(r"password", re.IGNORECASE)),
    ("token_keyword", re.compile(r"token", re.IGNORECASE)),
    ("api_key_keyword", re.compile(r"api_key", re.IGNORECASE)),
    ("ssh_public_key_keyword", re.compile(r"ssh-rsa", re.IGNORECASE)),
]

SKIP_DIRS = {
    ".git",
    "target",
    "build",
    "bin",
    "obj",
    ".gradle",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
}


def iter_files(roots: list[Path]):
    for root in roots:
        if not root.exists():
            continue
        if root.is_file():
            yield root
            continue
        for path in root.rglob("*"):
            if path.is_dir():
                continue
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            yield path


def read_text(path: Path) -> str | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if b"\x00" in data[:4096]:
        return None
    return data.decode("utf-8", errors="replace")


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def is_non_production_path(path: Path) -> bool:
    parts = {part.lower() for part in path.parts}
    return bool(parts & {"test", "tests", "fixture", "fixtures", "fuzz", "examples"}) or path.name == Path(__file__).name


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("roots", nargs="+")
    args = parser.parse_args()

    findings = []
    hard_failures = 0
    for path in iter_files([Path(root) for root in args.roots]):
        text = read_text(path)
        if text is None:
            continue
        digest = file_sha256(path)
        non_production = is_non_production_path(path)
        for line_no, line in enumerate(text.splitlines(), 1):
            for name, pattern in STRONG_PATTERNS:
                if pattern.search(line):
                    classification = (
                        "synthetic_or_detector_non_production_finding"
                        if non_production
                        else "potential_secret_material"
                    )
                    if classification == "potential_secret_material":
                        hard_failures += 1
                    findings.append(
                        {
                            "path": path.as_posix(),
                            "line": line_no,
                            "pattern": name,
                            "classification": classification,
                            "file_sha256": digest,
                        }
                    )
            for name, pattern in KEYWORD_PATTERNS:
                if pattern.search(line):
                    findings.append(
                        {
                            "path": path.as_posix(),
                            "line": line_no,
                            "pattern": name,
                            "classification": (
                                "synthetic_or_detector_non_production_finding"
                                if non_production
                                else "keyword_reference_no_secret_material"
                            ),
                            "file_sha256": digest,
                        }
                    )

    result = {
        "schema": "text_reflow.secret_scan.v1",
        "files_scanned": sum(1 for _ in iter_files([Path(root) for root in args.roots])),
        "scanned_roots": [str(Path(root).as_posix()) for root in args.roots],
        "excluded_directories": sorted(SKIP_DIRS),
        "finding_count": len(findings),
        "hard_failure_count": hard_failures,
        "findings": findings,
        "verdict": "pass" if hard_failures == 0 else "fail",
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0 if hard_failures == 0 else 2


if __name__ == "__main__":
    raise SystemExit(main())
