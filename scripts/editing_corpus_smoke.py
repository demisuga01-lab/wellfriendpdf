#!/usr/bin/env python3
"""Exercise editing/planning/apply/validate/undo surfaces across a PDF corpus.

The script never mutates original inputs. It extracts a small page-1 text
snippet when available, runs source-linked scene and reflow planning commands,
then attempts a same-length GeometricBlock apply into a temporary output PDF.
Successful applies are immediately validated and undone; unsupported or
ambiguous real-world files are counted as typed refusals, not engine failures.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
from typing import Any


STAGES = [
    "extract_page1_text",
    "scene_report_page1",
    "edit_eligibility",
    "reflow_preview",
    "overflow_report",
    "reflow_constraints",
    "reflow_confidence",
    "source_operator_apply",
    "reflow_region_apply",
    "reflow_region_validate",
    "reflow_region_undo",
]

# Parser/validation corpus stages own malformed-PDF failures. This editing
# smoke treats parse/password outcomes as non-mutating typed ineligibility for
# a specific edit operation, while panics, timeouts, and generic nonzero exits
# remain failures.
ACCEPTED_ERROR_CLASSES = {"none", "typed_refusal", "parse", "password"}


def iter_pdfs(corpus: Path) -> list[Path]:
    return sorted(p for p in corpus.rglob("*.pdf") if p.is_file())


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def classify(exit_code: int | None, stderr: str, timed_out: bool) -> str:
    if timed_out:
        return "timeout"
    if exit_code == 0:
        return "none"
    text = stderr.lower()
    if "text_reflow" in text and (
        "parse" in text
        or "not resolved" in text
        or "unsupported" in text
        or "refus" in text
    ):
        return "typed_refusal"
    if "not resolved" in text or "unsupported" in text or "refus" in text:
        return "typed_refusal"
    if "parse" in text or "xref" in text or "trailer" in text:
        return "parse"
    if "password" in text or "encrypted" in text:
        return "password"
    if "panic" in text or "panicked" in text:
        return "panic"
    return "nonzero_exit"


def run_cmd(argv: list[str], timeout: float) -> tuple[int | None, bool, float, int, str]:
    start = time.monotonic()
    try:
        proc = subprocess.run(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            text=True,
            errors="replace",
        )
        duration = (time.monotonic() - start) * 1000.0
        return proc.returncode, False, duration, len(proc.stdout.encode()), proc.stderr
    except subprocess.TimeoutExpired as err:
        duration = (time.monotonic() - start) * 1000.0
        stderr = err.stderr if isinstance(err.stderr, str) else ""
        return None, True, duration, 0, stderr


def compact_row(
    corpus: Path,
    pdf: Path,
    stage: str,
    exit_code: int | None,
    timed_out: bool,
    duration_ms: float,
    stdout_bytes: int,
    stderr: str,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    row: dict[str, Any] = {
        "stage": stage,
        "pdf": str(pdf.relative_to(corpus)),
        "pdf_bytes": pdf.stat().st_size,
        "pdf_sha256": sha256_file(pdf),
        "exit": exit_code,
        "timed_out": timed_out,
        "duration_ms": round(duration_ms, 3),
        "stdout_bytes": stdout_bytes,
        "error_class": classify(exit_code, stderr, timed_out),
        "stderr_sha256": hashlib.sha256(stderr.encode()).hexdigest() if stderr else None,
    }
    if extra:
        row.update(extra)
    return row


def first_snippet(text: str) -> str | None:
    candidates = candidate_snippets(text)
    return candidates[0] if candidates else None


def candidate_snippets(text: str) -> list[str]:
    collapsed = " ".join(text.split())
    if not collapsed:
        return []
    words = [
        match.group(0)
        for match in re.finditer(r"[A-Za-z][A-Za-z0-9]{3,15}", collapsed)
    ]
    seen: set[str] = set()
    candidates: list[str] = []
    for word in sorted(words, key=lambda value: (-len(value), value.lower())):
        lowered = word.lower()
        if lowered in seen:
            continue
        seen.add(lowered)
        candidates.append(word)
        if len(candidates) >= 3:
            break
    if candidates:
        return candidates
    return [collapsed[:32]]


def same_length_replacement(text: str) -> str:
    chars = list(text)
    if not chars:
        return text
    for idx, ch in enumerate(chars):
        if not ch.isspace():
            chars[idx] = "X" if ch != "X" else "Y"
            return "".join(chars)
    return text


def write_reflow_request(path: Path, source: str, replacement: str) -> None:
    path.write_text(
        json.dumps(
            {
                "requested_mode": "geometric_block",
                "page": 1,
                "source_text": source,
                "replacement_text": replacement,
                "region": [0, 0, 612, 792],
                "language": None,
                "direction": None,
                "font_policy": "rebuild_subset_or_generated_type0",
                "hyphenation": False,
                "allow_page_creation": False,
                "allow_font_reduction": False,
                "approve_low_confidence_structure": False,
                "signature_policy_override": False,
            },
            sort_keys=True,
        ),
        encoding="utf-8",
    )


def row_passed(row: dict[str, Any] | None) -> bool:
    return bool(
        row
        and not row.get("timed_out")
        and row.get("error_class") in ACCEPTED_ERROR_CLASSES
    )


def run_pdf(
    corpus: Path,
    binary: Path,
    result_dir: Path,
    pdf: Path,
    timeout: float,
    only_stages: set[str] | None = None,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(dir=result_dir) as tmp_name:
        tmp = Path(tmp_name)
        text_out = tmp / "page1.txt"
        need_snippet = only_stages is None or any(stage in (only_stages or set()) for stage in STAGES[2:])
        need_extract_row = only_stages is None or "extract_page1_text" in only_stages
        exit_code = 0
        timed_out = False
        duration_ms = 0.0
        stdout_bytes = 0
        stderr = ""
        if need_snippet or need_extract_row:
            exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                [str(binary), "extract-text", "--pages", "1", "-o", str(text_out), str(pdf)],
                timeout,
            )
        text = text_out.read_text(errors="replace") if text_out.exists() else ""
        snippet = first_snippet(text)
        if need_extract_row:
            rows.append(
                compact_row(
                    corpus,
                    pdf,
                    "extract_page1_text",
                    exit_code,
                    timed_out,
                    duration_ms,
                    stdout_bytes,
                    stderr,
                    {
                        "snippet_available": bool(snippet),
                        "extracted_bytes": text_out.stat().st_size if text_out.exists() else 0,
                    },
                )
            )

        if only_stages is None or "scene_report_page1" in only_stages:
            scene_out = tmp / "scene.json"
            exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                [str(binary), "scene-report", "--page", "1", "-o", str(scene_out), str(pdf)],
                timeout,
            )
            rows.append(
                compact_row(
                    corpus,
                    pdf,
                    "scene_report_page1",
                    exit_code,
                    timed_out,
                    duration_ms,
                    stdout_bytes,
                    stderr,
                    {"output_bytes": scene_out.stat().st_size if scene_out.exists() else 0},
                )
            )

        if not snippet:
            requested = (
                STAGES[2:]
                if only_stages is None
                else [s for s in STAGES[2:] if s in only_stages]
            )
            for stage in requested:
                rows.append(
                    compact_row(
                        corpus,
                        pdf,
                        stage,
                        0,
                        False,
                        0.0,
                        0,
                        "",
                        {"skipped": True, "skip_reason": "no_page1_text"},
                    )
                )
            return rows

        replacements = {
            "edit_eligibility": [
                str(binary),
                "edit-eligibility",
                "--page",
                "1",
                "--source-text",
                snippet,
                "--replacement-text",
                snippet,
                str(pdf),
            ],
            "reflow_preview": [
                str(binary),
                "reflow-preview",
                "--page",
                "1",
                "--source-text",
                snippet,
                "--replacement-text",
                snippet,
                "--region",
                "0,0,612,792",
                "--json-output",
                str(tmp / "reflow_preview.json"),
                str(pdf),
            ],
            "overflow_report": [
                str(binary),
                "overflow-report",
                "--page",
                "1",
                "--source-text",
                snippet,
                "--replacement-text",
                snippet,
                "--region",
                "0,0,612,792",
                "--json-output",
                str(tmp / "overflow_report.json"),
                str(pdf),
            ],
            "reflow_constraints": [
                str(binary),
                "reflow-constraints",
                "--page",
                "1",
                "--source-text",
                snippet,
                "--replacement-text",
                snippet,
                "--region",
                "0,0,612,792",
                "--json-output",
                str(tmp / "reflow_constraints.json"),
                str(pdf),
            ],
            "reflow_confidence": [
                str(binary),
                "reflow-confidence",
                "--page",
                "1",
                "--source-text",
                snippet,
                "--replacement-text",
                snippet,
                "--region",
                "0,0,612,792",
                "--json-output",
                str(tmp / "reflow_confidence.json"),
                str(pdf),
            ],
        }
        for stage, argv in replacements.items():
            if only_stages is not None and stage not in only_stages:
                continue
            exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(argv, timeout)
            out_path = Path(argv[-2]) if "--json-output" in argv else None
            rows.append(
                compact_row(
                    corpus,
                    pdf,
                    stage,
                    exit_code,
                    timed_out,
                    duration_ms,
                    stdout_bytes,
                    stderr,
                    {
                        "snippet_len": len(snippet),
                        "output_bytes": out_path.stat().st_size
                        if out_path is not None and out_path.exists()
                        else 0,
                    },
                )
            )

        if only_stages is None or "source_operator_apply" in only_stages:
            candidates = candidate_snippets(text)
            attempts = 0
            exit_code: int | None = 0
            timed_out = False
            duration_ms = 0.0
            stdout_bytes = 0
            stderr = ""
            output_pdf = tmp / "source_operator_edited.pdf"
            report_pdf = tmp / "source_operator_apply.json"
            applied = False
            replacement = same_length_replacement(snippet)
            for candidate in candidates:
                attempts += 1
                replacement = same_length_replacement(candidate)
                output_pdf = tmp / f"source_operator_edited_{attempts}.pdf"
                report_pdf = tmp / f"source_operator_apply_{attempts}.json"
                exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                    [
                        str(binary),
                        "edit-text-operator",
                        str(pdf),
                        "--page",
                        "1",
                        "--source-text",
                        candidate,
                        "--replacement-text",
                        replacement,
                        "--output",
                        str(output_pdf),
                        "--report",
                        str(report_pdf),
                    ],
                    timeout,
                )
                if exit_code == 0 and output_pdf.exists() and output_pdf.stat().st_size > 0:
                    snippet = candidate
                    applied = True
                    break
                if timed_out:
                    break
            rows.append(
                compact_row(
                    corpus,
                    pdf,
                    "source_operator_apply",
                    exit_code,
                    timed_out,
                    duration_ms,
                    stdout_bytes,
                    stderr,
                    {
                        "snippet_len": len(snippet),
                        "replacement_len": len(replacement),
                        "attempts": attempts,
                        "applied_output": applied,
                        "output_bytes": output_pdf.stat().st_size
                        if output_pdf.exists()
                        else 0,
                        "report_bytes": report_pdf.stat().st_size
                        if report_pdf.exists()
                        else 0,
                    },
                )
            )

        apply_needed = only_stages is None or "reflow_region_apply" in only_stages
        validate_needed = only_stages is None or "reflow_region_validate" in only_stages
        undo_needed = only_stages is None or "reflow_region_undo" in only_stages
        if apply_needed or validate_needed or undo_needed:
            candidates = candidate_snippets(text)
            request_path = tmp / "reflow_region_request.json"
            edited_pdf = tmp / "reflow_region_edited.pdf"
            apply_report = tmp / "reflow_region_apply.json"
            restored_pdf = tmp / "reflow_region_restored.pdf"
            undo_report = tmp / "reflow_region_undo.json"
            replacement = same_length_replacement(snippet)
            write_reflow_request(request_path, snippet, replacement)

            apply_row: dict[str, Any] | None = None
            applied = False
            if apply_needed or validate_needed or undo_needed:
                attempts = 0
                exit_code: int | None = 0
                timed_out = False
                duration_ms = 0.0
                stdout_bytes = 0
                stderr = ""
                for candidate in candidates:
                    attempts += 1
                    replacement = same_length_replacement(candidate)
                    request_path = tmp / f"reflow_region_request_{attempts}.json"
                    edited_pdf = tmp / f"reflow_region_edited_{attempts}.pdf"
                    apply_report = tmp / f"reflow_region_apply_{attempts}.json"
                    write_reflow_request(request_path, candidate, replacement)
                    exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                        [
                            str(binary),
                            "reflow-region",
                            str(pdf),
                            "--request",
                            str(request_path),
                            "--output",
                            str(edited_pdf),
                            "--report",
                            str(apply_report),
                        ],
                        timeout,
                    )
                    if exit_code == 0 and edited_pdf.exists() and edited_pdf.stat().st_size > 0:
                        snippet = candidate
                        applied = True
                        break
                    if timed_out:
                        break
                apply_row = compact_row(
                    corpus,
                    pdf,
                    "reflow_region_apply",
                    exit_code,
                    timed_out,
                    duration_ms,
                    stdout_bytes,
                    stderr,
                    {
                        "snippet_len": len(snippet),
                        "replacement_len": len(replacement),
                        "attempts": attempts,
                        "applied_output": applied,
                        "output_bytes": edited_pdf.stat().st_size if edited_pdf.exists() else 0,
                        "report_bytes": apply_report.stat().st_size
                        if apply_report.exists()
                        else 0,
                    },
                )
                if apply_needed:
                    rows.append(apply_row)

            if validate_needed:
                if applied and edited_pdf.exists():
                    validate_report = tmp / "reflow_region_validate.json"
                    exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                        [
                            str(binary),
                            "reflow-validate",
                            str(pdf),
                            "--output-pdf",
                            str(edited_pdf),
                            "--request",
                            str(request_path),
                            "--output",
                            str(validate_report),
                        ],
                        timeout,
                    )
                    rows.append(
                        compact_row(
                            corpus,
                            pdf,
                            "reflow_region_validate",
                            exit_code,
                            timed_out,
                            duration_ms,
                            stdout_bytes,
                            stderr,
                            {
                                "output_bytes": validate_report.stat().st_size
                                if validate_report.exists()
                                else 0,
                            },
                        )
                    )
                else:
                    rows.append(
                        compact_row(
                            corpus,
                            pdf,
                            "reflow_region_validate",
                            0,
                            False,
                            0.0,
                            0,
                            "",
                            {"skipped": True, "skip_reason": "apply_not_successful"},
                        )
                    )

            if undo_needed:
                if applied and edited_pdf.exists():
                    exit_code, timed_out, duration_ms, stdout_bytes, stderr = run_cmd(
                        [
                            str(binary),
                            "reflow-undo",
                            str(pdf),
                            "--output-pdf",
                            str(edited_pdf),
                            "--request",
                            str(request_path),
                            "--restored-pdf",
                            str(restored_pdf),
                            "--report",
                            str(undo_report),
                        ],
                        timeout,
                    )
                    rows.append(
                        compact_row(
                            corpus,
                            pdf,
                            "reflow_region_undo",
                            exit_code,
                            timed_out,
                            duration_ms,
                            stdout_bytes,
                            stderr,
                            {
                                "restored_bytes": restored_pdf.stat().st_size
                                if restored_pdf.exists()
                                else 0,
                                "report_bytes": undo_report.stat().st_size
                                if undo_report.exists()
                                else 0,
                            },
                        )
                    )
                else:
                    rows.append(
                        compact_row(
                            corpus,
                            pdf,
                            "reflow_region_undo",
                            0,
                            False,
                            0.0,
                            0,
                            "",
                            {"skipped": True, "skip_reason": "apply_not_successful"},
                        )
                    )
    return rows


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    idx = min(len(ordered) - 1, max(0, int(round((pct / 100.0) * (len(ordered) - 1)))))
    return round(ordered[idx], 3)


def write_summary(jsonl: Path, summary: Path) -> None:
    raw_rows = [json.loads(line) for line in jsonl.read_text().splitlines() if line.strip()]
    latest: dict[tuple[str, str], dict[str, Any]] = {}
    for row in raw_rows:
        latest[(row["pdf"], row["stage"])] = row
    rows = list(latest.values())
    by_stage: dict[str, dict[str, Any]] = {}
    for stage in STAGES:
        stage_rows = [row for row in rows if row["stage"] == stage]
        durations = [row["duration_ms"] for row in stage_rows]
        failures = [
            row
            for row in stage_rows
            if not row_passed(row)
        ]
        errors: dict[str, int] = {}
        for row in stage_rows:
            errors[row["error_class"]] = errors.get(row["error_class"], 0) + 1
        by_stage[stage] = {
            "files": len(stage_rows),
            "successes": len(stage_rows) - len(failures),
            "failures": len(failures),
            "timeouts": sum(1 for row in stage_rows if row.get("timed_out")),
            "skipped": sum(1 for row in stage_rows if row.get("skipped")),
            "applied_outputs": sum(1 for row in stage_rows if row.get("applied_output")),
            "median_ms": percentile(durations, 50),
            "p95_ms": percentile(durations, 95),
            "p99_ms": percentile(durations, 99),
            "error_classes": errors,
        }
    summary.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "kind": "wellfriend_editing_corpus_smoke",
                "total_rows": len(raw_rows),
                "latest_rows": len(rows),
                "pdf_files": len({row["pdf"] for row in rows}),
                "stages": STAGES,
                "by_stage": by_stage,
            },
            indent=2,
            sort_keys=True,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--result-dir", type=Path, required=True)
    parser.add_argument("--jsonl", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--timeout-sec", type=float, default=120.0)
    parser.add_argument("--max-files", type=int)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument(
        "--stages",
        nargs="+",
        choices=STAGES,
        help="Optional subset of stages to run; dependencies needed for snippets are still executed internally.",
    )
    args = parser.parse_args()
    selected_stages = set(args.stages) if args.stages else None
    required_stages = selected_stages or set(STAGES)

    args.result_dir.mkdir(parents=True, exist_ok=True)
    pdfs = iter_pdfs(args.corpus)
    if args.max_files:
        pdfs = pdfs[: args.max_files]
    if args.resume and args.jsonl.exists():
        rows = [json.loads(line) for line in args.jsonl.read_text().splitlines() if line.strip()]
        latest: dict[tuple[str, str], dict[str, Any]] = {}
        for row in rows:
            latest[(row["pdf"], row["stage"])] = row
        stages_by_pdf: dict[str, dict[str, dict[str, Any]]] = {}
        for (pdf, stage), row in latest.items():
            stages_by_pdf.setdefault(pdf, {})[stage] = row
        needed_by_pdf: dict[str, set[str] | None] = {}
        complete = {
            pdf
            for pdf, stages in stages_by_pdf.items()
            if all(row_passed(stages.get(stage)) for stage in required_stages)
        }
        for pdf in pdfs:
            rel = str(pdf.relative_to(args.corpus))
            if rel in complete:
                continue
            stages = stages_by_pdf.get(rel)
            if not stages:
                needed_by_pdf[rel] = selected_stages
            else:
                needed_by_pdf[rel] = {
                    stage for stage in required_stages if not row_passed(stages.get(stage))
                }
        pdfs = [pdf for pdf in pdfs if str(pdf.relative_to(args.corpus)) not in complete]
    elif args.jsonl.exists():
        args.jsonl.unlink()
        needed_by_pdf = {}
    else:
        needed_by_pdf = {}

    with args.jsonl.open("a", encoding="utf-8") as out:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
            futures = [
                pool.submit(
                    run_pdf,
                    args.corpus,
                    args.binary,
                    args.result_dir,
                    pdf,
                    args.timeout_sec,
                    needed_by_pdf.get(str(pdf.relative_to(args.corpus)), selected_stages),
                )
                for pdf in pdfs
            ]
            for future in concurrent.futures.as_completed(futures):
                for row in future.result():
                    out.write(json.dumps(row, sort_keys=True) + "\n")
                out.flush()
    write_summary(args.jsonl, args.summary)
    summary = json.loads(args.summary.read_text())
    return 0 if all(item["failures"] == 0 for item in summary["by_stage"].values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
