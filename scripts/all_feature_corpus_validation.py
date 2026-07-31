#!/usr/bin/env python3
"""Run Wellfriend feature surfaces over a PDF corpus and retain compact evidence.

The harness intentionally stores only aggregate JSON/JSONL metadata:
exit status, duration, timeout, output byte size, and short error class. Large
feature outputs are written to a temporary directory and removed after each
case unless --keep-outputs is selected.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from typing import Iterable


STAGES: dict[str, list[str]] = {
    "info": ["info", "--json", "{pdf}"],
    "parser_report": [
        "parser-report",
        "--json",
        "--include-source-metrics",
        "--include-decode",
        "--fail-on",
        "never",
        "-o",
        "{out}",
        "{pdf}",
    ],
    "security_report": ["security-report", "--json", "{pdf}"],
    "validate": ["validate", "--json", "--fail-on", "never", "{pdf}"],
    "fonts": ["fonts", "--json", "{pdf}"],
    "extract_text_structured": [
        "extract-text",
        "--structured",
        "--format",
        "json",
        "-o",
        "{out}",
        "{pdf}",
    ],
    "parse_json": ["parse", "--format", "json", "-o", "{out}", "{pdf}"],
    "extract_tables": ["extract-tables", "--format", "json", "-o", "{out}", "{pdf}"],
    "forms_report": ["forms-report", "-o", "{out}", "{pdf}"],
    "annotations_report": ["annotations-report", "-o", "{out}", "{pdf}"],
    "document_subsystems_report": ["document-subsystems-report", "-o", "{out}", "{pdf}"],
    "document_subsystems_analyze": ["document-subsystems-analyze", "-o", "{out}", "{pdf}"],
    "document_security_report": ["document-security-report", "-o", "{out}", "{pdf}"],
    "document_security_analyze": ["document-security-analyze", "-o", "{out}", "{pdf}"],
    "layout_analyze_page1": ["layout-analyze", "--page", "1", "--json-output", "{out}", "{pdf}"],
    "reading_order_report": ["reading-order-report", "-o", "{out}", "{pdf}"],
    "flow_graph_report": ["flow-graph-report", "-o", "{out}", "{pdf}"],
    "render_compare_page1": [
        "render-compare",
        "--pages",
        "1",
        "--render-quality",
        "compat",
        "-o",
        "{out}",
        "{pdf}",
    ],
}


ERROR_MARKERS = {
    "password": ["password", "encrypted"],
    "timeout": ["timed out"],
    "panic": ["panicked", "panic"],
    "oom": ["out of memory", "memory allocation"],
    "unsupported": ["unsupported", "not supported"],
    "parse": ["parse", "xref", "trailer", "malformed"],
}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def classify_error(exit_code: int | None, stderr: str, timed_out: bool) -> str:
    if timed_out:
        return "timeout"
    if exit_code == 0:
        return "none"
    text = stderr.lower()
    for label, markers in ERROR_MARKERS.items():
        if any(marker in text for marker in markers):
            return label
    return "nonzero_exit"


def iter_pdfs(corpus: Path) -> list[Path]:
    return sorted(p for p in corpus.rglob("*.pdf") if p.is_file())


def resolve_stages(selected: Iterable[str]) -> list[str]:
    out: list[str] = []
    for name in selected:
        if name == "all":
            out.extend(STAGES)
        elif name in STAGES:
            out.append(name)
        else:
            raise SystemExit(f"unknown stage: {name}")
    return list(dict.fromkeys(out))


def run_one(
    *,
    binary: Path,
    pdf: Path,
    corpus: Path,
    stage: str,
    timeout_sec: float,
    mode: str,
    temp_root: Path,
    keep_outputs: bool,
) -> dict:
    rel = pdf.relative_to(corpus).as_posix()
    stem = hashlib.sha256((stage + "\0" + rel).encode("utf-8")).hexdigest()[:24]
    work = temp_root / stage / stem
    work.mkdir(parents=True, exist_ok=True)
    out = work / "out.dat"
    # Do not inject the public execution mode by default. Several legacy
    # subcommands also define a local `--mode` flag with command-specific
    # values, and the corpus validator must exercise the real default Standard
    # path without changing subcommand semantics. Research-mode corpus sweeps
    # should be added as explicit per-stage command templates once each
    # subcommand's accepted option shape is normalized.
    argv = [str(binary)]
    argv.extend(part.format(pdf=str(pdf), out=str(out)) for part in STAGES[stage])
    started = time.perf_counter()
    timed_out = False
    exit_code: int | None = None
    stderr = ""
    stdout_size = 0
    try:
        with tempfile.TemporaryFile() as stdout_file:
            proc = subprocess.run(
                argv,
                stdout=stdout_file,
                stderr=subprocess.PIPE,
                text=True,
                timeout=timeout_sec,
                check=False,
            )
            exit_code = proc.returncode
            stderr = proc.stderr or ""
            stdout_file.seek(0, os.SEEK_END)
            stdout_size = stdout_file.tell()
    except subprocess.TimeoutExpired as exc:
        timed_out = True
        exit_code = None
        stderr = (exc.stderr or "") if isinstance(exc.stderr, str) else ""
    duration_ms = (time.perf_counter() - started) * 1000.0
    out_size = out.stat().st_size if out.exists() else 0
    row = {
        "stage": stage,
        "pdf": rel,
        "pdf_bytes": pdf.stat().st_size,
        "pdf_sha256": sha256_file(pdf),
        "exit": exit_code,
        "timed_out": timed_out,
        "duration_ms": round(duration_ms, 3),
        "stdout_bytes": stdout_size,
        "output_bytes": out_size,
        "error_class": classify_error(exit_code, stderr, timed_out),
        "stderr_sha256": hashlib.sha256(stderr.encode("utf-8", "replace")).hexdigest()
        if stderr
        else None,
        "stderr_preview": stderr[:240].replace("\r", " ").replace("\n", " "),
    }
    if not keep_outputs:
        shutil.rmtree(work, ignore_errors=True)
    return row


def summarize(rows: list[dict], corpus_files: int, stages: list[str]) -> dict:
    by_stage = {}
    for stage in stages:
        sr = [r for r in rows if r["stage"] == stage]
        durations = sorted(r["duration_ms"] for r in sr)
        def pct(q: float) -> float | None:
            if not durations:
                return None
            idx = min(len(durations) - 1, int(round((len(durations) - 1) * q)))
            return round(durations[idx], 3)
        by_stage[stage] = {
            "files": len(sr),
            "successes": sum(1 for r in sr if r["exit"] == 0 and not r["timed_out"]),
            "failures": sum(1 for r in sr if r["exit"] != 0 or r["timed_out"]),
            "timeouts": sum(1 for r in sr if r["timed_out"]),
            "median_ms": pct(0.50),
            "p95_ms": pct(0.95),
            "p99_ms": pct(0.99),
            "error_classes": {},
        }
        for r in sr:
            cls = r["error_class"]
            by_stage[stage]["error_classes"][cls] = by_stage[stage]["error_classes"].get(cls, 0) + 1
    return {
        "schema_version": "wellfriend.all_feature_corpus.v1",
        "corpus_files": corpus_files,
        "stages": stages,
        "total_stage_runs": len(rows),
        "by_stage": by_stage,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", type=Path)
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--result-dir", type=Path, required=True)
    ap.add_argument("--jsonl", type=Path)
    ap.add_argument("--summary", type=Path)
    ap.add_argument("--mode", default="standard", choices=["standard", "research"])
    ap.add_argument("--stages", nargs="+", default=["all"], help="stage names or all")
    ap.add_argument("--max-files", type=int)
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--timeout-sec", type=float, default=180.0)
    ap.add_argument("--keep-outputs", action="store_true")
    args = ap.parse_args()

    corpus = args.corpus.resolve()
    binary = args.binary.resolve()
    result_dir = args.result_dir.resolve()
    result_dir.mkdir(parents=True, exist_ok=True)
    jsonl = args.jsonl or result_dir / "all-feature-corpus.jsonl"
    summary_path = args.summary or result_dir / "all-feature-corpus-summary.json"
    stages = resolve_stages(args.stages)
    pdfs = iter_pdfs(corpus)
    if args.max_files is not None:
        pdfs = pdfs[: args.max_files]
    temp_root = result_dir / "tmp-feature-outputs"
    temp_root.mkdir(parents=True, exist_ok=True)

    jobs = [
        {
            "binary": binary,
            "pdf": pdf,
            "corpus": corpus,
            "stage": stage,
            "timeout_sec": args.timeout_sec,
            "mode": args.mode,
            "temp_root": temp_root,
            "keep_outputs": args.keep_outputs,
        }
        for pdf in pdfs
        for stage in stages
    ]
    rows: list[dict] = []
    with jsonl.open("w", encoding="utf-8") as jf:
        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers)) as ex:
            futs = [ex.submit(run_one, **job) for job in jobs]
            for fut in concurrent.futures.as_completed(futs):
                row = fut.result()
                rows.append(row)
                jf.write(json.dumps(row, sort_keys=True) + "\n")
                jf.flush()
    summary = summarize(rows, len(pdfs), stages)
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if not args.keep_outputs:
        shutil.rmtree(temp_root, ignore_errors=True)
    return 0 if all(v["failures"] == 0 for v in summary["by_stage"].values()) else 2


if __name__ == "__main__":
    raise SystemExit(main())
