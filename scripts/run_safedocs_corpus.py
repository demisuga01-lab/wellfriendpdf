#!/usr/bin/env python3
"""Run a bounded SafeDocs/fallback corpus sweep.

The runner attempts every file under the selected SafeDocs root. If no
SafeDocs root is available and --allow-unavailable is set, it records exact
unavailability and runs the closest configured fallback corpus roots instead.
Per-file work runs in subprocesses with timeout and process-tree RSS monitoring.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import subprocess
import time
from pathlib import Path


PDF_EXTS = {".pdf"}
SCHEMA = "fuzz_campaign.safedocs-corpus-run.v1"
DEFAULT_SAFEDOCS_ROOTS = [
    Path("/home/demisuga01/wellpdf/corpus/safedocs"),
    Path("/home/demisuga01/wellpdf/corpus/CC-MAIN-2021-31-PDF-UNTRUNCATED"),
    Path("/home/demisuga01/wellpdf/corpus/unsafe-docs"),
]
DEFAULT_FALLBACK_ROOTS = [
    Path("tests/corpus/pdfs"),
    Path("crates/engine/tests/fixtures"),
    Path("renderer-benchmark/corpus/real-world/pdfjs-full"),
]
SAFEDOCS_SOURCES = [
    {
        "name": "CC-MAIN-2021-31-PDF-UNTRUNCATED",
        "url": "https://digitalcorpora.org/corpora/file-corpora/cc-main-2021-31-pdf-untruncated/",
        "note": "SafeDocs public corpus; nearly 8 million PDFs and about 8 TB according to public corpus documentation.",
    },
    {
        "name": "UNSAFE-DOCS",
        "url": "https://digitalcorpora.org/corpora/file-corpora/unsafe-docs-cc-main-2021-31-unsafe/",
        "note": "SafeDocs unsafe/malformed corpus; millions of files and explicitly dangerous/malformed content.",
    },
    {
        "name": "PDF Association corpus index",
        "url": "https://github.com/pdf-association/pdf-corpora",
        "note": "Index documenting SafeDocs corpus families, cautioning that some corpora contain malicious/malformed files.",
    },
]


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_rss_kib(pid: int) -> int:
    try:
        for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith("VmRSS:"):
                parts = line.split()
                return int(parts[1]) if len(parts) >= 2 else 0
    except Exception:
        return 0
    return 0


def proc_parent_map() -> dict[int, int]:
    parents: dict[int, int] = {}
    if os.name == "nt" or not Path("/proc").exists():
        return parents
    for item in Path("/proc").iterdir():
        if not item.name.isdigit():
            continue
        try:
            stat = (item / "stat").read_text(encoding="utf-8", errors="replace")
            after_comm = stat.rsplit(")", 1)[1].strip().split()
            if len(after_comm) >= 2:
                parents[int(item.name)] = int(after_comm[1])
        except Exception:
            continue
    return parents


def process_tree_pids(root_pid: int) -> list[int]:
    parents = proc_parent_map()
    children: dict[int, list[int]] = {}
    for pid, ppid in parents.items():
        children.setdefault(ppid, []).append(pid)
    seen: set[int] = set()
    queue = [root_pid]
    while queue:
        pid = queue.pop(0)
        if pid in seen:
            continue
        seen.add(pid)
        queue.extend(children.get(pid, []))
    return sorted(seen)


def process_tree_rss_kib(root_pid: int) -> int:
    return sum(read_rss_kib(pid) for pid in process_tree_pids(root_pid))


def terminate_tree(proc: subprocess.Popen[object]) -> None:
    if proc.poll() is not None:
        return
    try:
        if os.name != "nt":
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        else:
            proc.terminate()
    except Exception:
        pass
    deadline = time.monotonic() + 3
    while proc.poll() is None and time.monotonic() < deadline:
        time.sleep(0.1)
    if proc.poll() is None:
        try:
            if os.name != "nt":
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            else:
                proc.kill()
        except Exception:
            pass


def select_roots(repo: Path, safedocs_roots: list[Path], fallback_roots: list[Path]) -> tuple[str, list[Path], list[dict[str, object]]]:
    checked = []
    available_safedocs = []
    for raw in safedocs_roots:
        root = raw if raw.is_absolute() else repo / raw
        checked.append({"path": str(root), "exists": root.exists(), "kind": "safedocs"})
        if root.exists() and any(p.is_file() for p in root.rglob("*")):
            available_safedocs.append(root)
    if available_safedocs:
        return "safedocs_available", available_safedocs, checked
    fallback = []
    for raw in fallback_roots:
        root = raw if raw.is_absolute() else repo / raw
        checked.append({"path": str(root), "exists": root.exists(), "kind": "fallback"})
        if root.exists() and any(p.is_file() for p in root.rglob("*")):
            fallback.append(root)
    return "unavailable_external_corpus", fallback, checked


def iter_candidate_files(roots: list[Path], max_bytes: int) -> list[dict[str, object]]:
    entries = []
    seen = set()
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            if path in seen:
                continue
            seen.add(path)
            try:
                size = path.stat().st_size
            except OSError:
                continue
            suffix = path.suffix.lower()
            is_pdf = suffix in PDF_EXTS or (size >= 5 and path.open("rb").read(5) == b"%PDF-")
            if not is_pdf:
                continue
            entries.append(
                {
                    "path": str(path),
                    "size_bytes": size,
                    "sha256": sha256(path) if size <= max_bytes else None,
                    "status": "candidate" if size <= max_bytes else "skipped_too_large",
                    "root": str(root),
                }
            )
    return entries


def run_one(path: Path, wellfriendpdf_bin: Path, result_dir: Path, timeout_seconds: int, memory_mb: int) -> dict[str, object]:
    rel_hash = hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:16]
    log_path = result_dir / "safedocs-file-logs" / f"{rel_hash}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        str(wellfriendpdf_bin),
        "parser-report",
        str(path),
        "--mode",
        "audit",
        "--json",
        "--include-decode",
    ]
    started = utc()
    start = time.monotonic()
    timed_out = False
    memory_exceeded = False
    peak_rss_kib = 0
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        log.write("$ " + " ".join(cmd) + "\n")
        log.flush()
        kwargs: dict[str, object] = {
            "stdout": log,
            "stderr": subprocess.STDOUT,
            "text": True,
        }
        if os.name != "nt":
            kwargs["preexec_fn"] = os.setsid
        proc = subprocess.Popen(cmd, **kwargs)
        cap_kib = memory_mb * 1024
        while proc.poll() is None:
            rss = process_tree_rss_kib(proc.pid) if cap_kib else 0
            peak_rss_kib = max(peak_rss_kib, rss)
            if cap_kib and rss > cap_kib:
                memory_exceeded = True
                log.write(f"\nMEMORY_LIMIT_EXCEEDED process_tree_rss_kib={rss} cap_kib={cap_kib}\n")
                terminate_tree(proc)
                break
            if time.monotonic() - start > timeout_seconds:
                timed_out = True
                log.write(f"\nTIMEOUT after {timeout_seconds}s\n")
                terminate_tree(proc)
                break
            time.sleep(0.2)
        exit_code = proc.poll()
    elapsed = round(time.monotonic() - start, 3)
    if memory_exceeded:
        status = "oom"
    elif timed_out:
        status = "timeout"
    elif exit_code == 0:
        status = "parsed_ok"
    elif exit_code in {1, 2}:
        status = "malformed_rejected_cleanly"
    else:
        status = "panic_crash"
    return {
        "path": str(path),
        "started_at_utc": started,
        "elapsed_seconds": elapsed,
        "timeout_seconds": timeout_seconds,
        "memory_cap_mib": memory_mb,
        "peak_rss_kib": peak_rss_kib,
        "exit_code": exit_code,
        "status": status,
        "log_path": str(log_path),
    }


def write_json(path: Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--result-root", type=Path, default=Path("target/fuzz_campaign-long-fuzz-safedocs"))
    parser.add_argument("--wellfriendpdf-bin", type=Path, required=True)
    parser.add_argument("--safedocs-root", type=Path, action="append", default=[])
    parser.add_argument("--fallback-root", type=Path, action="append", default=[])
    parser.add_argument("--timeout-seconds", type=int, default=20)
    parser.add_argument("--memory-mb", type=int, default=2048)
    parser.add_argument("--max-bytes", type=int, default=50 * 1024 * 1024)
    parser.add_argument("--allow-unavailable", action="store_true")
    args = parser.parse_args()

    repo = args.repo.resolve()
    result_root = args.result_root if args.result_root.is_absolute() else repo / args.result_root
    result_root.mkdir(parents=True, exist_ok=True)
    safedocs_roots = args.safedocs_root or DEFAULT_SAFEDOCS_ROOTS
    fallback_roots = args.fallback_root or DEFAULT_FALLBACK_ROOTS
    source_status, selected_roots, checked = select_roots(repo, safedocs_roots, fallback_roots)
    if source_status == "unavailable_external_corpus" and not args.allow_unavailable:
        raise SystemExit("SafeDocs corpus root unavailable and --allow-unavailable not set")

    manifest_entries = iter_candidate_files(selected_roots, args.max_bytes)
    run_entries = [entry for entry in manifest_entries if entry["status"] == "candidate"]
    started = utc()
    records = []
    for entry in run_entries:
        records.append(
            run_one(
                Path(str(entry["path"])),
                args.wellfriendpdf_bin,
                result_root,
                args.timeout_seconds,
                args.memory_mb,
            )
        )

    jsonl = result_root / "safedocs-per-file-results.jsonl"
    jsonl.write_text("".join(json.dumps(record, sort_keys=True) + "\n" for record in records), encoding="utf-8")
    counts: dict[str, int] = {}
    for record in records:
        counts[record["status"]] = counts.get(record["status"], 0) + 1
    skipped = [entry for entry in manifest_entries if entry["status"] != "candidate"]
    unclassified = [
        record
        for record in records
        if record["status"] in {"timeout", "oom", "panic_crash", "sanitizer_failure"}
    ]
    provenance = {
        "schema_version": "fuzz_campaign.safedocs-corpus-provenance.v1",
        "generated_at_utc": utc(),
        "source_status": source_status,
        "selected_roots": [str(root) for root in selected_roots],
        "checked_roots": checked,
        "public_sources": SAFEDOCS_SOURCES,
        "availability_note": (
            "SafeDocs root was available locally/VPS and every candidate file was attempted."
            if source_status == "safedocs_available"
            else "No local/VPS SafeDocs root was available. Public full corpora are multi-million/multi-terabyte scale, so this run records exact unavailable_external_corpus and executes fallback corpus roots."
        ),
    }
    summary = {
        "schema_version": "fuzz_campaign.safedocs-summary.v1",
        "generated_at_utc": utc(),
        "started_at_utc": started,
        "source_status": source_status,
        "file_count": len(manifest_entries),
        "attempted_count": len(records),
        "skipped_count": len(skipped),
        "status_counts": counts,
        "timeout_count": counts.get("timeout", 0),
        "oom_count": counts.get("oom", 0),
        "panic_crash_count": counts.get("panic_crash", 0),
        "max_bytes": args.max_bytes,
        "per_file_timeout_seconds": args.timeout_seconds,
        "per_file_memory_cap_mib": args.memory_mb,
    }
    classification = {
        "schema_version": "fuzz_campaign.safedocs-failure-classification.v1",
        "generated_at_utc": utc(),
        "skipped": skipped,
        "unclassified_failures": unclassified,
        "status_counts": counts,
        "verdict": "passed" if not unclassified else "failed",
    }
    crash_triage = {
        "schema_version": "fuzz_campaign.safedocs-crash-triage.v1",
        "generated_at_utc": utc(),
        "findings": [
            {
                "source_campaign": "safedocs",
                "target_or_file": record["path"],
                "exit_code": record["exit_code"],
                "status": record["status"],
                "log_path": record["log_path"],
                "fixed": False,
                "classification": "unclassified",
            }
            for record in unclassified
        ],
        "unclassified_count": len(unclassified),
        "verdict": "passed" if not unclassified else "failed",
    }
    final = {
        "schema_version": "fuzz_campaign.safedocs-final-verdict.v1",
        "generated_at_utc": utc(),
        "status": (
            "complete"
            if source_status == "safedocs_available" and not unclassified
            else "unavailable_external_corpus_with_fallback_passed"
            if source_status == "unavailable_external_corpus" and records and not unclassified
            else "not_complete"
        ),
        "source_status": source_status,
        "attempted_count": len(records),
        "unclassified_failure_count": len(unclassified),
    }
    run_plan = {
        "schema_version": "fuzz_campaign.safedocs-run-plan.v1",
        "generated_at_utc": utc(),
        "operations": ["parser-report --mode audit --json --include-decode"],
        "deterministic_order": True,
        "timeout_seconds": args.timeout_seconds,
        "memory_cap_mib": args.memory_mb,
        "max_bytes": args.max_bytes,
        "full_available_corpus_policy": "attempt every candidate PDF file under selected SafeDocs root; fallback is not called full SafeDocs",
    }
    manifest = {
        "schema_version": "fuzz_campaign.safedocs-corpus-manifest.v1",
        "generated_at_utc": utc(),
        "source_status": source_status,
        "file_count": len(manifest_entries),
        "entries": manifest_entries,
    }
    for name, payload in {
        "safedocs-corpus-provenance.json": provenance,
        "safedocs-corpus-manifest.json": manifest,
        "safedocs-run-plan.json": run_plan,
        "safedocs-summary.json": summary,
        "safedocs-failure-classification.json": classification,
        "safedocs-crash-triage.json": crash_triage,
        "safedocs-final-verdict.json": final,
    }.items():
        write_json(result_root / name, payload)
    print(json.dumps({"status": final["status"], "attempted": len(records), "source_status": source_status}, sort_keys=True))
    return 0 if final["status"] != "not_complete" else 2


if __name__ == "__main__":
    raise SystemExit(main())
