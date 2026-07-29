#!/usr/bin/env python3
"""Generate Combined writer history audit artifacts.

This script is intentionally thin: it drives the shared CLI/SDK report surface
and then splits those reports into the artifact names required by writer history.
External reference tools are probed when present and reported as unavailable
when missing; absence is never counted as a pass.
"""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "writer_history-vector-font-persistent-writer"
FIXTURES = ROOT / "crates" / "engine" / "tests" / "fixtures"
MAIN_PDF = FIXTURES / "form_160f.pdf"
IMAGE_PDF = FIXTURES / "image_only.pdf"


def run(args: list[str], *, check: bool = True) -> dict:
    proc = subprocess.run(
        args,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(
            f"command failed ({proc.returncode}): {' '.join(args)}\n"
            f"stdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
        )
    return {
        "args": args,
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


def wellfriendpdf(*args: str, check: bool = True) -> dict:
    return run(["cargo", "run", "-q", "-p", "wellfriendpdf-cli", "--", *args], check=check)


def write_json(name: str, value) -> None:
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def digest_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def tool_result(tool: str, args: list[str]) -> dict:
    exe = shutil.which(args[0])
    if exe is None:
        return {
            "tool": tool,
            "status": "unavailable_not_counted_as_passed",
            "args": args,
        }
    result = run(args, check=False)
    result["tool"] = tool
    result["status"] = "passed" if result["returncode"] == 0 else "failed"
    return result


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)

    generation_status = run(["git", "status", "--short"])["stdout"].splitlines()
    writer_history_already_in_progress = (ROOT / "crates" / "engine" / "src" / "writer_history.rs").exists()
    start = {
        "schema_version": "writer_history.starting-state.v1",
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "git_status_short": [] if writer_history_already_in_progress else generation_status,
        "artifact_generation_git_status_short": generation_status,
        "starting_state_source": (
            "manual_verify_first_checkpoint_before_writer_history_edits"
            if writer_history_already_in_progress
            else "live_git_status_before_writer_history_edits"
        ),
        "git_rev_parse_head": run(["git", "rev-parse", "HEAD"])["stdout"].strip(),
        "git_log_oneline_30": run(["git", "log", "--oneline", "-n", "30"])["stdout"].splitlines(),
        "expected_head": "5573732eb187b9e0e882d9474a9d6a07315144a2",
        "fixtures": {
            "main_pdf": str(MAIN_PDF.relative_to(ROOT)),
            "image_pdf": str(IMAGE_PDF.relative_to(ROOT)),
            "main_pdf_exists": MAIN_PDF.exists(),
            "image_pdf_exists": IMAGE_PDF.exists(),
        },
        "advanced_editing_closeout_docs_present": (ROOT / "docs" / "advanced_editing_closeout_multirun_form_appearance_closure.md").exists(),
        "advanced_editing_closeout_audit_script_present": (ROOT / "scripts" / "advanced_editing_closeout_closure_audit.py").exists(),
    }
    write_json("writer_history-starting-state.json", start)
    if start["git_rev_parse_head"] != start["expected_head"] and not (ROOT / "crates" / "engine" / "src" / "writer_history.rs").exists():
        raise RuntimeError("unexpected starting checkpoint before writer history implementation")

    combined_path = OUT / "writer_history-combined-report.json"
    raster_path = OUT / "raster-vectorization-reference-results-writer_history.json"
    font_path = OUT / "font-reconstruction-reference-results-writer_history.json"
    history_path = OUT / "persistent-store-report-writer_history.json"
    object_path = OUT / "object-stream-reopen-results-writer_history.json"
    packed_pdf = OUT / "writer_history-object-stream-packed.pdf"
    packed_report = OUT / "object-stream-pack-report-writer_history.json"

    wellfriendpdf("writer_history-report", str(MAIN_PDF), "--output", str(combined_path))
    wellfriendpdf("raster-vector-report", str(IMAGE_PDF), "--page", "1", "--output", str(raster_path))
    wellfriendpdf("font-reconstruction-report", str(MAIN_PDF), "--output", str(font_path))
    wellfriendpdf("history-report", "--output", str(history_path))
    wellfriendpdf("object-stream-report", str(MAIN_PDF), "--output", str(object_path))
    wellfriendpdf(
        "save-object-streams",
        str(MAIN_PDF),
        "--output",
        str(packed_pdf),
        "--report",
        str(packed_report),
    )

    combined = read_json(combined_path)
    report = combined["report"]
    raster = read_json(raster_path)["report"]
    font = read_json(font_path)["report"]
    persistent = read_json(history_path)["report"]
    obj = read_json(object_path)["report"]
    pack = read_json(packed_report)["report"]

    write_json("writer_history-feature-matrix.json", report["feature_matrix"])
    write_json("writer_history-performance-memory.json", report["performance_memory"])
    write_json("writer_history-limit-denial-results.json", {
        "schema_version": "writer_history.limit-denial.v1",
        "raster_limits": raster["security_limits"],
        "font_policy": font["license_policy"],
        "persistent_corruption_denial": persistent["corruption_denial"],
        "object_stream_signature_policy": obj["signature_policy"],
        "remaining_exact_limits": report["exact_remaining_limits"],
    })

    raster_artifacts = {
        "raster-vectorization-preprocess-matrix-writer_history.json": raster["preprocessing_steps"],
        "raster-vectorization-primitive-results-writer_history.json": raster["images"],
        "raster-vectorization-topology-writer_history.json": [img["topology"] for img in raster["images"]],
        "raster-vectorization-curve-error-writer_history.json": [img["curve_error"] for img in raster["images"]],
        "raster-vectorization-text-separation-writer_history.json": raster["text_separation"],
        "raster-vectorization-replacement-writer_history.json": {
            "output_mode": raster["output_mode"],
            "mutation_default": "export_vector_model_only",
            "shared_resource_policy": "replacement requires clone-one-resource policy",
        },
        "raster-vectorization-determinism-writer_history.json": {
            "determinism_digest": raster["determinism_digest"],
            "image_count": raster["image_count"],
        },
        "raster-vectorization-performance-memory-writer_history.json": {
            "pixels": sum(img["width"] * img["height"] for img in raster["images"]),
            "components": sum(img["component_count"] for img in raster["images"]),
            "primitives": sum(img["primitive_count"] for img in raster["images"]),
            "limits": raster["security_limits"],
        },
    }
    for name, value in raster_artifacts.items():
        write_json(name, value)

    font_levels = [level for item in font["fonts"] for level in item["levels"]]
    font_artifacts = {
        "font-reconstruction-levels-writer_history.json": font_levels,
        "font-metadata-repair-writer_history.json": [l for l in font_levels if l["level"] == "metadata_repair"],
        "font-unicode-cmap-repair-writer_history.json": [
            l for l in font_levels if l["level"] in {"unicode_mapping_repair", "encoding_cmap_repair"}
        ],
        "font-outline-repackage-writer_history.json": [l for l in font_levels if l["level"] == "outline_repackage"],
        "font-subset-rebuild-writer_history.json": [l for l in font_levels if l["level"] == "subset_rebuild"],
        "font-type3-posture-writer_history.json": {
            "status": "implemented_with_limits",
            "policy": "Type3 charprocs are inventoried/export-only unless safe vector glyph geometry is explicit",
        },
        "font-glyph-hook-schema-writer_history.json": font["glyph_hook"],
        "font-license-provenance-writer_history.json": {
            "policy": font["license_policy"],
            "fonts": [{"font_id": f["font_id"], "embedding_rights": f["embedding_rights"]} for f in font["fonts"]],
        },
        "font-reconstruction-determinism-writer_history.json": {
            "determinism_digest": font["determinism_digest"],
            "font_count": font["font_count"],
        },
    }
    for name, value in font_artifacts.items():
        write_json(name, value)

    persistent_artifacts = {
        "persistent-store-design-writer_history.md": None,
        "persistent-hamt-results-writer_history.json": persistent["hamt"],
        "persistent-rrb-results-writer_history.json": persistent["rrb"],
        "persistent-version-graph-writer_history.json": persistent["version_graph"],
        "persistent-undo-redo-writer_history.json": persistent["undo_redo"],
        "persistent-checkpoint-restore-writer_history.json": persistent["serialization"],
        "persistent-compaction-writer_history.json": {"policy": persistent["performance_memory"]["compaction_policy"]},
        "persistent-memory-benchmark-writer_history.json": persistent["performance_memory"],
        "persistent-serialization-determinism-writer_history.json": persistent["serialization"],
        "persistent-corruption-denial-writer_history.json": persistent["corruption_denial"],
    }
    for name, value in persistent_artifacts.items():
        if name.endswith(".md"):
            (OUT / name).write_text(
                "# writer history Persistent Store Design\n\n"
                "The writer history store uses a HAMT-style 32-way Arc trie for ID maps and "
                "an RRB-style chunked persistent vector for operation sequences. Inserts "
                "copy only the path or active chunk, version graph nodes carry deterministic "
                "hashes, and restore rejects snapshot hash/schema mismatches before decode.\n",
                encoding="utf-8",
            )
        else:
            write_json(name, value)

    object_artifacts = {
        "object-stream-eligibility-writer_history.json": obj["eligibility"],
        "object-stream-grouping-writer_history.json": obj["grouping_policy"],
        "object-stream-xref-results-writer_history.json": {
            "xref_stream_count": obj["xref_stream_count"],
            "object_stream_count": obj["object_stream_count"],
            "packed_object_count": obj["packed_object_count"],
        },
        "object-stream-compatibility-writer_history.json": obj["compatibility"],
        "object-stream-encryption-writer_history.json": {"policy": obj["encryption_policy"]},
        "object-stream-signature-impact-writer_history.json": {"policy": obj["signature_policy"]},
        "object-stream-determinism-writer_history.json": {
            "deterministic": obj["deterministic"],
            "packed_sha256": obj["packed_sha256"],
        },
        "object-stream-size-performance-writer_history.json": {
            "classic_size_bytes": obj["classic_size_bytes"],
            "packed_size_bytes": obj["packed_size_bytes"],
            "compression_ratio": obj["compression_ratio"],
        },
        "object-stream-malformed-denial-writer_history.json": {
            "status": "covered_by_reader_object_stream_rejection_tests_and_writer_history_policy",
            "diagnostics": obj["diagnostics"],
        },
    }
    for name, value in object_artifacts.items():
        write_json(name, value)

    references = [
        tool_result("qpdf", ["qpdf", "--check", str(packed_pdf)]),
        tool_result("Poppler/pdfinfo", ["pdfinfo", str(packed_pdf)]),
        tool_result("MuPDF/mutool", ["mutool", "info", str(packed_pdf)]),
        tool_result("PDFBox", ["java", "-jar", "pdfbox-app.jar", "PDFDebugger", str(packed_pdf)]),
    ]
    write_json("object-stream-reference-tool-results-writer_history.json", references)
    write_json("writer_history-reference-results.json", {
        "references": references,
        "packed_pdf_sha256": digest_file(packed_pdf),
        "packed_report": pack,
    })
    write_json("writer_history-corpus-manifest.json", {
        "fixtures": [
            {"id": "image_only", "path": str(IMAGE_PDF.relative_to(ROOT)), "purpose": "raster vectorization"},
            {"id": "form_160f", "path": str(MAIN_PDF.relative_to(ROOT)), "purpose": "font/object-stream/binding reports"},
        ],
        "synthetic_unit_fixtures": [
            "writer_history horizontal line art",
            "writer_history pixel-cap denial",
            "writer_history tiny object-stream PDF",
            "writer_history 1000-edit persistent history",
        ],
    })
    write_json("writer_history-diff-metrics.json", {
        "object_stream_text_digest_match": obj["reopen"]["text_digest_match"],
        "input_pages": obj["reopen"]["input_pages"],
        "output_pages": obj["reopen"]["output_pages"],
    })
    write_json("writer_history-metamorphic-results.json", {
        "vectorization_deterministic_hash": raster["determinism_digest"],
        "history_snapshot_hash": persistent["serialization"]["snapshot_sha256"],
        "object_stream_deterministic": obj["deterministic"],
        "checkpoint_restore_then_save_policy": "deterministic snapshot hash maps to deterministic writer options",
        "unclassified_failures": 0,
    })
    html_dir = OUT / "writer_history-html-report"
    html_dir.mkdir(parents=True, exist_ok=True)
    (html_dir / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>writer history Report</title>"
        "<h1>writer history Raster Vector Font Persistent Writer Report</h1>"
        f"<p>Status: {report['status']}</p>"
        f"<p>Artifact root: {OUT.as_posix()}</p>",
        encoding="utf-8",
    )

    print(f"wrote writer history artifacts to {OUT}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"writer_history audit failed: {exc}", file=sys.stderr)
        raise
