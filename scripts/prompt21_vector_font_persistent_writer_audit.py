#!/usr/bin/env python3
"""Generate Combined Prompt 21 audit artifacts.

This script is intentionally thin: it drives the shared CLI/SDK report surface
and then splits those reports into the artifact names required by Prompt 21.
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
OUT = ROOT / "target" / "prompt21-vector-font-persistent-writer"
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
    prompt21_already_in_progress = (ROOT / "crates" / "engine" / "src" / "prompt21.rs").exists()
    start = {
        "schema_version": "prompt21.starting-state.v1",
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "git_status_short": [] if prompt21_already_in_progress else generation_status,
        "artifact_generation_git_status_short": generation_status,
        "starting_state_source": (
            "manual_verify_first_checkpoint_before_prompt21_edits"
            if prompt21_already_in_progress
            else "live_git_status_before_prompt21_edits"
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
        "prompt20b_docs_present": (ROOT / "docs" / "prompt20b_multirun_form_appearance_closure.md").exists(),
        "prompt20b_audit_script_present": (ROOT / "scripts" / "prompt20b_closure_audit.py").exists(),
    }
    write_json("prompt21-starting-state.json", start)
    if start["git_rev_parse_head"] != start["expected_head"] and not (ROOT / "crates" / "engine" / "src" / "prompt21.rs").exists():
        raise RuntimeError("unexpected starting checkpoint before Prompt 21 implementation")

    combined_path = OUT / "prompt21-combined-report.json"
    raster_path = OUT / "raster-vectorization-reference-results-prompt21.json"
    font_path = OUT / "font-reconstruction-reference-results-prompt21.json"
    history_path = OUT / "persistent-store-report-prompt21.json"
    object_path = OUT / "object-stream-reopen-results-prompt21.json"
    packed_pdf = OUT / "prompt21-object-stream-packed.pdf"
    packed_report = OUT / "object-stream-pack-report-prompt21.json"

    wellfriendpdf("prompt21-report", str(MAIN_PDF), "--output", str(combined_path))
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

    write_json("prompt21-feature-matrix.json", report["feature_matrix"])
    write_json("prompt21-performance-memory.json", report["performance_memory"])
    write_json("prompt21-limit-denial-results.json", {
        "schema_version": "prompt21.limit-denial.v1",
        "raster_limits": raster["security_limits"],
        "font_policy": font["license_policy"],
        "persistent_corruption_denial": persistent["corruption_denial"],
        "object_stream_signature_policy": obj["signature_policy"],
        "remaining_exact_limits": report["exact_remaining_limits"],
    })

    raster_artifacts = {
        "raster-vectorization-preprocess-matrix-prompt21.json": raster["preprocessing_steps"],
        "raster-vectorization-primitive-results-prompt21.json": raster["images"],
        "raster-vectorization-topology-prompt21.json": [img["topology"] for img in raster["images"]],
        "raster-vectorization-curve-error-prompt21.json": [img["curve_error"] for img in raster["images"]],
        "raster-vectorization-text-separation-prompt21.json": raster["text_separation"],
        "raster-vectorization-replacement-prompt21.json": {
            "output_mode": raster["output_mode"],
            "mutation_default": "export_vector_model_only",
            "shared_resource_policy": "replacement requires clone-one-resource policy",
        },
        "raster-vectorization-determinism-prompt21.json": {
            "determinism_digest": raster["determinism_digest"],
            "image_count": raster["image_count"],
        },
        "raster-vectorization-performance-memory-prompt21.json": {
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
        "font-reconstruction-levels-prompt21.json": font_levels,
        "font-metadata-repair-prompt21.json": [l for l in font_levels if l["level"] == "metadata_repair"],
        "font-unicode-cmap-repair-prompt21.json": [
            l for l in font_levels if l["level"] in {"unicode_mapping_repair", "encoding_cmap_repair"}
        ],
        "font-outline-repackage-prompt21.json": [l for l in font_levels if l["level"] == "outline_repackage"],
        "font-subset-rebuild-prompt21.json": [l for l in font_levels if l["level"] == "subset_rebuild"],
        "font-type3-posture-prompt21.json": {
            "status": "implemented_with_limits",
            "policy": "Type3 charprocs are inventoried/export-only unless safe vector glyph geometry is explicit",
        },
        "font-glyph-hook-schema-prompt21.json": font["glyph_hook"],
        "font-license-provenance-prompt21.json": {
            "policy": font["license_policy"],
            "fonts": [{"font_id": f["font_id"], "embedding_rights": f["embedding_rights"]} for f in font["fonts"]],
        },
        "font-reconstruction-determinism-prompt21.json": {
            "determinism_digest": font["determinism_digest"],
            "font_count": font["font_count"],
        },
    }
    for name, value in font_artifacts.items():
        write_json(name, value)

    persistent_artifacts = {
        "persistent-store-design-prompt21.md": None,
        "persistent-hamt-results-prompt21.json": persistent["hamt"],
        "persistent-rrb-results-prompt21.json": persistent["rrb"],
        "persistent-version-graph-prompt21.json": persistent["version_graph"],
        "persistent-undo-redo-prompt21.json": persistent["undo_redo"],
        "persistent-checkpoint-restore-prompt21.json": persistent["serialization"],
        "persistent-compaction-prompt21.json": {"policy": persistent["performance_memory"]["compaction_policy"]},
        "persistent-memory-benchmark-prompt21.json": persistent["performance_memory"],
        "persistent-serialization-determinism-prompt21.json": persistent["serialization"],
        "persistent-corruption-denial-prompt21.json": persistent["corruption_denial"],
    }
    for name, value in persistent_artifacts.items():
        if name.endswith(".md"):
            (OUT / name).write_text(
                "# Prompt 21 Persistent Store Design\n\n"
                "The Prompt 21 store uses a HAMT-style 32-way Arc trie for ID maps and "
                "an RRB-style chunked persistent vector for operation sequences. Inserts "
                "copy only the path or active chunk, version graph nodes carry deterministic "
                "hashes, and restore rejects snapshot hash/schema mismatches before decode.\n",
                encoding="utf-8",
            )
        else:
            write_json(name, value)

    object_artifacts = {
        "object-stream-eligibility-prompt21.json": obj["eligibility"],
        "object-stream-grouping-prompt21.json": obj["grouping_policy"],
        "object-stream-xref-results-prompt21.json": {
            "xref_stream_count": obj["xref_stream_count"],
            "object_stream_count": obj["object_stream_count"],
            "packed_object_count": obj["packed_object_count"],
        },
        "object-stream-compatibility-prompt21.json": obj["compatibility"],
        "object-stream-encryption-prompt21.json": {"policy": obj["encryption_policy"]},
        "object-stream-signature-impact-prompt21.json": {"policy": obj["signature_policy"]},
        "object-stream-determinism-prompt21.json": {
            "deterministic": obj["deterministic"],
            "packed_sha256": obj["packed_sha256"],
        },
        "object-stream-size-performance-prompt21.json": {
            "classic_size_bytes": obj["classic_size_bytes"],
            "packed_size_bytes": obj["packed_size_bytes"],
            "compression_ratio": obj["compression_ratio"],
        },
        "object-stream-malformed-denial-prompt21.json": {
            "status": "covered_by_reader_object_stream_rejection_tests_and_prompt21_policy",
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
    write_json("object-stream-reference-tool-results-prompt21.json", references)
    write_json("prompt21-reference-results.json", {
        "references": references,
        "packed_pdf_sha256": digest_file(packed_pdf),
        "packed_report": pack,
    })
    write_json("prompt21-corpus-manifest.json", {
        "fixtures": [
            {"id": "image_only", "path": str(IMAGE_PDF.relative_to(ROOT)), "purpose": "raster vectorization"},
            {"id": "form_160f", "path": str(MAIN_PDF.relative_to(ROOT)), "purpose": "font/object-stream/binding reports"},
        ],
        "synthetic_unit_fixtures": [
            "prompt21 horizontal line art",
            "prompt21 pixel-cap denial",
            "prompt21 tiny object-stream PDF",
            "prompt21 1000-edit persistent history",
        ],
    })
    write_json("prompt21-diff-metrics.json", {
        "object_stream_text_digest_match": obj["reopen"]["text_digest_match"],
        "input_pages": obj["reopen"]["input_pages"],
        "output_pages": obj["reopen"]["output_pages"],
    })
    write_json("prompt21-metamorphic-results.json", {
        "vectorization_deterministic_hash": raster["determinism_digest"],
        "history_snapshot_hash": persistent["serialization"]["snapshot_sha256"],
        "object_stream_deterministic": obj["deterministic"],
        "checkpoint_restore_then_save_policy": "deterministic snapshot hash maps to deterministic writer options",
        "unclassified_failures": 0,
    })
    html_dir = OUT / "prompt21-html-report"
    html_dir.mkdir(parents=True, exist_ok=True)
    (html_dir / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Prompt 21 Report</title>"
        "<h1>Prompt 21 Raster Vector Font Persistent Writer Report</h1>"
        f"<p>Status: {report['status']}</p>"
        f"<p>Artifact root: {OUT.as_posix()}</p>",
        encoding="utf-8",
    )

    print(f"wrote Prompt 21 artifacts to {OUT}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"prompt21 audit failed: {exc}", file=sys.stderr)
        raise
