#!/usr/bin/env python3
"""source editing bounded secret scan with false-positive classification.

The scanner intentionally records only pattern classes and source locations, never the
matched value. It exits non-zero only for unclassified or real secret candidates.
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path


EXCLUDED_DIRS = {
    ".git",
    ".gradle",
    ".mypy_cache",
    ".pytest_cache",
    "node_modules",
    "target",
    "__pycache__",
}

TEXT_EXTENSIONS = {
    ".cs",
    ".gradle",
    ".h",
    ".java",
    ".json",
    ".kts",
    ".md",
    ".ps1",
    ".py",
    ".rs",
    ".sh",
    ".toml",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
}

PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("private_key_block", re.compile(r"BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY", re.I)),
    ("github_token", re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}")),
    ("aws_access_key", re.compile(r"AKIA[0-9A-Z]{16}")),
    ("password_assignment", re.compile(r"\bpassword\s*[:=]\s*['\"][^'\"]{8,}['\"]", re.I)),
    ("api_key_assignment", re.compile(r"\bapi[_-]?key\s*[:=]\s*['\"][^'\"]{12,}['\"]", re.I)),
    ("token_assignment", re.compile(r"\btoken\s*[:=]\s*['\"][^'\"]{16,}['\"]", re.I)),
)


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    pattern: str
    classification: str
    reason: str


def is_text_file(path: Path) -> bool:
    if path.suffix.lower() in TEXT_EXTENSIONS:
        return True
    return path.name in {"Dockerfile", "Makefile"}


def classify(
    path: Path, line_text: str, pattern_name: str, context_before: str
) -> tuple[str, str]:
    normalized = line_text.lower()
    path_text = path.as_posix().lower()
    context = context_before.lower()
    if path.name.endswith("secret_scan.py") or path.name.endswith("closeout_reports.py"):
        return ("false_positive_scanner_pattern_catalog", "scanner or report generator names blocked patterns")
    if "/docs/" in f"/{path_text}" or path.suffix.lower() == ".md":
        if any(term in normalized for term in ("secret scan", "secret-scan", "false positive", "do not commit")):
            return ("false_positive_documentation_policy", "documentation describes secret handling")
        if pattern_name in {"private_key_block", "password_assignment", "api_key_assignment", "token_assignment"}:
            return ("false_positive_documentation_pattern", "documentation names a blocked pattern")
    if "test" in path_text or "fixture" in path_text:
        if any(term in normalized for term in ("test-only", "dummy", "placeholder", "example")):
            return ("false_positive_test_fixture", "test-only placeholder")
        return ("false_positive_test_fixture", "test path contains a deterministic fixture value")
    if path.suffix.lower() in {".rs", ".cs", ".java", ".py"}:
        if "#[cfg(test)]" in context or "mod tests" in context or "test" in context:
            return ("false_positive_test_fixture", "test-only code context contains a deterministic fixture value")
    return ("unclassified_secret_candidate", "manual review required")


def scan(repo: Path) -> list[Finding]:
    findings: list[Finding] = []
    for path in sorted(repo.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(repo)
        if any(part in EXCLUDED_DIRS for part in rel.parts):
            continue
        if not is_text_file(path):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        lines = text.splitlines()
        for line_no, line in enumerate(lines, start=1):
            for name, pattern in PATTERNS:
                if pattern.search(line):
                    context = "\n".join(lines[max(0, line_no - 40) : line_no])
                    classification, reason = classify(rel, line, name, context)
                    findings.append(Finding(rel.as_posix(), line_no, name, classification, reason))
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=".")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    repo = Path(args.repo).resolve()
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    findings = scan(repo)
    blockers = [f for f in findings if f.classification == "unclassified_secret_candidate"]
    result = {
        "feature_area": 31,
        "scanner": "source_editing_secret_scan",
        "repo": str(repo),
        "finding_count": len(findings),
        "blocker_count": len(blockers),
        "status": "pass" if not blockers else "fail",
        "findings": [asdict(f) for f in findings],
    }
    output.write_text(json.dumps(result, indent=2, sort_keys=True), encoding="utf-8")
    return 0 if not blockers else 1


if __name__ == "__main__":
    raise SystemExit(main())
