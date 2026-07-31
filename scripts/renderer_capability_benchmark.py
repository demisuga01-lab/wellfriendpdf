#!/usr/bin/env python3
"""Renderer capability benchmark harness.

The harness is intentionally evidence-first:

* inputs are real PDF files discovered from a corpus directory;
* every tool row keeps failures and timeouts;
* outputs are rendered and discarded, never committed;
* wrappers are named as wrappers, not independent engines;
* per-file JSONL and aggregate JSON are written for audit.

It is designed for VPS execution. Do not use it to move private PDFs into Git.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import importlib.metadata
import json
import os
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any, Callable


def discover_pdfs(root: Path, limit: int | None) -> list[Path]:
    files = sorted(p for p in root.rglob("*.pdf") if p.is_file())
    if limit is not None:
        files = files[:limit]
    return files


def run_cmd(cmd: list[str], timeout: float, tmp_root: Path) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            cwd=tmp_root,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:
        stderr = exc.stderr or b""
        return {
            "exit": None,
            "duration_ms": (time.perf_counter() - started) * 1000.0,
            "stderr_sha256": hashlib.sha256(stderr).hexdigest(),
            "stderr_bytes": len(stderr),
            "error": "timeout",
        }
    return {
        "exit": proc.returncode,
        "duration_ms": (time.perf_counter() - started) * 1000.0,
        "stderr_sha256": hashlib.sha256(proc.stderr).hexdigest(),
        "stderr_bytes": len(proc.stderr),
    }


def run_cmd_json(cmd: list[str], timeout: float, tmp_root: Path) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=tmp_root,
            timeout=timeout,
            text=True,
        )
    except subprocess.TimeoutExpired as exc:
        stderr = exc.stderr or ""
        if isinstance(stderr, bytes):
            stderr_bytes = stderr
        else:
            stderr_bytes = stderr.encode("utf-8", errors="replace")
        return {
            "exit": None,
            "duration_ms": (time.perf_counter() - started) * 1000.0,
            "stderr_sha256": hashlib.sha256(stderr_bytes).hexdigest(),
            "stderr_bytes": len(stderr_bytes),
            "error": "timeout",
        }
    payload: dict[str, Any] = {}
    if proc.stdout.strip():
        try:
            payload = json.loads(proc.stdout)
        except json.JSONDecodeError:
            payload = {"stdout_json_error": True}
    return {
        "exit": proc.returncode,
        "duration_ms": (time.perf_counter() - started) * 1000.0,
        "stderr_sha256": hashlib.sha256(proc.stderr.encode("utf-8")).hexdigest(),
        "stderr_bytes": len(proc.stderr.encode("utf-8")),
        **payload,
    }


def render_poppler(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    if not shutil.which("pdftoppm"):
        return {"ok": False, "error": "unavailable:pdftoppm", "duration_ms": 0.0}
    with tempfile.TemporaryDirectory(prefix="poppler-", dir=tmp_root) as tmp:
        prefix = str(Path(tmp) / "page")
        cmd = ["pdftoppm", "-q", "-r", str(dpi), "-png"]
        if pages == "first":
            cmd += ["-f", "1", "-l", "1", "-singlefile"]
        cmd += [str(path), prefix]
        result = run_cmd(cmd, timeout, tmp_root)
        rendered = len(list(Path(tmp).glob("*.png")))
    return {
        "ok": result["exit"] == 0 and rendered > 0,
        "pages_rendered": rendered,
        **result,
    }


def render_mupdf(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    if not shutil.which("mutool"):
        return {"ok": False, "error": "unavailable:mutool", "duration_ms": 0.0}
    with tempfile.TemporaryDirectory(prefix="mupdf-", dir=tmp_root) as tmp:
        pattern = str(Path(tmp) / "page-%d.png")
        cmd = ["mutool", "draw", "-q", "-r", str(dpi), "-o", pattern, str(path)]
        if pages == "first":
            cmd.append("1")
        result = run_cmd(cmd, timeout, tmp_root)
        rendered = len(list(Path(tmp).glob("*.png")))
    return {
        "ok": result["exit"] == 0 and rendered > 0,
        "pages_rendered": rendered,
        **result,
    }


def render_pypdfium2(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    del tmp_root
    started = time.perf_counter()
    try:
        import pypdfium2 as pdfium  # type: ignore

        doc = pdfium.PdfDocument(str(path))
        try:
            scale = dpi / 72.0
            page_indexes = range(len(doc)) if pages == "all" else range(min(1, len(doc)))
            rendered = 0
            for index in page_indexes:
                if time.perf_counter() - started > timeout:
                    return {
                        "ok": False,
                        "error": "timeout",
                        "duration_ms": (time.perf_counter() - started) * 1000.0,
                        "pages_rendered": rendered,
                    }
                page = doc[index]
                bitmap = page.render(scale=scale)
                try:
                    _ = bitmap.to_numpy()
                finally:
                    close = getattr(bitmap, "close", None)
                    if close is not None:
                        close()
                    close = getattr(page, "close", None)
                    if close is not None:
                        close()
                rendered += 1
            return {
                "ok": rendered > 0,
                "duration_ms": (time.perf_counter() - started) * 1000.0,
                "pages_rendered": rendered,
            }
        finally:
            close = getattr(doc, "close", None)
            if close is not None:
                close()
    except Exception as exc:  # noqa: BLE001 - benchmark row records exact class
        return {
            "ok": False,
            "error": f"{type(exc).__name__}: {exc}",
            "duration_ms": (time.perf_counter() - started) * 1000.0,
        }


def render_pymupdf(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    del tmp_root
    started = time.perf_counter()
    try:
        import fitz  # type: ignore

        doc = fitz.open(str(path))
        scale = dpi / 72.0
        matrix = fitz.Matrix(scale, scale)
        page_indexes = range(doc.page_count) if pages == "all" else range(min(1, doc.page_count))
        rendered = 0
        for index in page_indexes:
            if time.perf_counter() - started > timeout:
                return {
                    "ok": False,
                    "error": "timeout",
                    "duration_ms": (time.perf_counter() - started) * 1000.0,
                    "pages_rendered": rendered,
                }
            pix = doc.load_page(index).get_pixmap(matrix=matrix, alpha=False)
            _ = pix.samples
            rendered += 1
        return {
            "ok": rendered > 0,
            "duration_ms": (time.perf_counter() - started) * 1000.0,
            "pages_rendered": rendered,
        }
    except Exception as exc:  # noqa: BLE001
        return {
            "ok": False,
            "error": f"{type(exc).__name__}: {exc}",
            "duration_ms": (time.perf_counter() - started) * 1000.0,
        }


def render_pdfbox(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    jar = os.environ.get("PDFBOX_APP_JAR")
    if not jar or not Path(jar).is_file() or not shutil.which("java"):
        return {"ok": False, "error": "unavailable:pdfbox_app_jar", "duration_ms": 0.0}
    with tempfile.TemporaryDirectory(prefix="pdfbox-", dir=tmp_root) as tmp:
        prefix = str(Path(tmp) / "page")
        cmd = [
            "java",
            "-jar",
            jar,
            "render",
            "-i",
            str(path),
            "-dpi",
            str(dpi),
            "-format",
            "png",
            "-prefix",
            prefix,
        ]
        if pages == "first":
            cmd += ["-startPage", "1", "-endPage", "1"]
        result = run_cmd(cmd, timeout, tmp_root)
        rendered = len(list(Path(tmp).glob("*.png")))
    return {
        "ok": result["exit"] == 0 and rendered > 0,
        "pages_rendered": rendered,
        **result,
    }


def render_pdfjs(path: Path, pages: str, dpi: int, timeout: float, tmp_root: Path) -> dict[str, Any]:
    script = os.environ.get("PDFJS_RENDERER_SCRIPT")
    if not script or not Path(script).is_file() or not shutil.which("node"):
        return {"ok": False, "error": "unavailable:pdfjs_renderer", "duration_ms": 0.0}
    result = run_cmd_json(
        [
            "node",
            script,
            "--input",
            str(path),
            "--pages",
            pages,
            "--dpi",
            str(dpi),
        ],
        timeout,
        tmp_root,
    )
    rendered = int(result.get("pages_rendered") or 0)
    return {
        "ok": result["exit"] == 0 and rendered > 0,
        "pages_rendered": rendered,
        **result,
    }


TOOL_RUNNERS: dict[str, Callable[[Path, str, int, float, Path], dict[str, Any]]] = {
    "poppler": render_poppler,
    "mupdf": render_mupdf,
    "pypdfium2": render_pypdfium2,
    "pymupdf": render_pymupdf,
    "pdfbox": render_pdfbox,
    "pdfjs": render_pdfjs,
}


def tool_versions() -> dict[str, Any]:
    versions: dict[str, Any] = {}
    for exe, cmd in {
        "pdftoppm": ["pdftoppm", "-v"],
        "mutool": ["mutool", "-v"],
        "java": ["java", "-version"],
    }.items():
        path = shutil.which(exe)
        if not path:
            versions[exe] = {"available": False}
            continue
        proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        versions[exe] = {
            "available": True,
            "path": path,
            "version": (proc.stdout or proc.stderr).splitlines()[:3],
        }
    for package in ["pypdfium2", "PyMuPDF", "pikepdf", "pyHanko", "pdfplumber"]:
        try:
            versions[package] = {
                "available": True,
                "version": importlib.metadata.version(package),
            }
        except importlib.metadata.PackageNotFoundError:
            versions[package] = {"available": False}
    versions["pdfbox_app_jar"] = {
        "available": bool(os.environ.get("PDFBOX_APP_JAR"))
        and Path(os.environ.get("PDFBOX_APP_JAR", "")).is_file(),
        "path": os.environ.get("PDFBOX_APP_JAR"),
    }
    versions["pdfjs_renderer_script"] = {
        "available": bool(os.environ.get("PDFJS_RENDERER_SCRIPT"))
        and Path(os.environ.get("PDFJS_RENDERER_SCRIPT", "")).is_file(),
        "path": os.environ.get("PDFJS_RENDERER_SCRIPT"),
    }
    return versions


def percentile(values: list[float], q: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = int((len(ordered) - 1) * q + 0.999999)
    return ordered[min(index, len(ordered) - 1)]


def run_one(args: tuple[str, Path, str, int, float, Path, int]) -> dict[str, Any]:
    tool, path, pages, dpi, timeout, tmp_root, index = args
    runner = TOOL_RUNNERS[tool]
    result = runner(path, pages, dpi, timeout, tmp_root)
    return {
        "tool": tool,
        "index": index,
        "path": str(path),
        "bytes": path.stat().st_size,
        **result,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--tmp-dir", required=True, type=Path)
    parser.add_argument("--tools", default="poppler,mupdf,pypdfium2,pymupdf,pdfbox")
    parser.add_argument("--pages", choices=["first", "all"], default="first")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout-sec", type=float, default=120.0)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--limit", type=int)
    ns = parser.parse_args()

    ns.out_dir.mkdir(parents=True, exist_ok=True)
    ns.tmp_dir.mkdir(parents=True, exist_ok=True)
    tools = [tool.strip() for tool in ns.tools.split(",") if tool.strip()]
    unknown = [tool for tool in tools if tool not in TOOL_RUNNERS]
    if unknown:
        raise SystemExit(f"unknown tools: {', '.join(unknown)}")

    files = discover_pdfs(ns.corpus, ns.limit)
    version_report = tool_versions()
    (ns.out_dir / "tool-versions.json").write_text(
        json.dumps(version_report, indent=2, sort_keys=True),
        encoding="utf-8",
    )

    rows: list[dict[str, Any]] = []
    jsonl_path = ns.out_dir / f"renderer-{ns.pages}-{ns.dpi}dpi.jsonl"
    started = time.perf_counter()
    with jsonl_path.open("w", encoding="utf-8") as out:
        for tool in tools:
            jobs = [
                (tool, path, ns.pages, ns.dpi, ns.timeout_sec, ns.tmp_dir, index)
                for index, path in enumerate(files)
            ]
            workers = max(1, ns.workers)
            with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
                for row in pool.map(run_one, jobs):
                    rows.append(row)
                    out.write(json.dumps(row, sort_keys=True) + "\n")
                    out.flush()

    summary_rows = []
    for tool in tools:
        tool_rows = [row for row in rows if row["tool"] == tool]
        durations = [row["duration_ms"] for row in tool_rows if row.get("ok")]
        summary_rows.append(
            {
                "tool": tool,
                "runs": len(tool_rows),
                "successes": sum(1 for row in tool_rows if row.get("ok")),
                "failures": sum(1 for row in tool_rows if not row.get("ok")),
                "pages_rendered": sum(int(row.get("pages_rendered") or 0) for row in tool_rows),
                "median_ms": statistics.median(durations) if durations else None,
                "p95_ms": percentile(durations, 0.95),
                "p99_ms": percentile(durations, 0.99),
                "max_ms": max(durations) if durations else None,
            }
        )
    summary = {
        "schema_version": "wellfriend.renderer_capability_comparator.v1",
        "corpus": str(ns.corpus),
        "files": len(files),
        "tools": tools,
        "pages": ns.pages,
        "dpi": ns.dpi,
        "workers": ns.workers,
        "timeout_sec": ns.timeout_sec,
        "duration_sec": time.perf_counter() - started,
        "jsonl": str(jsonl_path),
        "summary": summary_rows,
        "versions": version_report,
    }
    (ns.out_dir / f"renderer-{ns.pages}-{ns.dpi}dpi-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True),
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
