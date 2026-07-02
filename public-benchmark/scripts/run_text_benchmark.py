#!/usr/bin/env python3
"""Run Oxide vs public PDF text extraction competitors.

Methodology mirrors the published pdf_oxide benchmark shape:
single-threaded, no warm-up, one isolated subprocess per (tool, file), fixed
timeout, and crash/timeout/error recorded as data.
"""

from __future__ import annotations

import argparse
import ctypes
import difflib
import hashlib
import json
import math
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPO_ROOT / "public-benchmark" / "manifests" / "public_corpus_manifest.json"
DEFAULT_OUTPUT_DIR = REPO_ROOT / "public-benchmark" / "results" / "raw" / "latest"
DEFAULT_REPORT = REPO_ROOT / "docs" / "benchmark_public.md"
CAPABILITY_MATRIX = REPO_ROOT / "public-benchmark" / "capability_matrix.json"


PY_PYMUPDF = r"""
import sys
import fitz
doc = fitz.open(sys.argv[1])
try:
    text = "\n".join(page.get_text("text") or "" for page in doc)
finally:
    doc.close()
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write(text)
"""

PY_PYPDFIUM2 = r"""
import sys
import pypdfium2 as pdfium
pdf = pdfium.PdfDocument(sys.argv[1])
parts = []
for i in range(len(pdf)):
    page = pdf[i]
    textpage = page.get_textpage()
    parts.append(textpage.get_text_range() or "")
    textpage.close()
    page.close()
pdf.close()
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write("\n".join(parts))
"""

PY_PYMUPDF4LLM = r"""
import sys
import pymupdf4llm
try:
    text = pymupdf4llm.to_text(sys.argv[1], use_ocr=False)
except TypeError:
    text = pymupdf4llm.to_markdown(sys.argv[1])
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write(text or "")
"""

PY_PDFTEXT = r"""
import sys
path = sys.argv[1]
text = None
errors = []
try:
    from pdftext.extraction import plain_text_output
    value = plain_text_output(path)
    if isinstance(value, str):
        text = value
    elif isinstance(value, (list, tuple)):
        text = "\n".join(str(item) for item in value)
    elif isinstance(value, dict):
        text = "\n".join(str(v) for v in value.values())
except Exception as exc:
    errors.append(f"plain_text_output: {exc}")
if text is None:
    try:
        from pdftext.extraction import dictionary_output
        value = dictionary_output(path)
        pages = value if isinstance(value, list) else value.get("pages", [])
        chunks = []
        for page in pages:
            for block in page.get("blocks", []):
                if "text" in block:
                    chunks.append(str(block["text"]))
                for line in block.get("lines", []):
                    chunks.append(" ".join(str(span.get("text", "")) for span in line.get("spans", [])))
        text = "\n".join(chunks)
    except Exception as exc:
        errors.append(f"dictionary_output: {exc}")
if text is None:
    raise RuntimeError("; ".join(errors) or "no pdftext extractor worked")
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write(text)
"""

PY_PDFMINER = r"""
import sys
from pdfminer.high_level import extract_text
text = extract_text(sys.argv[1]) or ""
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write(text)
"""

PY_PDFPLUMBER = r"""
import sys
import pdfplumber
parts = []
with pdfplumber.open(sys.argv[1]) as pdf:
    for page in pdf.pages:
        parts.append(page.extract_text() or "")
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write("\n".join(parts))
"""

PY_MARKITDOWN = r"""
import sys
from markitdown import MarkItDown
result = MarkItDown(enable_plugins=False).convert(sys.argv[1])
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write(getattr(result, "text_content", "") or "")
"""

PY_PYPDF = r"""
import sys
from pypdf import PdfReader
reader = PdfReader(sys.argv[1])
if getattr(reader, "is_encrypted", False):
    try:
        reader.decrypt("")
    except Exception:
        pass
parts = []
for page in reader.pages:
    parts.append(page.extract_text() or "")
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write("\n".join(parts))
"""

PY_PDF_OXIDE = r"""
import sys
from pdf_oxide import PdfDocument
doc = PdfDocument(sys.argv[1])
parts = []
count_attr = getattr(doc, "page_count", None)
try:
    count = count_attr() if callable(count_attr) else int(count_attr)
except Exception:
    count = len(doc)
for i in range(count):
    if hasattr(doc, "extract_text"):
        parts.append(doc.extract_text(i) or "")
    else:
        page = doc[i]
        text_attr = getattr(page, "text", "")
        parts.append(text_attr() if callable(text_attr) else text_attr or "")
open(sys.argv[2], "w", encoding="utf-8", errors="replace").write("\n".join(parts))
"""


@dataclass
class CommandResult:
    ok: bool
    exit_code: int | None
    timed_out: bool
    memory_exceeded: bool
    duration_ms: int
    peak_memory_mb: float | None
    stdout: str
    stderr: str
    error: str | None = None

    def compact(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "exit_code": self.exit_code,
            "timed_out": self.timed_out,
            "memory_exceeded": self.memory_exceeded,
            "duration_ms": self.duration_ms,
            "peak_memory_mb": round(self.peak_memory_mb, 2) if self.peak_memory_mb is not None else None,
            "stdout": trim(self.stdout, 800),
            "stderr": trim(self.stderr, 1200),
            "error": self.error,
        }


@dataclass
class Tool:
    name: str
    kind: str
    import_name: str | None
    license: str
    command: Callable[[Path, Path, argparse.Namespace], list[str]]


def trim(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return value[:limit] + f"\n... truncated {len(value) - limit} chars ..."


def executable_name(name: str) -> str:
    if os.name == "nt" and not name.lower().endswith(".exe"):
        return name + ".exe"
    return name


def default_oxide_bin() -> Path:
    release = REPO_ROOT / "target" / "release" / executable_name("oxide")
    debug = REPO_ROOT / "target" / "debug" / executable_name("oxide")
    return release if release.exists() else debug


def python_cmd(code: str, pdf: Path, output: Path, _args: argparse.Namespace) -> list[str]:
    return [sys.executable, "-c", code, str(pdf), str(output)]


def oxide_cmd(pdf: Path, output: Path, args: argparse.Namespace) -> list[str]:
    return [str(Path(args.oxide_bin)), "extract-text", str(pdf), "--output", str(output)]


def optional_command_cmd(binary: str) -> Callable[[Path, Path, argparse.Namespace], list[str]]:
    def make(pdf: Path, output: Path, _args: argparse.Namespace) -> list[str]:
        return [binary, str(pdf), str(output)]

    return make


def tool_definitions() -> list[Tool]:
    return [
        Tool("oxide", "local", None, "MIT OR Apache-2.0", oxide_cmd),
        Tool("pdf_oxide", "python", "pdf_oxide", "MIT", lambda p, o, a: python_cmd(PY_PDF_OXIDE, p, o, a)),
        Tool("pymupdf", "python", "fitz", "AGPL-3.0/commercial", lambda p, o, a: python_cmd(PY_PYMUPDF, p, o, a)),
        Tool("pypdfium2", "python", "pypdfium2", "Apache-2.0/BSD-3-Clause", lambda p, o, a: python_cmd(PY_PYPDFIUM2, p, o, a)),
        Tool("pymupdf4llm", "python", "pymupdf4llm", "AGPL-3.0/commercial", lambda p, o, a: python_cmd(PY_PYMUPDF4LLM, p, o, a)),
        Tool("pdftext", "python", "pdftext", "Apache-2.0", lambda p, o, a: python_cmd(PY_PDFTEXT, p, o, a)),
        Tool("pdfminer.six", "python", "pdfminer", "MIT", lambda p, o, a: python_cmd(PY_PDFMINER, p, o, a)),
        Tool("pdfplumber", "python", "pdfplumber", "MIT", lambda p, o, a: python_cmd(PY_PDFPLUMBER, p, o, a)),
        Tool("markitdown", "python", "markitdown", "MIT", lambda p, o, a: python_cmd(PY_MARKITDOWN, p, o, a)),
        Tool("pypdf", "python", "pypdf", "BSD-3-Clause", lambda p, o, a: python_cmd(PY_PYPDF, p, o, a)),
        Tool("oxidize_pdf", "optional-rust", None, "unknown", optional_command_cmd("oxidize_pdf_text")),
        Tool("unpdf", "optional-rust", None, "unknown", optional_command_cmd("unpdf_text")),
        Tool("pdf_extract", "optional-rust", None, "unknown", optional_command_cmd("pdf_extract_text")),
        Tool("lopdf", "optional-rust", None, "MIT", optional_command_cmd("lopdf_text")),
    ]


def detect_tools(args: argparse.Namespace) -> tuple[list[Tool], dict[str, Any]]:
    availability: dict[str, Any] = {}
    available: list[Tool] = []
    for tool in tool_definitions():
        if tool.name == "oxide":
            ok = Path(args.oxide_bin).exists()
            availability[tool.name] = {"available": ok, "reason": None if ok else f"missing oxide binary: {args.oxide_bin}"}
        elif tool.kind == "python":
            proc = subprocess.run(
                [sys.executable, "-c", f"import {tool.import_name}; print('ok')"],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            ok = proc.returncode == 0
            availability[tool.name] = {"available": ok, "reason": None if ok else trim(proc.stderr or proc.stdout, 500)}
        else:
            binary = tool.command(Path("x.pdf"), Path("x.txt"), args)[0]
            found = shutil.which(binary) is not None
            availability[tool.name] = {
                "available": found,
                "reason": None if found else f"optional Rust text harness command not found: {binary}",
            }
            ok = found
        if ok:
            available.append(tool)
    return available, availability


def kill_process_tree(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], capture_output=True, text=True)
    else:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:
            proc.kill()


def process_rss_mb(pid: int) -> float | None:
    if os.name == "nt":
        return windows_process_rss_mb(pid)
    status = Path(f"/proc/{pid}/status")
    try:
        for line in status.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024.0
    except OSError:
        return None
    return None


def windows_process_rss_mb(pid: int) -> float | None:
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    PROCESS_VM_READ = 0x0010

    class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong),
            ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    try:
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, False, pid)
        if not handle:
            return None
        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(counters)
        ok = psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb)
        kernel32.CloseHandle(handle)
        if not ok:
            return None
        return counters.WorkingSetSize / (1024.0 * 1024.0)
    except Exception:
        return None


def benchmark_env() -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "RAYON_NUM_THREADS": "1",
            "OMP_NUM_THREADS": "1",
            "MKL_NUM_THREADS": "1",
            "OPENBLAS_NUM_THREADS": "1",
            "NUMEXPR_NUM_THREADS": "1",
            "TOKENIZERS_PARALLELISM": "false",
        }
    )
    return env


def run_monitored(cmd: list[str], *, timeout_sec: int, max_memory_mb: int | None, cwd: Path = REPO_ROOT) -> CommandResult:
    start = time.monotonic()
    peak: float | None = None
    timed_out = False
    memory_exceeded = False
    error: str | None = None
    creationflags = subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
    preexec_fn = None if os.name == "nt" else os.setsid
    try:
        proc = subprocess.Popen(
            cmd,
            cwd=str(cwd),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            env=benchmark_env(),
            creationflags=creationflags,
            preexec_fn=preexec_fn,
        )
    except FileNotFoundError as err:
        return CommandResult(False, None, False, False, 0, None, "", "", str(err))

    while proc.poll() is None:
        elapsed = time.monotonic() - start
        rss = process_rss_mb(proc.pid)
        if rss is not None:
            peak = rss if peak is None else max(peak, rss)
            if max_memory_mb is not None and rss > max_memory_mb:
                memory_exceeded = True
                error = f"memory cap exceeded: {rss:.1f} MB > {max_memory_mb} MB"
                kill_process_tree(proc)
                break
        if elapsed > timeout_sec:
            timed_out = True
            error = f"timeout after {timeout_sec}s"
            kill_process_tree(proc)
            break
        time.sleep(0.025)

    try:
        stdout, stderr = proc.communicate(timeout=2)
    except subprocess.TimeoutExpired:
        kill_process_tree(proc)
        stdout, stderr = proc.communicate()
    duration_ms = int(round((time.monotonic() - start) * 1000))
    ok = proc.returncode == 0 and not timed_out and not memory_exceeded
    return CommandResult(ok, proc.returncode, timed_out, memory_exceeded, duration_ms, peak, stdout, stderr, error)


def normalize_text(text: str) -> str:
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    text = re.sub(r"[ \t\f\v]+", " ", text)
    text = re.sub(r"\n{3,}", "\n\n", text)
    return text.strip()


def tokenize(text: str) -> list[str]:
    return re.findall(r"\w+|[^\w\s]", normalize_text(text).lower(), flags=re.UNICODE)


def token_dice(left: list[str], right: list[str]) -> float:
    if not left and not right:
        return 1.0
    if not left or not right:
        return 0.0
    lc = Counter(left)
    rc = Counter(right)
    overlap = sum(min(count, rc[token]) for token, count in lc.items())
    return (2.0 * overlap) / (len(left) + len(right))


def similarity(reference: str, candidate: str) -> dict[str, float]:
    ref_norm = normalize_text(reference)
    cand_norm = normalize_text(candidate)
    ref_tokens = tokenize(ref_norm)
    cand_tokens = tokenize(cand_norm)
    word_ratio = token_dice(ref_tokens, cand_tokens)
    if max(len(ref_norm), len(cand_norm)) > 25000:
        char_ratio = word_ratio
    else:
        char_ratio = difflib.SequenceMatcher(None, ref_norm, cand_norm).ratio()
    return {"word_ratio": round(word_ratio, 5), "char_ratio": round(char_ratio, 5)}


def load_manifest(path: Path) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    entries = []
    for raw in payload.get("entries", []):
        entry = dict(raw)
        pdf = Path(entry["path"])
        if not pdf.is_absolute():
            pdf = REPO_ROOT / pdf
        if not pdf.exists():
            continue
        entry["absolute_path"] = str(pdf)
        entries.append(entry)
    return payload, entries


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    pos = (len(ordered) - 1) * pct
    lo = math.floor(pos)
    hi = math.ceil(pos)
    if lo == hi:
        return ordered[lo]
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (pos - lo)


def summarize_records(records: list[dict[str, Any]], tool_names: list[str]) -> dict[str, Any]:
    nobody_passed = [rec["id"] for rec in records if not any(t.get("ok") for t in rec.get("tools", {}).values())]
    overall = {tool: summarize_tool([rec["tools"].get(tool) for rec in records]) for tool in tool_names}
    categories = sorted({tag for rec in records for tag in rec.get("tags", [rec.get("category", "unknown")])})
    per_category = {
        category: {
            tool: summarize_tool(
                [rec["tools"].get(tool) for rec in records if category in rec.get("tags", [rec.get("category", "unknown")])]
            )
            for tool in tool_names
        }
        for category in categories
    }
    return {"overall": overall, "per_category": per_category, "nobody_passed": nobody_passed}


def summarize_tool(items: list[dict[str, Any] | None]) -> dict[str, Any]:
    total = len(items)
    present = [item for item in items if item is not None]
    ok = [item for item in present if item.get("ok")]
    times = [item["duration_ms"] / 1000.0 for item in ok]
    mems = [item["peak_memory_mb"] for item in ok if item.get("peak_memory_mb") is not None]
    failures = Counter((item.get("failure_kind") or "error") for item in present if not item.get("ok"))
    return {
        "files": total,
        "attempted": len(present),
        "passed": len(ok),
        "pass_rate": round(100.0 * len(ok) / total, 3) if total else None,
        "mean_s": round(statistics.fmean(times), 6) if times else None,
        "p50_s": round(percentile(times, 0.50), 6) if times else None,
        "p95_s": round(percentile(times, 0.95), 6) if times else None,
        "p99_s": round(percentile(times, 0.99), 6) if times else None,
        "peak_memory_mb_mean": round(statistics.fmean(mems), 3) if mems else None,
        "peak_memory_mb_p95": round(percentile(mems, 0.95), 3) if mems else None,
        "failures": dict(failures),
    }


def failure_kind(result: CommandResult) -> str | None:
    if result.ok:
        return None
    if result.timed_out:
        return "timeout"
    if result.memory_exceeded:
        return "memory"
    if result.exit_code is None:
        return "launch"
    return "error"


def run_one_file(entry: dict[str, Any], tools: list[Tool], args: argparse.Namespace, sample: bool, work_root: Path) -> dict[str, Any]:
    pdf = Path(entry["absolute_path"])
    rec: dict[str, Any] = {
        "id": entry.get("id") or pdf.stem,
        "path": entry.get("path"),
        "sha256": entry.get("sha256"),
        "size_bytes": entry.get("size_bytes") or pdf.stat().st_size,
        "source": entry.get("source"),
        "category": entry.get("category"),
        "tags": entry.get("tags") or [entry.get("category") or "unknown"],
        "tools": {},
        "quality": {},
    }
    sample_outputs: dict[str, str] = {}
    file_work = work_root / sanitize(str(rec["id"]))
    file_work.mkdir(parents=True, exist_ok=True)
    for tool in tools:
        output = file_work / f"{tool.name}.txt"
        cmd = tool.command(pdf, output, args)
        result = run_monitored(cmd, timeout_sec=args.timeout, max_memory_mb=args.max_memory_mb)
        text_len = None
        text_sha = None
        if result.ok and output.exists():
            text = output.read_text(encoding="utf-8", errors="replace")
            text_norm = normalize_text(text)
            text_len = len(text_norm)
            text_sha = hashlib.sha256(text_norm.encode("utf-8", "replace")).hexdigest()
            if sample:
                sample_outputs[tool.name] = text_norm
        rec["tools"][tool.name] = {
            "ok": result.ok,
            "duration_ms": result.duration_ms,
            "peak_memory_mb": result.peak_memory_mb,
            "text_chars": text_len,
            "text_sha256": text_sha,
            "failure_kind": failure_kind(result),
            "command": result.compact() if not result.ok else None,
        }
        if not sample:
            try:
                output.unlink()
            except OSError:
                pass
    if sample and sample_outputs:
        reference = None
        for name in ["pymupdf", "pypdfium2", "pdf_oxide", "oxide"]:
            if sample_outputs.get(name):
                reference = name
                break
        if reference:
            rec["quality"]["reference_tool"] = reference
            rec["quality"]["sampled"] = True
            rec["quality"]["similarity"] = {
                name: similarity(sample_outputs[reference], text)
                for name, text in sample_outputs.items()
                if name != reference
            }
    return rec


def sanitize(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("._") or "file"


def aggregate_quality(records: list[dict[str, Any]]) -> dict[str, Any]:
    per_tool: dict[str, list[dict[str, float]]] = {}
    refs = Counter()
    for rec in records:
        quality = rec.get("quality", {})
        ref = quality.get("reference_tool")
        if ref:
            refs[ref] += 1
        for tool, scores in quality.get("similarity", {}).items():
            per_tool.setdefault(tool, []).append(scores)
    out = {
        "sampled_files": sum(refs.values()),
        "reference_tools": dict(refs),
        "tools": {},
    }
    for tool, rows in per_tool.items():
        out["tools"][tool] = {
            "mean_word_ratio": round(statistics.fmean(row["word_ratio"] for row in rows), 5),
            "mean_char_ratio": round(statistics.fmean(row["char_ratio"] for row in rows), 5),
            "files": len(rows),
        }
    return out


def make_worklist(summary: dict[str, Any], records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    overall = summary["overall"]
    oxide = overall.get("oxide", {})
    work: list[dict[str, Any]] = []
    faster = [
        (tool, stats)
        for tool, stats in overall.items()
        if tool != "oxide" and stats.get("mean_s") is not None and oxide.get("mean_s") is not None and stats["mean_s"] < oxide["mean_s"]
    ]
    if faster:
        work.append(
            {
                "rank": 1,
                "area": "performance",
                "finding": "Oxide mean extraction is slower than one or more leaders in this run.",
                "evidence": {tool: stats["mean_s"] for tool, stats in sorted(faster, key=lambda x: x[1]["mean_s"])[:5]},
            }
        )
    oxide_fail_others_pass = [
        rec["id"]
        for rec in records
        if not rec.get("tools", {}).get("oxide", {}).get("ok")
        and any(v.get("ok") for k, v in rec.get("tools", {}).items() if k != "oxide")
    ]
    if oxide_fail_others_pass:
        work.append(
            {
                "rank": len(work) + 1,
                "area": "fidelity",
                "finding": "Oxide failed files that another extractor passed.",
                "evidence_count": len(oxide_fail_others_pass),
                "examples": oxide_fail_others_pass[:20],
            }
        )
    quality = summary.get("quality", {}).get("tools", {})
    low_quality = {tool: row for tool, row in quality.items() if row.get("mean_word_ratio", 1.0) < 0.95}
    if low_quality:
        work.append(
            {
                "rank": len(work) + 1,
                "area": "text-quality",
                "finding": "Some tools diverged materially from the reference text sample; inspect Oxide if listed.",
                "evidence": low_quality,
            }
        )
    return work


def markdown_table(rows: list[list[str]]) -> list[str]:
    if not rows:
        return []
    widths = [max(len(str(row[i])) for row in rows) for i in range(len(rows[0]))]
    out = []
    for idx, row in enumerate(rows):
        out.append("| " + " | ".join(str(cell).ljust(widths[i]) for i, cell in enumerate(row)) + " |")
        if idx == 0:
            out.append("| " + " | ".join("-" * widths[i] for i in range(len(widths))) + " |")
    return out


def fmt(value: Any, suffix: str = "") -> str:
    if value is None:
        return "n/a"
    if isinstance(value, float):
        return f"{value:.3f}{suffix}"
    return f"{value}{suffix}"


def render_report(payload: dict[str, Any], report_path: Path) -> None:
    manifest = payload["manifest_summary"]
    summary = payload["summary"]
    capability = payload.get("capability_matrix") or {}
    lines = [
        "# Public PDF Text Extraction Benchmark",
        "",
        f"Generated: {payload['generated_at']}",
        f"Commit: `{payload['commit']}`",
        "",
        "## Scope And Method",
        "",
        f"- Corpus files in manifest: {manifest.get('entry_count')}",
        f"- Files benchmarked in this run: {payload['files_benchmarked']}",
        f"- Timeout per tool/file: {payload['timeout_s']}s",
        f"- Method: single-thread env, no warm-up, one isolated subprocess per tool/file.",
        f"- Pass definition: subprocess exits 0 before timeout/memory cap and writes text output.",
        f"- Nobody-passed files: {len(summary.get('nobody_passed', []))}",
        "",
        "Raw PDFs and per-file raw results are local-only and ignored by git. The manifest records source URLs and hashes for reproducibility.",
        "",
        "## Corpus Breakdown",
        "",
    ]
    rows = [["tag/category", "files"]]
    for key, value in sorted((manifest.get("category_breakdown") or {}).items()):
        rows.append([key, str(value)])
    lines.extend(markdown_table(rows))
    lines.extend(["", "## Tool Availability", ""])
    rows = [["tool", "available", "reason/license"]]
    for tool, info in payload["tool_availability"].items():
        rows.append([tool, "yes" if info.get("available") else "no", info.get("reason") or payload["tool_licenses"].get(tool, "")])
    lines.extend(markdown_table(rows))
    lines.extend(["", "## Overall Head-To-Head", ""])
    rows = [["tool", "pass %", "mean s", "p50 s", "p95 s", "p99 s", "mem p95 MB"]]
    for tool, stats in summary["overall"].items():
        rows.append(
            [
                tool,
                fmt(stats.get("pass_rate")),
                fmt(stats.get("mean_s")),
                fmt(stats.get("p50_s")),
                fmt(stats.get("p95_s")),
                fmt(stats.get("p99_s")),
                fmt(stats.get("peak_memory_mb_p95")),
            ]
        )
    lines.extend(markdown_table(rows))
    lines.extend(["", "## Per-Category Results", ""])
    for category, tool_stats in summary["per_category"].items():
        lines.extend([f"### {category}", ""])
        rows = [["tool", "files", "pass %", "mean s", "p95 s"]]
        for tool, stats in tool_stats.items():
            rows.append([tool, str(stats.get("files")), fmt(stats.get("pass_rate")), fmt(stats.get("mean_s")), fmt(stats.get("p95_s"))])
        lines.extend(markdown_table(rows))
        lines.append("")
    lines.extend(["## Text Quality Sample", ""])
    quality = summary.get("quality", {})
    lines.append(f"- Sampled files: {quality.get('sampled_files', 0)}")
    lines.append(f"- Reference tool selection: {quality.get('reference_tools', {})}")
    rows = [["tool", "files", "mean word ratio", "mean char ratio"]]
    for tool, stats in quality.get("tools", {}).items():
        rows.append([tool, str(stats.get("files")), fmt(stats.get("mean_word_ratio")), fmt(stats.get("mean_char_ratio"))])
    lines.extend(markdown_table(rows))
    lines.extend(["", "## Capability Matrix", ""])
    cap_tools = capability.get("tools", [])
    cap_rows = capability.get("capabilities", [])
    if cap_tools and cap_rows:
        rows = [["capability"] + cap_tools]
        for cap in cap_rows:
            rows.append([cap["name"]] + [cap.get("tools", {}).get(tool, "unknown") for tool in cap_tools])
        lines.extend(markdown_table(rows))
    else:
        lines.append("Capability matrix not loaded.")
    lines.extend(["", "## Feature Gaps Found", ""])
    for gap in capability.get("oxide_lacks", []):
        lines.append(f"- {gap}")
    lines.extend(["", "## Oxide Differentiators Found", ""])
    for win in capability.get("oxide_differentiators", []):
        lines.append(f"- {win}")
    lines.extend(["", "## Prioritized Work List", ""])
    for item in payload.get("worklist", []):
        lines.append(f"{item['rank']}. **{item['area']}**: {item['finding']} Evidence: `{json.dumps(item.get('evidence', item.get('examples', item.get('evidence_count'))), ensure_ascii=False)}`")
    if not payload.get("worklist"):
        lines.append("- No measured backlog item was produced by this run. Increase the corpus or install more competitors before treating that as a product claim.")
    lines.extend(
        [
            "",
            "## Provenance",
            "",
            f"- Python: `{sys.version.split()[0]}`",
            f"- Platform: `{sys.platform}`",
            f"- Oxide binary: `{payload['oxide_bin']}`",
            f"- Manifest: `{payload['manifest_path']}`",
            f"- Output JSON: `{payload['output_json']}`",
            "",
            "## Source Notes",
            "",
            "- pdf_oxide publishes a comparable 3,830-PDF benchmark using veraPDF, Mozilla pdf.js, and DARPA SafeDocs with single-thread, 60s timeout, no warm-up methodology.",
            "- The corpus script also uses arXiv for scale and diversity. arXiv paper license metadata varies by paper; PDFs remain local-only.",
        ]
    )
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def git_commit() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, text=True).strip()
    except Exception:
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST))
    parser.add_argument("--oxide-bin", default=str(default_oxide_bin()))
    parser.add_argument("--output-dir", default=str(DEFAULT_OUTPUT_DIR))
    parser.add_argument("--report", default=str(DEFAULT_REPORT))
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--max-memory-mb", type=int, default=2048)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--quality-sample", type=int, default=250)
    parser.add_argument("--category")
    parser.add_argument("--tool")
    args = parser.parse_args()

    manifest_path = Path(args.manifest)
    if not manifest_path.is_absolute():
        manifest_path = REPO_ROOT / manifest_path
    manifest, entries = load_manifest(manifest_path)
    if args.category:
        wanted = {item.strip() for item in args.category.split(",") if item.strip()}
        entries = [e for e in entries if wanted & set(e.get("tags", [e.get("category", "")]))]
    if args.limit is not None:
        entries = entries[: args.limit]
    if not entries:
        raise SystemExit(f"no benchmarkable PDFs found in {manifest_path}")

    output_dir = Path(args.output_dir)
    if not output_dir.is_absolute():
        output_dir = REPO_ROOT / output_dir
    output_dir.mkdir(parents=True, exist_ok=True)
    work_root = output_dir / "work"
    if work_root.exists():
        shutil.rmtree(work_root)
    work_root.mkdir(parents=True, exist_ok=True)

    tools, availability = detect_tools(args)
    if args.tool:
        selected = {item.strip() for item in args.tool.split(",") if item.strip()}
        tools = [tool for tool in tools if tool.name in selected]
    if not tools:
        raise SystemExit("no tools available")
    tool_names = [tool.name for tool in tools]
    tool_licenses = {tool.name: tool.license for tool in tool_definitions()}
    print("Available tools: " + ", ".join(tool_names), flush=True)

    records: list[dict[str, Any]] = []
    sample_ids = {entry.get("id") for entry in entries[: args.quality_sample]}
    for idx, entry in enumerate(entries, start=1):
        print(f"[{idx}/{len(entries)}] {entry.get('id') or Path(entry['path']).stem}", flush=True)
        records.append(run_one_file(entry, tools, args, entry.get("id") in sample_ids, work_root))

    summary = summarize_records(records, tool_names)
    summary["quality"] = aggregate_quality(records)
    capability = {}
    if CAPABILITY_MATRIX.exists():
        capability = json.loads(CAPABILITY_MATRIX.read_text(encoding="utf-8"))
    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "commit": git_commit(),
        "manifest_path": str(manifest_path.relative_to(REPO_ROOT) if manifest_path.is_relative_to(REPO_ROOT) else manifest_path),
        "oxide_bin": str(args.oxide_bin),
        "timeout_s": args.timeout,
        "max_memory_mb": args.max_memory_mb,
        "files_benchmarked": len(records),
        "manifest_summary": {
            "entry_count": manifest.get("entry_count", len(entries)),
            "category_breakdown": manifest.get("category_breakdown") or {},
            "complete": manifest.get("complete"),
            "target_count": manifest.get("target_count"),
        },
        "tool_availability": availability,
        "tool_licenses": tool_licenses,
        "summary": summary,
        "records": records,
        "capability_matrix": capability,
        "worklist": make_worklist(summary, records),
        "output_json": str((output_dir / "results.json").relative_to(REPO_ROOT) if (output_dir / "results.json").is_relative_to(REPO_ROOT) else output_dir / "results.json"),
    }
    (output_dir / "results.json").write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    render_report(payload, Path(args.report))
    print(f"Wrote {output_dir / 'results.json'}", flush=True)
    print(f"Wrote {args.report}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
