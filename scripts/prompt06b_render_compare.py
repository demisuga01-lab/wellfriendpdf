#!/usr/bin/env python3
"""Run the Prompt 06 corpus through Oxide, Poppler, PDFium, and MuPDF."""

from __future__ import annotations

import argparse
import html
import importlib.util
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
RENDER_DIR = OUT_DIR / "prompt06b-renders"
DIFF_DIR = OUT_DIR / "prompt06b-diffs"
LOG_DIR = OUT_DIR / "prompt06b-logs"
OXIDE_REPORT_DIR = OUT_DIR / "prompt06b-oxide-reports"

TOOL_MANIFEST = OUT_DIR / "reference-tool-manifest-prompt06b.json"
CORPUS_MANIFEST = OUT_DIR / "multi-reference-corpus-manifest-prompt06b.json"
RENDER_RESULTS = OUT_DIR / "multi-reference-render-results-prompt06b.json"
DIFF_METRICS = OUT_DIR / "multi-reference-diff-metrics-prompt06b.json"
DISAGREEMENT_SUMMARY = OUT_DIR / "reference-disagreement-summary-prompt06b.json"
TAXONOMY = OUT_DIR / "renderer-parity-taxonomy-prompt06b.json"
HTML_REPORT = OUT_DIR / "prompt06b-html-report" / "index.html"

PAIR_NAMES = [
    ("oxide", "poppler"),
    ("oxide", "pdfium"),
    ("oxide", "mupdf"),
    ("poppler", "pdfium"),
    ("poppler", "mupdf"),
    ("pdfium", "mupdf"),
]
REFERENCE_PAIRS = [("poppler", "pdfium"), ("poppler", "mupdf"), ("pdfium", "mupdf")]
OXIDE_PAIRS = [("oxide", "poppler"), ("oxide", "pdfium"), ("oxide", "mupdf")]
LATER_OWNED_CATEGORIES = {"pattern/later", "shading/later", "transparency/later"}


def prompt06_module() -> Any:
    script = Path("scripts/prompt06_renderer_parity_audit.py")
    spec = importlib.util.spec_from_file_location("prompt06_renderer_parity_audit", script)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to import {script}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    p = Path(path)
    try:
        return p.relative_to(Path.cwd()).as_posix()
    except ValueError:
        return p.as_posix()


def run_command(cmd: list[str], timeout: int = 120) -> dict[str, Any]:
    started = time.time()
    actual_cmd = cmd
    if cmd and cmd[0].lower().endswith((".cmd", ".bat")):
        actual_cmd = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *cmd]
    try:
        proc = subprocess.run(
            actual_cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "executed_command": actual_cmd,
            "exit_status": proc.returncode,
            "stdout": proc.stdout[-4000:],
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "executed_command": actual_cmd,
            "exit_status": None,
            "stdout": (exc.stdout or "")[-4000:] if isinstance(exc.stdout, str) else "",
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def load_manifest(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8-sig"))
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if payload.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    if missing:
        raise RuntimeError(f"Required reference renderers unavailable after bootstrap: {', '.join(missing)}")
    return payload


def oxide_base_command(oxide_bin: str | None) -> list[str]:
    if oxide_bin:
        return [str(Path(oxide_bin))]
    suffix = ".exe" if os.name == "nt" else ""
    for candidate in [Path("target/debug") / f"oxide{suffix}", Path("target/release") / f"oxide{suffix}"]:
        if candidate.exists():
            return [str(candidate)]
    return ["cargo", "run", "-p", "oxide-cli", "--quiet", "--"]


def render_oxide(base: list[str], entry: dict[str, Any], dpi: int, timeout: int) -> dict[str, Any]:
    render_dir = RENDER_DIR / "oxide"
    render_dir.mkdir(parents=True, exist_ok=True)
    zip_path = render_dir / f"{entry['id']}-p{entry['page']}.zip"
    png_path = render_dir / f"{entry['id']}-p{entry['page']}.png"
    report_path = OXIDE_REPORT_DIR / f"{entry['id']}-p{entry['page']}.json"
    for path in [zip_path, png_path, report_path]:
        if path.exists():
            path.unlink()
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
        str(zip_path),
        "--json",
    ]
    render_result = run_command(cmd, timeout=timeout)
    compare_cmd = [
        *base,
        "render-compare",
        entry["path"],
        "--pages",
        str(entry["page"]),
        "--dpi",
        str(dpi),
        "--output",
        str(report_path),
        "--pretty",
    ]
    compare_result = run_command(compare_cmd, timeout=timeout)
    counters: dict[str, Any] = {}
    if report_path.exists():
        try:
            report = json.loads(report_path.read_text(encoding="utf-8"))
            counters = report.get("totals", {})
        except json.JSONDecodeError:
            counters = {}
    status = "rendered"
    if render_result["timed_out"] or compare_result["timed_out"]:
        status = "render_timeout"
    elif render_result["exit_status"] != 0 or not zip_path.exists():
        status = "oxide_render_failure"
    else:
        try:
            with zipfile.ZipFile(zip_path) as zf:
                names = sorted(name for name in zf.namelist() if name.lower().endswith(".png"))
                if not names:
                    status = "blank_output"
                else:
                    png_path.write_bytes(zf.read(names[0]))
        except zipfile.BadZipFile:
            status = "oxide_render_failure"
    return {
        "status": status,
        "artifact": rel(png_path) if png_path.exists() else None,
        "zip_artifact": rel(zip_path) if zip_path.exists() else None,
        "render_report_artifact": rel(report_path) if report_path.exists() else None,
        "native_replay_counters": counters,
        "render_command": render_result,
        "render_compare_command": compare_result,
    }


def render_reference(
    engine: str,
    tool: dict[str, Any],
    entry: dict[str, Any],
    dpi: int,
    timeout: int,
) -> dict[str, Any]:
    render_dir = RENDER_DIR / engine
    render_dir.mkdir(parents=True, exist_ok=True)
    output = render_dir / f"{entry['id']}-p{entry['page']}.png"
    if output.exists():
        output.unlink()
    executable = str(tool["executable_path"])
    if engine == "poppler":
        prefix = render_dir / f"{entry['id']}-p{entry['page']}"
        for stale in render_dir.glob(f"{entry['id']}-p{entry['page']}-*.png"):
            stale.unlink()
        cmd = [
            executable,
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
        result = run_command(cmd, timeout=timeout)
        produced = render_dir / f"{entry['id']}-p{entry['page']}-{entry['page']}.png"
        if produced.exists():
            produced.replace(output)
    elif engine == "pdfium":
        cmd = [
            executable,
            "--png",
            f"--output={output}",
            f"--first-page={entry['page']}",
            f"--last-page={entry['page']}",
            f"--dpi={dpi}",
            entry["path"],
        ]
        result = run_command(cmd, timeout=timeout)
    elif engine == "mupdf":
        cmd = [executable, "draw", "-o", str(output), "-r", str(dpi), entry["path"], str(entry["page"])]
        result = run_command(cmd, timeout=timeout)
    else:
        raise ValueError(engine)

    log_path = LOG_DIR / engine / f"{entry['id']}-p{entry['page']}.json"
    write_json(log_path, result)
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
        "artifact": rel(output) if output.exists() else None,
        "log_artifact": rel(log_path),
        "command": result,
    }


def average_hash(image: Any) -> str:
    from PIL import Image  # type: ignore

    resampling = getattr(Image, "Resampling", Image).LANCZOS
    gray = image.convert("L").resize((8, 8), resampling)
    if hasattr(gray, "get_flattened_data"):
        pixels = list(gray.get_flattened_data())
    else:
        pixels = list(gray.getdata())
    avg = sum(pixels) / len(pixels)
    bits = "".join("1" if px >= avg else "0" for px in pixels)
    return f"{int(bits, 2):016x}"


def image_metrics(a_name: str, a_path: str | None, b_name: str, b_path: str | None, entry_id: str) -> dict[str, Any]:
    if not a_path or not b_path:
        return {"status": "missing_input", "threshold_pass": False}
    a = Path(a_path)
    b = Path(b_path)
    if not a.exists() or not b.exists():
        return {"status": "missing_input", "threshold_pass": False, "artifact_a": a_path, "artifact_b": b_path}
    try:
        from PIL import Image  # type: ignore
    except Exception as exc:
        return {"status": "unavailable_no_pillow", "threshold_pass": False, "error": str(exc)}

    with Image.open(a) as ia_raw, Image.open(b) as ib_raw:
        ia = ia_raw.convert("RGBA")
        ib = ib_raw.convert("RGBA")
        hash_a = average_hash(ia)
        hash_b = average_hash(ib)
        if ia.size != ib.size:
            return {
                "status": "dimension_mismatch",
                "threshold_pass": False,
                "dimensions_match": False,
                "size_a": list(ia.size),
                "size_b": list(ib.size),
                "visual_hash_a": hash_a,
                "visual_hash_b": hash_b,
            }
        bytes_a = ia.tobytes()
        bytes_b = ib.tobytes()
        changed_pixels = 0
        changed_pixels_threshold8 = 0
        max_delta = 0
        abs_sum = 0
        diff_bytes = bytearray(len(bytes_a))
        for idx in range(0, len(bytes_a), 4):
            pixel_changed = False
            pixel_delta = 0
            for channel in range(4):
                delta = abs(bytes_a[idx + channel] - bytes_b[idx + channel])
                pixel_delta += delta
                abs_sum += delta
                max_delta = max(max_delta, delta)
                if delta:
                    pixel_changed = True
                diff_bytes[idx + channel] = min(255, delta * 4)
            diff_bytes[idx + 3] = 255
            if pixel_changed:
                changed_pixels += 1
            if pixel_delta > 8:
                changed_pixels_threshold8 += 1
        total_pixels = ia.size[0] * ia.size[1]
        mean_abs = abs_sum / (total_pixels * 4) if total_pixels else 0.0
        changed_pct = changed_pixels / total_pixels if total_pixels else 0.0
        changed8_pct = changed_pixels_threshold8 / total_pixels if total_pixels else 0.0
        threshold_pass = mean_abs <= 2.0 or changed8_pct <= 0.02
        pair_dir = DIFF_DIR / f"{a_name}_vs_{b_name}"
        pair_dir.mkdir(parents=True, exist_ok=True)
        diff_path = pair_dir / f"{entry_id}.png"
        Image.frombytes("RGBA", ia.size, bytes(diff_bytes)).save(diff_path)
        return {
            "status": "computed",
            "threshold_pass": threshold_pass,
            "dimensions_match": True,
            "width": ia.size[0],
            "height": ia.size[1],
            "mean_abs_error": mean_abs,
            "max_channel_difference": max_delta,
            "changed_pixel_percentage": changed_pct,
            "changed_pixel_threshold8_percentage": changed8_pct,
            "visual_hash_a": hash_a,
            "visual_hash_b": hash_b,
            "visual_hash_match": hash_a == hash_b,
            "diff_artifact": rel(diff_path),
        }


def classify_page(category: str, renders: dict[str, Any], metrics: dict[str, Any]) -> str:
    if renders["oxide"]["status"] != "rendered":
        return "oxide_render_failure"
    if any(renders[name]["status"] != "rendered" for name in ["poppler", "pdfium", "mupdf"]):
        return "reference_tool_failure"
    if any(metric.get("status") == "dimension_mismatch" for metric in metrics.values()):
        return "dimension_mismatch"

    def pair_pass(a: str, b: str) -> bool:
        return bool(metrics[f"{a}_vs_{b}"].get("threshold_pass"))

    references_agree = all(pair_pass(a, b) for a, b in REFERENCE_PAIRS)
    oxide_matches = [b for a, b in OXIDE_PAIRS if pair_pass(a, b)]
    later_owned = category in LATER_OWNED_CATEGORIES
    if references_agree:
        if len(oxide_matches) == 3:
            return "all_references_agree_oxide_pass"
        return "needs_manual_review" if later_owned else "all_references_agree_oxide_mismatch"
    if len(oxide_matches) == 1:
        return f"references_disagree_oxide_matches_{oxide_matches[0]}"
    if len(oxide_matches) > 1:
        return "references_disagree_oxide_between_references"
    return "needs_manual_review" if later_owned else "references_disagree_oxide_between_references"


def pair_summary(metrics_pages: list[dict[str, Any]]) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    for a, b in PAIR_NAMES:
        key = f"{a}_vs_{b}"
        computed = sum(1 for page in metrics_pages if page["pairs"][key].get("status") == "computed")
        passed = sum(1 for page in metrics_pages if page["pairs"][key].get("threshold_pass"))
        dimensions = sum(1 for page in metrics_pages if page["pairs"][key].get("status") == "dimension_mismatch")
        missing = sum(1 for page in metrics_pages if page["pairs"][key].get("status") == "missing_input")
        summary[key] = {
            "computed": computed,
            "threshold_pass": passed,
            "threshold_mismatch": computed - passed,
            "dimension_mismatch": dimensions,
            "missing_or_failed_input": missing,
        }
    return summary


def render_html(results: dict[str, Any], summary: dict[str, Any]) -> None:
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for page in results["pages"]:
        pairs = page["pair_metrics"]
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['classification'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            f"<td>{pairs['oxide_vs_poppler'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['oxide_vs_pdfium'].get('changed_pixel_threshold8_percentage', '')}</td>"
            f"<td>{pairs['oxide_vs_mupdf'].get('changed_pixel_threshold8_percentage', '')}</td>"
            "</tr>"
        )
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Prompt 06B Multi-Reference Renderer Audit</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#1f2933}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 06B Multi-Reference Renderer Audit</h1>"
        f"<p>Pages: {results['page_count']}. Pairwise comparisons: {summary['total_pairwise_comparisons']}.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(summary['classification_counts'], indent=2, sort_keys=True))}</pre>"
        "<h2>Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Classification</th>"
        "<th>Poppler</th><th>PDFium</th><th>MuPDF</th><th>Ox/Pop changed8</th>"
        "<th>Ox/PDFium changed8</th><th>Ox/MuPDF changed8</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=TOOL_MANIFEST)
    parser.add_argument("--oxide-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    module = prompt06_module()
    corpus = module.corpus_entries()
    available_corpus = [entry for entry in corpus if entry.get("available")]
    categories: dict[str, int] = {}
    for entry in available_corpus:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
    write_json(
        CORPUS_MANIFEST,
        {
            "schema_version": 1,
            "kind": "prompt06b_multi_reference_corpus_manifest",
            "page_count": len(available_corpus),
            "categories": categories,
            "added_prompt06b_fixtures": [],
            "entries": available_corpus,
        },
    )

    base = oxide_base_command(args.oxide_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    classification_counts: dict[str, int] = {}
    later_owned_pages: list[str] = []

    for entry in available_corpus:
        renders = {
            "oxide": render_oxide(base, entry, args.dpi, args.timeout),
            "poppler": render_reference("poppler", manifest["tools"]["poppler"], entry, args.dpi, args.timeout),
            "pdfium": render_reference("pdfium", manifest["tools"]["pdfium"], entry, args.dpi, args.timeout),
            "mupdf": render_reference("mupdf", manifest["tools"]["mupdf"], entry, args.dpi, args.timeout),
        }
        pair_metrics = {
            f"{a}_vs_{b}": image_metrics(
                a,
                renders[a].get("artifact"),
                b,
                renders[b].get("artifact"),
                f"{entry['id']}-p{entry['page']}",
            )
            for a, b in PAIR_NAMES
        }
        classification = classify_page(entry["category"], renders, pair_metrics)
        classification_counts[classification] = classification_counts.get(classification, 0) + 1
        if entry["category"] in LATER_OWNED_CATEGORIES:
            later_owned_pages.append(entry["id"])
        page = {
            "id": entry["id"],
            "category": entry["category"],
            "page": entry["page"],
            "input": entry["path"],
            "classification": classification,
            "later_owned_renderer_category": entry["category"] in LATER_OWNED_CATEGORIES,
            "renders": renders,
            "pair_metrics": pair_metrics,
            "native_replay_counters": renders["oxide"].get("native_replay_counters", {}),
        }
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "category": entry["category"], "pairs": pair_metrics})

    pair_counts = pair_summary(metrics_pages)
    total_pairwise = len(available_corpus) * len(PAIR_NAMES)
    summary = {
        "schema_version": 1,
        "kind": "prompt06b_reference_disagreement_summary",
        "page_count": len(available_corpus),
        "total_pairwise_comparisons": total_pairwise,
        "classification_counts": classification_counts,
        "pair_summary": pair_counts,
        "later_owned_renderer_categories": sorted(LATER_OWNED_CATEGORIES),
        "later_owned_pages": sorted(later_owned_pages),
        "notable_examples": [
            {
                "id": page["id"],
                "category": page["category"],
                "classification": page["classification"],
            }
            for page in pages
            if page["classification"] != "all_references_agree_oxide_pass"
        ][:10],
    }
    results = {
        "schema_version": 1,
        "kind": "prompt06b_multi_reference_render_results",
        "dpi": args.dpi,
        "page_count": len(available_corpus),
        "categories": categories,
        "tool_manifest": rel(args.manifest),
        "reference_tools": manifest["tools"],
        "pages": pages,
    }
    taxonomy = {
        "schema_version": 1,
        "kind": "prompt06b_renderer_parity_taxonomy",
        "classification_categories": [
            "all_references_agree_oxide_pass",
            "all_references_agree_oxide_mismatch",
            "references_disagree_oxide_matches_poppler",
            "references_disagree_oxide_matches_pdfium",
            "references_disagree_oxide_matches_mupdf",
            "references_disagree_oxide_between_references",
            "reference_tool_failure",
            "oxide_render_failure",
            "dimension_mismatch",
            "needs_manual_review",
        ],
        "later_owned_categories": sorted(LATER_OWNED_CATEGORIES),
        "renderer_gap_matrix_updates": {
            "pdfium_reference_comparison": "implemented_public",
            "mupdf_reference_comparison": "implemented_public",
            "poppler_reference_comparison": "implemented_public",
            "pattern_full_parity": "later_owned",
            "shading_full_parity": "later_owned",
            "transparency_soft_mask_full_parity": "later_owned",
        },
    }

    write_json(RENDER_RESULTS, results)
    write_json(DIFF_METRICS, {"schema_version": 1, "kind": "prompt06b_multi_reference_diff_metrics", "pages": metrics_pages})
    write_json(DISAGREEMENT_SUMMARY, summary)
    write_json(TAXONOMY, taxonomy)
    render_html(results, summary)

    closure_failures = [
        page
        for page in pages
        if page["classification"] in {"reference_tool_failure", "oxide_render_failure"}
    ]
    print(
        json.dumps(
            {
                "status": "passed" if not closure_failures else "failed",
                "page_count": len(available_corpus),
                "total_pairwise_comparisons": total_pairwise,
                "artifacts": {
                    "results": rel(RENDER_RESULTS),
                    "metrics": rel(DIFF_METRICS),
                    "summary": rel(DISAGREEMENT_SUMMARY),
                    "html": rel(HTML_REPORT),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if not closure_failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
