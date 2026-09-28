#!/usr/bin/env python3
"""Exercise revision-bound text editing against a public PDF corpus.

The harness deliberately uses the SDK for source-text discovery and reopen
verification.  qpdf and Poppler are independent post-write checks; qpdf is a
structural validator, not a renderer.  Originals are never modified.
"""

from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
from typing import Any


WORD = re.compile(r"[^\W\d_]{2,32}", re.UNICODE)
SKIP_WORDS = {
    "about", "copyright", "document", "information", "license", "table",
    "figure", "section", "using", "which", "their", "these", "where",
}


def run(argv: list[str], timeout: float) -> dict[str, Any]:
    started = time.monotonic()
    try:
        proc = subprocess.run(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            timeout=timeout,
        )
        return {
            "exit": proc.returncode,
            "timed_out": False,
            "duration_ms": round((time.monotonic() - started) * 1000, 3),
            "stdout": proc.stdout,
            "stderr": proc.stderr,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "exit": None,
            "timed_out": True,
            "duration_ms": round((time.monotonic() - started) * 1000, 3),
            "stdout": _expired_text(exc.stdout),
            "stderr": _expired_text(exc.stderr),
        }


def _expired_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    return value.decode(errors="replace") if isinstance(value, bytes) else value


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def compact_command(result: dict[str, Any]) -> dict[str, Any]:
    stderr = result["stderr"]
    return {
        "exit": result["exit"],
        "timed_out": result["timed_out"],
        "duration_ms": result["duration_ms"],
        "stderr_tail": stderr[-2000:],
    }


def normalized_qpdf_diagnostics(result: dict[str, Any], pdf: Path) -> Counter[str]:
    """Return path-independent qpdf diagnostics for baseline comparison."""
    combined = f"{result['stdout']}\n{result['stderr']}"
    path_forms = {str(pdf), str(pdf.resolve())}
    diagnostics: list[str] = []
    for raw in combined.splitlines():
        line = raw.strip()
        if not line:
            continue
        if not (
            line.startswith("WARNING:")
            or line.startswith("ERROR:")
            or line.startswith("qpdf:")
        ):
            continue
        for path in path_forms:
            line = line.replace(path, "<PDF>")
        diagnostics.append(line)
    return Counter(diagnostics)


def qpdf_regression_report(
    original: dict[str, Any],
    edited: dict[str, Any],
    original_pdf: Path,
    edited_pdf: Path,
) -> dict[str, Any]:
    before = normalized_qpdf_diagnostics(original, original_pdf)
    after = normalized_qpdf_diagnostics(edited, edited_pdf)
    introduced = list((after - before).elements())
    exit_regression_free = edited["exit"] == 0 or (
        original["exit"] in {2, 3} and edited["exit"] == original["exit"]
    )
    structurally_clean = edited["exit"] == 0
    return {
        "original_exit": original["exit"],
        "edited_exit": edited["exit"],
        "original_diagnostics": list(before.elements()),
        "edited_diagnostics": list(after.elements()),
        "introduced_diagnostics": introduced,
        "structural_regression_free": structurally_clean
        or (exit_regression_free and not introduced),
    }


def source_aware_apply_proof(apply_report: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
    evidence = apply_report.get("operation_report", {}).get("validation_evidence", {})
    proof = evidence.get("unaffected_content_proof", {}) if isinstance(evidence, dict) else {}
    if not isinstance(proof, dict):
        return False, {"status": "missing_or_invalid"}
    logical = proof.get("source_aware_logical_range_proof")
    if isinstance(logical, dict):
        valid = (
            proof.get("status") == "pass_with_documented_layout_whitespace_policy"
            and logical.get("selected_source_matches") is True
            and logical.get("logical_text_exact_under_layout_whitespace_policy") is True
        )
        return valid, {
            "kind": "source_aware_logical_range",
            "status": proof.get("status"),
            "range": logical.get("range"),
            "selected_source_matches": logical.get("selected_source_matches"),
            "logical_text_exact_under_layout_whitespace_policy": logical.get(
                "logical_text_exact_under_layout_whitespace_policy"
            ),
            "expected_sha256": logical.get("expected_sha256"),
            "actual_sha256": logical.get("actual_sha256"),
        }
    valid = (
        proof.get("overlay_used") is False
        and proof.get("replacement_extracts") is True
        and proof.get("old_text_absent") is True
    )
    return valid, {
        "kind": "operator_source_rewrite",
        "overlay_used": proof.get("overlay_used"),
        "replacement_extracts": proof.get("replacement_extracts"),
        "old_text_absent": proof.get("old_text_absent"),
    }


def sdk_extract(binary: Path, pdf: Path, destination: Path, timeout: float) -> tuple[dict[str, Any], str]:
    result = run(
        [str(binary), "extract-text", "--pages", "1", "-o", str(destination), str(pdf)],
        timeout,
    )
    text = destination.read_text(encoding="utf-8", errors="replace") if destination.exists() else ""
    return result, text


def candidate_words(text: str, independent_text: str) -> list[str]:
    words = WORD.findall(text)
    folded_counts = Counter(word.casefold() for word in words)
    independent_counts = Counter(word.casefold() for word in WORD.findall(independent_text))
    candidates = [
        word
        for word in words
        if folded_counts[word.casefold()] == 1
        and independent_counts[word.casefold()] == 1
        and word.casefold() not in SKIP_WORDS
    ]
    # A target must be visible to both the SDK and an independent extractor.
    # Trying several unique words avoids mistaking an extractor-only logical
    # fragment for a general document editing failure.
    return sorted(candidates, key=lambda item: (-len(item), item.casefold()))


def replacement_for(source: str) -> str:
    alphabet = "Verification"
    replacement = "".join(alphabet[index % len(alphabet)] for index in range(len(source)))
    if replacement.casefold() == source.casefold():
        replacement = "".join("Z" if char != "Z" else "Y" for char in source)
    return replacement


def exact_candidate(plan: dict[str, Any]) -> dict[str, Any] | None:
    candidates = [candidate for candidate in plan.get("candidates", []) if candidate.get("exact")]
    if not candidates:
        return None
    candidates.sort(
        key=lambda candidate: (
            -float(candidate.get("confidence", 0.0)),
            bool(candidate.get("shared_resource")),
            str(candidate.get("candidate_id", "")),
        )
    )
    return candidates[0]


def approved_font_for(plan: dict[str, Any], candidate_id: str) -> str | None:
    report = plan.get("implementation_report", {})
    fonts = report.get("operation", {}).get("font_substitution", {})
    candidate_fonts = fonts.get("by_candidate", {}).get(candidate_id, {})
    approved = candidate_fonts.get("approved_candidates", [])
    return str(approved[0]) if approved else None


def json_from(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def classify_plan_state(state: str) -> str:
    return {
        "policy_denied": "typed_policy_denial",
        "target_not_found": "typed_target_not_found",
        "irrecoverable_input": "typed_irrecoverable_input",
    }.get(state, f"unexpected_plan_state:{state}")


def exercise_one(
    binary: Path,
    corpus: Path,
    pdf: Path,
    result_root: Path,
    index: int,
    timeout: float,
) -> dict[str, Any]:
    started = time.monotonic()
    input_sha256 = digest_file(pdf)
    item = result_root / "files" / f"{index:03d}-{input_sha256[:16]}"
    item.mkdir(parents=True, exist_ok=True)
    row: dict[str, Any] = {
        "index": index,
        "relative_path": str(pdf.relative_to(corpus)),
        "input_bytes": pdf.stat().st_size,
        "input_sha256": input_sha256,
    }

    extract_result, before_text = sdk_extract(binary, pdf, item / "before.txt", timeout)
    row["extract_before"] = compact_command(extract_result)
    if extract_result["exit"] != 0:
        row["status"] = "extract_before_timeout" if extract_result["timed_out"] else "extract_before_failed"
        return finish(row, started)

    poppler_before_result = run(
        ["pdftotext", "-f", "1", "-l", "1", str(pdf), "-"], min(timeout, 60)
    )
    row["poppler_extract_before"] = compact_command(poppler_before_result)
    if poppler_before_result["exit"] != 0:
        row["status"] = "poppler_extract_before_failed"
        return finish(row, started)
    sources = candidate_words(before_text, poppler_before_result["stdout"])
    if not sources:
        row["status"] = "no_common_visible_unique_word"
        return finish(row, started)

    plan: dict[str, Any] | None = None
    plan_path: Path | None = None
    plan_result: dict[str, Any] | None = None
    candidate: dict[str, Any] | None = None
    attempts: list[dict[str, Any]] = []
    source = ""
    replacement = ""
    for attempt_number, proposed_source in enumerate(sources[:12], start=1):
        proposed_replacement = replacement_for(proposed_source)
        request = {
            "operation": {
                "kind": "text",
                "request": {
                    "requested_mode": "operator_preserving",
                    "page": 1,
                    "source_text": proposed_source,
                    "replacement_text": proposed_replacement,
                },
            },
            "policy": {
                "mutation_mode": "authorized_rewrite",
                "ambiguity": "preview_and_confirm",
                "allow_font_substitution": True,
                "allow_deterministic_repair": True,
                "require_conformance_preservation": False,
            },
        }
        request_path = item / f"request-attempt-{attempt_number:02d}.json"
        proposed_plan_path = item / f"plan-attempt-{attempt_number:02d}.json"
        request_path.write_text(json.dumps(request, indent=2), encoding="utf-8")
        proposed_result = run(
            [
                str(binary), "universal-edit-plan", str(pdf),
                "--request", str(request_path), "-o", str(proposed_plan_path),
            ],
            timeout,
        )
        attempt: dict[str, Any] = {
            "attempt": attempt_number,
            "source_text": proposed_source,
            "replacement_text": proposed_replacement,
            "plan": compact_command(proposed_result),
        }
        if proposed_result["exit"] != 0 or not proposed_plan_path.exists():
            attempt["state"] = "plan_timeout" if proposed_result["timed_out"] else "plan_failed"
            attempts.append(attempt)
            continue
        try:
            proposed_plan = json_from(proposed_plan_path)
        except (OSError, json.JSONDecodeError) as exc:
            attempt.update(state="invalid_plan_json", error=type(exc).__name__)
            attempts.append(attempt)
            continue
        proposed_state = str(proposed_plan.get("state", "missing"))
        proposed_candidate = exact_candidate(proposed_plan)
        attempt.update(
            state=proposed_state,
            approval_reasons=proposed_plan.get("approval_reasons", []),
            exact_candidate_available=proposed_candidate is not None,
        )
        attempts.append(attempt)
        if proposed_state not in {"ready", "approval_required"}:
            continue
        if proposed_state == "approval_required" and proposed_candidate is None:
            continue
        source = proposed_source
        replacement = proposed_replacement
        plan = proposed_plan
        plan_path = proposed_plan_path
        plan_result = proposed_result
        candidate = proposed_candidate
        break

    row["target_attempts"] = attempts
    if plan is None or plan_path is None or plan_result is None:
        row["status"] = "no_provenance_complete_visible_target"
        return finish(row, started)
    row.update(source_text=source, replacement_text=replacement)
    row["plan"] = compact_command(plan_result)
    state = str(plan.get("state", "missing"))
    row["plan_state"] = state
    row["approval_reasons"] = plan.get("approval_reasons", [])
    approval_path: Path | None = None
    if state == "approval_required":
        if candidate is None:
            row["status"] = "approval_required_without_exact_candidate"
            return finish(row, started)
        candidate_id = str(candidate["candidate_id"])
        approved_font = approved_font_for(plan, candidate_id)
        decision = {
            "selected_candidate_ids": [candidate_id],
            "approved_font": approved_font,
            "mutation_mode": "authorized_rewrite",
            "accept_visual_change": True,
            "accept_signature_invalidation": True,
        }
        decision_path = item / "decision.json"
        approval_path = item / "approval.json"
        decision_path.write_text(json.dumps(decision, indent=2), encoding="utf-8")
        approve_result = run(
            [str(binary), "universal-edit-approve", "--plan", str(plan_path), "--decision", str(decision_path), "-o", str(approval_path)],
            timeout,
        )
        row["approval"] = compact_command(approve_result)
        row["selected_candidate_id"] = candidate_id
        row["approved_font"] = approved_font
        if approve_result["exit"] != 0 or not approval_path.exists():
            row["status"] = "approval_timeout" if approve_result["timed_out"] else "approval_failed"
            return finish(row, started)

    output_pdf = item / "edited.pdf"
    apply_report_path = item / "apply-report.json"
    apply_argv = [
        str(binary), "universal-edit-apply", str(pdf), "--plan", str(plan_path),
        "-o", str(output_pdf), "--report", str(apply_report_path),
    ]
    if approval_path is not None:
        apply_argv.extend(["--approval", str(approval_path)])
    apply_result = run(apply_argv, timeout)
    row["apply"] = compact_command(apply_result)
    if apply_result["exit"] != 0 or not output_pdf.exists():
        row["status"] = "apply_timeout" if apply_result["timed_out"] else "apply_failed"
        return finish(row, started)

    row["output_bytes"] = output_pdf.stat().st_size
    row["output_sha256"] = digest_file(output_pdf)
    apply_report: dict[str, Any] = {}
    try:
        apply_report = json_from(apply_report_path)
        row["apply_outcome"] = apply_report.get("outcome")
        row["changed"] = apply_report.get("changed")
        row["affected_pages"] = apply_report.get("affected_pages", [])
    except (OSError, json.JSONDecodeError) as exc:
        row["apply_report_error"] = type(exc).__name__

    if row.get("apply_outcome") != "applied" or row.get("changed") is not True:
        unchanged = row["output_sha256"] == row["input_sha256"]
        row["no_change_proof"] = {
            "input_output_sha256_equal": unchanged,
            "report_changed_false": row.get("changed") is False,
        }
        if unchanged and row.get("changed") is False:
            outcome = str(row.get("apply_outcome", "missing")).lower()
            row["status"] = f"typed_apply_{outcome}"
        else:
            row["status"] = "apply_outcome_contract_failed"
        return finish(row, started)

    qpdf_original_result = run(["qpdf", "--check", str(pdf)], min(timeout, 60))
    qpdf_result = run(["qpdf", "--check", str(output_pdf)], min(timeout, 60))
    row["qpdf_original"] = compact_command(qpdf_original_result)
    row["qpdf"] = compact_command(qpdf_result)
    row["qpdf_comparison"] = qpdf_regression_report(
        qpdf_original_result, qpdf_result, pdf, output_pdf
    )
    after_result, after_text = sdk_extract(binary, output_pdf, item / "after.txt", timeout)
    row["extract_after"] = compact_command(after_result)
    poppler_result = run(["pdftotext", "-f", "1", "-l", "1", str(output_pdf), "-"], min(timeout, 60))
    row["poppler_extract"] = compact_command(poppler_result)

    before_source_count = before_text.count(source)
    after_source_count = after_text.count(source)
    after_replacement_count = after_text.count(replacement)
    poppler_replacement_count = poppler_result["stdout"].count(replacement)
    proof_valid, proof_summary = source_aware_apply_proof(apply_report)
    row["text_postconditions"] = {
        "before_source_count": before_source_count,
        "after_source_count": after_source_count,
        "after_replacement_count": after_replacement_count,
        "poppler_replacement_count": poppler_replacement_count,
        "flat_extraction_source_count_decreased": after_source_count == before_source_count - 1,
        "external_replacement_observed": (
            after_result["exit"] == 0
            and after_replacement_count >= 1
            and poppler_result["exit"] == 0
            and poppler_replacement_count >= 1
        ),
        "source_aware_apply_proof": proof_summary,
        "sdk_selected_occurrence_replaced": proof_valid,
    }
    verified = (
        row["qpdf_comparison"]["structural_regression_free"]
        and after_result["exit"] == 0
        and row["text_postconditions"]["external_replacement_observed"]
        and row["text_postconditions"]["sdk_selected_occurrence_replaced"]
    )
    row["status"] = "applied_verified" if verified else "applied_verification_failed"
    return finish(row, started)


def finish(row: dict[str, Any], started: float) -> dict[str, Any]:
    row["elapsed_ms"] = round((time.monotonic() - started) * 1000, 3)
    return row


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--timeout-sec", type=float, default=180.0)
    parser.add_argument("--workers", type=int, default=1)
    args = parser.parse_args()

    pdfs = sorted(path for path in args.corpus.rglob("*.pdf") if path.is_file())[: args.limit]
    args.output.mkdir(parents=True, exist_ok=True)
    results_path = args.output / "results.jsonl"
    ordered_rows: list[dict[str, Any] | None] = [None] * len(pdfs)
    with results_path.open("w", encoding="utf-8") as stream:
        with ThreadPoolExecutor(max_workers=max(1, args.workers)) as executor:
            futures = {
                executor.submit(
                    exercise_one,
                    args.binary,
                    args.corpus,
                    pdf,
                    args.output,
                    index,
                    args.timeout_sec,
                ): index
                for index, pdf in enumerate(pdfs, start=1)
            }
            completed = 0
            for future in as_completed(futures):
                index = futures[future]
                row = future.result()
                ordered_rows[index - 1] = row
                completed += 1
                stream.write(json.dumps(row, sort_keys=True) + "\n")
                stream.flush()
                print(
                    f"[{completed}/{len(pdfs)}] {row['status']} {row['relative_path']}",
                    flush=True,
                )

    rows = [row for row in ordered_rows if row is not None]

    counts = Counter(str(row["status"]) for row in rows)
    summary = {
        "schema_version": "wellfriend.universal_editing.corpus_qualification.v1",
        "corpus": str(args.corpus),
        "files_discovered": len(pdfs),
        "limit": args.limit,
        "workers": max(1, args.workers),
        "binary": str(args.binary),
        "status_counts": dict(sorted(counts.items())),
        "applied": sum(1 for row in rows if str(row["status"]).startswith("applied_")),
        "applied_verified": counts.get("applied_verified", 0),
        "typed_refusals": sum(value for key, value in counts.items() if key.startswith("typed_")),
        "timeouts": sum(value for key, value in counts.items() if "timeout" in key),
        "input_manifest_sha256": hashlib.sha256(
            "\n".join(f"{row['input_sha256']}  {row['relative_path']}" for row in rows).encode()
        ).hexdigest(),
    }
    (args.output / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if counts.get("applied_verification_failed", 0) == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
