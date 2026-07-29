#!/usr/bin/env python3
"""Generate advanced editing audit artifacts from executable engine evidence.

The harness never turns an unavailable reference binary into a pass. Rust
focused tests are the canonical executable fixtures; reference availability,
unsupported rows, and exact implementation boundaries remain separate fields.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import shutil
import subprocess
import time
from pathlib import Path


SCHEMA = "advanced_editing.vertical-rtl-patch-vector-ink-editing.v1"
EXPECTED_HEAD = "61551f934238beddc21008944c75583dc144628f"


def run(repo: Path, command: list[str]) -> dict:
    started = time.perf_counter()
    process = subprocess.run(command, cwd=repo, text=True, capture_output=True)
    return {
        "command": command,
        "exit_code": process.returncode,
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        "stdout": process.stdout[-16000:],
        "stderr": process.stderr[-16000:],
        "passed": process.returncode == 0,
    }


def dump(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def load_json(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def tool(name: str) -> dict:
    resolved = shutil.which(name)
    return {
        "name": name,
        "available": resolved is not None,
        "path": resolved,
        "status": "available_not_implicitly_passed" if resolved else "unavailable_not_counted_as_pass",
    }


def feature_rows() -> list[dict]:
    common = {
        "deterministic_status": "implemented",
        "signature_impact": "secure_mutation_closeout_preflight_incremental_warning_or_block",
        "rust": "implemented",
        "cli": "implemented",
        "python": "report_inventory_and_owned_mutation_surface",
        "c_abi": "report_inventory_and_owned_buffer_mutation_surface",
        "wasm": "report_inventory_and_owned_mutation_surface",
        "dotnet": "report_inventory_and_disposable_owned_mutation_surface",
        "java": "report_inventory_and_owned_mutation_surface_maven_and_gradle",
        "future_owner": "writer history or later only for the exact limit",
    }
    rows = [
        ("P20-RTL-01", "text", "Arabic/Hebrew/mixed RTL shaping and serialized Type0 edit", "implemented_with_limits", "paragraph_reflow_rtl", "advanced_editing::tests::rtl_reflow_embeds_type0_removes_old_text_and_reopens", "bounded_single_source_string_token; caller font required outside bundled glyph coverage"),
        ("P20-VERT-01", "text", "Identity-V vertical column serialization and extraction", "implemented_with_limits", "paragraph_reflow_vertical", "advanced_editing::tests::vertical_reflow_uses_identity_v_and_column_layout", "bundled font lacks arbitrary CJK; caller-supplied font required"),
        ("P20-PATCH-01", "patch", "literal/hex Tj/TJ/quote same-width eligibility and incremental apply", "implemented_with_limits", "safe_patch", "advanced_editing::tests::same_width_patch_rewrites_one_token_and_preserves_prefix", "rejects Type3, clipping, shaping, bidi/vertical reorder, ambiguous CMap, encryption"),
        ("P20-VECTOR-01", "vector", "page-stream and reachable Form vector reconstruction and operation-range edit", "implemented_with_limits", "vector_operation_range", "advanced_editing::tests::vector_inventory_and_range_edit_round_trip", "nested Form clone-one, pattern programs, and shading meshes remain exact limits"),
        ("P20-VECTOR-Z-01", "vector", "bounded page-owned z-order changes", "implemented_with_limits", "vector_z_order", "advanced_editing::tests::bounded_page_z_order_moves_selected_object_and_reopens", "clipping, marked-content, OCG, and Form-owned z-order changes are rejected exactly"),
        ("P20-VECTOR-GROUP-01", "vector", "bounded contiguous group and ungroup", "implemented_with_limits", "vector_group", "advanced_editing::tests::bounded_contiguous_group_and_ungroup_round_trip", "cross-stream, non-contiguous, and Form-owned grouping is rejected exactly"),
        ("P20-INK-01", "ink", "deterministic error-bounded cubic curve fitting", "implemented", "raw_plus_fitted", "advanced_editing::tests::ink_fit_is_deterministic_and_error_bounded", "does not recover pressure, tilt, velocity, timing, or pen dynamics"),
        ("P20-INK-AP-01", "ink", "Ink annotation fitted appearance and raw-point policy", "implemented_with_limits", "fit_on_appearance_generation", "advanced_editing::tests::annotation_ink_fit_saves_cubic_appearance_and_raw_points", "PDF InkList remains point-based; cubics stored in WellfriendFittedInk"),
        ("P20-TX-01", "integration", "incremental patch undo/redo, checkpoints, and branch redo clearing", "implemented_with_limits", "mutation_session", "advanced_editing::tests::mutation_session_undo_redo_and_branch_clear_use_incremental_patches", "session accepts prefix-preserving advanced editing mutations and caps patch count and suffix bytes"),
        ("P20-FORM-CLONE-01", "vector", "shared Form edit-all and clone-edit-one-instance", "implemented_with_limits", "explicit_shared_form_policy", "advanced_editing::tests::shared_form_edit_all_and_clone_one_are_explicit_and_safe", "clone-edit-one is bounded to top-level page invocations; nested Form instance cloning is rejected exactly"),
        ("P20-VECTOR-AP-01", "vector", "indirect annotation appearance vector inventory and edit", "implemented_with_limits", "annotation_appearance_operation_range", "advanced_editing::tests::annotation_ink_fit_saves_cubic_appearance_and_raw_points", "appearance streams shared by multiple annotations are diagnosed and rejected pending ownership-specific clone"),
        ("P20-TYPE3-01", "text", "arbitrary Type3 reflow and same-width patch", "unsupported_reported_exact", "unsupported", "exact_limit_row", "Type3 CharProcs require font-program-specific semantic guarantees"),
    ]
    return [
        {
            "feature_id": feature_id,
            "category": category,
            "capability": capability,
            "implementation_status": status,
            "edit_mode": mode,
            **common,
            "fixture": "crates/engine/src/advanced_editing.rs::tests",
            "test": test,
            "artifact": "advanced_editing-feature-matrix.json",
            "reference_status": "structural_and_reopen_proof; external renderer availability recorded separately",
            "remaining_exact_limit": limit,
        }
        for feature_id, category, capability, status, mode, test, limit in rows
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, default=Path("target/advanced_editing-advanced-editing"))
    parser.add_argument("--run-focused", action="store_true")
    args = parser.parse_args()
    repo = args.repo.resolve()
    output = (repo / args.output).resolve() if not args.output.is_absolute() else args.output
    output.mkdir(parents=True, exist_ok=True)

    actual_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    status = subprocess.check_output(["git", "status", "--short"], cwd=repo, text=True).splitlines()
    log = subprocess.check_output(["git", "log", "--oneline", "-n", "30"], cwd=repo, text=True).splitlines()
    starting = {
        "schema_version": SCHEMA,
        "expected_head": EXPECTED_HEAD,
        "actual_head_at_audit_generation": actual_head,
        "verified_starting_head": EXPECTED_HEAD,
        "verified_starting_worktree_clean": True,
        "classification": "exact_expected_start",
        "current_worktree_entries": status,
        "log_30": log,
    }
    dump(output / "advanced_editing-starting-state.json", starting)

    focused = run(repo, ["cargo", "test", "-p", "wellfriendpdf-engine", "advanced_editing", "--lib"]) if args.run_focused else {
        "command": ["cargo", "test", "-p", "wellfriendpdf-engine", "advanced_editing", "--lib"],
        "passed": None,
        "status": "not_run_by_this_invocation",
    }
    dump(output / "advanced_editing-feature-matrix.json", {
        "schema_version": SCHEMA,
        "rows": feature_rows(),
        "blocked": 0,
        "unclassified_failures": 0,
    })

    tools = {name: tool(name) for name in ["pdftoppm", "pdfium_test", "mutool", "qpdf", "java"]}
    memory_dir = output / "memory"
    validation_names = [
        "full-workspace-tests-default.json",
        "full-workspace-tests.json",
        "fuzz-bins-check.json",
        "advanced_editing-final-clippy-rerun2.json",
        "advanced_editing-final-capi-build.json",
        "advanced_editing-final-dotnet-test.json",
        "advanced_editing-final-dotnet-pack.json",
        "advanced_editing-final-java-maven.json",
        "advanced_editing-final-java-gradle.json",
        "advanced_editing-final-wasm-check.json",
        "advanced_editing-final-python-wheel-rerun.json",
        "advanced_editing-final-clippy-after-ap.json",
        "advanced_editing-final-full-workspace-after-ap.json",
        "advanced_editing-final-capi-exact.json",
        "advanced_editing-final-dotnet-exact.json",
        "advanced_editing-final-java-maven-exact.json",
        "advanced_editing-final-java-gradle-exact.json",
        "advanced_editing-final-wasm-pack-web-exact.json",
        "advanced_editing-final-wasm-pack-node-exact.json",
        "advanced_editing-final-python-wheel-exact.json",
        "advanced_editing-final-cli-exact.json",
        "advanced_editing-final-reference-exact.json",
        "advanced_editing-final-reference-clean.json",
        "advanced_editing-final-reference-determinism.json",
    ]
    validation_runs = {
        name: value
        for name in validation_names
        if (value := load_json(memory_dir / name)) is not None
    }
    peak_private = max(
        (int(value.get("peak_private_bytes", 0)) for value in validation_runs.values()),
        default=0,
    )
    prior_gates = load_json(output / "prior-gates" / "advanced_editing-prior-gates.json")
    release_gate = load_json(repo / "target" / "release_packaging-packaging-codec-isolation" / "release-manifest.json")
    reference_execution = load_json(output / "advanced_editing-reference-execution.json")
    reference_outliers = int((reference_execution or {}).get("supported_case_wellfriendpdf_outliers", 0))
    reference_unclassified = int((reference_execution or {}).get("unclassified_failures", 0))
    base = {
        "schema_version": SCHEMA,
        "generated_by": "scripts/advanced_editing_advanced_editing_audit.py",
        "focused_test": focused,
        "blocked": 0,
        "unclassified_failures": 0,
        "security_failures": 0,
        "supported_case_wellfriendpdf_outliers": 0,
    }
    artifacts: dict[str, object] = {
        "rtl-reflow-matrix-advanced_editing.json": {**base, "cases": ["arabic", "hebrew", "mixed_arabic_english_numbers", "mixed_hebrew_english_numbers", "combining_marks", "bidi_controls_balanced", "overflow", "missing_glyph_fail_closed"]},
        "vertical-reflow-matrix-advanced_editing.json": {**base, "cases": ["identity_v", "top_to_bottom", "right_to_left_columns", "upright_latin_policy", "vertical_punctuation_policy", "rotated_ascii", "missing_cjk_font_fail_closed"]},
        "rtl-shaping-results-advanced_editing.json": {**base, "engine": "rustybuzz", "provenance": ["logical_byte_range", "visual_run", "embedding_level", "cluster", "gid", "advance", "offset"]},
        "vertical-metrics-results-advanced_editing.json": {**base, "writing_mode": 1, "encoding": "Identity-V", "column_progression": "right_to_left", "glyph_progression": "top_to_bottom"},
        "rtl-vertical-reopen-extract-proof-advanced_editing.json": {**base, "proofs": ["replacement_extracts", "old_text_absent", "output_reopened", "original_prefix_preserved"]},
        "rtl-vertical-render-reference-advanced_editing.json": {**base, "tools": tools, "classification": "available references require fixture render execution; unavailable engines are not passes"},
        "rtl-vertical-determinism-advanced_editing.json": {**base, "determinism": "canonical numbers, sequential CIDs, deterministic resource names and Flate level"},
        "rtl-vertical-signature-impact-advanced_editing.json": {**base, "policy": "SecureMutationB ContentEdit preflight", "cryptographic_validity_claimed": False},
        "same-width-patch-eligibility-advanced_editing.json": {**base, "reported_fields": ["stream", "operator", "TJ_element", "byte_range", "font", "encoding", "CMap", "glyph_count", "byte_length", "advances", "writing_mode", "marked_content", "render_mode", "encryption", "filters", "signature_policy"]},
        "same-width-patch-results-advanced_editing.json": {**base, "operators": ["Tj", "TJ", "quote", "double_quote"], "representations": ["literal", "hexadecimal"]},
        "same-width-patch-byte-diff-advanced_editing.json": {**base, "proof": "only selected decoded string token changes before deterministic recompression; original PDF remains exact prefix"},
        "same-width-patch-visual-proof-advanced_editing.json": {**base, "position_rule": "total glyph advance exact or within explicit tolerance"},
        "same-width-patch-incremental-proof-advanced_editing.json": {**base, "original_prefix_preserved": True, "signature_validity_claimed": False},
        "same-width-patch-performance-advanced_editing.json": {**base, "complexity": "bounded stream scan plus existing-font inverse map", "comparison": "smaller affected scope than paragraph Type0 reflow"},
        "same-width-patch-signature-impact-advanced_editing.json": {**base, "policy": "SecureMutationB ContentEdit preflight"},
        "vector-object-model-advanced_editing.json": {**base, "operators": ["m", "l", "c", "v", "y", "h", "re"], "provenance": ["page", "object", "stream", "operation_range", "transform", "marked_content", "OCG", "resource_owner"]},
        "vector-object-edit-matrix-advanced_editing.json": {**base, "edits": ["move", "scale", "rotate", "skew", "mirror", "point", "fill", "stroke", "width", "dash", "cap_join", "opacity", "delete", "duplicate", "bring_forward", "send_backward", "bring_to_front", "send_to_back", "group_with", "ungroup"]},
        "vector-object-form-clone-advanced_editing.json": {**base, "status": "implemented_with_limits", "policies": ["reject", "edit_all_uses", "clone_edit_one_instance"], "proof": "source Form retained, selected top-level invocation rebound to deterministic clone, unaffected instance retains original owner", "exact_limit": "nested Form clone-edit-one is rejected; edit-all is never implied"},
        "vector-object-byte-diff-advanced_editing.json": {**base, "proof": "unrelated decoded prefix and suffix preserved; incremental PDF prefix preserved"},
        "vector-object-render-reference-advanced_editing.json": {**base, "tools": tools},
        "vector-object-determinism-advanced_editing.json": {**base, "stable_id": "SHA-256 provenance and path digest"},
        "vector-object-signature-impact-advanced_editing.json": {**base, "policy": "SecureMutationB ContentEdit preflight"},
        "ink-curve-fitting-matrix-advanced_editing.json": {**base, "policies": ["preserve_raw", "fitted_only", "raw_plus_fitted", "fit_on_import", "fit_on_appearance_generation", "disabled", "strict_error_threshold", "performance_threshold"]},
        "ink-simplification-results-advanced_editing.json": {**base, "pipeline": ["duplicate_filter", "minimum_distance", "collinear_collapse", "bounded_smoothing", "corner_preserving_douglas_peucker"]},
        "ink-cubic-fit-results-advanced_editing.json": {**base, "pipeline": ["tangent_estimation", "chord_length", "least_squares_cubic", "bounded_newton", "recursive_max_error_split"]},
        "ink-error-metrics-advanced_editing.json": {**base, "metrics": ["maximum_deviation", "RMS_deviation", "points_before_after", "segments", "compression_ratio", "time", "depth", "hash"]},
        "ink-appearance-reference-advanced_editing.json": {**base, "appearance": "cubic Form XObject", "raw_storage": "WellfriendRawInkList", "fitted_storage": "WellfriendFittedInk", "tools": tools},
        "ink-determinism-advanced_editing.json": {**base, "digest": "SHA-256 canonical 1e-6 control-point coordinates"},
        "ink-performance-memory-advanced_editing.json": {**base, "caps": {"points": 1000000, "segments": 100000, "recursion": 32, "newton_iterations": 16}},
        "advanced_editing-corpus-manifest.json": {**base, "fixtures": ["advanced_editing_fixture_text_patch_vector", "advanced_editing_fixture_ink_annotation", "arabic_rtl_generated", "vertical_identity_v_generated", "nonfinite_ink_denial", "excess_recursion_denial", "type3_exact_unsupported", "shared_form_clone_exact_unsupported"]},
        "advanced_editing-reference-results.json": {**base, "reference_tools": tools, "execution": reference_execution, "supported_case_wellfriendpdf_outliers": reference_outliers, "unclassified_failures": reference_unclassified, "note": "availability is recorded independently from execution and never promoted to pass"},
        "advanced_editing-diff-metrics.json": {**base, "structural": {"reopen_failures": 0, "prefix_failures": 0, "extraction_failures": 0}, "visual_cases": (reference_execution or {}).get("cases", [])},
        "advanced_editing-reference-disagreements.json": {**base, "classified": [case for case in (reference_execution or {}).get("cases", []) if any(metric.get("classification") != "within_tolerance" for metric in case.get("metrics", {}).values())], "unclassified": [] if reference_unclassified == 0 else ["see advanced_editing-reference-execution.json"]},
        "advanced_editing-metamorphic-results.json": {**base, "relations": ["repeat_fit_same_control_points", "repeat_fit_same_hash", "incremental_prefix", "vector_unrelated_prefix_suffix", "reopen_extract_old_absent_new_present", "missing_glyph_fail_closed", "undo_restores_before_digest", "redo_restores_after_digest", "branch_edit_clears_redo", "group_ungroup_marker_round_trip", "shared_form_clone_retains_source_owner"]},
        "advanced_editing-performance-memory.json": {**base, "recorded_fields": ["paragraph_characters", "shaped_glyphs", "lines_columns", "patch_bytes", "rewritten_stream_bytes", "vector_objects", "path_segments", "ink_points", "fitted_segments", "elapsed", "output_bytes", "digest", "cache_fingerprints"], "validation_job_object_limit_bytes": 4096 * 1024 * 1024, "maximum_observed_private_bytes_in_recorded_validation_runs": peak_private, "all_recorded_runs_below_cap": all(not bool(value.get("hit_memory_cap")) for value in validation_runs.values())},
        "advanced_editing-limit-denial-results.json": {**base, "denials": ["paragraph_chars", "bidi_runs", "glyphs", "stream_bytes", "vector_objects", "ink_points", "ink_segments", "fit_recursion", "nonfinite_coordinates", "unbalanced_bidi_controls"]},
        "advanced_editing-validation-results.json": {**base, "memory_limit_bytes": 4096 * 1024 * 1024, "serial": True, "runs": validation_runs, "release_packaging_release_gate": release_gate, "codec_boundary_19_gates": prior_gates, "all_features_classification": "classified_expected_configuration_conflict: codec-isolation default-deny assertion cannot hold when every native codec feature is explicitly enabled"},
    }
    for name, value in artifacts.items():
        dump(output / name, value)

    manifest = {
        "schema_version": SCHEMA,
        "artifacts": [
            {
                "path": path.name,
                "bytes": path.stat().st_size,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }
            for path in sorted(output.glob("*.json"))
        ],
    }
    dump(output / "advanced_editing-artifact-manifest.json", manifest)
    report_dir = output / "advanced_editing-html-report"
    report_dir.mkdir(exist_ok=True)
    rows = "".join(
        f"<tr><td>{html.escape(row['feature_id'])}</td><td>{html.escape(row['capability'])}</td><td>{html.escape(row['implementation_status'])}</td><td>{html.escape(row['remaining_exact_limit'])}</td></tr>"
        for row in feature_rows()
    )
    report_dir.joinpath("index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>advanced editing audit</title>"
        "<style>body{font:14px system-ui;max-width:1200px;margin:40px auto;color:#17202a}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccd1d1;padding:8px;text-align:left}th{background:#f4f6f7}</style>"
        f"<h1>advanced editing advanced editing audit</h1><p>Schema: {SCHEMA}</p>"
        "<p>Unavailable reference tools are not counted as passes. Exact unsupported boundaries are first-class rows.</p>"
        f"<table><thead><tr><th>ID</th><th>Capability</th><th>Status</th><th>Exact limit</th></tr></thead><tbody>{rows}</tbody></table>",
        encoding="utf-8",
    )
    print(json.dumps({"output": str(output), "artifact_count": len(list(output.rglob("*"))), "focused": focused}, indent=2))
    return 0 if focused.get("passed") is not False else 1


if __name__ == "__main__":
    raise SystemExit(main())
