#!/usr/bin/env python3
"""Prompt 29 bounded malformed-PDF corpus runner.

The runner deliberately records only summaries in JSON and stores raw tool output
under the chosen output directory. It accepts public/user corpus roots when
present, adds compact generated malformed fixtures, and falls back to repository
fixtures/seeds without claiming public-world coverage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import resource
import subprocess
import time
from collections import Counter
from pathlib import Path
from typing import Iterable


PDF_SUFFIXES = {".pdf", ".seed"}


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: Iterable[dict[str, object]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True) + "\n")


def generated_malformed_fixtures(root: Path) -> list[Path]:
    root.mkdir(parents=True, exist_ok=True)
    fixtures: dict[str, bytes] = {
        "truncated-header.pdf": b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\n",
        "broken-xref.pdf": (
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
            b"2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj\nxref\n0 3\n"
            b"0000000000 65535 f \n9999999999 00000 n \ntrailer\n<< /Root 1 0 R >>\nstartxref\n9\n%%EOF\n"
        ),
        "bad-stream-length.pdf": (
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
            b"2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n"
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n"
            b"4 0 obj\n<< /Length 999999 >>\nstream\nBT /F1 12 Tf (x) Tj ET\nendstream\nendobj\n%%EOF\n"
        ),
        "false-stream-markers.pdf": b"%PDF-1.4\nstream stream endobj endstream xref trailer %%EOF\n",
        "huge-object-number.pdf": b"%PDF-1.7\n42949672999 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n",
        "cyclic-pages.pdf": (
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
            b"2 0 obj\n<< /Type /Pages /Count 1 /Kids [2 0 R] >>\nendobj\n%%EOF\n"
        ),
        "broken-encryption-dict.pdf": (
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n"
            b"trailer\n<< /Root 1 0 R /Encrypt << /Filter /Standard /V 99 /R 99 >> >>\n%%EOF\n"
        ),
    }
    paths = []
    for name, data in fixtures.items():
        path = root / name
        path.write_bytes(data)
        paths.append(path)
    return paths


def default_roots(repo: Path, output_root: Path) -> list[tuple[Path, str, str]]:
    roots: list[tuple[Path, str, str]] = [
        (Path("/home/demisuga01/wellpdf/corpus/malformed"), "external_public_or_user", "result_only"),
        (Path("/home/demisuga01/wellpdf/corpus/safedocs"), "external_public_or_user", "result_only"),
        (Path("/home/demisuga01/wellpdf/corpus/unsafe-docs"), "external_public_or_user", "result_only"),
        (repo / "tests/corpus/pdfs", "repo_fixture", "tracked_or_source"),
        (repo / "crates/engine/tests/fixtures", "repo_fixture", "tracked_or_source"),
        (repo / "fuzz/seeds/prompt28", "committed_seed", "tracked_or_source"),
        (output_root / "generated-malformed", "generated_prompt29", "result_only"),
    ]
    generated_malformed_fixtures(output_root / "generated-malformed")
    return roots


def iter_candidate_files(root: Path, limit: int, max_bytes: int) -> list[Path]:
    if not root.exists():
        return []
    candidates = [root] if root.is_file() else sorted(p for p in root.rglob("*") if p.is_file())
    selected = []
    for path in candidates:
        if path.suffix.lower() not in PDF_SUFFIXES:
            continue
        try:
            if path.stat().st_size > max_bytes:
                continue
        except OSError:
            continue
        selected.append(path)
        if len(selected) >= limit:
            break
    return selected


def category_for(path: Path) -> list[str]:
    name = path.as_posix().lower()
    categories = []
    mapping = {
        "damaged_xref": ["xref", "hybrid"],
        "broken_trailer_root": ["trailer", "root", "catalog"],
        "object_stream_damage": ["object_stream", "objstm"],
        "xref_stream_damage": ["xref_stream"],
        "malformed_stream_length": ["stream", "length"],
        "decompression_bomb_candidate": ["flate", "predictor", "lzw"],
        "image_codec_malformed": ["jpeg", "jpx", "jbig2", "ccitt", "image"],
        "malformed_content_stream": ["content", "path", "text", "shading"],
        "broken_page_tree": ["page", "pages", "cyclic"],
        "encrypted_broken_file": ["encrypt", "protected", "password"],
        "signed_broken_file": ["sig", "signed", "signature"],
        "standards_metadata_malformed": ["pdfa", "pdfua", "pdfx", "metadata"],
        "annotations_forms_damage": ["annot", "form", "field"],
    }
    for category, hints in mapping.items():
        if any(hint in name for hint in hints):
            categories.append(category)
    return categories or ["unknown_malformed"]


def limit_process(memory_mb: int) -> None:
    if memory_mb > 0:
        limit = memory_mb * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))


def run_command(
    cmd: list[str],
    timeout_seconds: float,
    memory_mb: int,
    raw_log: Path,
) -> dict[str, object]:
    start = time.perf_counter()
    raw_log.parent.mkdir(parents=True, exist_ok=True)
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            preexec_fn=lambda: limit_process(memory_mb) if os.name == "posix" else None,
        )
        status = "passed" if proc.returncode == 0 else "failed_cleanly"
        stdout = proc.stdout
        stderr = proc.stderr
        exit_code: int | None = proc.returncode
    except FileNotFoundError as exc:
        status = "unavailable_external_tool"
        stdout = ""
        stderr = str(exc)
        exit_code = None
    except subprocess.TimeoutExpired as exc:
        status = "timeout"
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        exit_code = None
    elapsed = time.perf_counter() - start
    raw_log.write_text(
        "$ " + " ".join(cmd) + "\n\n--- stdout ---\n" + stdout + "\n--- stderr ---\n" + stderr,
        encoding="utf-8",
        errors="ignore",
    )
    lowered = (stdout + stderr).lower()
    if "panicked at" in lowered or "segmentation fault" in lowered or "addresssanitizer" in lowered:
        status = "panic_crash"
    if "memory allocation" in lowered or "cannot allocate memory" in lowered:
        status = "oom"
    return {
        "command": cmd,
        "exit_code": exit_code,
        "status": status,
        "time_seconds": round(elapsed, 6),
        "raw_log_path": str(raw_log),
        "raw_log_sha256": sha256(raw_log),
    }


def page_count(pdfinfo_result: dict[str, object]) -> int | None:
    log_path = pdfinfo_result.get("raw_log_path")
    if not log_path:
        return None
    text = Path(str(log_path)).read_text(encoding="utf-8", errors="ignore")
    for line in text.splitlines():
        if line.lower().startswith("pages:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError:
                return None
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--output-root", type=Path, default=Path("target/prompt29-malformed-differential-coverage"))
    parser.add_argument("--wellfriendpdf-bin", type=Path, required=True)
    parser.add_argument("--corpus-root", action="append", type=Path, default=[])
    parser.add_argument("--limit", type=int, default=300)
    parser.add_argument("--max-bytes", type=int, default=50 * 1024 * 1024)
    parser.add_argument("--timeout-seconds", type=float, default=20.0)
    parser.add_argument("--memory-mb", type=int, default=2048)
    args = parser.parse_args()

    repo = args.repo.resolve()
    out = args.output_root.resolve()
    roots = [(p, "user_supplied", "result_only") for p in args.corpus_root] if args.corpus_root else default_roots(repo, out)
    raw_dir = out / "raw" / "malformed-corpus"
    sources = []
    manifest = []
    seen_hashes = set()
    per_root_limit = max(1, args.limit)
    for root, source_category, retention in roots:
        files = iter_candidate_files(root, per_root_limit, args.max_bytes)
        sources.append(
            {
                "root": str(root),
                "exists": root.exists(),
                "source_category": source_category,
                "retention": retention,
                "candidate_count": len(files),
            }
        )
        for path in files:
            digest = sha256(path)
            if digest in seen_hashes:
                continue
            seen_hashes.add(digest)
            manifest.append(
                {
                    "path": str(path),
                    "sha256": digest,
                    "bytes": path.stat().st_size,
                    "source_category": source_category,
                    "retention": retention,
                    "license_provenance": "repo_or_user_supplied_or_generated; see source inventory",
                    "privacy_sensitivity": "public_or_generated_or_repo_fixture",
                    "category_tags": category_for(path),
                    "expected_behavior": "clean_reject_or_clean_diagnostic_no_crash",
                }
            )
            if len(manifest) >= args.limit:
                break
        if len(manifest) >= args.limit:
            break

    rows = []
    for index, item in enumerate(manifest):
        path = Path(str(item["path"]))
        file_dir = raw_dir / f"{index:05d}-{path.stem[:48]}"
        parser_audit = run_command(
            [str(args.wellfriendpdf_bin), "parser-report", str(path), "--mode", "audit", "--json"],
            args.timeout_seconds,
            args.memory_mb,
            file_dir / "wellfriend-parser-audit.log",
        )
        parser_repair = run_command(
            [str(args.wellfriendpdf_bin), "parser-report", str(path), "--mode", "repair", "--json"],
            args.timeout_seconds,
            args.memory_mb,
            file_dir / "wellfriend-parser-repair.log",
        )
        extract = run_command(
            [str(args.wellfriendpdf_bin), "extract-text", str(path), "--format", "json"],
            args.timeout_seconds,
            args.memory_mb,
            file_dir / "wellfriend-extract-text.log",
        )
        render = run_command(
            [
                str(args.wellfriendpdf_bin),
                "render",
                str(path),
                "--pages",
                "1",
                "--dpi",
                "36",
                "--format",
                "png",
                "--json",
                "--max-render-pixels",
                "4194304",
                "--output",
                str(file_dir / "render.zip"),
            ],
            args.timeout_seconds,
            args.memory_mb,
            file_dir / "wellfriend-render-smoke.log",
        )
        pdfinfo = run_command(["pdfinfo", str(path)], args.timeout_seconds, args.memory_mb, file_dir / "pdfinfo.log")
        hard_statuses = {parser_audit["status"], parser_repair["status"], extract["status"], render["status"]}
        crash_like = sorted(hard_statuses & {"panic_crash", "timeout", "oom"})
        outcome = "parsed_ok" if parser_audit["status"] == "passed" else "malformed_rejected_cleanly"
        if crash_like:
            outcome = crash_like[0]
        rows.append(
            {
                **item,
                "page_count": page_count(pdfinfo),
                "operations": {
                    "parser_audit": parser_audit,
                    "parser_repair": parser_repair,
                    "extract_text": extract,
                    "render_smoke": render,
                    "pdfinfo": pdfinfo,
                },
                "outcome": outcome,
                "unclassified_failure": outcome in {"panic_crash", "timeout", "oom"},
            }
        )

    counts = Counter(str(row["outcome"]) for row in rows)
    failure_rows = [row for row in rows if row["unclassified_failure"]]
    buckets = {
        "schema_version": "prompt29.malformed-corpus.failure-buckets.v1",
        "generated_at_utc": utc(),
        "buckets": dict(counts),
        "unclassified": [
            {"path": row["path"], "sha256": row["sha256"], "outcome": row["outcome"]} for row in failure_rows
        ],
        "verdict": "passed" if not failure_rows else "failed",
    }
    scorecard = {
        "schema_version": "prompt29.malformed-corpus.survival-scorecard.v1",
        "generated_at_utc": utc(),
        "attempted_count": len(rows),
        "crash_hang_oom_count": len(failure_rows),
        "clean_handled_count": len(rows) - len(failure_rows),
        "status_counts": dict(counts),
        "verdict": "passed" if rows and not failure_rows else "failed",
    }
    write_json(out / "malformed-corpus-source-inventory.json", {"schema_version": "prompt29.corpus-source-inventory.v1", "generated_at_utc": utc(), "sources": sources, "verdict": "passed" if rows else "failed"})
    write_json(out / "malformed-corpus-manifest.json", {"schema_version": "prompt29.malformed-corpus-manifest.v1", "generated_at_utc": utc(), "files": manifest, "file_count": len(manifest), "verdict": "passed" if manifest else "failed"})
    write_json(out / "malformed-corpus-run-results.json", {"schema_version": "prompt29.malformed-corpus-run-results.v1", "generated_at_utc": utc(), "rows": rows, "verdict": "passed" if rows and not failure_rows else "failed"})
    write_jsonl(out / "malformed-corpus-per-file-results.jsonl", rows)
    write_json(out / "malformed-corpus-failure-buckets.json", buckets)
    write_json(out / "malformed-corpus-survival-scorecard.json", scorecard)
    print(json.dumps({"status": scorecard["verdict"], "attempted": len(rows), "crash_hang_oom": len(failure_rows), "artifact": str(out / "malformed-corpus-survival-scorecard.json")}, sort_keys=True))
    return 0 if scorecard["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
