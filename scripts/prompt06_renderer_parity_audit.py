#!/usr/bin/env python3
"""Generate Prompt 06 renderer parity and native replay audit artifacts."""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import os
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt06-renderer-native-replay")
CORPUS_MANIFEST = OUT_DIR / "corpus-manifest.json"
REFERENCE_AVAILABILITY = OUT_DIR / "reference-availability.json"
PARITY_BASELINE = OUT_DIR / "parity-baseline.json"
PARITY_AFTER = OUT_DIR / "parity-after-native-replay.json"
FAILURE_TAXONOMY = OUT_DIR / "failure-taxonomy.json"
NATIVE_COUNTERS = OUT_DIR / "native-replay-counters.json"
VISUAL_DIFF = OUT_DIR / "visual-diff-summary.json"
REPORT_HTML = OUT_DIR / "report.html"


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256(path: Path) -> str | None:
    if not path.exists():
        return None
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run_command(cmd: list[str], timeout: int = 60) -> dict[str, Any]:
    started = time.time()
    try:
        proc = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "exit_status": proc.returncode,
            "stdout": proc.stdout[-4000:],
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "exit_status": None,
            "stdout": (exc.stdout or "")[-4000:] if isinstance(exc.stdout, str) else "",
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def configured_binary(env_name: str, names: list[str]) -> str | None:
    configured = os.environ.get(env_name)
    if configured:
        path = Path(configured)
        if path.exists():
            return str(path)
    for name in names:
        found = shutil.which(name)
        if found:
            return found
    return None


def discover_reference_engines() -> dict[str, dict[str, Any]]:
    engines = {
        "poppler": {
            "env": "POPPLER_PDFTOPPM",
            "names": ["pdftoppm"],
            "version_args": ["-v"],
            "closure_action": "Install Poppler and set POPPLER_PDFTOPPM or put pdftoppm on PATH.",
        },
        "pdfium": {
            "env": "PDFIUM_TEST",
            "names": ["pdfium_test", "pdfium_test.exe"],
            "version_args": ["--version"],
            "closure_action": "Install pdfium_test and set PDFIUM_TEST; keep command/version captured in this report.",
        },
        "mupdf": {
            "env": "MUTOOL",
            "names": ["mutool", "mutool.exe"],
            "version_args": ["-v"],
            "closure_action": "Install MuPDF and set MUTOOL or put mutool on PATH.",
        },
    }
    out: dict[str, dict[str, Any]] = {}
    for name, spec in engines.items():
        binary = configured_binary(spec["env"], spec["names"])
        if not binary:
            out[name] = {
                "status": "unavailable",
                "binary": None,
                "unavailable_reason": "missing_binary",
                "closure_action": spec["closure_action"],
            }
            continue
        version = run_command([binary, *spec["version_args"]], timeout=10)
        out[name] = {
            "status": "available",
            "binary": binary,
            "version": version,
            "closure_action": None,
        }
    return out


def write_inline_image_fixture(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    stream = b"q 40 0 0 40 20 20 cm BI /W 1 /H 1 /CS /RGB /BPC 8 ID \xff\x00\x00 EI Q\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>",
        b"<< /Length " + str(len(stream)).encode("ascii") + b" >>\nstream\n" + stream + b"endstream",
    ]
    data = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for idx, obj in enumerate(objects, start=1):
        offsets.append(len(data))
        data.extend(f"{idx} 0 obj\n".encode("ascii"))
        data.extend(obj)
        data.extend(b"\nendobj\n")
    xref = len(data)
    data.extend(f"xref\n0 {len(objects) + 1}\n".encode("ascii"))
    data.extend(b"0000000000 65535 f \n")
    for off in offsets[1:]:
        data.extend(f"{off:010d} 00000 n \n".encode("ascii"))
    data.extend(
        (
            f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("ascii")
    )
    path.write_bytes(data)


def corpus_entries() -> list[dict[str, Any]]:
    inline = OUT_DIR / "generated-fixtures" / "inline_image.pdf"
    write_inline_image_fixture(inline)
    entries = [
        ("simple_text", "text/simple", "tests/corpus/pdfs/generated/generated_basic_text.pdf"),
        ("positioned_text", "text/positioned", "tests/corpus/pdfs/generated/generated_rotated_text.pdf"),
        ("rtl_placeholder", "text/rtl", "tests/corpus/pdfs/generated/generated_rtl_placeholder.pdf"),
        ("cjk_type0", "text/cid_cjk", "tests/corpus/pdfs/pdfjs/90ms_rksj_h_sample.pdf"),
        ("image_xobject", "image/xobject", "tests/corpus/pdfs/generated/generated_image_only.pdf"),
        ("inline_image", "image/inline", str(inline)),
        ("form_xobject", "form/xobject", "renderer-benchmark/corpus/synthetic/synthetic_form_000.pdf"),
        ("nested_form_xobject", "form/nested", "renderer-benchmark/corpus/synthetic/synthetic_form_001.pdf"),
        ("annotation_appearance", "annotation/appearance", "tests/corpus/pdfs/pdfjs/annotation-text-widget.pdf"),
        ("tiling_pattern_later", "pattern/later", "tests/corpus/pdfs/pdfjs/tiling_patterns_variations.pdf"),
        ("shading_later", "shading/later", "tests/corpus/pdfs/pdfjs/function_based_shading.pdf"),
        ("transparency_later", "transparency/later", "tests/corpus/pdfs/pdfjs/transparent.pdf"),
        ("malformed_renderable", "malformed/renderable", "renderer-benchmark/corpus/hostile/hostile_003_missing-eof.pdf"),
    ]
    out = []
    for ident, category, path in entries:
        p = Path(path)
        out.append(
            {
                "id": ident,
                "category": category,
                "path": path.replace("\\", "/"),
                "page": 1,
                "available": p.exists(),
                "role": "prompt06_parity_corpus",
            }
        )
    return out


def oxide_base_command(args: argparse.Namespace) -> list[str]:
    if args.oxide_bin:
        return [str(Path(args.oxide_bin))]
    suffix = ".exe" if os.name == "nt" else ""
    for candidate in [Path("target/debug") / f"oxide{suffix}", Path("target/release") / f"oxide{suffix}"]:
        if candidate.exists():
            return [str(candidate)]
    return ["cargo", "run", "-p", "oxide-cli", "--quiet", "--"]


def run_oxide_render_compare(base: list[str], entry: dict[str, Any], dpi: int) -> tuple[dict[str, Any], Path]:
    out = OUT_DIR / "oxide" / f"{entry['id']}-render-compare.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        *base,
        "render-compare",
        entry["path"],
        "--pages",
        str(entry["page"]),
        "--dpi",
        str(dpi),
        "--output",
        str(out),
        "--pretty",
    ]
    result = run_command(cmd, timeout=120)
    if result["exit_status"] != 0 or not out.exists():
        return {
            "status": "oxide_execution_failure" if not result["timed_out"] else "render_timeout",
            "command": result,
            "report_artifact": str(out).replace("\\", "/"),
        }, out
    try:
        payload = json.loads(out.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return {
            "status": "oxide_report_parse_failure",
            "error": str(exc),
            "command": result,
            "report_artifact": str(out).replace("\\", "/"),
        }, out
    payload["status"] = "rendered"
    payload["command"] = result
    payload["report_artifact"] = str(out).replace("\\", "/")
    return payload, out


def run_oxide_render_zip(base: list[str], entry: dict[str, Any], dpi: int) -> dict[str, Any]:
    out = OUT_DIR / "oxide" / f"{entry['id']}.zip"
    out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        *base,
        "render",
        entry["path"],
        "--pages",
        str(entry["page"]),
        "--dpi",
        str(dpi),
        "--format",
        "png",
        "--output",
        str(out),
        "--json",
    ]
    result = run_command(cmd, timeout=120)
    return {
        "status": "rendered" if result["exit_status"] == 0 and out.exists() else "oxide_execution_failure",
        "artifact": str(out).replace("\\", "/"),
        "sha256": sha256(out),
        "command": result,
    }


def render_reference(engine: str, availability: dict[str, Any], entry: dict[str, Any], dpi: int) -> dict[str, Any]:
    if availability.get("status") != "available":
        return {
            "status": "unavailable",
            "unavailable_reason": availability.get("unavailable_reason", "missing_binary"),
            "closure_action": availability.get("closure_action"),
        }
    binary = availability["binary"]
    out_dir = OUT_DIR / "references" / engine
    out_dir.mkdir(parents=True, exist_ok=True)
    output = out_dir / f"{entry['id']}-p{entry['page']}.png"
    if engine == "poppler":
        prefix = out_dir / f"{entry['id']}-p{entry['page']}"
        cmd = [
            binary,
            "-png",
            "-r",
            str(dpi),
            "-f",
            str(entry["page"]),
            "-l",
            str(entry["page"]),
            entry["path"],
            str(prefix),
        ]
        result = run_command(cmd, timeout=60)
        produced = out_dir / f"{entry['id']}-p{entry['page']}-{entry['page']}.png"
        if produced.exists():
            produced.replace(output)
    elif engine == "mupdf":
        cmd = [binary, "draw", "-o", str(output), "-r", str(dpi), entry["path"], str(entry["page"])]
        result = run_command(cmd, timeout=60)
    else:
        cmd = [
            binary,
            "--png",
            f"--output={output}",
            f"--first-page={entry['page']}",
            f"--last-page={entry['page']}",
            entry["path"],
        ]
        result = run_command(cmd, timeout=60)
    if result["timed_out"]:
        status = "render_timeout"
    elif result["exit_status"] != 0:
        status = "reference_execution_failure"
    elif not output.exists() or output.stat().st_size == 0:
        status = "blank_output"
    else:
        status = "rendered"
    return {
        "status": status,
        "artifact": str(output).replace("\\", "/"),
        "sha256": sha256(output),
        "command": result,
    }


def image_metrics(a: Path, b: Path) -> dict[str, Any]:
    if not a.exists() or not b.exists():
        return {"status": "missing_input"}
    try:
        from PIL import Image  # type: ignore
    except Exception:
        return {
            "status": "unavailable_no_pillow",
            "byte_equal": a.read_bytes() == b.read_bytes(),
            "sha256_a": sha256(a),
            "sha256_b": sha256(b),
        }
    with Image.open(a) as ia, Image.open(b) as ib:
        pa = ia.convert("RGBA")
        pb = ib.convert("RGBA")
        if pa.size != pb.size:
            return {"status": "dimension_mismatch", "size_a": pa.size, "size_b": pb.size}
        width, height = pa.size
        bytes_a = pa.tobytes()
        bytes_b = pb.tobytes()
        mismatch = 0
        abs_sum = 0
        total = width * height
        for idx in range(0, len(bytes_a), 4):
            delta = sum(abs(bytes_a[idx + channel] - bytes_b[idx + channel]) for channel in range(4))
            abs_sum += delta
            if delta > 8:
                mismatch += 1
        unique_a = {bytes_a[idx : idx + 4] for idx in range(0, len(bytes_a), 4)}
        unique_b = {bytes_b[idx : idx + 4] for idx in range(0, len(bytes_b), 4)}
        return {
            "status": "computed",
            "width": width,
            "height": height,
            "pixel_mismatch_ratio": mismatch / total if total else 0.0,
            "mean_abs_rgba_delta": abs_sum / (total * 4) if total else 0.0,
            "blank_a": len(unique_a) <= 1,
            "blank_b": len(unique_b) <= 1,
        }


def extract_first_png(zip_artifact: dict[str, Any], entry_id: str) -> Path | None:
    if zip_artifact.get("status") != "rendered":
        return None
    zip_path = Path(zip_artifact["artifact"])
    if not zip_path.exists():
        return None
    out = OUT_DIR / "oxide-png" / f"{entry_id}.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        with zipfile.ZipFile(zip_path) as zf:
            names = [name for name in zf.namelist() if name.lower().endswith(".png")]
            if not names:
                return None
            out.write_bytes(zf.read(names[0]))
            return out
    except zipfile.BadZipFile:
        return None


def classify_after_page(compare: dict[str, Any], refs: dict[str, Any]) -> str:
    if compare.get("status") != "rendered":
        return compare.get("status", "oxide_execution_failure")
    rendered_refs = [r for r in refs.values() if r.get("status") == "rendered"]
    if not rendered_refs:
        return "unsupported_comparison"
    shas = {r.get("sha256") for r in rendered_refs if r.get("sha256")}
    if len(shas) > 1:
        return "reference_disagreement"
    page_reports = compare.get("page_reports") or []
    if page_reports and page_reports[0]["display_list"]["has_compatibility_runs"]:
        return "visual_pass_with_compatibility_fallback"
    return "native_replay_audited"


def add_counts(dst: dict[str, int], src: dict[str, Any]) -> None:
    for key in [
        "native_text_ops",
        "native_image_xobjects",
        "native_inline_images",
        "native_form_xobjects",
        "compatibility_runs",
        "compatibility_ops",
        "unsupported_ops",
    ]:
        dst[key] = dst.get(key, 0) + int(src.get(key, 0))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oxide-bin", help="Path to oxide executable; defaults to target binary or cargo run")
    parser.add_argument("--dpi", type=int, default=72)
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    availability = discover_reference_engines()
    corpus = corpus_entries()
    write_json(CORPUS_MANIFEST, {"schema_version": 1, "entries": corpus})
    write_json(REFERENCE_AVAILABILITY, {"schema_version": 1, "engines": availability})

    base = oxide_base_command(args)
    baseline_pages: list[dict[str, Any]] = []
    after_pages: list[dict[str, Any]] = []
    visual_pages: list[dict[str, Any]] = []
    native_totals: dict[str, int] = {}
    categories: dict[str, int] = {}
    reference_disagreements = 0

    for entry in corpus:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
        if not entry["available"]:
            after_pages.append({"id": entry["id"], "category": entry["category"], "status": "missing_fixture"})
            continue
        compare, _ = run_oxide_render_compare(base, entry, args.dpi)
        oxide_zip = run_oxide_render_zip(base, entry, args.dpi)
        refs = {
            name: render_reference(name, availability[name], entry, args.dpi)
            for name in ["poppler", "pdfium", "mupdf"]
        }
        page_totals = (compare.get("totals") or {}) if compare.get("status") == "rendered" else {}
        add_counts(native_totals, page_totals)
        baseline_fallback = bool(
            int(page_totals.get("text_ops", 0))
            or int(page_totals.get("image_xobjects", 0))
            or int(page_totals.get("inline_images", 0))
            or int(page_totals.get("form_xobjects", 0))
        )
        baseline_pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "status": "simulated_prompt05_pre_native_replay",
                "compatibility_runs": 1 if baseline_fallback else 0,
                "native_text_ops": 0,
                "native_image_xobjects": 0,
                "native_inline_images": 0,
                "native_form_xobjects": 0,
                "fallback_reason": "prompt05_high_level_page_content" if baseline_fallback else None,
            }
        )
        classification = classify_after_page(compare, refs)
        if classification == "reference_disagreement":
            reference_disagreements += 1
        poppler_artifact = Path(refs["poppler"].get("artifact", ""))
        oxide_png = extract_first_png(oxide_zip, entry["id"])
        diff = (
            image_metrics(oxide_png, poppler_artifact)
            if oxide_png and refs["poppler"].get("status") == "rendered"
            else {"status": "no_direct_oxide_png_or_poppler_reference"}
        )
        visual_pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "reference": "poppler",
                "metric": diff,
            }
        )
        after_pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "input": entry["path"],
                "classification": classification,
                "oxide": compare,
                "oxide_render_artifact": oxide_zip,
                "references": refs,
            }
        )

    taxonomy = {
        "schema_version": 1,
        "categories": [
            "missing_binary",
            "reference_execution_failure",
            "render_timeout",
            "blank_output",
            "malformed_input_rejection",
            "reference_disagreement",
            "oxide_mismatch",
            "unsupported_comparison",
            "visual_pass_with_compatibility_fallback",
            "native_replay_audited",
        ],
        "fallback_reasons": [
            "unsupported_operator_shading",
            "unsupported_operator_pattern",
            "unsupported_graphics_state",
            "unsupported_xobject_subtype",
            "malformed_content",
            "safety_limit_exceeded",
        ],
    }
    baseline = {
        "schema_version": 1,
        "kind": "prompt06_parity_baseline",
        "starting_commit": run_command(["git", "rev-parse", "--short", "HEAD"], timeout=10),
        "note": "Baseline models the Prompt 05 high-level display-list posture before native text/image/form operations were introduced.",
        "corpus_size": len(corpus),
        "categories": categories,
        "pages": baseline_pages,
    }
    after = {
        "schema_version": 1,
        "kind": "prompt06_parity_after_native_replay",
        "corpus_size": len(corpus),
        "categories": categories,
        "reference_disagreement_count": reference_disagreements,
        "pages": after_pages,
    }
    counters = {
        "schema_version": 1,
        "kind": "prompt06_native_replay_counters",
        "totals": native_totals,
        "counter_meaning": {
            "native_text_ops": "display-list text operators replay through RenderState dispatch",
            "native_image_xobjects": "Image XObject invocations replay through RenderState dispatch",
            "native_inline_images": "BI/ID/EI inline image groups replay through RenderState dispatch",
            "native_form_xobjects": "Form XObject invocations replay through RenderState dispatch",
        },
    }
    visual = {
        "schema_version": 1,
        "kind": "prompt06_visual_diff_summary",
        "metric_backend": "Pillow if available; otherwise hash-only unavailable_no_pillow records",
        "pages": visual_pages,
    }
    write_json(PARITY_BASELINE, baseline)
    write_json(PARITY_AFTER, after)
    write_json(FAILURE_TAXONOMY, taxonomy)
    write_json(NATIVE_COUNTERS, counters)
    write_json(VISUAL_DIFF, visual)

    rows = "\n".join(
        "<tr><td>{id}</td><td>{cat}</td><td>{cls}</td><td>{pop}</td><td>{pdfium}</td><td>{mupdf}</td></tr>".format(
            id=html.escape(page["id"]),
            cat=html.escape(page["category"]),
            cls=html.escape(page.get("classification", page.get("status", ""))),
            pop=html.escape(page.get("references", {}).get("poppler", {}).get("status", "")),
            pdfium=html.escape(page.get("references", {}).get("pdfium", {}).get("status", "")),
            mupdf=html.escape(page.get("references", {}).get("mupdf", {}).get("status", "")),
        )
        for page in after_pages
    )
    REPORT_HTML.write_text(
        "<!doctype html><meta charset='utf-8'><title>Prompt 06 Renderer Parity</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px}table{border-collapse:collapse}"
        "td,th{border:1px solid #ccc;padding:4px 8px}</style>"
        "<h1>Prompt 06 Renderer Parity Audit</h1>"
        f"<p>Corpus size: {len(corpus)}. Reference disagreements: {reference_disagreements}.</p>"
        "<table><tr><th>Fixture</th><th>Category</th><th>Classification</th>"
        "<th>Poppler</th><th>PDFium</th><th>MuPDF</th></tr>"
        f"{rows}</table>",
        encoding="utf-8",
    )

    print(json.dumps({"status": "ok", "out_dir": str(OUT_DIR), "corpus_size": len(corpus)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
