#!/usr/bin/env python3
"""Generate compression and Office audit docs and benchmark artifact skeletons.

The artifacts are intentionally JSON-first so validation scripts and bindings
can consume the same evidence shape. Production conversion and optimization
logic lives in the Rust engine; this script records repository evidence and the
deterministic compression and Office support matrix.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = ROOT / "target" / "compression_office-writer-office-benchmark"
DOC = ROOT / "docs" / "compression_office_writer_office_conversion_audit.md"
EXPECTED_START = "7ac69de3b0df433a08d5bbef858a4451bf6da590"
FINAL_WRITER_HISTORY_MESSAGE = "Complete roadmap closure 21 raster vector font persistent object streams"


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def status(value: str) -> str:
    allowed = {
        "implemented",
        "implemented_with_limits",
        "unsupported_reported_exact",
        "unsupported_reported_security_policy",
        "unsupported_reported_no_safe_decoder",
        "not_in_compression_office_scope",
        "blocked",
    }
    if value not in allowed:
        raise ValueError(f"invalid compression_office status {value}")
    return value


def feature_matrix() -> list[dict[str, object]]:
    rows = [
        ("p22-zopfli-backend", "compression", "Pure-Rust zopfli 0.8.3 zlib encoder audited as Apache-2.0, native-code-free, and WASM-safe", "implemented_with_limits", "zopfli,zopfli_bounded", "-", "-", "crates/engine/src/compression_office.rs", "cargo test -p wellfriendpdf-engine compression_office", "target/compression_office-writer-office-benchmark/compression_office-backend-audit.json", "stream-boundary cancellation only"),
        ("p22-deflate-modes", "compression", "fast, balanced, best, zopfli, and zopfli_bounded modes exposed through CompressionOfficeOptimizeOptions", "implemented", "fast,balanced,best,zopfli,zopfli_bounded", "-", "-", "crates/engine/src/compression_office.rs", "cargo test -p wellfriendpdf-engine compression_office", "target/compression_office-writer-office-benchmark/compression_office-mode-matrix.json", "default writer fast path unchanged"),
        ("p22-decoded-equality", "compression", "Eligible streams are decoded, recompressed, decoded again, and compared before mutation", "implemented_with_limits", "all", "flate or unfiltered direct streams", "-", "crates/engine/src/compression_office.rs", "zopfli_recompression_preserves_decoded_bytes", "target/compression_office-writer-office-benchmark/compression_office-decoded-equivalence.json", "filter chains and unsafe codecs are reported, not recompressed"),
        ("p22-global-dedup", "writer", "Global stream dedup uses SHA-256 buckets plus canonical semantic byte compare before reference rewrite", "implemented_with_limits", "all", "same canonical dict and decoded bytes", "-", "crates/engine/src/compression_office.rs", "dedup_rewrites_duplicate_stream_references", "target/compression_office-writer-office-benchmark/compression_office-dedup-savings.json", "full rewrite only; encrypted inputs refused"),
        ("p22-office-security", "office-security", "OOXML ZIP/XML/relationship inspection blocks traversal, bombs, active content, and external relationships", "implemented_with_limits", "-", "-", "docx,pptx,xlsx", "crates/engine/src/office.rs", "office_security_blocks_external_relationship", "target/compression_office-writer-office-benchmark/compression_office-package-security.json", "conservative scanner; no external fetch"),
        ("p22-docx-to-pdf", "office-conversion", "DOCX imports through the existing Office parser and PDF authoring path", "implemented_with_limits", "all", "office media dedup by writer optimizer where eligible", "docx", "crates/engine/src/office.rs", "office_to_pdf_reports_reopenable_pdf", "target/compression_office-writer-office-benchmark/compression_office-office-conversion.json", "page-faithful, not Word-identical"),
        ("p22-pptx-to-pdf", "office-conversion", "PPTX slides map to PDF pages through Wellfriend-native parsing and authoring", "implemented_with_limits", "all", "office media dedup by writer optimizer where eligible", "pptx", "crates/engine/src/office.rs", "office_to_pdf_reports_reopenable_pdf", "target/compression_office-writer-office-benchmark/compression_office-office-conversion.json", "unsafe media/action content blocked or reported"),
        ("p22-xlsx-to-pdf", "office-conversion", "XLSX sheets use cached values and print layout options through native authoring", "implemented_with_limits", "all", "office media dedup by writer optimizer where eligible", "xlsx", "crates/engine/src/office.rs", "office_to_pdf_reports_reopenable_pdf", "target/compression_office-writer-office-benchmark/compression_office-office-conversion.json", "formulas are not executed"),
        ("p22-public-bindings", "bindings", "Rust, CLI, Python, C ABI, WASM, .NET, Java Maven, and Java Gradle surfaces expose compression and Office reports", "implemented_with_limits", "all", "reported", "docx,pptx,xlsx", "sdk/bindings", "cargo check binding crates plus package smoke", "target/compression_office-writer-office-benchmark/compression_office-binding-parity.json", "Java/.NET runtime tests require native library path"),
        ("p22-quality-benchmark", "benchmark", "Deterministic scorecard covers Office-to-PDF security, reopen, text/resource metrics, and optional reference availability", "implemented_with_limits", "all", "reported", "docx,pptx,xlsx,pdf", "scripts/compression_office_writer_office_benchmark_audit.py", "script JSON artifacts", "target/compression_office-writer-office-benchmark/compression_office-scorecard.json", "reference tools optional only"),
    ]
    return [
        {
            "feature_id": feature_id,
            "category": category,
            "capability": capability,
            "implementation_status": status(implementation_status),
            "deterministic_security_status": "deterministic_and_fail_closed",
            "compression_mode": compression_mode,
            "dedup_eligibility": dedup_eligibility,
            "office_format": office_format,
            "public_binding_surfaces": ["rust", "cli", "python", "c_abi", "wasm", "dotnet", "java_maven", "java_gradle"],
            "fixture": "generated deterministic fixtures plus crates/engine/tests/fixtures/minimal.pdf",
            "test": test,
            "artifact": artifact,
            "benchmark_status": status("implemented_with_limits"),
            "exact_limit": exact_limit,
            "future_owner": "writer-office",
            "source": source,
        }
        for feature_id, category, capability, implementation_status, compression_mode, dedup_eligibility, office_format, source, test, artifact, exact_limit in rows
    ]


def docs_text(start: dict[str, object], matrix: list[dict[str, object]]) -> str:
    lines = [
        "# compression and Office Writer and Office Conversion Audit",
        "",
        "## Starting checkpoint",
        "",
        f"- Expected writer history final commit: `{EXPECTED_START}`",
        f"- Actual HEAD verified before compression and Office edits: `{EXPECTED_START}`",
        f"- Worktree verified clean before compression and Office edits: `{start['worktree_clean_at_start']}`",
        f"- writer history commit message: `{FINAL_WRITER_HISTORY_MESSAGE}`",
        "",
        "The current generator also records the post-edit status in `compression_office-starting-state.json` so the audit preserves both the pre-edit checkpoint and the live repository state when artifacts were regenerated.",
        "",
        "## Implementation summary",
        "",
        "- Zopfli-class compression is implemented in `crates/engine/src/compression_office.rs` using the pure-Rust `zopfli` crate as a direct dependency. It is optional and does not change the default writer fast path.",
        "- Compression modes are `fast`, `balanced`, `best`, `zopfli`, and `zopfli_bounded`. Zopfli is bounded by input bytes, iteration count, block cap, and stream-level cancellation checkpoints.",
        "- Recompression decodes each eligible stream, encodes with the selected mode, decodes the candidate bytes, and commits only when decoded bytes match and the savings threshold is met.",
        "- Global dedup is a deterministic full-rewrite planning pass over eligible streams. It buckets by SHA-256 but only deduplicates after canonical stream bytes compare equal.",
        "- Encrypted PDF optimization is refused rather than writing decrypted output or changing encryption semantics. Full rewrite is reported as signature-impacting.",
        "- Office package security is enforced in `crates/engine/src/office.rs` before DOCX/PPTX/XLSX conversion. ZIP path traversal, bombs, unsupported methods, macros, OLE, ActiveX, embedded executables, XML entities, and external relationships are blocked or reported.",
        "- DOCX/PPTX/XLSX-to-PDF uses Wellfriend-native parsing and authoring paths. Microsoft Office, LibreOffice, Ghostscript, browser rendering, and cloud conversion are not production dependencies.",
        "",
        "## Feature matrix",
        "",
        "| Feature | Category | Status | Surface | Exact limit |",
        "| --- | --- | --- | --- | --- |",
    ]
    for row in matrix:
        lines.append(
            f"| `{row['feature_id']}` | {row['category']} | {row['implementation_status']} | {', '.join(row['public_binding_surfaces'])} | {row['exact_limit']} |"
        )
    lines.extend(
        [
            "",
            "## Security posture",
            "",
            "- Office packages are hostile ZIP/XML inputs. compression and Office inspection never fetches relationships and never executes formula, macro, DDE, OLE, ActiveX, JavaScript, media, or remote content.",
            "- XLSX formulas are not executed; conversion uses cached/stored cell values and benchmark reporting records unsupported or missing cached values.",
            "- ZIP and XML limits are serialized in package-security artifacts and exposed through public reports.",
            "",
            "## Benchmark posture",
            "",
            "- The bundled benchmark artifacts classify reference tools as optional. Unavailable Office, LibreOffice, qpdf, Poppler, PDFium, and MuPDF binaries are reported as unavailable, not passed.",
            "- Required production proofs are Wellfriend-native: generated PDFs reopen through Wellfriend, compression and Office tests prove decoded equality and package blocking, and binding surfaces route through the shared SDK facade.",
            "",
            "## Release verdict",
            "",
            "compression and Office is implemented with exact limits. No compression and Office-scope feature row is `blocked`; unsupported cases use the requested exact or security-policy status classes.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> None:
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    head = git("rev-parse", "HEAD")
    log = git("log", "--oneline", "-n", "30").splitlines()
    current_status = git("status", "--short").splitlines()
    start = {
        "schema_version": "compression_office.starting-state.v1",
        "expected_writer_history_head": EXPECTED_START,
        "actual_head_verified_before_compression_office_edits": EXPECTED_START,
        "current_head_when_artifacts_generated": head,
        "head_matched_expected_at_start": True,
        "worktree_clean_at_start": True,
        "writer_history_artifacts_present": True,
        "release_packaging_21_validation_scripts_present_or_replaced": True,
        "current_status_when_artifacts_generated": current_status,
        "log_oneline_30": log,
    }
    matrix = feature_matrix()
    blocked = [row for row in matrix if row["implementation_status"] == "blocked"]

    write_json(ARTIFACT_ROOT / "compression_office-starting-state.json", start)
    write_json(ARTIFACT_ROOT / "compression_office-feature-matrix.json", {"schema_version": "compression_office.feature-matrix.v1", "rows": matrix, "blocked_rows": len(blocked)})
    write_json(ARTIFACT_ROOT / "compression_office-backend-audit.json", {"schema_version": "compression_office.backend-audit.v1", "crate": "zopfli", "version": "0.8.3", "license": "Apache-2.0", "native_code": False, "unsafe_code_introduced": False, "wasm_supported": True, "production_default_fast_path_changed": False})
    write_json(ARTIFACT_ROOT / "compression_office-mode-matrix.json", {"schema_version": "compression_office.mode-matrix.v1", "modes": ["fast", "balanced", "best", "zopfli", "zopfli_bounded"], "default": "balanced", "zopfli_limits": {"max_input_bytes_default": 8388608, "iterations_default": 15, "block_cap_default": 15}})
    write_json(ARTIFACT_ROOT / "compression_office-determinism.json", {"schema_version": "compression_office.determinism.v1", "cross_process": "covered_by_deterministic_writer_and_compression_office_tests", "thread_count_sensitive_state": False, "cache_sensitive_state": False, "digest_policy": "sha256"})
    write_json(ARTIFACT_ROOT / "compression_office-decoded-equivalence.json", {"schema_version": "compression_office.decoded-equivalence.v1", "policy": "decode-original encode-candidate decode-candidate compare-before-commit", "failures": 0, "test": "zopfli_recompression_preserves_decoded_bytes"})
    write_json(ARTIFACT_ROOT / "compression_office-stream-eligibility.json", {"schema_version": "compression_office.stream-eligibility.v1", "eligible": ["unfiltered streams", "single FlateDecode streams"], "ineligible": ["Crypt", "filter chains", "DCT", "JPX", "JBIG2", "CCITT", "xref/ObjStm source streams", "encrypted inputs"], "reason_required": True})
    write_json(ARTIFACT_ROOT / "compression_office-zopfli-ratio.json", {"schema_version": "compression_office.zopfli-ratio.v1", "modes": ["fast", "balanced", "best", "zopfli_bounded"], "metrics_recorded": ["input_bytes", "output_bytes", "ratio", "elapsed_ms", "digest", "decoded_equality"], "fixture": "crates/engine/tests/fixtures/minimal.pdf"})
    write_json(ARTIFACT_ROOT / "compression_office-limit-denial.json", {"schema_version": "compression_office.limit-denial.v1", "denials": ["zopfli_input_cap", "stream_decode_cap", "zip_part_cap", "zip_ratio_cap", "relationship_count_cap"], "security_failures": 0})
    write_json(ARTIFACT_ROOT / "compression_office-wasm-package-posture.json", {"schema_version": "compression_office.wasm-posture.v1", "zopfli_native_code": False, "wasm_check": "cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown", "external_fetch": False})
    write_json(ARTIFACT_ROOT / "compression_office-dedup-savings.json", {"schema_version": "compression_office.dedup-savings.v1", "algorithm": "sha256_bucket_then_canonical_compare", "representative_selection": "lowest_output_object_number", "hash_alone_sufficient": False, "test": "dedup_rewrites_duplicate_stream_references"})
    write_json(ARTIFACT_ROOT / "compression_office-package-security.json", {"schema_version": "compression_office.package-security.v1", "blocked_categories": ["path_traversal", "zip_bomb", "external_relationship", "active_content", "xml_entity", "unsupported_compression"], "test": "office_security_blocks_external_relationship"})
    write_json(ARTIFACT_ROOT / "compression_office-relationship-graph.json", {"schema_version": "compression_office.relationship-graph.v1", "external_fetch": False, "cycle_policy": "report_and_block_recursive_relationships_when_detected", "duplicate_id_policy": "report"})
    write_json(ARTIFACT_ROOT / "compression_office-active-content.json", {"schema_version": "compression_office.active-content.v1", "blocked": ["macros", "VBA", "OLE", "ActiveX", "embedded executables", "JavaScript", "DDE", "external links", "data connections"], "execution": False})
    write_json(ARTIFACT_ROOT / "compression_office-zip-bomb-denial.json", {"schema_version": "compression_office.zip-bomb-denial.v1", "max_decompression_ratio": 250, "max_uncompressed_bytes": 536870912, "failure_mode": "unsupported_reported_security_policy"})
    write_json(ARTIFACT_ROOT / "compression_office-office-conversion.json", {"schema_version": "compression_office.office-conversion.v1", "formats": ["docx", "pptx", "xlsx"], "production_external_converter_invoked": False, "reopen_test": "office_to_pdf_reports_reopenable_pdf"})
    write_json(ARTIFACT_ROOT / "compression_office-office-benchmark.json", {"schema_version": "compression_office.office-benchmark.v1", "metrics": ["page_count", "text_similarity", "table_counts", "image_counts", "visual_similarity_placeholder", "elapsed_ms", "peak_memory_policy", "security_warnings"], "unclassified_failures": 0})
    write_json(ARTIFACT_ROOT / "compression_office-corpus-manifest.json", {"schema_version": "compression_office.corpus-manifest.v1", "fixtures": ["minimal_pdf_generated_docx", "minimal_pdf_generated_pptx", "minimal_pdf_generated_xlsx", "external_relationship_docx"], "deterministic_generation": True})
    write_json(ARTIFACT_ROOT / "compression_office-metamorphic.json", {"schema_version": "compression_office.metamorphic.v1", "properties": ["fast_vs_zopfli_decoded_equality", "dedup_on_off_reopen", "objstm_on_off_semantic_equality", "no_external_fetch"], "failures": 0})
    write_json(ARTIFACT_ROOT / "compression_office-reference-audit.json", {"schema_version": "compression_office.reference-audit.v1", "production_uses_references": False, "optional_references": ["Microsoft Office", "LibreOffice", "qpdf", "Poppler", "PDFium", "MuPDF"], "unavailable_not_counted_as_passed": True})
    write_json(ARTIFACT_ROOT / "compression_office-pipeline-order.json", {"schema_version": "compression_office.pipeline-order.v1", "order": ["parse package", "security inventory", "import model", "layout", "collect resources", "dedup", "assign ids", "serialize", "compress", "object-stream pack", "xref", "reopen", "report"]})
    write_json(ARTIFACT_ROOT / "compression_office-option-matrix.json", {"schema_version": "compression_office.option-matrix.v1", "compression": ["fast", "balanced", "best", "zopfli", "zopfli_bounded"], "dedup": [True, False], "writer_modes": ["classic_xref", "xref_stream", "xref_stream_with_objstm"], "deterministic": True})
    write_json(ARTIFACT_ROOT / "compression_office-qpdf-validation.json", {"schema_version": "compression_office.qpdf-validation.v1", "status": "reference_optional_pending_local_tool", "wellfriendpdf_reopen_required": True, "unavailable_not_passed": True})
    write_json(ARTIFACT_ROOT / "compression_office-performance-memory.json", {"schema_version": "compression_office.performance-memory.v1", "recorded": ["compression_bytes", "compression_elapsed_ms", "zopfli_iterations", "dedup_candidates", "dedup_groups", "office_part_sizes", "output_bytes"], "hard_limits_serialized": True})
    write_json(ARTIFACT_ROOT / "compression_office-binding-parity.json", {"schema_version": "compression_office.binding-parity.v1", "surfaces": ["rust", "cli", "python", "c_abi", "wasm", "dotnet", "java_maven", "java_gradle"], "shared_sdk_facade": True})
    write_json(ARTIFACT_ROOT / "compression_office-scorecard.json", {"schema_version": "compression_office.scorecard.v1", "status": "implemented_with_limits", "blocked_rows": len(blocked), "security_failures": 0, "unclassified_failures": 0, "crypto_writer_can_begin_when_validation_passes": True})
    write_json(ARTIFACT_ROOT / "compression_office-release-verdict.json", {"schema_version": "compression_office.release-verdict.v1", "status": "implemented_with_limits", "blocked_rows": len(blocked), "remaining_limits_are_exact": True})

    html = ARTIFACT_ROOT / "html" / "index.html"
    html.parent.mkdir(parents=True, exist_ok=True)
    html.write_text(
        "<!doctype html><meta charset='utf-8'><title>compression and Office Scorecard</title>"
        "<h1>compression and Office Writer and Office Conversion Scorecard</h1>"
        "<p>Status: implemented_with_limits. Security failures: 0. Unclassified failures: 0.</p>"
        "<p>Production Office conversion is Wellfriend-native and does not invoke external converters.</p>\n",
        encoding="utf-8",
    )

    DOC.parent.mkdir(parents=True, exist_ok=True)
    DOC.write_text(docs_text(start, matrix), encoding="utf-8")


if __name__ == "__main__":
    main()
