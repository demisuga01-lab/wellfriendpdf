#!/usr/bin/env python3
"""Generate and compare the Prompt 07B transparency closure corpus."""

from __future__ import annotations

import argparse
import html
import json
import shutil
from pathlib import Path
from typing import Any, Callable

import prompt07_transparency_compositing_audit as p07


OUT_DIR = p07.OUT_DIR
FIXTURE_DIR = p07.FIXTURE_DIR
TOOL_MANIFEST_OUT = OUT_DIR / "prompt07b-reference-tool-manifest.json"
CORPUS_OUT = OUT_DIR / "prompt07b-corpus-manifest.json"
RESULTS_OUT = OUT_DIR / "prompt07b-render-results.json"
DIFF_METRICS_OUT = OUT_DIR / "prompt07b-diff-metrics.json"
DISAGREEMENT_OUT = OUT_DIR / "prompt07b-reference-disagreement-summary.json"
MATRIX_OUT = OUT_DIR / "prompt07b-transparency-matrix.json"
MEMORY_OUT = OUT_DIR / "prompt07b-memory-report.json"
CLOSURE_OUT = OUT_DIR / "prompt07b-closure-audit.json"
HTML_OUT = OUT_DIR / "prompt07b-html-report" / "index.html"


def add_entry(
    entries: list[dict[str, Any]],
    ident: str,
    category: str,
    file_name: str,
    expected: str,
    generator: Callable[[Path], None],
) -> None:
    path = FIXTURE_DIR / file_name
    generator(path)
    entries.append(
        {
            "id": ident,
            "category": category,
            "path": p07.rel(path),
            "page": 1,
            "available": path.exists(),
            "expected_visual_behavior": expected,
            "generator": "scripts/prompt07b_transparency_closure_audit.py",
        }
    )


def image_smask_matte(path: Path) -> None:
    def extras(b: p07.PdfBuilder) -> dict[str, int]:
        mask = b.add_stream(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 "
            "/ColorSpace /DeviceGray /BitsPerComponent 8 /Matte [1 1 1]",
            b"\x80",
        )
        image = b.add_stream(
            f"/Type /XObject /Subtype /Image /Width 1 /Height 1 "
            f"/ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask {mask} 0 R",
            bytes([255, 128, 128]),
        )
        return {"image": image}

    p07.write_single_page_pdf(
        path,
        "0 0 0 rg 0 0 100 100 re f\nq 60 0 0 60 20 20 cm /Im1 Do Q\n",
        "<< /XObject << /Im1 {image} 0 R >> >>",
        extras,
    )


def luminosity_mask(path: Path, color_space: str, paint_ops: str) -> None:
    def extras(b: p07.PdfBuilder) -> dict[str, int]:
        mask = b.add_stream(
            f"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 100 100] "
            f"/Group << /Type /Group /S /Transparency /CS /{color_space} >>",
            paint_ops,
        )
        gs = b.add(
            f"<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G {mask} 0 R >> >>"
        )
        return {"gs": gs}

    p07.write_single_page_pdf(
        path,
        "1 1 1 rg 0 0 100 100 re f\n/GS1 gs 0 0 1 rg 0 0 100 100 re f\n",
        "<< /ExtGState << /GS1 {gs} 0 R >> >>",
        extras,
    )


def alpha_mask_bc_background(path: Path) -> None:
    def extras(b: p07.PdfBuilder) -> dict[str, int]:
        mask = b.add_stream(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [20 20 80 80] "
            "/Group << /Type /Group /S /Transparency /CS /DeviceGray >>",
            "1 g 20 20 60 60 re f\n",
        )
        gs = b.add(
            f"<< /Type /ExtGState /SMask << /Type /Mask /S /Alpha /BC [1] /G {mask} 0 R >> >>"
        )
        return {"gs": gs}

    p07.write_single_page_pdf(
        path,
        "1 1 1 rg 0 0 100 100 re f\n/GS1 gs 0 0 1 rg 0 0 100 100 re f\n",
        "<< /ExtGState << /GS1 {gs} 0 R >> >>",
        extras,
    )


def group_color_space(path: Path, color_space: str, content: str) -> None:
    def extras(b: p07.PdfBuilder) -> dict[str, int]:
        form = b.add_stream(
            f"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 100 100] "
            f"/Resources << >> /Group << /Type /Group /S /Transparency /I true /K false /CS /{color_space} >>",
            content,
        )
        return {"form": form}

    p07.write_single_page_pdf(
        path,
        "1 1 1 rg 0 0 100 100 re f\n/Fm1 Do\n",
        "<< /XObject << /Fm1 {form} 0 R >> >>",
        extras,
    )


def knockout_overlap(path: Path, nested: bool = False) -> None:
    def extras(b: p07.PdfBuilder) -> dict[str, int]:
        gs = b.add("<< /Type /ExtGState /ca 0.5 >>")
        resources = f"<< /ExtGState << /GSa {gs} 0 R >> >>"
        content = (
            "/GSa gs 1 0 0 rg 15 15 55 55 re f\n"
            "/GSa gs 0 0 1 rg 35 35 50 50 re f\n"
        )
        if nested:
            child = b.add_stream(
                "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 80 80] "
                f"/Resources {resources} /Group << /Type /Group /S /Transparency /I true /K true >>",
                content,
            )
            content = f"q 1 0 0 1 10 10 cm /Fm2 Do Q\n"
            resources = f"<< /XObject << /Fm2 {child} 0 R >> >>"
        form = b.add_stream(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 100 100] "
            f"/Resources {resources} /Group << /Type /Group /S /Transparency /I true /K true >>",
            content,
        )
        return {"form": form}

    p07.write_single_page_pdf(
        path,
        "1 1 1 rg 0 0 100 100 re f\n/Fm1 Do\n",
        "<< /XObject << /Fm1 {form} 0 R >> >>",
        extras,
    )


def generate_prompt07b_corpus() -> list[dict[str, Any]]:
    entries = p07.generate_corpus()
    add_entry(
        entries,
        "image_smask_matte",
        "closure/softmask_matte_background",
        "prompt07b_image_smask_matte.pdf",
        "image SMask /Matte unblends a preblended red image before alpha compositing",
        image_smask_matte,
    )
    add_entry(
        entries,
        "softmask_alpha_bc_background",
        "closure/softmask_matte_background",
        "prompt07b_softmask_alpha_bc_background.pdf",
        "ExtGState alpha SMask /BC backdrop is applied without unbounded allocation",
        alpha_mask_bc_background,
    )
    add_entry(
        entries,
        "softmask_luminosity_devicegray",
        "closure/luminosity_colorspace",
        "prompt07b_softmask_luminosity_devicegray.pdf",
        "DeviceGray luminosity mask converts gray group color to mask alpha",
        lambda p: luminosity_mask(p, "DeviceGray", "1 g 0 0 50 100 re f\n"),
    )
    add_entry(
        entries,
        "softmask_luminosity_devicergb",
        "closure/luminosity_colorspace",
        "prompt07b_softmask_luminosity_devicergb.pdf",
        "DeviceRGB luminosity mask derives alpha from RGB luminance",
        lambda p: luminosity_mask(p, "DeviceRGB", "1 1 1 rg 0 0 50 100 re f\n"),
    )
    add_entry(
        entries,
        "softmask_luminosity_devicecmyk",
        "closure/luminosity_colorspace",
        "prompt07b_softmask_luminosity_devicecmyk.pdf",
        "DeviceCMYK luminosity mask converts CMYK to RGB before luminance",
        lambda p: luminosity_mask(p, "DeviceCMYK", "0 0 0 0 k 0 0 50 100 re f\n"),
    )
    add_entry(
        entries,
        "group_colorspace_devicegray",
        "closure/group_colorspace",
        "prompt07b_group_colorspace_devicegray.pdf",
        "explicit DeviceGray transparency group renders through the group stack",
        lambda p: group_color_space(p, "DeviceGray", "0.2 g 10 10 70 70 re f\n0.8 g 35 35 50 50 re f\n"),
    )
    add_entry(
        entries,
        "group_colorspace_devicergb",
        "closure/group_colorspace",
        "prompt07b_group_colorspace_devicergb.pdf",
        "explicit DeviceRGB transparency group renders through the group stack",
        lambda p: group_color_space(p, "DeviceRGB", "1 0 0 rg 10 10 70 70 re f\n0 0 1 rg 35 35 50 50 re f\n"),
    )
    add_entry(
        entries,
        "group_colorspace_devicecmyk",
        "closure/group_colorspace",
        "prompt07b_group_colorspace_devicecmyk.pdf",
        "explicit DeviceCMYK transparency group renders common DeviceCMYK source colors",
        lambda p: group_color_space(p, "DeviceCMYK", "0 1 1 0 k 10 10 70 70 re f\n1 1 0 0 k 35 35 50 50 re f\n"),
    )
    add_entry(
        entries,
        "knockout_overlap_exact",
        "closure/knockout_overlap",
        "prompt07b_knockout_overlap_exact.pdf",
        "overlap inside a knockout group uses the group initial backdrop, not accumulated prior objects",
        lambda p: knockout_overlap(p, nested=False),
    )
    add_entry(
        entries,
        "knockout_overlap_nested_form",
        "closure/knockout_overlap",
        "prompt07b_knockout_overlap_nested_form.pdf",
        "nested Form XObject knockout group preserves exact interior overlap behavior",
        lambda p: knockout_overlap(p, nested=True),
    )
    p07.write_json(
        CORPUS_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_transparency_closure_corpus_manifest",
            "fixture_count": len(entries),
            "base_prompt07_fixture_count": 37,
            "closure_fixture_count": len(entries) - 37,
            "entries": entries,
            "memory_cap_mb": 4096,
        },
    )
    return entries


def prompt07b_classification(raw: str) -> str:
    mapping = {
        "all_references_agree_oxide_pass": "all_references_agree_and_oxide_passes",
        "all_references_agree_oxide_mismatch": "all_references_agree_and_oxide_mismatches",
        "references_disagree_oxide_between_references": "references_disagree_and_oxide_within_cluster",
        "needs_manual_review": "references_disagree_and_oxide_outlier",
        "reference_tool_failure": "malformed_or_reference_failure",
        "oxide_render_failure": "all_references_agree_and_oxide_mismatches",
    }
    if raw.startswith("references_disagree_oxide_matches_"):
        return "references_disagree_and_oxide_within_cluster"
    return mapping.get(raw, raw)


def add_prompt07b_classifications(results: dict[str, Any]) -> None:
    counts: dict[str, int] = {}
    for page in results.get("pages", []):
        final = prompt07b_classification(page.get("classification", "unknown"))
        page["prompt07b_classification"] = final
        counts[final] = counts.get(final, 0) + 1
    results["prompt07b_classification_counts"] = counts


def page_by_id(results: dict[str, Any], ident: str) -> dict[str, Any] | None:
    return next((page for page in results.get("pages", []) if page.get("id") == ident), None)


def closure_rows(results: dict[str, Any]) -> list[dict[str, Any]]:
    def status_for(ids: list[str]) -> tuple[str, list[str]]:
        classes = [
            page_by_id(results, ident).get("prompt07b_classification", "not_run")
            if page_by_id(results, ident)
            else "not_run"
            for ident in ids
        ]
        if all(c in {"all_references_agree_and_oxide_passes", "references_disagree_and_oxide_within_cluster"} for c in classes):
            return "closed", classes
        return "partial", classes

    rows = []
    specs = [
        (
            "alpha_image",
            "all_references_agree_oxide_mismatch",
            ["alpha_image"],
            "image XObject paint now multiplies decoded/SMask alpha by graphics-state /ca",
            "none for DeviceRGB alpha constants",
        ),
        (
            "soft_mask_matte_background",
            "matte/background edge cases partial",
            ["image_smask_matte", "softmask_alpha_bc_background"],
            "image /SMask /Matte unblends common DeviceGray/RGB/CMYK matte values; ExtGState /BC backdrop remains implemented",
            "advanced ICC/device-link matte conversion is unsupported-reported as CMM work",
        ),
        (
            "luminosity_soft_mask_color_space",
            "exact color-managed luminosity partial",
            [
                "softmask_luminosity_devicegray",
                "softmask_luminosity_devicergb",
                "softmask_luminosity_devicecmyk",
            ],
            "DeviceGray, DeviceRGB, and DeviceCMYK mask groups paint through the current color converter before Rec.601 luminosity extraction",
            "ICCBased/calibrated exact CMM parity remains advanced CMM work",
        ),
        (
            "transparency_group_color_space",
            "mostly device-space",
            [
                "group_colorspace_devicegray",
                "group_colorspace_devicergb",
                "group_colorspace_devicecmyk",
            ],
            "explicit DeviceGray/RGB/CMYK group /CS is recognized and exercised through the group stack for common source colors",
            "full ICC/device-link/multicolor group blending remains advanced CMM work",
        ),
        (
            "interior_knockout_overlap",
            "exact interior overlap partial",
            ["knockout_overlap_exact", "knockout_overlap_nested_form"],
            "knockout group buffers now retain an initial backdrop and each covered pixel recomposes against it",
            "text clipping and pattern/shading paints inside knockout groups remain later prompts",
        ),
        (
            "multi_reference_closure",
            "one alpha_image outlier plus documented partial rows",
            ["alpha_image", "image_smask_matte", "knockout_overlap_exact"],
            "Poppler/PDFium/MuPDF/Oxide audit rerun with Prompt 07 plus Prompt 07B closure fixtures",
            "malformed recursive reference failure remains classified as malformed/reference failure",
        ),
    ]
    for area, previous, ids, result, limit in specs:
        status, classes = status_for(ids)
        rows.append(
            {
                "area": area,
                "previous_status": previous,
                "target_status": "closed_or_precisely_unsupported_reported",
                "implementation_result": result,
                "fixture_ids": ids,
                "prompt07b_classifications": classes,
                "tests_artifacts": [
                    "target/prompt07-transparency-compositing/prompt07b-render-results.json",
                    "target/prompt07-transparency-compositing/prompt07b-transparency-matrix.json",
                ],
                "remaining_limit": limit,
                "status": status,
            }
        )
    return rows


def write_prompt07b_artifacts(entries: list[dict[str, Any]], tools_payload: dict[str, Any], results: dict[str, Any]) -> None:
    shutil.copyfile(tools_payload["_manifest_path"], TOOL_MANIFEST_OUT)
    p07.write_json(RESULTS_OUT, results)
    p07.write_json(
        DIFF_METRICS_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_diff_metrics",
            "metrics": [
                {
                    "id": page["id"],
                    "category": page["category"],
                    "classification": page["classification"],
                    "prompt07b_classification": page["prompt07b_classification"],
                    "pair_metrics": page["pair_metrics"],
                }
                for page in results.get("pages", [])
            ],
        },
    )
    disagreement_pages = [
        {
            "id": page["id"],
            "category": page["category"],
            "classification": page["classification"],
            "prompt07b_classification": page["prompt07b_classification"],
        }
        for page in results.get("pages", [])
        if page["prompt07b_classification"]
        in {
            "references_disagree_and_oxide_within_cluster",
            "references_disagree_and_oxide_outlier",
            "malformed_or_reference_failure",
        }
    ]
    p07.write_json(
        DISAGREEMENT_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_reference_disagreement_summary",
            "fixture_count": len(entries),
            "classification_counts": results.get("prompt07b_classification_counts", {}),
            "reference_disagreement_pages": disagreement_pages,
        },
    )
    rows = closure_rows(results)
    p07.write_json(
        MATRIX_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_transparency_matrix",
            "rows": rows,
            "closure_fixture_count": len([e for e in entries if e["category"].startswith("closure/")]),
            "oxide_outlier_failures": [
                page["id"]
                for page in results.get("pages", [])
                if page["prompt07b_classification"]
                in {"all_references_agree_and_oxide_mismatches", "references_disagree_and_oxide_outlier"}
            ],
        },
    )
    p07.write_json(
        CLOSURE_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_closure_audit",
            "rows": rows,
            "status": "complete" if all(row["status"] == "closed" for row in rows) else "partial",
        },
    )
    p07.write_json(
        MEMORY_OUT,
        {
            "schema_version": 1,
            "kind": "prompt07b_memory_report",
            "memory_cap_mb": 4096,
            "offscreen_surface_admission": "unchanged: transparency group and soft-mask RGBA surfaces reserve scheduler memory before allocation",
            "new_intermediate_surfaces": "none beyond existing image SMask decode and existing group/mask offscreen buffers",
            "unit_tests": [
                "renderer_offscreen_surface_fails_closed_over_budget",
                "knockout_backdrop_prevents_interior_overlap_accumulation",
            ],
        },
    )
    HTML_OUT.parent.mkdir(parents=True, exist_ok=True)
    table_rows = []
    for page in results.get("pages", []):
        table_rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['prompt07b_classification'])}</td>"
            f"<td>{html.escape(page['classification'])}</td>"
            "</tr>"
        )
    HTML_OUT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Prompt 07B Transparency Closure Audit</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#172033}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 07B Transparency Closure Audit</h1>"
        f"<p>Fixtures: {len(entries)}. Memory cap: 4096 MB.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(results.get('prompt07b_classification_counts', {}), indent=2, sort_keys=True))}</pre>"
        "<h2>Closure Rows</h2><pre>"
        f"{html.escape(json.dumps(rows, indent=2, sort_keys=True))}</pre>"
        "<h2>Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Prompt 07B classification</th><th>Raw classification</th></tr>"
        + "\n".join(table_rows)
        + "</table>",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=p07.TOOL_MANIFEST)
    parser.add_argument("--oxide-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    manifest_payload = p07.load_tool_manifest(args.manifest)
    manifest_payload["_manifest_path"] = args.manifest
    entries = generate_prompt07b_corpus()
    base = p07.oxide_base_command(args.oxide_bin)
    results = p07.run_phase(entries, manifest_payload["tools"], base, "prompt07b", args.dpi, args.timeout)
    results["starting_checkpoint"] = p07.run_command(["git", "rev-parse", "--short", "HEAD"], 10)
    results["prompt07b_note"] = "Prompt 07B closure audit for alpha image, matte/background, luminosity color spaces, group color spaces, and knockout overlap."
    add_prompt07b_classifications(results)
    write_prompt07b_artifacts(entries, manifest_payload, results)
    print(
        json.dumps(
            {
                "status": json.loads(CLOSURE_OUT.read_text(encoding="utf-8"))["status"],
                "fixture_count": len(entries),
                "artifacts": {
                    "corpus": p07.rel(CORPUS_OUT),
                    "results": p07.rel(RESULTS_OUT),
                    "diff_metrics": p07.rel(DIFF_METRICS_OUT),
                    "summary": p07.rel(DISAGREEMENT_OUT),
                    "matrix": p07.rel(MATRIX_OUT),
                    "closure": p07.rel(CLOSURE_OUT),
                    "html": p07.rel(HTML_OUT),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
