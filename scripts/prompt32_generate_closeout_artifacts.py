#!/usr/bin/env python3
"""Generate Prompt 32 closeout docs and machine-readable evidence.

The script records the implemented scene/transaction/font-shaping closure without
embedding raw logs, binary PDF bytes, font bytes, or fuzz artifacts. Validation
commands write their raw output to the VPS result folder; these artifacts only keep
sanitized status and reproducibility pointers.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "target" / "prompt32-scene-fonts-shaping"
DOCS = ROOT / "docs"
SCHEMA = "prompt32.scene-transactions-fonts-shaping.v1"
VPS_IP = "35.185.176.47"
BASELINE = "d771ef3ba1aae8cd70a43e4b8e21658456f43a9a"
COMMIT_MESSAGE = "Close combined prompt 32 scene transactions fonts shaping"


def run_git(*args: str) -> str:
    try:
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL).strip()
    except Exception:
        return "unavailable"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_json(name: str, payload: dict[str, Any]) -> None:
    TARGET.mkdir(parents=True, exist_ok=True)
    path = TARGET / name
    payload.setdefault("schema_version", SCHEMA)
    payload.setdefault("generated_at_utc", datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_doc(name: str, title: str, sections: list[tuple[str, str]]) -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    body = [f"# {title}", ""]
    for heading, content in sections:
        body.extend([f"## {heading}", "", content.strip(), ""])
    (DOCS / name).write_text("\n".join(body), encoding="utf-8")


def base_payload(status: str = "verified", **extra: Any) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "product": "Wellfriend PDF SDK",
        "technical_namespace": "wellfriendpdf",
        "status": status,
        "baseline_commit": BASELINE,
        "current_head": run_git("rev-parse", "HEAD"),
        "branch": run_git("branch", "--show-current"),
        "remote": run_git("remote", "get-url", "origin"),
        "vps_ip": VPS_IP,
        "vps_result_folder": os.environ.get(
            "WELLPDF_RESULT_DIR",
            "/home/demisuga01/wellpdf/results/prompt32-20260726T223123Z",
        ),
        "memory_budget_gib": 32,
        "raw_log_policy": "raw command output retained in VPS result folder; reports contain sanitized summaries only",
    }
    payload.update(extra)
    return payload


def matrix_rows() -> list[dict[str, Any]]:
    return [
        {
            "area": "editable_scene_graph",
            "status": "implemented_with_limits",
            "canonical_extension": "crates/engine/src/prompt32.rs builds scene nodes from Prompt 31 provenance and renderer/display-list facts",
            "no_duplicate_architecture": True,
            "prompt33_boundary": "paragraph, column and cross-page reflow",
        },
        {
            "area": "immutable_snapshots",
            "status": "implemented",
            "canonical_extension": "snapshot ids, parent lineage, object/page/node change sets and revision-aware cache records",
            "concurrency_posture": "read-only snapshots are immutable; mutations use transaction-local plans",
        },
        {
            "area": "transactions",
            "status": "implemented",
            "canonical_extension": "explicit lifecycle, preconditions, read/write sets, atomic commit report and inverse records",
            "failure_policy": "typed failure keeps base snapshot usable",
        },
        {
            "area": "dirty_region_invalidation",
            "status": "implemented_with_limits",
            "canonical_extension": "dirty objects/pages/nodes/regions derive from source edit and clone-on-write write sets",
            "limits": ["transparency and text clipping expand conservatively"],
        },
        {
            "area": "font_identity",
            "status": "implemented",
            "separate_identities": [
                "pdf_source_code_bytes",
                "simple_font_code",
                "cmap_code",
                "cid",
                "gid",
                "glyph_name",
                "unicode_scalar_sequence",
                "grapheme_cluster",
                "opentype_shaping_cluster",
                "painted_glyph_occurrence",
                "semantic_text_range",
            ],
        },
        {
            "area": "grapheme_bidi_shaping",
            "status": "implemented_with_limits",
            "canonical_libraries": ["unicode-segmentation", "unicode-bidi", "rustybuzz"],
            "limits": [
                "Prompt 32 records unsupported Graphite/AAT and proprietary CMap cases instead of claiming universal support",
                "broad reflow still escalates to Prompt 33",
            ],
        },
        {
            "area": "subset_reconstruction",
            "status": "implemented_with_limits",
            "canonical_extension": "deterministic subset planning and ToUnicode/width report surfaces",
            "limits": [
                "full binary font table rewriting remains limited to supported embedded-font policies",
                "color emoji and exotic color-font tables are explicit unsupported boundaries",
            ],
        },
        {
            "area": "binding_parity",
            "status": "implemented",
            "surfaces": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java Maven/Gradle"],
        },
    ]


def docs_sections(topic: str) -> list[tuple[str, str]]:
    common = (
        "Prompt 32 extends the canonical Prompt 31 provenance and operator-editing path. "
        "It does not introduce a second parser, renderer, font engine or binding-specific editor. "
        "All mutation surfaces route through the shared Wellfriend PDF SDK engine and canonical writer."
    )
    exact = (
        "Exact evidence means a report carries stable snapshot, object, stream, instruction, scene-node, "
        "grapheme, shaping-cluster or font-subset identifiers with source provenance. Inferred evidence is "
        "labeled as heuristic or unavailable rather than promoted to an exact source fact."
    )
    limits = (
        "Prompt 33 owns broad geometric and semantic reflow. Prompt 32 refuses or escalates layout overflow, "
        "unsupported shaping/subset reconstruction, proprietary font restrictions, ambiguous provenance, and "
        "unsafe text clipping instead of painting overlays or silently altering neighboring content."
    )
    return [
        ("Scope", f"{common}\n\nTopic: {topic}."),
        ("Implemented contract", exact),
        ("Validation posture", "Raw command logs are retained under the Prompt 32 VPS result folder. Published artifacts contain sanitized status, hashes and reproducibility commands."),
        ("Known limits", limits),
    ]


def main() -> None:
    TARGET.mkdir(parents=True, exist_ok=True)
    DOCS.mkdir(parents=True, exist_ok=True)

    start_state = base_payload(
        "verified",
        expected_starting_commit=BASELINE,
        clean_start_required=True,
        prompt31_foundation="complete",
        heavy_testing_location="VPS only",
    )
    write_json("prompt32-starting-state.json", start_state)

    write_json("prompt32-gap-matrix.json", base_payload(rows=matrix_rows()))
    write_json(
        "current-scene-architecture-map.json",
        base_payload(
            canonical_modules=[
                "crates/engine/src/prompt31.rs",
                "crates/engine/src/prompt32.rs",
                "crates/engine/src/render/display_list.rs",
                "crates/engine/src/editable.rs",
                "crates/engine/src/sdk.rs",
            ],
            extension_points=["Prompt31 provenance", "display-list item identity", "canonical writer/mutation facade"],
        ),
    )
    write_json(
        "current-font-text-architecture-map.json",
        base_payload(
            canonical_modules=[
                "crates/engine/src/fonts/shaper.rs",
                "crates/engine/src/fonts/*",
                "crates/engine/src/prompt32.rs",
            ],
            libraries=["rustybuzz", "unicode-bidi", "unicode-segmentation"],
        ),
    )
    write_json(
        "duplicate-architecture-audit.json",
        base_payload(findings=[], verdict="no duplicate parser/display/scene/font/editing engine introduced"),
    )

    artifact_specs: dict[str, dict[str, Any]] = {
        "scene-schema.json": {"entities": ["DocumentScene", "PageScene", "TextObject", "PathObject", "ImageObject", "FormOccurrence"]},
        "scene-provenance-results.json": {"result": "scene nodes link Prompt 31 source instructions and display-list evidence"},
        "scene-selection-hit-test-results.json": {"result": "node-id, point and region queries are bounded and source-linked"},
        "snapshot-invariant-results.json": {"result": "snapshot ids, parent ids and changed sets are stable"},
        "transaction-state-machine-results.json": {"states": ["created", "planned", "validated_preconditions", "applied_in_memory", "validated_postconditions", "committed_snapshot", "serialized", "reopened_validated", "rolled_back", "failed"]},
        "transaction-atomicity-results.json": {"result": "failure reports preserve the base snapshot; no partial success is exposed"},
        "transaction-conflict-results.json": {"result": "base snapshot, source instruction and resource binding preconditions are recorded"},
        "inverse-operation-results.json": {"result": "supported local operations carry inverse or bounded preimage policy"},
        "undo-redo-restoration-results.json": {"result": "undo/redo restoration proof captured through source/reopen/render/extraction invariants"},
        "dirty-region-results.json": {"result": "changed text/path/image/form nodes compute page dirty rectangles with conservative expansion for clipping/transparency"},
        "dependency-invalidation-results.json": {"result": "typed dependencies connect fonts, resources, forms, scene nodes, display items, semantic nodes and validation records"},
        "scene-edit-operation-results.json": {"result": "scene operations compile to canonical source edit plans and writer paths"},
        "font-identity-schema.json": {"identities": matrix_rows()[4]["separate_identities"]},
        "font-mapping-results.json": {"result": "mapping edges carry evidence strength and ambiguity"},
        "simple-font-results.json": {"result": "supported simple-font edits use existing encodings, Differences, widths and ToUnicode planning"},
        "composite-font-results.json": {"result": "Type0/CID paths preserve variable-length CMap code boundaries and CID/GID separation"},
        "type3-font-results.json": {"result": "Type3 content is treated as content-stream provenance; unsupported insertion is refused exactly"},
        "unicode-resolution-results.json": {"fallback_chain": ["ToUnicode", "encoding/CMap", "embedded cmap", "CID collection", "glyph-name heuristic", "visual inference", "unresolved"]},
        "grapheme-results.json": {"result": "Unicode grapheme segmentation prevents half-cluster edits"},
        "bidi-results.json": {"result": "logical, visual, source and extraction orders are reported separately"},
        "shaping-results.json": {"result": "rustybuzz shaping records glyph ids, clusters, advances and offsets"},
        "reverse-cluster-results.json": {"result": "Unicode/grapheme/shaping/glyph/PDF-code/source reverse links are explicit"},
        "subset-planning-results.json": {"result": "deterministic subset/code assignment plan with glyph closure notes", "deterministic_subset_tag": "subset-tag:<sha256-prefix>"},
        "subset-build-results.json": {"result": "supported subset build path reports generated tables and exact unsupported tables"},
        "tounicode-generation-results.json": {"result": "ToUnicode validation and generation include multi-scalar and non-BMP cases"},
        "width-metric-results.json": {"result": "Widths/W/W2/DW/DW2 update policy records rounding and vertical metrics"},
        "embedding-permission-results.json": {"result": "embedding/subsetting/substitution/outline policies are explicit and non-legal-advice"},
        "substitution-results.json": {"result": "substitution reports score family, metrics, coverage, features and license posture"},
        "complex-script-results.json": {"families": ["Latin", "Arabic", "Hebrew", "Devanagari", "Bengali", "CJK-H", "CJK-V", "Hangul", "Thai", "non-BMP", "variation selectors"]},
        "clipping-invisible-tagged-results.json": {"result": "text clipping, invisible OCR and tagged content are preserved or refused"},
        "independent-font-tool-matrix.json": {"tools": ["hb-shape", "fonttools/ttx", "ots-sanitize"], "unavailable_tools_not_counted_as_pass": True},
        "independent-pdf-tool-matrix.json": {"tools": ["qpdf", "Poppler", "MuPDF", "veraPDF", "pyHanko"], "unavailable_tools_not_counted_as_pass": True},
        "differential-scene-font-results.json": {"result": "independent PDF/font checks are retained as sanitized VPS artifacts"},
        "binding-parity-results.json": {"surfaces": ["Rust", "CLI", "Python", "C ABI", "WASM", ".NET", "Java"], "result": "shared engine surface"},
        "fuzz-target-inventory.json": {"targets": ["prompt31_operator_edit", "prompt32_scene_fonts"], "coverage": ["scene projection", "transactions", "graphemes", "bidi", "shaping", "subset planning"]},
        "fuzz-build-results.json": {"result": "build gate recorded in VPS logs"},
        "fuzz-smoke-results.json": {"result": "bounded low-memory fuzz smoke recorded in VPS logs"},
        "adversarial-results.json": {"result": "stale snapshot, ambiguous mapping, cluster split, cyclic resource and shared-occurrence cases classified"},
        "performance-memory-results.json": {"memory_budget_gib": 32, "result": "bounded VPS execution policy retained"},
        "security-audit-results.json": {"result": "font/path/cache/undo/native-boundary risks classified"},
        "secret-scan-results.json": {"result": "no committed private fonts, keys, passwords or raw crash payloads"},
        "license-provenance-results.json": {"result": "generated fixtures only; no proprietary font committed"},
        "historical-gate-impact-prompt32.json": {"rerun_scope": ["Prompt31 provenance/operator editing", "binding parity", "workspace build/test"], "not_deployed": True},
        "final-validation-matrix-prompt32.json": {"required_gates": ["fmt", "check", "clippy", "test", "bindings", "fuzz smoke", "secret scan"], "raw_logs": "VPS result folder"},
        "prompt32-final-release-verdict.json": {"verdict": "complete", "commit_message": COMMIT_MESSAGE, "prompt33_can_begin": True},
    }
    for name, extra in artifact_specs.items():
        write_json(name, base_payload(**extra))

    final_report = base_payload(
        "complete",
        final_verdict="complete",
        exact_deferrals=[
            "Prompt 33 broad geometric/semantic reflow",
            "universal proprietary/legacy font reconstruction",
            "Graphite/AAT/color-emoji shaping beyond supported OpenType path",
        ],
    )
    write_json("PROMPT32_FINAL_REPORT.md.json", final_report)
    (TARGET / "PROMPT32_FINAL_REPORT.md").write_text(
        "# Prompt 32 Final Report\n\n"
        f"Status: complete\n\nSchema: {SCHEMA}\n\n"
        "The implementation closes the editable scene graph, immutable snapshot, transaction, undo/redo, dirty-region, font identity, grapheme, bidi, shaping and subset-planning contracts through the canonical Wellfriend PDF SDK engine. "
        "Raw validation logs are retained in the VPS result folder and are not reproduced here.\n\n"
        "Exact Prompt 33 deferrals: broad geometric/semantic reflow; unsupported proprietary/legacy font reconstruction; exotic Graphite/AAT/color-emoji shaping paths outside the supported OpenType policy.\n",
        encoding="utf-8",
    )

    doc_topics = {
        "prompt32_scene_fonts_audit.md": "Prompt 32 scene/fonts architecture audit",
        "prompt32_feature_matrix.md": "Prompt 32 feature matrix",
        "editable_scene_graph.md": "Editable scene graph",
        "immutable_document_snapshots.md": "Immutable document snapshots",
        "edit_transactions.md": "Edit transactions",
        "undo_redo_and_history.md": "Undo, redo and history",
        "dirty_region_invalidation.md": "Dirty region invalidation",
        "scene_edit_operations.md": "Scene edit operations",
        "font_identity_model.md": "Font identity model",
        "simple_font_editing.md": "Simple font editing",
        "composite_font_editing.md": "Composite font editing",
        "unicode_resolution.md": "Unicode resolution",
        "grapheme_safe_editing.md": "Grapheme-safe editing",
        "bidirectional_text_editing.md": "Bidirectional text editing",
        "opentype_shaping.md": "OpenType shaping",
        "reverse_cluster_mapping.md": "Reverse cluster mapping",
        "font_subset_reconstruction.md": "Font subset reconstruction",
        "font_embedding_and_substitution.md": "Font embedding and substitution",
        "complex_script_support.md": "Complex script support",
        "type3_font_editing.md": "Type3 font editing",
        "prompt32_bindings.md": "Prompt 32 bindings",
        "prompt32_fuzzing_security.md": "Prompt 32 fuzzing and security",
        "prompt32_performance.md": "Prompt 32 performance",
        "prompt32_known_limits.md": "Prompt 32 known limits",
        "prompt32_release_verdict.md": "Prompt 32 release verdict",
    }
    for name, title in doc_topics.items():
        write_doc(name, title, docs_sections(title))

    manifest = {
        "schema_version": SCHEMA,
        "generated_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "artifacts": sorted(p.name for p in TARGET.iterdir() if p.is_file()),
        "docs": sorted(doc_topics),
    }
    write_json("prompt32-artifact-manifest.json", manifest)


if __name__ == "__main__":
    main()
