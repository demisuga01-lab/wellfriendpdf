#!/usr/bin/env python3
"""Smoke the installed Prompt 15 Python wheel and write parity evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import wellfriendpdf


def envelope(value: object, kind: str) -> dict:
    if not isinstance(value, dict):
        raise AssertionError(f"{kind} did not return a dictionary")
    if value.get("schema_version") != 1 or value.get("kind") != kind:
        raise AssertionError(f"invalid {kind} envelope: {value}")
    report = value.get("report")
    if not isinstance(report, dict):
        raise AssertionError(f"{kind} report payload is not an object")
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("fixture", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    feature = envelope(wellfriendpdf.feature_report(), "feature_report")
    prompt15 = feature["prompt15_semantic_binding_rag_benchmark_closeout"]
    if prompt15["closure_gates"]["blocked_count"] != 0:
        raise AssertionError("Prompt 15 feature report has blocked rows")

    document = wellfriendpdf.open(args.fixture)
    semantic = envelope(document.semantic_bundle(), "semantic_binding_report")
    chunks = envelope(document.advanced_chunks(), "advanced_rag_chunk_set")
    search = envelope(document.semantic_search("Hello"), "semantic_search_report")
    table_status = envelope(document.table_proposal_status(), "table_proposal_status")
    prompt20b = envelope(document.prompt20b_report(), "prompt20b_report")
    range_model = envelope(
        document.prompt20b_text_range_analyze(1),
        "prompt20b_multi_run_range_model",
    )
    first_span = range_model["source_spans"][0]
    range_bytes, range_report = document.edit_text_range(
        json.dumps(
            {
                "page": 1,
                "logical_start": first_span["logical_range"][0],
                "logical_end": first_span["logical_range"][1],
                "replacement_text": "PyWheel20B",
                "mode": "paragraph_reflow_horizontal",
                "style_policy": "inherit_leading",
                "options": {
                    "region": [20.0, 80.0, 180.0, 140.0],
                    "font_size": 12.0,
                    "line_spacing": 1.2,
                    "max_lines_or_columns": 4096,
                    "overflow_policy": "error",
                    "signature_policy_override": False,
                    "deterministic": True,
                },
            }
        )
    )
    prompt20b_edit = envelope(range_report, "prompt20b_multi_run_text_edit_report")

    if semantic["schema_version"] != "prompt15.semantic_binding.v1":
        raise AssertionError("unexpected semantic binding schema")
    if chunks["schema_version"] != "prompt15.rag_chunk.v1":
        raise AssertionError("unexpected RAG chunk schema")
    if not search["provenance_preserved"]:
        raise AssertionError("semantic search dropped provenance")
    if table_status["model_weights_bundled"]:
        raise AssertionError("table status incorrectly claims bundled weights")
    if prompt20b["schema_version"] != "prompt20b.multirun-form-appearance-closure.v1":
        raise AssertionError("unexpected Prompt 20B report schema")
    if range_model["schema_version"] != "prompt20b.multirun-form-appearance-closure.v1":
        raise AssertionError("unexpected Prompt 20B range schema")
    if not bytes(range_bytes).startswith(b"%PDF-"):
        raise AssertionError("Prompt 20B edit did not return PDF bytes")
    if not prompt20b_edit["replacement_extracts"] or not prompt20b_edit["old_selected_text_absent"]:
        raise AssertionError("Prompt 20B range edit did not prove replacement/old-text absence")

    payload = {
        "schema_version": "prompt15.python_wheel_smoke.v1",
        "status": "passed",
        "module_path": str(Path(wellfriendpdf.__file__).resolve()),
        "module_version": wellfriendpdf.__version__,
        "fixture": str(args.fixture.resolve()),
        "fixture_sha256": "sha256:" + hashlib.sha256(args.fixture.read_bytes()).hexdigest(),
        "semantic_schema": semantic["schema_version"],
        "semantic_pages": semantic["summary"]["page_count"],
        "rag_schema": chunks["schema_version"],
        "rag_chunk_count": len(chunks["chunks"]),
        "search_match_count": len(search["semantic_matches"]),
        "table_backend_status": table_status["local_backend_status"],
        "prompt15_blocked_count": prompt15["closure_gates"]["blocked_count"],
        "prompt20b_schema": prompt20b["schema_version"],
        "prompt20b_range_spans": len(range_model["source_spans"]),
        "prompt20b_range_edit_bytes": len(range_bytes),
        "prompt20b_range_edit_sha256": "sha256:" + hashlib.sha256(bytes(range_bytes)).hexdigest(),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(payload, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
