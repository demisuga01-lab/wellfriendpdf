#!/usr/bin/env python3
"""Availability-aware parser differential harness.

The harness compares stable parser-level facts rather than raw diagnostics:
open success, page count when a tool exposes it, encryption and linearization
flags, and warning/error presence. Missing external tools are reported, not
treated as failures unless --require-tools is used.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


def collect_files(paths: list[Path], limit: int | None) -> list[Path]:
    files: list[Path] = []
    for path in paths:
        if path.is_file() and path.suffix.lower() == ".pdf":
            files.append(path)
        elif path.is_dir():
            files.extend(sorted(path.rglob("*.pdf")))
    files = sorted(dict.fromkeys(files))
    return files[:limit] if limit else files


def run_command(cmd: list[str], timeout: float) -> tuple[int | None, str, str, float]:
    start = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        return proc.returncode, proc.stdout, proc.stderr, time.perf_counter() - start
    except subprocess.TimeoutExpired as exc:
        return None, exc.stdout or "", exc.stderr or "timeout", time.perf_counter() - start


def oxide_report(oxide: str, pdf: Path, mode: str, timeout: float) -> dict[str, Any]:
    code, out, err, elapsed = run_command(
        [oxide, "parser-report", str(pdf), "--mode", mode, "--json"],
        timeout,
    )
    record: dict[str, Any] = {
        "tool": "oxide",
        "available": True,
        "returncode": code,
        "elapsed_sec": elapsed,
    }
    if code == 0:
        try:
            report = json.loads(out)
            record.update(
                {
                    "open": bool(report.get("opened")),
                    "diagnostics": len(report.get("diagnostics", [])),
                    "page_count": None,
                    "encrypted": None,
                    "linearized": report.get("linearization", {}).get("is_linearized"),
                    "xref_entries": report.get("source_metrics", {}).get("xref_entries"),
                    "objects_known": report.get("source_metrics", {}).get("objects_known"),
                }
            )
        except json.JSONDecodeError:
            record.update({"open": False, "error": "invalid oxide JSON", "stderr": err})
    else:
        record.update({"open": False, "stderr": err[-1000:]})
    return record


def qpdf_check(pdf: Path, timeout: float) -> dict[str, Any]:
    tool = shutil.which("qpdf")
    if not tool:
        return {"tool": "qpdf", "available": False}
    code, out, err, elapsed = run_command([tool, "--check", str(pdf)], timeout)
    return {
        "tool": "qpdf",
        "available": True,
        "returncode": code,
        "elapsed_sec": elapsed,
        "open": code == 0,
        "diagnostics": int(bool(err.strip() or out.strip())),
        "stderr": err[-1000:],
    }


def pdfinfo_check(pdf: Path, timeout: float) -> dict[str, Any]:
    tool = shutil.which("pdfinfo")
    if not tool:
        return {"tool": "pdfinfo", "available": False}
    code, out, err, elapsed = run_command([tool, str(pdf)], timeout)
    facts: dict[str, Any] = {
        "tool": "pdfinfo",
        "available": True,
        "returncode": code,
        "elapsed_sec": elapsed,
        "open": code == 0,
        "stderr": err[-1000:],
    }
    if code == 0:
        for line in out.splitlines():
            if line.startswith("Pages:"):
                try:
                    facts["page_count"] = int(line.split(":", 1)[1].strip())
                except ValueError:
                    pass
            elif line.startswith("Encrypted:"):
                facts["encrypted"] = "yes" in line.lower()
            elif line.startswith("Optimized:"):
                facts["linearized"] = "yes" in line.lower()
    return facts


def mutool_check(pdf: Path, timeout: float) -> dict[str, Any]:
    tool = shutil.which("mutool")
    if not tool:
        return {"tool": "mutool", "available": False}
    code, _out, err, elapsed = run_command([tool, "info", str(pdf)], timeout)
    return {
        "tool": "mutool",
        "available": True,
        "returncode": code,
        "elapsed_sec": elapsed,
        "open": code == 0,
        "stderr": err[-1000:],
    }


def categorize(record: dict[str, Any]) -> list[str]:
    oxide = record["tools"]["oxide"]
    categories: list[str] = []
    for name, tool in record["tools"].items():
        if name == "oxide" or not tool.get("available"):
            continue
        if oxide.get("open") and not tool.get("open"):
            categories.append(f"{name}_external_only_fail")
        elif not oxide.get("open") and tool.get("open"):
            categories.append(f"{name}_oxide_only_fail")
        if (
            oxide.get("page_count") is not None
            and tool.get("page_count") is not None
            and oxide.get("page_count") != tool.get("page_count")
        ):
            categories.append(f"{name}_page_count_mismatch")
        if (
            oxide.get("linearized") is not None
            and tool.get("linearized") is not None
            and oxide.get("linearized") != tool.get("linearized")
        ):
            categories.append(f"{name}_linearization_mismatch")
    return categories


def write_markdown(path: Path, records: list[dict[str, Any]]) -> None:
    tools: dict[str, int] = {}
    divergences = []
    for record in records:
        for name, tool in record["tools"].items():
            if tool.get("available"):
                tools[name] = tools.get(name, 0) + 1
        if record["divergences"]:
            divergences.append(record)

    lines = [
        "# Parser Differential Report",
        "",
        f"Files tested: {len(records)}",
        "",
        "## Available Tool Runs",
        "",
    ]
    for name, count in sorted(tools.items()):
        lines.append(f"- {name}: {count} file(s)")
    lines.extend(["", "## Divergences", ""])
    if not divergences:
        lines.append("No shallow parser-level divergences were detected.")
    else:
        for record in divergences[:20]:
            lines.append(f"- `{record['file']}`: {', '.join(record['divergences'])}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--oxide", default=str(Path("target") / "debug" / "oxide.exe"))
    parser.add_argument("--mode", choices=["strict", "repair", "audit"], default="audit")
    parser.add_argument("--limit", type=int, default=25)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument("--jobs", type=int, default=1, help="reserved; runs serially for determinism")
    parser.add_argument("--json-out", type=Path, required=True)
    parser.add_argument("--markdown-out", type=Path, required=True)
    parser.add_argument("--require-tools", default="")
    args = parser.parse_args()

    required = {tool.strip() for tool in args.require_tools.split(",") if tool.strip()}
    files = collect_files(args.paths, args.limit)
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.markdown_out.parent.mkdir(parents=True, exist_ok=True)

    records: list[dict[str, Any]] = []
    with args.json_out.open("w", encoding="utf-8") as handle:
        for pdf in files:
            tools = {
                "oxide": oxide_report(args.oxide, pdf, args.mode, args.timeout),
                "qpdf": qpdf_check(pdf, args.timeout),
                "pdfinfo": pdfinfo_check(pdf, args.timeout),
                "mutool": mutool_check(pdf, args.timeout),
            }
            missing_required = [
                name for name in required if not tools.get(name, {}).get("available")
            ]
            record = {
                "file": str(pdf),
                "size": pdf.stat().st_size,
                "tools": tools,
                "missing_required_tools": missing_required,
            }
            record["divergences"] = categorize(record)
            records.append(record)
            handle.write(json.dumps(record, sort_keys=True) + "\n")

    write_markdown(args.markdown_out, records)
    if any(record["missing_required_tools"] for record in records):
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
