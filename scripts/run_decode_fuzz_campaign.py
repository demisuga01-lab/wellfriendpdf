#!/usr/bin/env python3
"""Run bounded decode/parser fuzz campaigns and write reproducible summaries."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


TARGET_GROUPS = {
    "quick": ["filters", "predictor", "image_decoders"],
    "parser-decode": ["filters", "predictor", "parser_report", "xref_stream", "object_stream"],
    "risky-codec": ["image_decoders"],
    "all": [
        "filters",
        "predictor",
        "image_decoders",
        "parser_report",
        "xref_stream",
        "object_stream",
        "cos_object",
    ],
}


def run(cmd: list[str], cwd: Path, timeout: int | None = None) -> dict:
    started = datetime.now(timezone.utc)
    try:
        completed = subprocess.run(
            cmd,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout,
        )
        status = "ok" if completed.returncode == 0 else "failed"
        output = completed.stdout
        returncode = completed.returncode
    except subprocess.TimeoutExpired as exc:
        status = "timeout"
        output = (exc.stdout or "") if isinstance(exc.stdout, str) else ""
        returncode = None
    ended = datetime.now(timezone.utc)
    return {
        "cmd": cmd,
        "status": status,
        "returncode": returncode,
        "started": started.isoformat(),
        "ended": ended.isoformat(),
        "output_tail": output[-8000:],
    }


def git_value(repo: Path, args: list[str]) -> str:
    try:
        return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()
    except Exception:
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--group", choices=sorted(TARGET_GROUPS), default="quick")
    parser.add_argument("--target", action="append", help="Specific fuzz target; may be repeated")
    parser.add_argument("--runs", type=int, default=256, help="libFuzzer -runs value per target")
    parser.add_argument("--timeout", type=int, default=300, help="timeout seconds per target")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args()

    repo = args.repo.resolve()
    targets = args.target or TARGET_GROUPS[args.group]
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out_dir = args.out_dir or repo / "target" / "fuzz-campaigns" / f"{stamp}-{args.group}"
    out_dir.mkdir(parents=True, exist_ok=True)

    summary = {
        "repo": str(repo),
        "commit": git_value(repo, ["rev-parse", "--short", "HEAD"]),
        "rustc": run(["rustc", "--version"], repo)["output_tail"].strip(),
        "cargo": run(["cargo", "--version"], repo)["output_tail"].strip(),
        "group": args.group,
        "targets": targets,
        "runs": args.runs,
        "dry_run": args.dry_run,
        "started": datetime.now(timezone.utc).isoformat(),
        "results": [],
    }

    for target in targets:
        cmd = [
            "cargo",
            "+nightly",
            "fuzz",
            "run",
            target,
            "--",
            f"-runs={args.runs}",
        ]
        if args.dry_run:
            result = {"cmd": cmd, "status": "dry_run", "returncode": 0, "output_tail": ""}
        else:
            result = run(cmd, repo, timeout=args.timeout)
        result["target"] = target
        summary["results"].append(result)
        (out_dir / f"{target}.log").write_text(result.get("output_tail", ""), encoding="utf-8")

    summary["ended"] = datetime.now(timezone.utc).isoformat()
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps({"summary": str(out_dir / "summary.json"), "results": summary["results"]}, indent=2))
    return 0 if all(item["status"] in {"ok", "dry_run"} for item in summary["results"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
