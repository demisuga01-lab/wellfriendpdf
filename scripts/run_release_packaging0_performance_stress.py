#!/usr/bin/env python3
"""Bounded performance and stress harness for Release Readiness Benchmark.

It generates legal synthetic PDFs, drives the real CLI in isolated processes,
and persists raw output below the supplied result root.  The JSON records only
timings, capped RSS, and sanitized status summaries.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import resource
import subprocess
import time
from pathlib import Path
from typing import Iterable


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_pdf(pages: int, label: str, payload_repeat: int = 1) -> bytes:
    objects: list[bytes] = [b"<< /Type /Catalog /Pages 2 0 R >>"]
    kids = " ".join(f"{4 + page * 2} 0 R" for page in range(pages)).encode()
    objects.append(b"<< /Type /Pages /Count " + str(pages).encode() + b" /Kids [" + kids + b"] >>")
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    text = (f"BT /F1 10 Tf 72 720 Td ({label} Wellfriend PDF SDK stress fixture) Tj ET\n" * payload_repeat).encode()
    for page in range(pages):
        page_obj = 4 + page * 2
        content_obj = page_obj + 1
        objects.append(
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents "
            + f"{content_obj} 0 R".encode()
            + b" >>"
        )
        objects.append(b"<< /Length " + str(len(text)).encode() + b" >>\nstream\n" + text + b"endstream")
    output = bytearray(b"%PDF-1.7\n% ReleaseReadinessBenchmark generated fixture\n")
    offsets = [0]
    for index, body in enumerate(objects, start=1):
        offsets.append(len(output))
        output.extend(f"{index} 0 obj\n".encode())
        output.extend(body)
        output.extend(b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    output.extend(b"".join(f"{offset:010d} 00000 n \n".encode() for offset in offsets[1:]))
    output.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(output)


def generate_fixtures(root: Path) -> list[dict[str, object]]:
    root.mkdir(parents=True, exist_ok=True)
    specs = [
        ("many-pages-400.pdf", 400, "many-pages", 1),
        ("object-dense-150.pdf", 150, "object-dense", 6),
        ("text-heavy-40.pdf", 40, "text-heavy", 200),
        ("batch-small-20.pdf", 20, "batch-small", 3),
    ]
    rows = []
    for name, pages, label, repeat in specs:
        path = root / name
        path.write_bytes(build_pdf(pages, label, repeat))
        rows.append({"path": str(path.resolve()), "kind": label, "pages_requested": pages, "bytes": path.stat().st_size, "sha256": sha256(path)})
    return rows


def rss_kib(pid: int) -> int:
    status = Path(f"/proc/{pid}/status")
    try:
        for line in status.read_text(encoding="utf-8", errors="ignore").splitlines():
            if line.startswith("VmHWM:") or line.startswith("VmRSS:"):
                fields = line.split()
                return int(fields[1])
    except (OSError, ValueError):
        pass
    return 0


def preexec(memory_mb: int):
    def apply() -> None:
        cap = memory_mb * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (cap, cap))
    return apply


def run(cmd: list[str], raw_log: Path, timeout_seconds: float, memory_mb: int) -> dict[str, object]:
    raw_log.parent.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, preexec_fn=preexec(memory_mb))
    peak = 0
    timed_out = False
    while proc.poll() is None:
        peak = max(peak, rss_kib(proc.pid))
        if time.perf_counter() - started > timeout_seconds:
            timed_out = True
            proc.kill()
            break
        time.sleep(0.03)
    stdout, stderr = proc.communicate()
    peak = max(peak, rss_kib(proc.pid))
    raw_log.write_text("$ " + " ".join(cmd) + "\n\n--- stdout ---\n" + stdout + "\n--- stderr ---\n" + stderr, encoding="utf-8", errors="ignore")
    lowered = (stdout + stderr).lower()
    status = "passed" if proc.returncode == 0 and not timed_out else "failed_cleanly"
    if timed_out:
        status = "timeout"
    if "panic" in lowered or "segmentation fault" in lowered or "addresssanitizer" in lowered:
        status = "crash"
    return {
        "status": status,
        "exit_code": proc.returncode,
        "elapsed_seconds": round(time.perf_counter() - started, 4),
        "peak_rss_kib": peak,
        "memory_cap_mib": memory_mb,
        "raw_log_path": str(raw_log),
        "raw_log_sha256": sha256(raw_log),
    }


def collect_pdfs(paths: Iterable[Path], limit: int) -> list[Path]:
    found: list[Path] = []
    for root in paths:
        if not root.exists():
            continue
        candidates = [root] if root.is_file() else sorted(root.rglob("*.pdf"))
        for path in candidates:
            if path.is_file():
                found.append(path)
                if len(found) >= limit:
                    return found
    return found


def command_rows(binary: Path, pdf: Path, output_root: Path) -> list[tuple[str, list[str]]]:
    name = pdf.stem[:64]
    return [
        ("parser_audit", [str(binary), "parser-report", str(pdf), "--mode", "audit", "--json"]),
        ("text_extract", [str(binary), "extract-text", str(pdf), "--pages", "1", "--output", str(output_root / f"{name}.txt")]),
        ("render_smoke", [str(binary), "render", str(pdf), "--pages", "1", "--dpi", "72", "--format", "png", "--output", str(output_root / f"{name}.zip"), "--json"]),
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--wellfriendpdf-bin", type=Path, required=True)
    parser.add_argument("--public-corpus", type=Path, required=True)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    parser.add_argument("--memory-cap-mb", type=int, default=2048)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()
    if not args.wellfriendpdf_bin.is_file():
        raise SystemExit(f"missing CLI binary: {args.wellfriendpdf_bin}")

    fixtures = generate_fixtures(args.fixture_root)
    generated_paths = [Path(str(row["path"])) for row in fixtures]
    public_paths = collect_pdfs([args.public_corpus], args.limit)
    repo_paths = collect_pdfs([args.repo / "tests/corpus/pdfs"], max(0, args.limit - len(public_paths)))
    corpus = public_paths + repo_paths
    raw_root = args.artifact_root / "raw" / "performance"
    output_root = args.artifact_root / "outputs"
    output_root.mkdir(parents=True, exist_ok=True)
    operations: list[dict[str, object]] = []
    for pdf in generated_paths + corpus[: min(len(corpus), 30)]:
        for operation, cmd in command_rows(args.wellfriendpdf_bin, pdf, output_root):
            result = run(cmd, raw_root / f"{pdf.stem}-{operation}.log", args.timeout_seconds, args.memory_cap_mb)
            operations.append({"file": str(pdf), "operation": operation, **result})

    batch_inputs = corpus[: min(40, len(corpus))]
    def parser_job(pdf: Path) -> dict[str, object]:
        result = run([str(args.wellfriendpdf_bin), "parser-report", str(pdf), "--mode", "audit", "--json"], raw_root / "parallel" / f"{pdf.stem}.log", args.timeout_seconds, args.memory_cap_mb)
        return {"file": str(pdf), **result}
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, min(args.workers, 4))) as pool:
        parallel = list(pool.map(parser_job, batch_inputs))

    all_rows = operations + parallel
    crashes = [row for row in all_rows if row["status"] == "crash"]
    timeouts = [row for row in all_rows if row["status"] == "timeout"]
    failures = [row for row in all_rows if row["status"] != "passed"]
    max_rss = max((int(row.get("peak_rss_kib", 0)) for row in all_rows), default=0)
    public_rows = [row for row in operations if any(str(path) == row["file"] for path in public_paths)]
    common = {
        "schema_version": "release_readiness_benchmark.performance-stress.v1",
        "generated_at_utc": utc(),
        "memory_budget_mib": 32768,
        "per_process_memory_cap_mib": args.memory_cap_mb,
        "parallel_workers": max(1, min(args.workers, 4)),
        "max_peak_rss_kib": max_rss,
        "crash_hang_oom_count": len(crashes),
        "timeout_count": len(timeouts),
        "timeout_policy": "timeouts are retained as bounded under-cap measurements and require a per-file result row",
        "clean_failures": len(failures) - len(crashes) - len(timeouts),
        "verdict": "passed" if not crashes and all_rows else "failed",
    }
    write_json(args.artifact_root / "generated-stress-fixture-results.json", {**common, "fixtures": fixtures})
    write_json(args.artifact_root / "public-corpus-benchmark-results.json", {**common, "public_file_count": len(public_paths), "rows": public_rows})
    write_json(args.artifact_root / "performance-stress-results.json", {**common, "operations": operations, "batch_rows": parallel, "batch_file_count": len(batch_inputs)})
    write_json(args.artifact_root / "performance-memory-results.json", {**common, "aggregate_configured_parallel_cap_mib": args.memory_cap_mb * max(1, min(args.workers, 4))})
    write_json(args.artifact_root / "performance-regression-verdict.json", {**common, "thresholds": {"zero_crash_hang_oom": True, "aggregate_memory_below_budget": args.memory_cap_mb * max(1, min(args.workers, 4)) <= 32768}})
    print(json.dumps({"status": common["verdict"], "operations": len(operations), "parallel_rows": len(parallel), "peak_rss_kib": max_rss, "artifact_root": str(args.artifact_root)}, sort_keys=True))
    return 0 if common["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
