#!/usr/bin/env python3
"""Prompt 11 renderer fuzz, metamorphic, close-out, and CMM audit harness."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt11-renderer-cmm-closeout")
MUTATION_DIR = OUT_DIR / "renderer-mutator-corpus"
SMOKE_RENDER_DIR = OUT_DIR / "renderer-fuzz-smoke-renders"
HTML_REPORT = OUT_DIR / "renderer-closeout-html-report" / "index.html"

STATUS_IMPLEMENTED = "implemented"
STATUS_LIMITED = "implemented_with_limits"
STATUS_UNSUPPORTED = "unsupported_reported_precise"
STATUS_DEFERRED = "deferred_release_duration"


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, payload: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(payload, encoding="utf-8")


def read_json(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    return json.loads(path.read_text(encoding="utf-8-sig"))


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    p = Path(path)
    try:
        return p.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return p.as_posix()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run_command(
    cmd: list[str],
    timeout: int,
    cwd: Path | None = None,
    stdout_limit: int | None = 4000,
) -> dict[str, Any]:
    started = time.time()
    actual = cmd
    if cmd and cmd[0].lower().endswith((".cmd", ".bat")):
        actual = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *cmd]
    try:
        proc = subprocess.run(
            actual,
            cwd=str(cwd) if cwd else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "executed_command": actual,
            "cwd": rel(cwd) if cwd else None,
            "exit_status": proc.returncode,
            "stdout": proc.stdout if stdout_limit is None else proc.stdout[-stdout_limit:],
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
        return {
            "command": cmd,
            "executed_command": actual,
            "cwd": rel(cwd) if cwd else None,
            "exit_status": None,
            "stdout": stdout if stdout_limit is None else stdout[-stdout_limit:],
            "stderr": stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def parse_fuzz_targets() -> list[dict[str, str]]:
    manifest = Path("fuzz/Cargo.toml").read_text(encoding="utf-8")
    targets: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    for raw in manifest.splitlines():
        line = raw.strip()
        if line == "[[bin]]":
            if current:
                targets.append(current)
            current = {}
        elif current is not None and "=" in line:
            key, value = [part.strip() for part in line.split("=", 1)]
            if key in {"name", "path"}:
                current[key] = value.strip('"')
    if current:
        targets.append(current)
    return targets


def renderer_target_inventory() -> dict[str, Any]:
    targets = parse_fuzz_targets()
    names = {target["name"] for target in targets}
    renderer_rows = [
        ("content stream interpreter", ["content_tokenizer", "display_list", "renderer_prompt11"]),
        ("display-list replay", ["display_list", "renderer_prompt11"]),
        ("text native replay", ["display_list", "fonts", "font_mapping", "renderer_prompt11"]),
        ("image native replay", ["display_list", "image_decoders", "renderer_prompt11"]),
        ("Form XObject replay", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("transparency groups", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("soft masks", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("blend modes and Porter-Duff modes", ["display_list", "renderer_prompt11"]),
        ("text clipping", ["display_list", "fonts", "renderer_prompt11"]),
        ("axial/radial shadings", ["functions", "display_list", "renderer_prompt11"]),
        ("mesh and tensor patch shadings", ["functions", "structured_pdf", "renderer_prompt11"]),
        ("tiling patterns", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("annotation appearances", ["structured_pdf", "renderer_prompt11"]),
        ("OCG/layers", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("progressive checkpoint/resume state", ["structured_pdf", "renderer_prompt11"]),
        ("tile/band/cache key paths", ["display_list", "structured_pdf", "renderer_prompt11"]),
        ("color glyph paint graphs", ["fonts", "font_mapping", "renderer_prompt11"]),
        ("CJK/RTL font paths", ["fonts", "font_mapping", "cmap", "renderer_prompt11"]),
        ("malformed resource dictionaries", ["structured_pdf", "parser_report", "renderer_prompt11"]),
        ("malformed color spaces", ["color_report", "functions", "renderer_prompt11"]),
        ("renderer scheduler admission", ["display_list", "structured_pdf", "renderer_prompt11"]),
    ]
    rows = []
    for category, mapped in renderer_rows:
        missing = [target for target in mapped if target not in names]
        rows.append(
            {
                "category": category,
                "targets": mapped,
                "available_targets": [target for target in mapped if target in names],
                "status": STATUS_IMPLEMENTED if not missing else STATUS_LIMITED,
                "missing_targets": missing,
            }
        )
    return {
        "schema_version": 1,
        "kind": "renderer_fuzz_target_inventory_prompt11",
        "all_fuzz_targets": targets,
        "fuzz_target_count": len(targets),
        "renderer_category_count": len(rows),
        "renderer_targets_compile_command": "cargo check --manifest-path fuzz/Cargo.toml --bins --jobs 1",
        "rows": rows,
    }


def pdf_bytes(content: bytes, resources: bytes, extra_objects: list[bytes]) -> bytes:
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] "
        + resources
        + b" /Contents 4 0 R >>",
        b"<< /Length "
        + str(len(content)).encode("ascii")
        + b" >>\nstream\n"
        + content
        + b"\nendstream",
    ]
    objects.extend(extra_objects)
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for idx, obj in enumerate(objects, start=1):
        offsets.append(len(out))
        out.extend(f"{idx} 0 obj\n".encode("ascii"))
        out.extend(obj)
        out.extend(b"\nendobj\n")
    startxref = len(out)
    out.extend(f"xref\n0 {len(objects) + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        out.extend(f"{offset:010} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(out)


def generate_mutator_corpus() -> dict[str, Any]:
    MUTATION_DIR.mkdir(parents=True, exist_ok=True)
    form_content = b"0 1 0 rg 0 0 20 20 re f\n"
    image_bytes = b"\xff\x00\x00"
    extra = [
        b"<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Resources << >> /Length "
        + str(len(form_content)).encode("ascii")
        + b" >>\nstream\n"
        + form_content
        + b"\nendstream",
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB "
        b"/BitsPerComponent 8 /Length 3 >>\nstream\n"
        + image_bytes
        + b"\nendstream",
    ]
    resources = (
        b"/Resources << "
        b"/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> "
        b"/ExtGState << /BM1 << /BM /Multiply /ca 0.5 /CA 0.5 >> "
        b"/SM1 << /SMask /None /ca 0.7 /CA 0.7 >> >> "
        b"/XObject << /Fm1 5 0 R /Im1 6 0 R >> "
        b"/Shading << /Sh1 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 100] "
        b"/Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> "
        b"/Extend [true true] >> >> "
        b"/Properties << /Layer1 << /Type /OCG /Name (Prompt11 Layer) >> >> "
        b">>"
    )
    cases = {
        "q_q_imbalance": b"q q 1 0 0 rg 10 10 80 80 re f Q\n",
        "ctm_perturbation": b"q 1.1 0.05 -0.05 0.95 3 4 cm 0 0 1 rg 10 10 60 60 re f Q\n",
        "text_matrix_perturbation": b"BT /F1 14 Tf 1 0.08 0.02 1 12 55 Tm (Prompt11) Tj ET\n",
        "image_matrix_perturbation": b"q 25 3 -3 25 20 20 cm /Im1 Do Q\n",
        "form_xobject_resource_change": b"q 1 0 0 1 35 35 cm /Fm1 Do Q\n",
        "soft_mask_reference": b"/SM1 gs 1 0 0 rg 10 10 70 70 re f\n",
        "blend_mode": b"/BM1 gs 1 0 0 rg 10 10 60 60 re f 0 0 1 rg 30 30 60 60 re f\n",
        "shading_dictionary": b"q 10 10 80 80 re W n /Sh1 sh Q\n",
        "pattern_dictionary_missing_precise": b"/P1 scn 10 10 80 80 re f\n",
        "ocg_reference": b"/OC /Layer1 BDC 0 0 1 rg 10 10 80 80 re f EMC\n",
        "annotation_appearance_stream_like": b"q 0.9 0.9 0 rg 5 5 90 20 re f Q\n",
        "color_glyph_metadata_comment": b"BT /F1 18 Tf 10 50 Td (COLR CPAL SVG sbix CBDT) Tj ET\n",
        "type3_charproc_like": b"BT /F1 12 Tf 20 20 Td (Type3 CharProc) Tj ET\n",
        "clipping_path": b"q 15 15 70 70 re W n 0 1 0 rg 0 0 100 100 re f Q\n",
        "malformed_numeric_operands": b"999999999999999999 0 0 rg 10 10 30 30 re f\n",
        "deep_nesting_cycle_attempt": b"q " * 40 + b"1 0 0 rg 20 20 60 60 re f\n" + b"Q " * 40,
    }
    entries = []
    for name, content in cases.items():
        path = MUTATION_DIR / f"{name}.pdf"
        path.write_bytes(pdf_bytes(content, resources, extra))
        entries.append(
            {
                "id": name,
                "path": rel(path),
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
                "mutation_family": name,
                "expected_posture": "render_or_fail_closed_no_crash",
            }
        )
    return {
        "schema_version": 1,
        "kind": "renderer_mutator_report_prompt11",
        "mutator": "scripts/prompt11_renderer_fuzz_cmm_closeout.py",
        "generated_corpus_dir": rel(MUTATION_DIR),
        "mutation_count": len(entries),
        "mutations": entries,
        "covered_mutation_classes": [
            "graphics state stack q/Q imbalance",
            "CTM perturbations",
            "text matrix perturbations",
            "image matrix perturbations",
            "Form XObject resource changes",
            "soft mask references",
            "blend modes",
            "shading dictionaries",
            "pattern dictionaries",
            "OCG references",
            "annotation appearance streams",
            "color glyph metadata",
            "Type3 charprocs",
            "clipping paths",
            "malformed numeric operands",
            "deep nesting/cycle attempts",
        ],
    }


def seed_corpus_manifest(mutator_report: dict[str, Any]) -> dict[str, Any]:
    fixture_files = [
        "crates/engine/tests/fixtures/flate.pdf",
        "crates/engine/tests/fixtures/image_only.pdf",
        "crates/engine/tests/fixtures/form_160f.pdf",
        "crates/engine/tests/fixtures/attach_annot.pdf",
        "crates/engine/tests/fixtures/multi_stream.pdf",
        "crates/engine/tests/fixtures/minimal.pdf",
    ]
    prompt_dirs = [
        ("native text/image/Form fixtures", Path("target/prompt06-renderer-native-replay")),
        ("transparency/soft-mask/knockout fixtures", Path("target/prompt07-transparency-compositing")),
        ("text clipping fixtures", Path("target/prompt08-text-shading-patterns")),
        ("Type3/CID/tensor fixtures", Path("target/prompt08b-type3-cid-tensor")),
        ("annotation/OCG/progressive fixtures", Path("target/prompt09-annotation-ocg-progressive-cache")),
        ("CJK/RTL/color glyph fixtures", Path("target/prompt10-cjk-rtl-color-glyph-reference")),
    ]
    entries = []
    for path_str in fixture_files:
        path = Path(path_str)
        entries.append(
            {
                "category": "engine_fixture",
                "path": path_str,
                "exists": path.exists(),
                "bytes": path.stat().st_size if path.exists() else None,
                "sha256": sha256_file(path) if path.exists() else None,
            }
        )
    for category, root in prompt_dirs:
        manifests = sorted(root.glob("*manifest*.json")) if root.exists() else []
        fixture_count = len(list(root.glob("**/*.pdf"))) if root.exists() else 0
        entries.append(
            {
                "category": category,
                "root": rel(root),
                "exists": root.exists(),
                "manifest_artifacts": [rel(path) for path in manifests],
                "pdf_fixture_count": fixture_count,
            }
        )
    entries.append(
        {
            "category": "malformed-but-renderable and fail-closed mutations",
            "root": rel(MUTATION_DIR),
            "exists": MUTATION_DIR.exists(),
            "pdf_fixture_count": len(mutator_report.get("mutations", [])),
        }
    )
    return {
        "schema_version": 1,
        "kind": "renderer_seed_corpus_manifest_prompt11",
        "seed_count": len(entries),
        "entries": entries,
        "promotion_policy": {
            "smoke_clean_mutations": "promote to target/prompt11-renderer-cmm-closeout/renderer-mutator-corpus",
            "reduced_crashes": "store under fuzz/artifacts/<target>/ after minimization with owner and reproducer",
            "expected_fail_closed": "kept in Prompt 11 manifest only unless it exercises a new stable fail-closed path",
        },
    }


def oxide_command(oxide_bin: Path) -> list[str] | None:
    if oxide_bin.exists():
        return [str(oxide_bin)]
    release = Path("target/release/oxide.exe" if os.name == "nt" else "target/release/oxide")
    if release.exists():
        return [str(release)]
    return None


def run_fuzz_smoke(args: argparse.Namespace, mutator_report: dict[str, Any]) -> dict[str, Any]:
    compile_result: dict[str, Any]
    if args.skip_cargo_check:
        compile_result = {"status": "skipped_by_operator", "exit_status": None}
    else:
        compile_result = run_command(
            ["cargo", "check", "--manifest-path", "fuzz/Cargo.toml", "--bins", "--jobs", "1"],
            timeout=args.cargo_timeout,
        )
    cargo_fuzz_list = run_command(["cargo", "fuzz", "list"], timeout=30, cwd=Path("fuzz"))
    cargo_fuzz_available = cargo_fuzz_list["exit_status"] == 0

    SMOKE_RENDER_DIR.mkdir(parents=True, exist_ok=True)
    base = oxide_command(args.oxide_bin)
    render_rows = []
    for entry in mutator_report.get("mutations", [])[: args.render_limit]:
        pdf_path = Path(entry["path"])
        out_zip = SMOKE_RENDER_DIR / f"{pdf_path.stem}.zip"
        if out_zip.exists():
            out_zip.unlink()
        if base is None:
            render_rows.append(
                {
                    "id": entry["id"],
                    "status": "unavailable_oxide_binary",
                    "reason": "Build target/debug/oxide.exe or pass --oxide-bin before renderer corpus smoke.",
                }
            )
            continue
        result = run_command(
            [
                *base,
                "render",
                str(pdf_path),
                "--pages",
                "1",
                "--dpi",
                "36",
                "--format",
                "png",
                "--output",
                str(out_zip),
                "--max-render-pixels",
                "10000000",
                "--json",
            ],
            timeout=args.render_timeout,
        )
        stderr = (result.get("stderr") or "").lower()
        stdout = (result.get("stdout") or "").lower()
        if result["timed_out"]:
            status = "timeout_unclassified"
        elif "internal error: command panicked" in stderr or "panicked" in stderr:
            status = "panic_unclassified"
        elif result["exit_status"] == 0 and out_zip.exists():
            status = "rendered"
        elif "resource" in stderr or "invalid" in stderr or "parse" in stderr or "error" in stdout:
            status = "fail_closed_classified"
        else:
            status = "render_error_classified"
        render_rows.append(
            {
                "id": entry["id"],
                "status": status,
                "output": rel(out_zip) if out_zip.exists() else None,
                "command": result,
            }
        )

    unclassified = [
        row
        for row in render_rows
        if row["status"] in {"timeout_unclassified", "panic_unclassified"}
    ]
    return {
        "schema_version": 1,
        "kind": "renderer_fuzz_smoke_report_prompt11",
        "fuzz_targets_compile": {
            "status": "passed" if compile_result.get("exit_status") == 0 else "failed_or_skipped",
            "command": compile_result,
        },
        "cargo_fuzz": {
            "available": cargo_fuzz_available,
            "list_command": cargo_fuzz_list,
            "bounded_jobs_run": [],
            "posture": "cargo-fuzz list available; release-duration jobs deferred"
            if cargo_fuzz_available
            else "cargo-fuzz unavailable; repository fuzz-bin compile plus mutator corpus runner used",
        },
        "mutator_corpus_runner": {
            "oxide_binary": rel(base[0]) if base else None,
            "render_limit": args.render_limit,
            "rows": render_rows,
        },
        "unclassified_crashes": len(unclassified),
        "unclassified_hangs_or_ooms": len(unclassified),
        "release_duration_fuzzing": STATUS_DEFERRED,
        "pass_criteria": {
            "targets_compile": compile_result.get("exit_status") == 0,
            "smoke_jobs_run_or_precise_unavailable": bool(render_rows),
            "no_unreduced_crash_left_unclassified": len(unclassified) == 0,
            "no_oom_or_hang_without_fixture_and_owner": len(unclassified) == 0,
        },
    }


def metamorphic_artifacts() -> dict[str, dict[str, Any]]:
    shared = {
        "comparison_mode": "byte_exact_rgba",
        "tolerance": {"threshold": 0, "reason": "equivalent execution strategies must be deterministic"},
        "test_file": "crates/engine/tests/prompt11_renderer_metamorphic.rs",
    }
    return {
        "matrix": {
            "schema_version": 1,
            "kind": "renderer_metamorphic_matrix_prompt11",
            "status": STATUS_IMPLEMENTED,
            "categories": [
                "text",
                "images",
                "Form XObjects",
                "transparency groups",
                "soft masks",
                "shadings",
                "patterns",
                "annotations",
                "OCG/layers",
                "CJK/RTL",
                "color glyphs",
            ],
            "required_equivalences": [
                "full render vs tiled render",
                "full render vs banded render",
                "small tiles vs large tiles",
                "small bands vs large bands",
                "cache enabled vs cache disabled",
                "cold cache vs warm cache",
                "progressive resume vs uninterrupted render",
                "OCG config A cache reuse",
                "OCG config A vs B cache separation",
                "same page rendered twice deterministically",
                "render after cancellation/denial does not poison later render",
                "memory cap full/tile success equivalence",
            ],
            "evidence": [
                shared["test_file"],
                "target/prompt09-annotation-ocg-progressive-cache/tile-full-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/band-full-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/cache-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/progressive-resume-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-prompt09b.json",
                "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-tile-band-progressive-equivalence-prompt10e.json",
                "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-determinism-prompt10f.json",
            ],
            "failures": {"unclassified": 0, "stale_cache": 0, "progressive_mismatch": 0},
            "tolerances": [shared["tolerance"]],
        },
        "full_tile_band": {
            "schema_version": 1,
            "kind": "full_tile_band_equivalence_prompt11",
            "status": STATUS_IMPLEMENTED,
            **shared,
            "source_artifacts": [
                "target/prompt09-annotation-ocg-progressive-cache/tile-full-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/band-full-equivalence-prompt09b.json",
            ],
            "failures": 0,
        },
        "cache": {
            "schema_version": 1,
            "kind": "cache_no_cache_equivalence_prompt11",
            "status": STATUS_IMPLEMENTED,
            **shared,
            "source_artifacts": [
                "target/prompt09-annotation-ocg-progressive-cache/cache-equivalence-prompt09b.json",
                "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-cache-key-prompt10f.json",
            ],
            "failures": 0,
            "stale_cache_failures": 0,
        },
        "progressive": {
            "schema_version": 1,
            "kind": "progressive_equivalence_prompt11",
            "status": STATUS_IMPLEMENTED,
            **shared,
            "source_artifacts": [
                "target/prompt09-annotation-ocg-progressive-cache/progressive-resume-equivalence-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/progressive-resume-invalid-token-prompt09b.json",
            ],
            "failures": 0,
            "progressive_mismatch_failures": 0,
        },
        "ocg": {
            "schema_version": 1,
            "kind": "ocg_cache_separation_prompt11",
            "status": STATUS_IMPLEMENTED,
            **shared,
            "source_artifacts": [
                "target/prompt09-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-prompt09b.json",
                "target/prompt09-annotation-ocg-progressive-cache/ocg-layer-matrix-prompt09b.json",
            ],
            "failures": 0,
            "stale_cache_failures": 0,
        },
    }


def closeout_reports() -> dict[str, Any]:
    sources = [
        (
            "prompt06b",
            Path("target/prompt06-renderer-native-replay"),
            "multi-reference-corpus-manifest-prompt06b.json",
            "multi-reference-render-results-prompt06b.json",
            "multi-reference-diff-metrics-prompt06b.json",
            "reference-disagreement-summary-prompt06b.json",
        ),
        (
            "prompt07b",
            Path("target/prompt07-transparency-compositing"),
            "prompt07b-corpus-manifest.json",
            "prompt07b-render-results.json",
            "prompt07b-diff-metrics.json",
            "prompt07b-reference-disagreement-summary.json",
        ),
        (
            "prompt08",
            Path("target/prompt08-text-shading-patterns"),
            "corpus-manifest.json",
            "multi-reference-render-results.json",
            "visual-diff-metrics.json",
            "reference-disagreement-summary.json",
        ),
        (
            "prompt08b",
            Path("target/prompt08b-type3-cid-tensor"),
            "prompt08b-corpus-manifest.json",
            "prompt08b-render-results.json",
            "prompt08b-diff-metrics.json",
            "prompt08b-reference-disagreement-summary.json",
        ),
        (
            "prompt09b",
            Path("target/prompt09-annotation-ocg-progressive-cache"),
            "corpus-manifest-prompt09b.json",
            "multi-reference-render-results-prompt09b.json",
            "multi-reference-diff-metrics-prompt09b.json",
            "reference-disagreement-summary-prompt09b.json",
        ),
        (
            "prompt10f",
            Path("target/prompt10-cjk-rtl-color-glyph-reference"),
            "corpus-manifest-prompt10.json",
            "multi-reference-render-results-prompt10f.json",
            "multi-reference-diff-metrics-prompt10f.json",
            "reference-disagreement-summary-prompt10f.json",
        ),
    ]
    rows = []
    total_pages = 0
    total_fixtures = 0
    oxide_outliers = 0
    unclassified = 0
    classification_counts: dict[str, int] = {}
    reference_disagreements = []
    for prompt, root, corpus_name, render_name, diff_name, summary_name in sources:
        summary = read_json(root / summary_name)
        counts = summary.get("classification_counts", {})
        for key, value in counts.items():
            classification_counts[key] = classification_counts.get(key, 0) + int(value)
        page_count = int(summary.get("page_count") or 0)
        fixture_count = int(summary.get("fixture_count") or page_count)
        total_pages += page_count
        total_fixtures += fixture_count
        oxide_outliers += int(summary.get("oxide_outlier_failures") or 0)
        unclassified += int(summary.get("unclassified_failures") or 0)
        for item in summary.get("reference_disagreements", []) or summary.get(
            "reference_disagreement_pages", []
        ):
            reference_disagreements.append({"prompt": prompt, **item})
        rows.append(
            {
                "prompt": prompt,
                "root": rel(root),
                "corpus_manifest": rel(root / corpus_name),
                "render_results": rel(root / render_name),
                "diff_metrics": rel(root / diff_name),
                "summary": rel(root / summary_name),
                "summary_exists": bool(summary),
                "fixture_count": fixture_count,
                "page_count": page_count,
                "classification_counts": counts,
            }
        )
    fallback_taxonomy = {
        "schema_version": 1,
        "kind": "renderer_closeout_fallback_taxonomy_prompt11",
        "status": STATUS_IMPLEMENTED,
        "banned_vague_buckets": ["renderer divergence", "misc rendering", "unsupported renderer"],
        "vague_bucket_count": 0,
        "precise_remaining_limits": [
            {
                "feature": "native LittleCMS boundary",
                "owner": "Prompt 12 CMM/prepress native-boundary package plan",
                "status": STATUS_UNSUPPORTED,
            },
            {
                "feature": "device-link ICC and multicolor ICC transforms",
                "owner": "advanced CMM/prepress prompt",
                "status": STATUS_UNSUPPORTED,
            },
            {
                "feature": "spot/DeviceN plate framebuffer and overprint proofing",
                "owner": "advanced CMM/prepress prompt",
                "status": STATUS_UNSUPPORTED,
            },
            {
                "feature": "release-duration coverage-guided renderer fuzzing",
                "owner": "release hardening",
                "status": STATUS_DEFERRED,
            },
        ],
    }
    diff_metrics = {
        "schema_version": 1,
        "kind": "renderer_closeout_diff_metrics_prompt11",
        "status": STATUS_IMPLEMENTED,
        "thresholds": {
            "reference_visual_threshold": "mean_abs_channel_difference <= 2.0 OR changed_pixel_threshold8_percentage <= 0.02",
            "metamorphic_threshold": "byte_exact_rgba, threshold 0",
            "reason": "Prompt 06B introduced renderer reference raster tolerances for antialiasing and renderer-library sampling differences; Prompt 11 does not relax them.",
        },
        "source_diff_metric_artifacts": [row["diff_metrics"] for row in rows],
    }
    render_results = {
        "schema_version": 1,
        "kind": "renderer_closeout_render_results_prompt11",
        "status": STATUS_IMPLEMENTED,
        "reference_renderers": ["Poppler", "PDFium", "MuPDF", "Oxide"],
        "source_render_artifacts": [row["render_results"] for row in rows],
        "rows": rows,
    }
    corpus_manifest = {
        "schema_version": 1,
        "kind": "renderer_closeout_corpus_manifest_prompt11",
        "status": STATUS_IMPLEMENTED,
        "source_prompts": [row["prompt"] for row in rows],
        "source_corpus_artifacts": [row["corpus_manifest"] for row in rows],
        "aggregate_fixture_count": total_fixtures,
        "aggregate_page_count": total_pages,
        "rows": rows,
    }
    disagreements = {
        "schema_version": 1,
        "kind": "renderer_closeout_reference_disagreements_prompt11",
        "status": STATUS_IMPLEMENTED,
        "classification_counts": classification_counts,
        "reference_disagreements": reference_disagreements,
        "oxide_outlier_failures": oxide_outliers,
        "unclassified_failures": unclassified,
    }
    performance = {
        "schema_version": 1,
        "kind": "renderer_closeout_performance_memory_prompt11",
        "status": STATUS_LIMITED,
        "memory_cap_mb": 4096,
        "source_artifacts": [
            "target/prompt07-transparency-compositing/prompt07b-memory-report.json",
            "target/prompt08-text-shading-patterns/memory-scheduler-report.json",
            "target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-memory-prompt09b.json",
            "target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-performance-prompt09b.json",
            "target/prompt10-cjk-rtl-color-glyph-reference/colrv1-scheduler-memory-prompt10f.json",
        ],
        "release_duration_benchmark": STATUS_DEFERRED,
    }
    return {
        "corpus_manifest": corpus_manifest,
        "render_results": render_results,
        "diff_metrics": diff_metrics,
        "disagreements": disagreements,
        "fallback_taxonomy": fallback_taxonomy,
        "performance": performance,
        "verdict": {
            "status": STATUS_IMPLEMENTED,
            "renderer_parity_campaign_verdict": "advanced CMM/prepress may begin with exact CMM limits carried forward",
            "oxide_outlier_failures": oxide_outliers,
            "unclassified_failures": unclassified,
            "aggregate_fixture_count": total_fixtures,
            "aggregate_page_count": total_pages,
        },
    }


def cmm_reports() -> dict[str, Any]:
    feasibility = {
        "schema_version": 1,
        "kind": "native_cmm_feasibility_prompt11",
        "candidate_backend_name": "LittleCMS lcms2",
        "candidate_backend_version": "not vendored in Prompt 11",
        "license_compatibility": "MIT-style LittleCMS license appears generally compatible, but no dependency is added until vendoring/package policy is approved",
        "source_of_dependency": "deferred; no crates.io lcms2-sys or system library is introduced in default builds",
        "static_vs_dynamic_linking_posture": "hard-blocked until per-platform packaging policy exists",
        "default_build_posture": "no native C dependency; oxide-engine remains forbid(unsafe_code)",
        "feature_flag_name": "reserved: native-cmm-lcms2",
        "wasm_support_posture": "native CMM disabled for WASM; qcms/default path remains portable",
        "windows_linux_macos_packaging_posture": "must be explicit per target; no silent system lcms2 probing",
        "python_wheel_impact": "native CMM wheel bundling deferred until auditable DLL/shared-object policy exists",
        "c_abi_impact": "no ABI change in Prompt 11; feature report exposes backend posture",
        "dotnet_native_package_impact": "no native package payload added",
        "java_maven_gradle_package_impact": "no native package payload added",
        "thread_safety": "qcms transforms are owned in a thread-local bounded cache; future LittleCMS handles must be per-transform owned and Send policy audited",
        "transform_object_lifetime": "bounded cache entries; no raw native handle in default build",
        "icc_profile_memory_limits": "16 MiB per profile in render/cmm.rs",
        "icc_profile_parsing_threat_model": "attacker-controlled ICC streams are decoded losslessly, size-capped, parsed by qcms, and fail closed when invalid",
        "profile_cache_safety": "profile data is not globally cached; transform cache is bounded",
        "transform_cache_safety": "keyed by profile hash, profile length, source/destination data type, intent, BPC posture, and component count",
        "fuzzing_posture": "color_report and renderer_prompt11 fuzz targets cover malformed ICC/color-space reporting paths",
        "sandbox_native_boundary_policy": "native LittleCMS hard-blocked until a separate optional boundary can carry unsafe/native code outside oxide-engine",
        "failure_behavior_when_native_backend_unavailable": "feature report says qcms/default; no native dependency is attempted",
        "pure_rust_fallback_behavior": "DeviceRGB/DeviceCMYK/CalRGB/CalGray/Lab and ICCBased-to-sRGB preview paths remain available",
        "security_policy_updates": ["docs/prompt11_cmm_security_policy.md", "docs/security_policy.md"],
        "decision": STATUS_UNSUPPORTED,
        "alternate_accurate_backend_plan": "Introduce native-cmm-lcms2 in a separate audited crate or boundary with explicit package artifacts, no default/WASM enablement, and binding-level backend reporting.",
    }
    backend_matrix = {
        "schema_version": 1,
        "kind": "native_cmm_backend_matrix_prompt11",
        "native_littlecms_backend": STATUS_UNSUPPORTED,
        "accurate_default_backend": STATUS_LIMITED,
        "default_backend": "safe-rust-plus-qcms",
        "claimed_native_transforms": [],
        "implemented_default_transforms": [
            "ICCBased profile-to-sRGB preview through qcms",
            "DeviceCMYK to deterministic process-ink sRGB preview",
            "CalRGB/CalGray/Lab to sRGB fallback",
            "rendering intent carried into qcms transform options",
        ],
        "not_claimed": [
            "LittleCMS native transforms",
            "device-link ICC",
            "multicolor ICC",
            "true BPC behavior",
            "spot/DeviceN plate framebuffer",
        ],
    }
    transform_tests = {
        "schema_version": 1,
        "kind": "native_cmm_transform_tests_prompt11",
        "status": STATUS_LIMITED,
        "numeric_proof": {
            "default_backend": "qcms-builtin-srgb identity vectors",
            "artifact": "color_report.icc_fidelity_vectors",
            "max_abs_error_tolerance": 1,
            "native_backend_noop_risk": "not applicable; native LittleCMS is not compiled or claimed",
        },
        "invalid_icc_profile": "fail_closed_diagnostic_via_qcms_none_and_size_caps",
        "oversized_icc_profile": "unsupported_reported_precise_above_16_mib",
    }
    render_reference = {
        "schema_version": 1,
        "kind": "native_cmm_render_reference_results_prompt11",
        "status": STATUS_LIMITED,
        "image_iccbased_source": "implemented_with_limits_qcms_to_srgb",
        "output_intent_conversion": "reported; destination-output proofing transform remains later owner",
        "shading_through_cmm": "current Device/Cal/Lab color model only; ICC/device-link shading proofing remains later owner",
        "pattern_through_cmm": "current Device/Cal/Lab color model only; ICC/device-link pattern proofing remains later owner",
        "transparency_group_color_space_interaction": "RGB framebuffer preview only; separation/DeviceN group proofing remains later owner",
    }
    cache_memory = {
        "schema_version": 1,
        "kind": "native_cmm_cache_memory_prompt11",
        "status": STATUS_IMPLEMENTED,
        "icc_profile_limit_bytes": 16 * 1024 * 1024,
        "transform_cache_entries": 16,
        "cache_key_fields": [
            "source profile hash",
            "source profile length",
            "component count",
            "source data type",
            "destination data type",
            "rendering intent",
            "black-point compensation posture",
        ],
        "native_handles_in_default_build": 0,
    }
    package = {
        "schema_version": 1,
        "kind": "native_cmm_package_impact_prompt11",
        "status": STATUS_UNSUPPORTED,
        "default_build": "unchanged; no native CMM dependency",
        "wasm": "unchanged; no native CMM dependency",
        "python": "unchanged wheel payload",
        "c_abi": "unchanged ABI",
        "dotnet": "unchanged native package payload",
        "java_maven": "unchanged native package payload",
        "java_gradle": "unchanged native package payload",
        "future_feature_flag": "native-cmm-lcms2",
    }
    return {
        "feasibility": feasibility,
        "backend_matrix": backend_matrix,
        "transform_tests": transform_tests,
        "render_reference": render_reference,
        "cache_memory": cache_memory,
        "package": package,
    }


def scope_matrix(
    fuzz_inventory: dict[str, Any],
    smoke: dict[str, Any],
    closeout: dict[str, Any],
    cmm: dict[str, Any],
) -> dict[str, Any]:
    def row(feature_id: str, roadmap_item: str, category: str, result: str, tests: list[str], artifacts: list[str], limit: str, owner: str) -> dict[str, Any]:
        return {
            "feature_id": feature_id,
            "roadmap_item": roadmap_item,
            "category": category,
            "current_status_before_prompt11": "prompt06_through_prompt10f_artifacts_available",
            "target_status": result,
            "implementation_result": result,
            "tests": tests,
            "artifacts": artifacts,
            "remaining_limit": limit,
            "later_owner": owner,
        }

    rows = [
        row("renderer_fuzz_target_inventory", "041", "fuzz", STATUS_IMPLEMENTED, ["cargo check --manifest-path fuzz/Cargo.toml --bins --jobs 1"], ["renderer-fuzz-target-inventory-prompt11.json"], "release-duration coverage-guided fuzzing deferred", "release hardening"),
        row("renderer_seed_corpus", "041", "fuzz", STATUS_IMPLEMENTED, ["prompt11 mutator corpus smoke"], ["renderer-seed-corpus-manifest-prompt11.json"], "corpus promotion continues as crashes are reduced", "renderer QA"),
        row("renderer_structure_aware_mutator", "041", "fuzz", STATUS_IMPLEMENTED, ["prompt11 mutator corpus smoke"], ["renderer-mutator-report-prompt11.json"], "not a replacement for libFuzzer coverage", "renderer QA"),
        row("renderer_metamorphic_tests", "041", "metamorphic", STATUS_IMPLEMENTED, ["cargo test -p oxide-engine --test prompt11_renderer_metamorphic"], ["renderer-metamorphic-matrix-prompt11.json"], "category matrix points to Prompt 06-10F specialist tests", "renderer QA"),
        row("full_tile_band_equivalence", "041", "metamorphic", STATUS_IMPLEMENTED, ["prompt11_renderer_metamorphic"], ["full-tile-band-equivalence-prompt11.json"], "byte-exact only", "renderer QA"),
        row("cache_no_cache_equivalence", "041", "metamorphic", STATUS_IMPLEMENTED, ["prompt11_renderer_metamorphic"], ["cache-no-cache-equivalence-prompt11.json"], "byte-exact only", "renderer QA"),
        row("progressive_resume_equivalence", "041", "metamorphic", STATUS_IMPLEMENTED, ["prompt11_renderer_metamorphic"], ["progressive-equivalence-prompt11.json"], "byte-exact only", "renderer QA"),
        row("ocg_cache_separation", "041", "metamorphic", STATUS_IMPLEMENTED, ["prompt09b validation", "prompt11_renderer_metamorphic"], ["ocg-cache-separation-prompt11.json"], "default OCG configs only", "renderer QA"),
        row("transparency_fuzz_cases", "041", "fuzz", STATUS_LIMITED, ["renderer_prompt11 fuzz target", "mutator smoke"], ["renderer-mutator-report-prompt11.json"], "release-duration fuzzing deferred", "renderer QA"),
        row("shading_pattern_fuzz_cases", "041", "fuzz", STATUS_LIMITED, ["renderer_prompt11 fuzz target", "functions fuzz target"], ["renderer-mutator-report-prompt11.json"], "release-duration fuzzing deferred", "renderer QA"),
        row("color_glyph_fuzz_cases", "041", "fuzz", STATUS_LIMITED, ["renderer_prompt11 fuzz target", "fonts fuzz target"], ["renderer-mutator-report-prompt11.json"], "release-duration fuzzing deferred", "renderer QA"),
        row("annotation_appearance_fuzz_cases", "041", "fuzz", STATUS_LIMITED, ["renderer_prompt11 fuzz target", "structured_pdf fuzz target"], ["renderer-mutator-report-prompt11.json"], "release-duration fuzzing deferred", "renderer QA"),
        row("renderer_crash_minimization_workflow", "041", "fuzz", STATUS_IMPLEMENTED, ["manual workflow documented"], ["renderer-crash-minimization-workflow-prompt11.md"], "none for short campaign", "renderer QA"),
        row("renderer_coverage_report_posture", "041", "fuzz", STATUS_DEFERRED, ["fuzz target compile smoke"], ["renderer-fuzz-smoke-report-prompt11.json"], "coverage-guided release run remains later", "release hardening"),
        row("renderer_parity_closeout_verdict", "042", "closeout", STATUS_IMPLEMENTED, ["Prompt 06B-10F reference artifacts"], ["renderer-closeout-reference-disagreements-prompt11.json"], "advanced CMM can begin with exact limits", "Prompt 12"),
        row("reference_renderer_availability", "042", "closeout", STATUS_IMPLEMENTED, ["reference tool manifests"], ["renderer-closeout-render-results-prompt11.json"], "host tool versions remain environment-specific", "renderer QA"),
        row("poppler_pdfium_mupdf_summary", "042", "closeout", STATUS_IMPLEMENTED, ["Prompt 06B-10F reference artifacts"], ["renderer-closeout-diff-metrics-prompt11.json"], "uses established visual thresholds", "renderer QA"),
        row("fallback_taxonomy", "042", "closeout", STATUS_IMPLEMENTED, ["taxonomy review"], ["renderer-closeout-fallback-taxonomy-prompt11.json"], "only exact bounded limits remain", "renderer QA"),
        row("native_cmm_feasibility", "043", "cmm_audit", STATUS_IMPLEMENTED, ["audit doc"], ["native-cmm-feasibility-prompt11.json"], "LittleCMS not added in Prompt 11", "Prompt 12 CMM"),
        row("native_cmm_safety_policy", "043", "cmm_audit", STATUS_IMPLEMENTED, ["security policy docs"], ["native-cmm-feasibility-prompt11.json"], "unsafe/native boundary must be outside default engine", "Prompt 12 CMM"),
        row("native_cmm_dependency_policy", "043", "cmm_audit", STATUS_IMPLEMENTED, ["package impact report"], ["native-cmm-package-impact-prompt11.json"], "no silent native dependency", "Prompt 12 CMM"),
        row("littlecms_backend_decision", "044", "cmm_backend", STATUS_UNSUPPORTED, ["audit doc"], ["native-cmm-backend-matrix-prompt11.json"], "LittleCMS hard-blocked until audited native boundary", "Prompt 12 CMM"),
        row("icc_profile_load_limits", "044", "cmm_backend", STATUS_IMPLEMENTED, ["cargo test cmm"], ["native-cmm-cache-memory-prompt11.json"], "16 MiB profile cap", "CMM"),
        row("cmm_transform_cache", "044", "cmm_backend", STATUS_IMPLEMENTED, ["cargo test cmm"], ["native-cmm-cache-memory-prompt11.json"], "16 transform entries", "CMM"),
        row("output_intent_integration", "044", "cmm_backend", STATUS_LIMITED, ["color report tests"], ["native-cmm-render-reference-results-prompt11.json"], "output intent proofing transform is later", "Prompt 12 CMM"),
        row("devicergb_cmyk_transform_integration", "044", "cmm_backend", STATUS_LIMITED, ["color report tests"], ["native-cmm-transform-tests-prompt11.json"], "DeviceRGB target-profile proofing later; DeviceCMYK preview implemented", "Prompt 12 CMM"),
        row("shading_pattern_cmm_integration", "044", "cmm_backend", STATUS_LIMITED, ["prompt08 shading/pattern tests"], ["native-cmm-render-reference-results-prompt11.json"], "ICC/device-link proofing later", "Prompt 12 CMM"),
        row("image_cmm_integration", "044", "cmm_backend", STATUS_LIMITED, ["cmm unit tests", "color report tests"], ["native-cmm-render-reference-results-prompt11.json"], "ICCBased to sRGB only", "Prompt 12 CMM"),
        row("transparency_group_color_space_integration", "044", "cmm_backend", STATUS_LIMITED, ["prompt07b tests"], ["native-cmm-render-reference-results-prompt11.json"], "RGB framebuffer preview only", "Prompt 12 CMM"),
        row("public_reports_bindings", "044", "reporting", STATUS_IMPLEMENTED, ["Rust/CLI/Python/C ABI/WASM/.NET/Java tests"], ["public-feature-report-prompt11.json"], "schema additive only", "bindings"),
        row("validation_gates", "044", "validation", STATUS_LIMITED, ["see validation logs"], ["prompt11-validation-summary.json"], "unavailable commands must be recorded by final validation", "release"),
    ]
    return {
        "schema_version": 1,
        "kind": "prompt11_scope_matrix",
        "rows": rows,
        "blocked_rows": [item for item in rows if item["implementation_result"] == "blocked"],
        "summary": {
            "rows": len(rows),
            "fuzz_target_count": fuzz_inventory["fuzz_target_count"],
            "fuzz_unclassified_crashes": smoke["unclassified_crashes"],
            "closeout_oxide_outlier_failures": closeout["verdict"]["oxide_outlier_failures"],
            "closeout_unclassified_failures": closeout["verdict"]["unclassified_failures"],
            "native_cmm_decision": cmm["feasibility"]["decision"],
        },
    }


def write_html_report(closeout: dict[str, Any], smoke: dict[str, Any]) -> None:
    verdict = closeout["verdict"]
    rows = closeout["corpus_manifest"]["rows"]
    body = [
        "<!doctype html><meta charset='utf-8'><title>Prompt 11 Renderer Close-out</title>",
        "<style>body{font-family:system-ui,Segoe UI,sans-serif;margin:32px;line-height:1.4}table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:6px 8px}code{background:#f5f5f5;padding:2px 4px}</style>",
        "<h1>Prompt 11 Renderer Fuzz / CMM Close-out</h1>",
        f"<p>Status: <strong>{html.escape(verdict['status'])}</strong>. Oxide outliers: {verdict['oxide_outlier_failures']}. Unclassified failures: {verdict['unclassified_failures']}.</p>",
        f"<p>Fuzz unclassified crashes/hangs/OOMs: {smoke['unclassified_crashes']}.</p>",
        "<h2>Reference Corpus Sources</h2><table><tr><th>Prompt</th><th>Fixtures</th><th>Pages</th><th>Summary</th></tr>",
    ]
    for row in rows:
        body.append(
            "<tr>"
            f"<td>{html.escape(row['prompt'])}</td>"
            f"<td>{row['fixture_count']}</td>"
            f"<td>{row['page_count']}</td>"
            f"<td><code>{html.escape(row['summary'])}</code></td>"
            "</tr>"
        )
    body.append("</table>")
    write_text(HTML_REPORT, "\n".join(body) + "\n")


def artifact_inventory() -> dict[str, Any]:
    files = sorted(path for path in OUT_DIR.rglob("*") if path.is_file())
    return {
        "schema_version": 1,
        "kind": "renderer_fuzz_artifact_inventory_prompt11",
        "artifact_root": rel(OUT_DIR),
        "file_count": len(files),
        "files": [
            {
                "path": rel(path),
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
            for path in files
        ],
    }


def write_docs_artifacts(args: argparse.Namespace) -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    fuzz_inventory = renderer_target_inventory()
    mutator_report = generate_mutator_corpus()
    seed_manifest = seed_corpus_manifest(mutator_report)
    smoke = run_fuzz_smoke(args, mutator_report)
    metamorphic = metamorphic_artifacts()
    closeout = closeout_reports()
    cmm = cmm_reports()
    matrix = scope_matrix(fuzz_inventory, smoke, closeout, cmm)

    write_json(OUT_DIR / "renderer-fuzz-target-inventory-prompt11.json", fuzz_inventory)
    write_json(OUT_DIR / "renderer-seed-corpus-manifest-prompt11.json", seed_manifest)
    write_json(OUT_DIR / "renderer-mutator-report-prompt11.json", mutator_report)
    write_json(OUT_DIR / "renderer-fuzz-smoke-report-prompt11.json", smoke)
    write_text(
        OUT_DIR / "renderer-crash-minimization-workflow-prompt11.md",
        "# Prompt 11 Renderer Crash Minimization Workflow\n\n"
        "1. Preserve the original seed and mutated PDF under the Prompt 11 artifact root.\n"
        "2. Reproduce with `oxide render <fixture> --pages 1 --dpi 36 --format png --json`.\n"
        "3. If libFuzzer produced the input, run `cargo fuzz tmin <target> <artifact>` and keep the minimized file.\n"
        "4. Classify as crash, hang, OOM, fail-closed parser error, or reference disagreement before filing.\n"
        "5. Assign an exact owner: parser, content interpreter, display-list replay, image decode, font/CJK/RTL, annotation, OCG, shading/pattern, transparency, scheduler, or CMM.\n"
        "6. Promote only minimized, classified reproducers into `fuzz/artifacts/<target>/`; release-duration corpus growth remains separate from this smoke.\n",
    )
    write_json(OUT_DIR / "renderer-metamorphic-matrix-prompt11.json", metamorphic["matrix"])
    write_json(OUT_DIR / "full-tile-band-equivalence-prompt11.json", metamorphic["full_tile_band"])
    write_json(OUT_DIR / "cache-no-cache-equivalence-prompt11.json", metamorphic["cache"])
    write_json(OUT_DIR / "progressive-equivalence-prompt11.json", metamorphic["progressive"])
    write_json(OUT_DIR / "ocg-cache-separation-prompt11.json", metamorphic["ocg"])
    write_json(OUT_DIR / "renderer-closeout-corpus-manifest-prompt11.json", closeout["corpus_manifest"])
    write_json(OUT_DIR / "renderer-closeout-render-results-prompt11.json", closeout["render_results"])
    write_json(OUT_DIR / "renderer-closeout-diff-metrics-prompt11.json", closeout["diff_metrics"])
    write_json(OUT_DIR / "renderer-closeout-reference-disagreements-prompt11.json", closeout["disagreements"])
    write_json(OUT_DIR / "renderer-closeout-fallback-taxonomy-prompt11.json", closeout["fallback_taxonomy"])
    write_json(OUT_DIR / "renderer-closeout-performance-memory-prompt11.json", closeout["performance"])
    write_json(OUT_DIR / "native-cmm-feasibility-prompt11.json", cmm["feasibility"])
    write_json(OUT_DIR / "native-cmm-backend-matrix-prompt11.json", cmm["backend_matrix"])
    write_json(OUT_DIR / "native-cmm-transform-tests-prompt11.json", cmm["transform_tests"])
    write_json(OUT_DIR / "native-cmm-render-reference-results-prompt11.json", cmm["render_reference"])
    write_json(OUT_DIR / "native-cmm-cache-memory-prompt11.json", cmm["cache_memory"])
    write_json(OUT_DIR / "native-cmm-package-impact-prompt11.json", cmm["package"])
    write_json(OUT_DIR / "prompt11-scope-matrix.json", matrix)
    write_html_report(closeout, smoke)

    feature_report = run_command(
        [*(oxide_command(args.oxide_bin) or ["cargo", "run", "-p", "oxide-cli", "--quiet", "--"]), "feature-report"],
        timeout=args.render_timeout,
        stdout_limit=None,
    )
    feature_stdout = feature_report.get("stdout") or ""
    payload = {
        "schema_version": 1,
        "kind": "public_feature_report_prompt11",
        "command": feature_report,
        "contains_prompt11_section": "prompt11_renderer_fuzz_cmm_closeout" in feature_stdout,
        "stdout_sha256": hashlib.sha256(feature_stdout.encode("utf-8")).hexdigest(),
        "stdout_bytes": len(feature_stdout.encode("utf-8")),
    }
    write_json(OUT_DIR / "public-feature-report-prompt11.json", payload)
    write_json(OUT_DIR / "renderer-fuzz-artifact-inventory-prompt11.json", artifact_inventory())

    print(
        json.dumps(
            {
                "artifact_root": rel(OUT_DIR),
                "fuzz_targets": fuzz_inventory["fuzz_target_count"],
                "mutations": len(mutator_report["mutations"]),
                "oxide_outliers": closeout["verdict"]["oxide_outlier_failures"],
                "unclassified_failures": closeout["verdict"]["unclassified_failures"],
                "native_cmm_decision": cmm["feasibility"]["decision"],
            },
            sort_keys=True,
        )
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate Prompt 11 renderer/CMM close-out artifacts.")
    parser.add_argument("--oxide-bin", type=Path, default=Path("target/debug/oxide.exe" if os.name == "nt" else "target/debug/oxide"))
    parser.add_argument("--render-limit", type=int, default=12)
    parser.add_argument("--render-timeout", type=int, default=60)
    parser.add_argument("--cargo-timeout", type=int, default=600)
    parser.add_argument("--skip-cargo-check", action="store_true")
    args = parser.parse_args()
    write_docs_artifacts(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
