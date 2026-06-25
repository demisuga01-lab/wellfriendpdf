#!/usr/bin/env python3
"""Run a crash-safe wild-PDF robustness benchmark.

This harness reuses the subprocess monitor from
extraction-benchmark/scripts/competitive_benchmark.py: isolated child process,
timeout, RSS memory polling, process-tree kill, and pipe cleanup. Results are
checkpointed as one JSONL record per (tool, file).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import importlib.util
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


REPO = Path(__file__).resolve().parents[2]
COMPETITIVE = REPO / "extraction-benchmark" / "scripts" / "competitive_benchmark.py"
DEFAULT_MANIFEST = REPO / "robustness-benchmark" / "manifest.json"
DEFAULT_OUTPUT = REPO / "target" / "robustness-benchmark" / "latest"
DEFAULT_REPORT = REPO / "docs" / "robustness_benchmark.md"

spec = importlib.util.spec_from_file_location("competitive_benchmark", COMPETITIVE)
if spec is None or spec.loader is None:
    raise RuntimeError(f"cannot load {COMPETITIVE}")
competitive = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = competitive
spec.loader.exec_module(competitive)
monitored = competitive.monitored


PY_PYMUPDF_TEXT = r'''
import sys
try:
    import fitz
    parts=[]
    doc=fitz.open(sys.argv[1])
    try:
        for p in doc:
            parts.append(p.get_text("text") or "")
    finally:
        doc.close()
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PYPDFIUM2_TEXT = r'''
import sys
try:
    import pypdfium2 as pdfium
    parts=[]
    pdf=pdfium.PdfDocument(sys.argv[1])
    try:
        for i in range(len(pdf)):
            page=pdf[i]
            tp=page.get_textpage()
            try:
                parts.append(tp.get_text_range() or "")
            finally:
                tp.close(); page.close()
    finally:
        pdf.close()
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PDF_OXIDE_TEXT = r'''
import sys
try:
    from pdf_oxide import PdfDocument
    doc=PdfDocument(sys.argv[1])
    parts=[]
    try:
        pc=getattr(doc,"page_count",None)
        n=pc() if callable(pc) else int(pc)
    except Exception:
        n=len(doc)
    for i in range(n):
        if hasattr(doc,"extract_text"):
            parts.append(doc.extract_text(i) or "")
        else:
            p=doc[i]
            t=getattr(p,"text","")
            parts.append(t() if callable(t) else t or "")
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PDFMINER_TEXT = r'''
import sys
try:
    from pdfminer.high_level import extract_text
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(extract_text(sys.argv[1]) or "")
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PDFPLUMBER_TEXT = r'''
import sys
try:
    import pdfplumber
    parts=[]
    with pdfplumber.open(sys.argv[1]) as pdf:
        for p in pdf.pages:
            parts.append(p.extract_text() or "")
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PYPDF_TEXT = r'''
import sys
try:
    from pypdf import PdfReader
    r=PdfReader(sys.argv[1])
    if getattr(r,"is_encrypted",False):
        try:
            r.decrypt("")
        except Exception:
            pass
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join((p.extract_text() or "") for p in r.pages))
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PDFTEXT_TEXT = r'''
import sys
try:
    from pdftext.extraction import plain_text_output
    v=plain_text_output(sys.argv[1])
    if isinstance(v,str):
        text=v
    elif isinstance(v,(list,tuple)):
        text="\n".join(str(x) for x in v)
    elif isinstance(v,dict):
        text="\n".join(str(x) for x in v.values())
    else:
        text=""
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(text or "")
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_MARKITDOWN_TEXT = r'''
import sys
try:
    from markitdown import MarkItDown
    r=MarkItDown(enable_plugins=False).convert(sys.argv[1])
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(getattr(r,"text_content","") or "")
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''

PY_PYMUPDF4LLM_TEXT = r'''
import sys
try:
    import pymupdf4llm
    try:
        text=pymupdf4llm.to_text(sys.argv[1], use_ocr=False)
    except Exception:
        text=pymupdf4llm.to_markdown(sys.argv[1])
    open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(text or "")
except Exception as e:
    sys.stderr.write("CLEAN_ERROR: "+repr(e))
    sys.exit(2)
'''


@dataclass(frozen=True)
class Tool:
    name: str
    kind: str
    import_name: str | None
    dist: str | None
    command: Callable[[Path, Path, argparse.Namespace], list[str]]
    license: str
    heavy: bool = False


def exe(name: str) -> str:
    return name + ".exe" if os.name == "nt" and not name.endswith(".exe") else name


def default_oxide() -> Path:
    rel = REPO / "target" / "release" / exe("oxide")
    dbg = REPO / "target" / "debug" / exe("oxide")
    return rel if rel.exists() else dbg


def pycmd(code: str, pdf: Path, out: Path, _args: argparse.Namespace) -> list[str]:
    return [sys.executable, "-c", code, str(pdf), str(out)]


def oxide_text(pdf: Path, out: Path, args: argparse.Namespace) -> list[str]:
    return [str(Path(args.oxide_bin)), "extract-text", str(pdf), "--output", str(out)]


def poppler_text(pdf: Path, out: Path, _args: argparse.Namespace) -> list[str]:
    return ["pdftotext", "-layout", str(pdf), str(out)]


def docling_text(pdf: Path, out: Path, _args: argparse.Namespace) -> list[str]:
    code = (
        "import sys\n"
        "try:\n"
        " from docling.document_converter import DocumentConverter\n"
        " r=DocumentConverter().convert(sys.argv[1]); d=r.document\n"
        " text=d.export_to_markdown() if hasattr(d,'export_to_markdown') else str(d)\n"
        " open(sys.argv[2],'w',encoding='utf-8',errors='replace').write(text or '')\n"
        "except Exception as e:\n"
        " sys.stderr.write('CLEAN_ERROR: '+repr(e)); sys.exit(2)\n"
    )
    return [sys.executable, "-c", code, str(pdf), str(out)]


def all_tools() -> list[Tool]:
    return [
        Tool("oxide", "local", None, None, oxide_text, "MIT OR Apache-2.0"),
        Tool("pdf_oxide", "python", "pdf_oxide", "pdf_oxide", lambda p, o, a: pycmd(PY_PDF_OXIDE_TEXT, p, o, a), "MIT"),
        Tool("pymupdf", "python", "fitz", "PyMuPDF", lambda p, o, a: pycmd(PY_PYMUPDF_TEXT, p, o, a), "AGPL-3.0/commercial"),
        Tool("pypdfium2", "python", "pypdfium2", "pypdfium2", lambda p, o, a: pycmd(PY_PYPDFIUM2_TEXT, p, o, a), "Apache-2.0/BSD-3-Clause"),
        Tool("poppler", "cli", None, None, poppler_text, "GPL-2.0-or-later"),
        Tool("pdfminer.six", "python", "pdfminer", "pdfminer.six", lambda p, o, a: pycmd(PY_PDFMINER_TEXT, p, o, a), "MIT"),
        Tool("pdfplumber", "python", "pdfplumber", "pdfplumber", lambda p, o, a: pycmd(PY_PDFPLUMBER_TEXT, p, o, a), "MIT"),
        Tool("pypdf", "python", "pypdf", "pypdf", lambda p, o, a: pycmd(PY_PYPDF_TEXT, p, o, a), "BSD-3-Clause"),
        Tool("pdftext", "python", "pdftext", "pdftext", lambda p, o, a: pycmd(PY_PDFTEXT_TEXT, p, o, a), "Apache-2.0"),
        Tool("markitdown", "python", "markitdown", "markitdown", lambda p, o, a: pycmd(PY_MARKITDOWN_TEXT, p, o, a), "MIT"),
        Tool("pymupdf4llm", "python", "pymupdf4llm", "pymupdf4llm", lambda p, o, a: pycmd(PY_PYMUPDF4LLM_TEXT, p, o, a), "AGPL-3.0/commercial"),
        Tool("docling", "python", "docling", "docling", docling_text, "MIT", heavy=True),
    ]


def run_cap(cmd: list[str], timeout: int = 20) -> tuple[bool, str]:
    try:
        p = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        return p.returncode == 0, (p.stdout or p.stderr or "").strip()
    except Exception as err:
        return False, str(err)


def detect_tool(tool: Tool, oxide_bin: Path) -> dict[str, Any]:
    if tool.name == "oxide":
        ok = oxide_bin.exists()
        version = None
        if ok:
            _ok, out = run_cap([str(oxide_bin), "--version"])
            version = out.splitlines()[0] if out else None
        return {"available": ok, "version": version, "reason": None if ok else f"missing {oxide_bin}", "license": tool.license}
    if tool.name == "poppler":
        ok = shutil.which("pdftotext") is not None
        version = None
        reason = None
        if ok:
            _ok, out = run_cap(["pdftotext", "-v"])
            version = out.splitlines()[0] if out else None
        else:
            reason = "pdftotext missing"
        return {"available": bool(ok), "version": version, "reason": reason, "license": tool.license}
    if tool.kind == "python":
        code = (
            "import importlib, importlib.metadata as m\n"
            f"importlib.import_module({tool.import_name!r})\n"
            "try:\n"
            f" print(m.version({(tool.dist or tool.import_name)!r}))\n"
            "except Exception:\n"
            " print('import-ok/version-unknown')\n"
        )
        p = subprocess.run([sys.executable, "-c", code], cwd=REPO, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30)
        return {
            "available": p.returncode == 0,
            "version": p.stdout.strip() if p.returncode == 0 else None,
            "reason": None if p.returncode == 0 else trim(p.stderr or p.stdout, 500),
            "license": tool.license,
        }
    return {"available": False, "version": None, "reason": "unknown detector", "license": tool.license}


def trim(text: str | None, limit: int = 800) -> str:
    if not text:
        return ""
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return text if len(text) <= limit else text[:limit] + " ..."


PANIC_MARKERS = (
    "panicked at",
    "thread 'main' panicked",
    "stack backtrace",
    "segmentation fault",
    "access violation",
    "0xc0000005",
    "fatal runtime error",
    "memory allocation of",
)


def classify_result(result: Any, out_path: Path) -> tuple[str, bool, int, str]:
    stdout = result.out or ""
    stderr = result.err or ""
    combined = f"{stdout}\n{stderr}\n{result.error or ''}".lower()
    output_bytes = out_path.stat().st_size if out_path.exists() else 0
    if result.timeout:
        return "timeout", False, output_bytes, "TIMEOUT/HANG: exceeded per-file timeout"
    if result.mem_exceeded:
        return "oom", False, output_bytes, "OOM: exceeded memory cap"
    if result.ok:
        if out_path.exists():
            return "pass", True, output_bytes, "PASS: exited 0 and produced a text artifact"
        return "error", False, output_bytes, "ERROR: exited 0 but did not produce output"
    if any(marker in combined for marker in PANIC_MARKERS):
        return "crash_panic", False, output_bytes, "CRASH/PANIC: abnormal abort or panic marker"
    if result.code in (134, 139, 3221225477, -6, -11):
        return "crash_panic", False, output_bytes, f"CRASH/PANIC: abnormal exit code {result.code}"
    return "clean_error", True, output_bytes, "ERROR: clean handled error return"


def load_manifest(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    entries = []
    for raw in data.get("entries", []):
        entry = dict(raw)
        pdf = REPO / entry["path"]
        entry["absolute_path"] = str(pdf)
        if pdf.exists():
            entries.append(entry)
    data["entries"] = entries
    return data


def wanted_tools(args: argparse.Namespace, availability: dict[str, dict[str, Any]]) -> list[Tool]:
    tools = all_tools()
    if args.tools:
        names = {x.strip() for x in args.tools.split(",") if x.strip()}
        tools = [t for t in tools if t.name in names]
    elif not args.include_heavy:
        tools = [t for t in tools if not t.heavy]
    return [t for t in tools if availability.get(t.name, {}).get("available")]


def done_keys(records_path: Path) -> set[tuple[str, str]]:
    done: set[tuple[str, str]] = set()
    if not records_path.exists():
        return done
    for line in records_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip():
            continue
        try:
            rec = json.loads(line)
            done.add((rec["file_id"], rec["tool"]))
        except Exception:
            continue
    return done


def task_id(file_id: str, tool: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", f"{file_id}_{tool}")[:180]


def run_one(entry: dict[str, Any], tool: Tool, args: argparse.Namespace, work_root: Path) -> dict[str, Any]:
    pdf = Path(entry["absolute_path"])
    work = work_root / task_id(entry["id"], tool.name)
    work.mkdir(parents=True, exist_ok=True)
    out_path = work / f"{tool.name}.txt"
    start = time.monotonic()
    result = monitored(tool.command(pdf, out_path, args), args)
    outcome, survived, output_bytes, definition = classify_result(result, out_path)
    text_hash = None
    if out_path.exists():
        try:
            text_hash = __import__("hashlib").sha256(out_path.read_bytes()).hexdigest()
        except OSError:
            text_hash = None
    try:
        if out_path.exists():
            out_path.unlink()
        work.rmdir()
    except OSError:
        pass
    return {
        "record_type": "tool_file_result",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "file_id": entry["id"],
        "path": entry["path"],
        "selection_index": entry.get("selection_index"),
        "source_tier": entry.get("source_tier"),
        "origin": entry.get("origin"),
        "stress_tag": entry.get("stress_tag"),
        "tags": entry.get("tags") or [],
        "size_bytes": entry.get("size_bytes"),
        "tool": tool.name,
        "outcome": outcome,
        "survived": survived,
        "pass_definition": definition,
        "output_bytes": output_bytes,
        "text_sha256": text_hash,
        "duration_ms": result.ms,
        "wall_ms": int((time.monotonic() - start) * 1000),
        "peak_memory_mb": result.peak_mb,
        "exit_code": result.code,
        "timeout": result.timeout,
        "memory_exceeded": result.mem_exceeded,
        "stdout": trim(result.out),
        "stderr": trim(result.err),
        "error": trim(result.error),
    }


def load_records(path: Path) -> list[dict[str, Any]]:
    out = []
    if not path.exists():
        return out
    seen: dict[tuple[str, str], dict[str, Any]] = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip():
            continue
        try:
            rec = json.loads(line)
            seen[(rec["file_id"], rec["tool"])] = rec
        except Exception:
            continue
    return [seen[k] for k in sorted(seen)]


def pct(num: int, den: int) -> float:
    return round(100.0 * num / den, 3) if den else 0.0


def classify_root_cause(rec: dict[str, Any]) -> str:
    tag = str(rec.get("stress_tag") or "").lower()
    text = " ".join(str(rec.get(k) or "") for k in ("stderr", "stdout", "error")).lower()
    outcome = rec.get("outcome")
    if outcome == "timeout":
        return "timeout/hang"
    if outcome == "oom":
        return "allocation/resource bound"
    if outcome == "crash_panic":
        if "memory allocation" in text:
            return "allocation/resource bound"
        if "panicked at" in text:
            return "panic on malformed input"
        return "process crash"
    if "xref" in tag or "startxref" in tag or "xref" in text or "trailer" in text:
        return "corrupt xref/trailer recovery"
    if "truncated" in tag or "eof" in text or "unexpected end" in text:
        return "truncated file handling"
    if "huge" in tag or "length" in text:
        return "allocation from untrusted size"
    if "filter" in tag or "jbig2" in text or "jpx" in text or "ccitt" in text:
        return "unsupported filter/image decode"
    if "encrypt" in tag or "password" in text or "security handler" in text:
        return "encryption edge"
    if "deep" in tag or "recursion" in text or "nested" in text:
        return "recursion/nesting bound"
    if outcome == "clean_error":
        return "clean parser rejection"
    return "other"


def aggregate(records: list[dict[str, Any]], manifest: dict[str, Any], availability: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    by_tool: dict[str, list[dict[str, Any]]] = defaultdict(list)
    by_file: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for rec in records:
        by_tool[rec["tool"]].append(rec)
        by_file[rec["file_id"]].append(rec)

    tool_rows = []
    for tool, items in sorted(by_tool.items()):
        outcomes = Counter(r["outcome"] for r in items)
        survived = sum(1 for r in items if r.get("survived"))
        parsed = outcomes.get("pass", 0)
        hard_fail = sum(outcomes.get(k, 0) for k in ("crash_panic", "timeout", "oom"))
        durations = [r["duration_ms"] / 1000 for r in items if isinstance(r.get("duration_ms"), int) and r["duration_ms"] >= 0]
        tool_rows.append(
            {
                "tool": tool,
                "attempted": len(items),
                "parsed_pass": parsed,
                "clean_errors": outcomes.get("clean_error", 0),
                "survival_rate": pct(survived, len(items)),
                "parsed_pass_rate": pct(parsed, len(items)),
                "hard_failures": hard_fail,
                "crash_panic": outcomes.get("crash_panic", 0),
                "timeout": outcomes.get("timeout", 0),
                "oom": outcomes.get("oom", 0),
                "other_errors": outcomes.get("error", 0),
                "mean_s": round(statistics.fmean(durations), 5) if durations else None,
            }
        )
    tool_rows.sort(key=lambda r: (-r["survival_rate"], -r["parsed_pass_rate"], r["hard_failures"], r["tool"]))

    oxide_fail_comp_survives = []
    oxide_clean_error_comp_pass = []
    no_tool_parsed = []
    for file_id, items in sorted(by_file.items()):
        ox = next((r for r in items if r["tool"] == "oxide"), None)
        if not ox:
            continue
        competitor_pass = sorted(r["tool"] for r in items if r["tool"] != "oxide" and r["outcome"] == "pass")
        competitor_survive = sorted(r["tool"] for r in items if r["tool"] != "oxide" and r.get("survived"))
        if ox["outcome"] in ("crash_panic", "timeout", "oom", "error") and competitor_survive:
            oxide_fail_comp_survives.append(
                {
                    "file_id": file_id,
                    "path": ox["path"],
                    "stress_tag": ox.get("stress_tag"),
                    "oxide_outcome": ox["outcome"],
                    "oxide_error": trim(ox.get("stderr") or ox.get("error") or ox.get("stdout"), 300),
                    "competitors_survived": competitor_survive,
                    "root_cause": classify_root_cause(ox),
                }
            )
        if ox["outcome"] == "clean_error" and competitor_pass:
            oxide_clean_error_comp_pass.append(
                {
                    "file_id": file_id,
                    "path": ox["path"],
                    "stress_tag": ox.get("stress_tag"),
                    "oxide_error": trim(ox.get("stderr") or ox.get("error") or ox.get("stdout"), 300),
                    "competitors_parsed": competitor_pass,
                    "root_cause": classify_root_cause(ox),
                }
            )
        if not any(r["outcome"] == "pass" for r in items):
            no_tool_parsed.append({"file_id": file_id, "path": ox["path"], "stress_tag": ox.get("stress_tag")})

    oxide_bad = [
        r
        for r in records
        if r["tool"] == "oxide" and r["outcome"] in ("crash_panic", "timeout", "oom", "error", "clean_error")
    ]
    root_counts = Counter(classify_root_cause(r) for r in oxide_bad)
    hard_root_counts = Counter(
        classify_root_cause(r)
        for r in records
        if r["tool"] == "oxide" and r["outcome"] in ("crash_panic", "timeout", "oom", "error")
    )
    fix_counts = hard_root_counts or root_counts
    prioritized = [
        {"rank": i + 1, "category": cat, "files": count, "reason": fix_reason(cat)}
        for i, (cat, count) in enumerate(fix_counts.most_common())
    ]

    source_breakdown = Counter(e.get("source_tier") for e in manifest.get("entries", []))
    stress_breakdown = Counter(e.get("stress_tag") for e in manifest.get("entries", []))
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "repo_commit": git_commit(),
        "python": sys.version.split()[0],
        "platform": sys.platform,
        "hardware": hardware_summary(),
        "label": "indicative (approx 200-file subset)",
        "args": vars(args),
        "corpus": {
            "files": len(manifest.get("entries", [])),
            "source_breakdown": dict(sorted(source_breakdown.items())),
            "stress_breakdown": dict(sorted(stress_breakdown.items())),
            "network_status": manifest.get("network_status"),
            "selection": manifest.get("target", {}).get("selection"),
        },
        "availability": availability,
        "tool_rows": tool_rows,
        "oxide_fails_competitor_survives": oxide_fail_comp_survives[:100],
        "oxide_clean_error_competitor_parses": oxide_clean_error_comp_pass[:100],
        "no_tool_parsed": no_tool_parsed[:100],
        "root_cause_counts": dict(root_counts.most_common()),
        "hard_root_cause_counts": dict(hard_root_counts.most_common()),
        "prioritized_fix_list": prioritized[:20],
    }


def fix_reason(category: str) -> str:
    reasons = {
        "corrupt xref/trailer recovery": "Improve best-effort xref/trailer recovery and object scanning.",
        "truncated file handling": "Return clean typed errors for truncated streams/objects; parse recoverable prefix when safe.",
        "allocation/resource bound": "Cap allocations and decode output sizes derived from file-controlled values.",
        "allocation from untrusted size": "Validate declared stream lengths against actual remaining file bytes before allocating.",
        "recursion/nesting bound": "Keep recursion limits explicit and convert over-depth input into clean errors.",
        "unsupported filter/image decode": "Handle rare filters with clean errors or add bounded decoders.",
        "encryption edge": "Harden encrypted-file detection and unsupported security-handler errors.",
        "panic on malformed input": "Replace panicking untrusted-input paths with typed errors.",
        "clean parser rejection": "Not a crash, but a competitor-parses gap; consider best-effort recovery if common.",
    }
    return reasons.get(category, "Inspect representative files and convert hard failures into clean errors or bounded recovery.")


def git_commit() -> str | None:
    ok, out = run_cap(["git", "rev-parse", "HEAD"])
    return out.strip() if ok else None


def hardware_summary() -> str:
    try:
        import platform

        return f"{platform.processor() or platform.machine()} / {os.cpu_count()} logical CPUs"
    except Exception:
        return f"{os.cpu_count()} logical CPUs"


def md(headers: list[str], rows: list[list[Any]]) -> str:
    out = ["| " + " | ".join(headers) + " |", "| " + " | ".join(["---"] * len(headers)) + " |"]
    for row in rows:
        out.append("| " + " | ".join(str(x).replace("\n", " ") for x in row) + " |")
    return "\n".join(out)


def write_report(summary: dict[str, Any], path: Path) -> None:
    lines: list[str] = []
    w = lines.append
    tool_rows = summary["tool_rows"]
    oxide = next((r for r in tool_rows if r["tool"] == "oxide"), None) or {}
    leaders = [r for r in tool_rows if r["tool"] in {"pdf_oxide", "pymupdf", "pypdfium2", "poppler"}]
    top3 = summary["prioritized_fix_list"][:3]
    top_text = ", ".join(f"{x['category']} ({x['files']})" for x in top3) if top3 else "no hard failure category found"

    w("# Robustness Benchmark: Wild-PDF Survival\n")
    w(
        "**Plain-language summary.** On this indicative (approx 200-file subset) robustness run, "
        f"Oxide survived {oxide.get('survival_rate', '-')}% of attempted files and produced parsed text artifacts for "
        f"{oxide.get('parsed_pass_rate', '-')}%. The main Prompt 2 targets are: {top_text}. "
        "Clean handled errors are separated from crashes/timeouts/OOMs because a clean rejection is acceptable for malformed input, "
        "while a hard failure is not.\n"
    )
    w("## Scope And Corpus\n")
    w("This is a SMALL indicative robustness corpus, not a final robustness claim. It has no ground-truth text labels, so it measures survival only.\n")
    c = summary["corpus"]
    w(md([ "metric", "value" ], [["label", summary["label"]], ["files", c["files"]], ["selection", c.get("selection")]]))
    w("")
    w("### Source Breakdown\n")
    w(md(["source tier", "files"], [[k, v] for k, v in c["source_breakdown"].items()]))
    w("")
    w("### Stress Tags\n")
    w(md(["stress tag", "files"], [[k, v] for k, v in c["stress_breakdown"].items()]))
    w("")
    w("### Public Source Reachability\n")
    w(md(["source", "status"], [[k, v] for k, v in (c.get("network_status") or {}).items()]))
    w("")
    w("## Provenance\n")
    args = summary["args"]
    w(md(["item", "value"], [
        ["generated", summary["generated_at"]],
        ["commit", summary.get("repo_commit")],
        ["python", summary.get("python")],
        ["platform", summary.get("platform")],
        ["hardware", summary.get("hardware")],
        ["timeout", f"{args.get('timeout')}s"],
        ["memory cap", f"{args.get('max_memory_mb')} MB"],
        ["max workers", args.get("max_workers")],
        ["pass definition", "PASS exits 0 and writes an output artifact; CLEAN_ERROR is a handled non-zero error and counts as survival, not parsed output"],
    ]))
    w("")
    w("## Tools Run Vs Skipped\n")
    w(md(["tool", "run", "version", "reason/license"], [
        [tool, "yes" if info.get("run") else ("available, not run" if info.get("available") else "no"), info.get("version") or "-", info.get("reason") or info.get("license") or "-"]
        for tool, info in sorted(summary["availability"].items())
    ]))
    w("")
    w("## Ranked Robustness Table\n")
    w("Rates below are indicative (approx 200-file subset). Survival = PASS + CLEAN_ERROR. Parsed pass = PASS only.\n")
    w(md(
        ["rank", "tool", "survival %", "parsed pass %", "parsed", "clean errors", "hard failures", "crash", "timeout", "OOM", "mean s"],
        [
            [i + 1, r["tool"], r["survival_rate"], r["parsed_pass_rate"], r["parsed_pass"], r["clean_errors"], r["hard_failures"], r["crash_panic"], r["timeout"], r["oom"], r["mean_s"]]
            for i, r in enumerate(tool_rows)
        ],
    ))
    w("")
    if leaders:
        w("Leader comparison set: " + ", ".join(f"{r['tool']} {r['survival_rate']}%" for r in leaders) + ".")
        w("")
    w("## Oxide Hard-Fails But A Competitor Survives\n")
    hard = summary["oxide_fails_competitor_survives"]
    if hard:
        w(md(["file", "tag", "Oxide outcome", "root cause", "competitors survived", "Oxide error"], [
            [x["path"], x["stress_tag"], x["oxide_outcome"], x["root_cause"], ", ".join(x["competitors_survived"]), x["oxide_error"]]
            for x in hard[:50]
        ]))
    else:
        w("No Oxide crash/timeout/OOM/missing-output hard failures had a competitor survival on this run.")
    w("")
    w("## Oxide Clean-Errors But A Competitor Parses\n")
    clean = summary["oxide_clean_error_competitor_parses"]
    if clean:
        w("These are not crash bugs, but they are best-effort recovery gaps for Prompt 2 if the category is common.\n")
        w(md(["file", "tag", "root cause", "competitors parsed", "Oxide error"], [
            [x["path"], x["stress_tag"], x["root_cause"], ", ".join(x["competitors_parsed"]), x["oxide_error"]]
            for x in clean[:50]
        ]))
    else:
        w("No Oxide clean-error/competitor-parse gaps were found.")
    w("")
    w("## Oxide Root-Cause Grouping\n")
    w("Clean errors are included here but are distinguished from hard failures.")
    w(md(["category", "all Oxide non-pass files", "hard-failure subset"], [
        [cat, count, summary["hard_root_cause_counts"].get(cat, 0)]
        for cat, count in summary["root_cause_counts"].items()
    ]))
    w("")
    w("## Files No Tool Parsed\n")
    none = summary["no_tool_parsed"]
    if none:
        w(md(["file", "tag"], [[x["path"], x["stress_tag"]] for x in none[:50]]))
    else:
        w("Every file had at least one tool produce a parsed text artifact.")
    w("")
    w("## Prioritized Fix List For Prompt 2\n")
    if summary["prioritized_fix_list"]:
        for item in summary["prioritized_fix_list"]:
            w(f"{item['rank']}. **{item['category']}** ({item['files']} files): {item['reason']}")
    else:
        w("No hard-failure fix category was found; Prompt 2 should focus on the largest clean-error competitor-parse recovery gap.")
    w("")
    w("## Still Unmeasured\n")
    w("This run is small, text-extraction-only, and indicative. It does not prove final real-world robustness, does not score text correctness, and does not include a separate image/rendering robustness pass. The larger wild run belongs in Prompt 10.")
    w("")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest", default=str(DEFAULT_MANIFEST))
    p.add_argument("--oxide-bin", default=str(default_oxide()))
    p.add_argument("--output-dir", default=str(DEFAULT_OUTPUT))
    p.add_argument("--report", default=str(DEFAULT_REPORT))
    p.add_argument("--tools", help="comma-separated tool names; default runs all non-heavy installed tools")
    p.add_argument("--include-heavy", action="store_true", help="include heavy tools such as docling")
    p.add_argument("--timeout", type=int, default=60)
    p.add_argument("--max-memory-mb", type=int, default=2048)
    p.add_argument("--poll-interval-ms", type=int, default=100)
    p.add_argument("--max-workers", type=int, default=4)
    p.add_argument("--limit", type=int)
    p.add_argument("--resume", action="store_true")
    p.add_argument("--aggregate-only", action="store_true")
    return p.parse_args()


def main() -> int:
    args = parse_args()
    if args.max_workers > 4:
        raise SystemExit("max-workers must be <= 4 for text robustness runs")
    output = Path(args.output_dir)
    output.mkdir(parents=True, exist_ok=True)
    work = output / "work"
    work.mkdir(exist_ok=True)
    records_path = output / "records.jsonl"

    manifest = load_manifest(Path(args.manifest))
    if args.limit:
        manifest["entries"] = manifest["entries"][: args.limit]

    oxide_bin = Path(args.oxide_bin)
    availability = {tool.name: detect_tool(tool, oxide_bin) for tool in all_tools()}
    tools = wanted_tools(args, availability)
    active = {t.name for t in tools}
    for name, info in availability.items():
        info["run"] = name in active
        if info.get("available") and not info["run"] and name == "docling" and not args.include_heavy:
            info["reason"] = "installed but skipped in default run because it is a heavyweight ML converter; pass --include-heavy to run it"

    metadata = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "args": vars(args),
        "availability": availability,
        "active_tools": sorted(active),
        "files": len(manifest["entries"]),
    }
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    done = done_keys(records_path) if (args.resume or args.aggregate_only) else set()
    tasks = [(entry, tool) for entry in manifest["entries"] for tool in tools if (entry["id"], tool.name) not in done]
    if not args.aggregate_only:
        print(f"files={len(manifest['entries'])} tools={len(tools)} pending_tool_file_records={len(tasks)} max_workers={args.max_workers}", flush=True)
        mode = "a" if args.resume else "w"
        with records_path.open(mode, encoding="utf-8") as fh:
            completed = 0

            def write_record(rec: dict[str, Any]) -> None:
                nonlocal completed
                fh.write(json.dumps(rec, ensure_ascii=False) + "\n")
                fh.flush()
                completed += 1
                if completed == 1 or completed % 50 == 0 or completed == len(tasks):
                    print(f"[{completed}/{len(tasks)}] {rec['file_id']} {rec['tool']} {rec['outcome']}", flush=True)

            if args.max_workers <= 1:
                for entry, tool in tasks:
                    write_record(run_one(entry, tool, args, work))
            else:
                with concurrent.futures.ThreadPoolExecutor(max_workers=args.max_workers) as ex:
                    futs = {ex.submit(run_one, entry, tool, args, work): (entry, tool) for entry, tool in tasks}
                    for fut in concurrent.futures.as_completed(futs):
                        write_record(fut.result())

    records = load_records(records_path)
    summary = aggregate(records, manifest, availability, args)
    (output / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    write_report(summary, Path(args.report))
    print(f"wrote {output / 'summary.json'}")
    print(f"wrote {args.report}")
    print(f"records: {records_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
