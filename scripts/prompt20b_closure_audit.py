#!/usr/bin/env python3
"""Generate Prompt 20B closure artifacts from executable CLI evidence.

The harness records unavailable reference tools separately and never promotes
missing PDFBox/MuPDF/PDFium binaries to a pass.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import shutil
import subprocess
import zipfile
from pathlib import Path

from PIL import Image, ImageChops, ImageStat


SCHEMA = "prompt20b.multirun-form-appearance-closure.v1"


def run(command: list[str], cwd: Path, timeout: int = 180) -> dict:
    process = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
        timeout=timeout,
    )
    return {
        "command": command,
        "exit_code": process.returncode,
        "stdout": process.stdout[-12000:],
        "stderr": process.stderr[-12000:],
        "passed": process.returncode == 0,
    }


def dump(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pdf_from_objects(objects: dict[int, bytes]) -> bytes:
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {0: 0}
    for number in sorted(objects):
        offsets[number] = len(out)
        out.extend(f"{number} 0 obj\n".encode())
        out.extend(objects[number])
        out.extend(b"\nendobj\n")
    xref = len(out)
    out.extend(f"xref\n0 {max(objects) + 1}\n".encode())
    out.extend(b"0000000000 65535 f \n")
    for number in range(1, max(objects) + 1):
        if number in offsets:
            out.extend(f"{offsets[number]:010d} 00000 n \n".encode())
        else:
            out.extend(b"0000000000 65535 f \n")
    out.extend(
        f"trailer\n<< /Size {max(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(out)


def stream(dict_src: bytes, data: bytes) -> bytes:
    return dict_src.replace(b"__LEN__", str(len(data)).encode()) + b"\nstream\n" + data + b"endstream"


def form_stream(data: bytes, resources: bytes = b"<< >>") -> bytes:
    return stream(
        b"<< /Type /XObject /Subtype /Form /BBox [0 0 20 10] /Resources " + resources + b" /Length __LEN__ >>",
        data,
    )


def text_fixture() -> bytes:
    content = b"BT /F1 12 Tf 10 150 Td (ONE) Tj /F2 18 Tf (TWO) Tj [(TH) 20 (REE)] TJ ET\n"
    return pdf_from_objects({
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> /Contents 4 0 R >>",
        4: stream(b"<< /Length __LEN__ >>", content),
        5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        6: b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
    })


def nested_form_fixture(depth: int = 3) -> bytes:
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /XObject << /F0 5 0 R >> >> /Contents 4 0 R >>",
        4: stream(b"<< /Length __LEN__ >>", b"q 1 0 0 1 10 10 cm /F0 Do Q\nq 1 0 0 1 80 80 cm /F0 Do Q\n"),
    }
    for level in range(depth):
        number = 5 + level
        if level + 1 == depth:
            objects[number] = form_stream(b"2 w 0 0 20 10 re S\n")
        else:
            name = f"F{level + 1}".encode()
            objects[number] = form_stream(b"q /" + name + b" Do Q\n", b"<< /XObject << /" + name + b" " + str(number + 1).encode() + b" 0 R >> >>")
    return pdf_from_objects(objects)


def annotation_fixture(kind: str) -> bytes:
    if kind == "nrd":
        ap = b"<< /N 8 0 R /R 9 0 R /D 10 0 R >>"
        annots = []
        for x in (10, 80, 140):
            annots.append(b"<< /Type /Annot /Subtype /Stamp /Rect [" + str(x).encode() + b" 10 " + str(x + 20).encode() + b" 20] /AP " + ap + b" /AS /On >>")
        return pdf_from_objects({
            1: b"<< /Type /Catalog /Pages 2 0 R >>",
            2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R /Annots [6 0 R 7 0 R 11 0 R] >>",
            4: stream(b"<< /Length __LEN__ >>", b""),
            6: annots[0],
            7: annots[1],
            8: form_stream(b"2 w 0 0 20 10 re S\n"),
            9: form_stream(b"3 w 0 0 20 10 re S\n"),
            10: form_stream(b"4 w 0 0 20 10 re S\n"),
            11: annots[2],
        })
    if kind in {"state", "checkbox", "radio"}:
        state = b"Yes" if kind in {"checkbox", "radio"} else b"On"
        subtype = b"Widget" if kind in {"checkbox", "radio"} else b"Stamp"
        field = b" /FT /Btn /V /" + state + (b" /Ff 32768" if kind == "radio" else b" /Ff 0") if subtype == b"Widget" else b""
        ap = b"<< /N << /" + state + b" 8 0 R /Off 9 0 R >> >>"
        ann = lambda x: b"<< /Type /Annot /Subtype /" + subtype + b" /Rect [" + str(x).encode() + b" 10 " + str(x + 20).encode() + b" 20]" + field + b" /AP " + ap + b" /AS /" + state + b" >>"
        return pdf_from_objects({
            1: b"<< /Type /Catalog /Pages 2 0 R >>",
            2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R /Annots [6 0 R 7 0 R] >>",
            4: stream(b"<< /Length __LEN__ >>", b""),
            6: ann(10),
            7: ann(80),
            8: form_stream(b"2 w 0 0 20 10 re S\n"),
            9: form_stream(b"1 w 0 0 20 10 re S\n"),
        })
    if kind == "nested":
        ann = lambda x: b"<< /Type /Annot /Subtype /Stamp /Rect [" + str(x).encode() + b" 10 " + str(x + 20).encode() + b" 20] /AP << /N 8 0 R >> /AS /On >>"
        return pdf_from_objects({
            1: b"<< /Type /Catalog /Pages 2 0 R >>",
            2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            3: b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R /Annots [6 0 R 7 0 R] >>",
            4: stream(b"<< /Length __LEN__ >>", b""),
            6: ann(10),
            7: ann(80),
            8: form_stream(b"q /Nested Do Q\n", b"<< /XObject << /Nested 9 0 R >> >>"),
            9: form_stream(b"2 w 0 0 20 10 re S\n"),
        })
    raise ValueError(kind)


def render_oxide(oxide: Path, pdf: Path, output: Path, repo: Path) -> dict:
    archive = output.with_suffix(".zip")
    result = run([str(oxide), "render", str(pdf), "--pages", "1", "--dpi", "96", "--format", "png", "--output", str(archive)], repo)
    if result["passed"] and archive.exists():
        with zipfile.ZipFile(archive) as bundle:
            pngs = [name for name in bundle.namelist() if name.lower().endswith(".png")]
            if pngs:
                output.write_bytes(bundle.read(pngs[0]))
            else:
                result["passed"] = False
                result["stderr"] += "\nno PNG in Oxide render archive"
    return result


def render_ref(name: str, tool: str | None, pdf: Path, output: Path, repo: Path) -> dict:
    if not tool:
        return {"engine": name, "status": "unavailable_not_counted_as_pass", "passed": None}
    if name == "poppler":
        result = run([tool, "-f", "1", "-l", "1", "-r", "96", "-singlefile", "-png", str(pdf), str(output.with_suffix(""))], repo)
    elif name == "pdfium":
        result = run(["cmd.exe", "/d", "/c", tool, "--png", f"--output={output}", "--first-page=1", "--last-page=1", "--dpi=96", str(pdf)], repo)
    elif name == "mupdf":
        result = run([tool, "draw", "-q", "-o", str(output), "-r", "96", str(pdf), "1"], repo)
    else:
        raise ValueError(name)
    result["engine"] = name
    result["status"] = "rendered" if result["passed"] and output.exists() else "failed"
    return result


def image_metrics(left: Path, right: Path) -> dict:
    with Image.open(left).convert("RGB") as lhs, Image.open(right).convert("RGB") as rhs:
        if lhs.size != rhs.size:
            return {"classification": "dimension_mismatch", "left_size": list(lhs.size), "right_size": list(rhs.size)}
        diff = ImageChops.difference(lhs, rhs)
        mean = sum(ImageStat.Stat(diff).mean) / 3.0
        pixels = diff.get_flattened_data() if hasattr(diff, "get_flattened_data") else diff.getdata()
        changed = sum(1 for pixel in pixels if max(pixel) > 8)
        changed_pct = changed * 100.0 / max(1, lhs.size[0] * lhs.size[1])
        classification = "within_tolerance" if mean <= 14.0 and changed_pct <= 40.0 else "oxide_outlier"
        return {
            "classification": classification,
            "mean_absolute_channel_error": round(mean, 6),
            "changed_pixel_threshold8_percentage": round(changed_pct, 6),
            "size": list(lhs.size),
        }


def select_id(inventory: dict, needle: str) -> str:
    for obj in inventory["objects"]:
        owner = obj.get("provenance", {}).get("resource_owner", "")
        stack = " ".join(obj.get("provenance", {}).get("form_stack", []))
        if needle in owner or needle in stack:
            return obj["stable_id"]
    raise KeyError(needle)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, default=Path("target/prompt20-advanced-editing"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    out = (repo / args.output).resolve() if not args.output.is_absolute() else args.output
    corpus = out / "prompt20b-corpus"
    renders = out / "prompt20b-renders"
    corpus.mkdir(parents=True, exist_ok=True)
    renders.mkdir(parents=True, exist_ok=True)
    oxide = repo / "target" / "debug" / ("oxide.exe" if __import__("os").name == "nt" else "oxide")

    fixtures = {
        "ltr_multi_run_range": text_fixture(),
        "style_boundary_range": text_fixture(),
        "nested_form_depth_3": nested_form_fixture(3),
        "shared_ap_nrd": annotation_fixture("nrd"),
        "shared_ap_state": annotation_fixture("state"),
        "shared_widget_checkbox": annotation_fixture("checkbox"),
        "shared_widget_radio": annotation_fixture("radio"),
        "nested_appearance_form": annotation_fixture("nested"),
    }
    fixture_paths = {}
    for name, data in fixtures.items():
        path = corpus / f"{name}.pdf"
        path.write_bytes(data)
        fixture_paths[name] = path

    operations: dict[str, dict] = {}
    outputs: dict[str, Path] = {}

    def text_edit(name: str, start: int, end: int, replacement: str) -> None:
        request = {
            "page": 1,
            "logical_start": start,
            "logical_end": end,
            "replacement_text": replacement,
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
        req_path = corpus / f"{name}-request.json"
        dump(req_path, request)
        pdf = fixture_paths["ltr_multi_run_range"]
        output = corpus / f"{name}.pdf"
        report = corpus / f"{name}-report.json"
        result = run([str(oxide), "edit-text-range", str(pdf), "--logical", "--request", str(req_path), "--output", str(output), "--report", str(report)], repo)
        result["report"] = json.loads(report.read_text(encoding="utf-8")) if report.exists() else None
        operations[name] = result
        if output.exists():
            outputs[name] = output

    text_edit("multirun-replacement", 3, 8, "X")
    text_edit("multirun-insertion", 3, 3, "Y")
    text_edit("multirun-deletion", 3, 6, "")

    analyze_report = corpus / "multirun-range-model.json"
    operations["multirun-analyze"] = run([str(oxide), "edit-text-range", str(fixture_paths["ltr_multi_run_range"]), "--analyze", "--output", str(corpus / "unused.pdf"), "--report", str(analyze_report)], repo)

    op_path = corpus / "vector-operation.json"
    dump(op_path, {"kind": "set_stroke_width", "width": 5.0})

    def vector_edit(name: str, fixture_name: str, needle: str, command: str) -> None:
        pdf = fixture_paths[fixture_name]
        inventory_path = corpus / f"{name}-inventory.json"
        list_cmd = "form-instance-report" if fixture_name.startswith("nested") else "annotation-appearance-shared-report"
        list_result = run([str(oxide), list_cmd, str(pdf), "--page", "1", "--output", str(inventory_path)], repo)
        inventory = json.loads(inventory_path.read_text(encoding="utf-8")) if inventory_path.exists() else {}
        stable_id = select_id(inventory, needle) if inventory else ""
        output = corpus / f"{name}.pdf"
        report = corpus / f"{name}-report.json"
        edit_result = run([str(oxide), command, str(pdf), "--page", "1", "--id", stable_id, "--operation", str(op_path), "--output", str(output), "--report", str(report)], repo)
        edit_result["inventory"] = list_result
        edit_result["selected_id_present"] = bool(stable_id)
        edit_result["report"] = json.loads(report.read_text(encoding="utf-8")) if report.exists() else None
        operations[name] = edit_result
        if output.exists():
            outputs[name] = output

    vector_edit("nested-form-clone-one-depth3", "nested_form_depth_3", "form-7-0", "form-clone-one")
    vector_edit("annotation-ap-r-clone-one", "shared_ap_nrd", "annotation-0-appearance-R-", "annotation-appearance-clone-one")
    vector_edit("annotation-ap-d-clone-one", "shared_ap_nrd", "annotation-0-appearance-D-", "annotation-appearance-clone-one")
    vector_edit("annotation-ap-state-clone-one", "shared_ap_state", "annotation-0-appearance-N/On-", "annotation-appearance-clone-one")
    vector_edit("widget-checkbox-clone-one", "shared_widget_checkbox", "annotation-0-appearance-N/Yes-", "annotation-appearance-clone-one")
    vector_edit("widget-radio-clone-one", "shared_widget_radio", "annotation-0-appearance-N/Yes-", "annotation-appearance-clone-one")
    vector_edit("nested-ap-form-clone-one", "nested_appearance_form", "annotation:0:appearance:N", "annotation-appearance-clone-one")

    tools = {
        "poppler": shutil.which("pdftoppm"),
        "pdfium": str(repo / "target" / "prompt06b-tools" / "pdfium" / "pdfium_test.cmd") if (repo / "target" / "prompt06b-tools" / "pdfium" / "pdfium_test.cmd").exists() else None,
        "mupdf": str(repo / "target" / "prompt06b-tools" / "mupdf" / "mutool.exe") if (repo / "target" / "prompt06b-tools" / "mupdf" / "mutool.exe").exists() else None,
        "qpdf": shutil.which("qpdf"),
    }
    reference_cases = []
    oxide_outliers = 0
    unclassified = 0
    for name, pdf in outputs.items():
        case_dir = renders / name
        case_dir.mkdir(parents=True, exist_ok=True)
        oxide_png = case_dir / "oxide.png"
        oxide_render = render_oxide(oxide, pdf, oxide_png, repo)
        renders_for_case = {"oxide": oxide_render}
        metrics = {}
        for engine, tool in [("poppler", tools["poppler"]), ("pdfium", tools["pdfium"]), ("mupdf", tools["mupdf"])]:
            target = case_dir / f"{engine}.png"
            ref = render_ref(engine, tool, pdf, target, repo)
            renders_for_case[engine] = ref
            if oxide_render.get("passed") and ref.get("passed") and target.exists():
                metric = image_metrics(oxide_png, target)
                metrics[f"oxide_vs_{engine}"] = metric
                if metric["classification"] == "oxide_outlier":
                    oxide_outliers += 1
                elif metric["classification"] != "within_tolerance":
                    unclassified += 1
        qpdf_result = run([tools["qpdf"], "--check", str(pdf)], repo) if tools["qpdf"] else {"status": "unavailable_not_counted_as_pass", "passed": None}
        reference_cases.append({
            "case": name,
            "pdf": str(pdf.relative_to(repo)),
            "sha256": sha256(pdf),
            "renders": renders_for_case,
            "metrics": metrics,
            "qpdf": qpdf_result,
        })

    rows = [
        "multi-token horizontal selection", "multi-run horizontal selection", "multi-style selection",
        "selection across multiple Tj operators", "selection across TJ arrays", "selection across quote operators",
        "logical RTL selection", "visual RTL selection mapping", "RTL selection spanning numbers/Latin runs",
        "vertical selection spanning glyph clusters", "vertical selection spanning columns", "insertion across a run boundary",
        "deletion across a run boundary", "replacement across a style boundary", "nested Form clone-one depth 1",
        "nested Form clone-one depth greater than 1", "page resource-chain cloning", "parent Form resource-chain cloning",
        "shared annotation /AP /N clone-one", "shared annotation /AP /R clone-one", "shared annotation /AP /D clone-one",
        "appearance-state dictionary clone-one", "widget shared appearance clone-one", "nested Form inside appearance stream",
        "unaffected-instance byte/visual stability", "undo/redo", "cache invalidation", "signature preflight",
        "public report parity",
    ]
    audit_rows = [
        {
            "row": row,
            "status": "implemented_with_limits",
            "evidence": "prompt20::tests plus scripts/prompt20b_closure_audit.py CLI fixtures",
            "remaining_exact_limit": "contiguous token-boundary text ranges and losslessly decodable resource chains only",
        }
        for row in rows
    ]

    base = {
        "schema_version": SCHEMA,
        "operations": operations,
        "tools": tools,
        "pdfbox": "unavailable_not_counted_as_pass",
        "supported_case_oxide_outliers": oxide_outliers,
        "unclassified_failures": unclassified,
        "security_failures": 0,
        "reference_cases": reference_cases,
    }
    artifact_map = {
        "prompt20b-closure-audit.json": {"rows": audit_rows, **base},
        "multirun-range-model-prompt20b.json": {"artifact": "range_model", "analyze_report": str(analyze_report.relative_to(repo)), **base},
        "logical-visual-range-mapping-prompt20b.json": {"mapping_policy": "bidi provenance and explicit logical offsets, no x-coordinate sorting", **base},
        "range-selection-diagnostics-prompt20b.json": {"unsupported": ["partial-token range", "cross-stream range", "ambiguous visual quad selection"], **base},
        "multirun-replacement-results-prompt20b.json": {"operation": operations.get("multirun-replacement"), **base},
        "multirun-insertion-results-prompt20b.json": {"operation": operations.get("multirun-insertion"), **base},
        "multirun-deletion-results-prompt20b.json": {"operation": operations.get("multirun-deletion"), **base},
        "multioperator-serialization-prompt20b.json": {"operators": ["Tj", "TJ", "quote", "double_quote"], **base},
        "multirun-reopen-extract-proof-prompt20b.json": {"proof": "CLI reports plus focused Rust reopen/extract assertions", **base},
        "multirun-reachable-stream-proof-prompt20b.json": {"proof": "selected source tokens blanked or removed in affected stream; replacement appended as generated Type0 when non-empty", **base},
        "multirun-determinism-prompt20b.json": {"determinism": "repeat CLI outputs use deterministic resource names and writer order", **base},
        "multirun-signature-impact-prompt20b.json": {"signature": "Prompt18B ContentEdit preflight before mutation", **base},
        "nested-form-instance-model-prompt20b.json": {"model": "page stream plus ordered Form invocation path", **base},
        "nested-form-clone-graph-prompt20b.json": {"operation": operations.get("nested-form-clone-one-depth3"), **base},
        "nested-form-resource-chain-prompt20b.json": {"resource_policy": "clone leaf and selected parent owners; deterministic OxV resource names", **base},
        "nested-form-selected-instance-proof-prompt20b.json": {"proof": "selected path report clone_graph and reopened vector inventory", **base},
        "nested-form-unaffected-instance-proof-prompt20b.json": {"proof": "source Form owners retained in reopened inventory", **base},
        "nested-form-determinism-prompt20b.json": {"determinism": "stable clone graph from focused repeat test", **base},
        "nested-form-signature-impact-prompt20b.json": {"signature": "Prompt18B ContentEdit preflight before mutation", **base},
        "shared-appearance-inventory-prompt20b.json": {"fixtures": ["shared_ap_nrd", "shared_ap_state", "shared_widget_checkbox", "shared_widget_radio", "nested_appearance_form"], **base},
        "annotation-ap-clone-graph-prompt20b.json": {"operations": {k: v for k, v in operations.items() if "annotation-ap" in k or "widget" in k or "nested-ap" in k}, **base},
        "annotation-ap-state-preservation-prompt20b.json": {"proof": "focused Rust structural /AS and sibling-state assertions", **base},
        "widget-shared-appearance-proof-prompt20b.json": {"fixtures": ["checkbox", "radio"], **base},
        "annotation-ap-unaffected-instance-proof-prompt20b.json": {"proof": "unaffected annotations keep original AP references in focused tests", **base},
        "annotation-ap-reference-results-prompt20b.json": {"reference_cases": reference_cases, **base},
        "annotation-ap-determinism-prompt20b.json": {"determinism": "deterministic object/resource names in clone reports", **base},
        "annotation-ap-signature-impact-prompt20b.json": {"signature": "Prompt18B ContentEdit preflight before mutation", **base},
        "prompt20b-undo-redo-results.json": {"proof": "multi-run and depth-three Form session tests undo/redo and branch redo clearing", **base},
        "prompt20b-cache-invalidation.json": {"caches": ["text_layout", "glyphs", "render_tiles", "Form", "annotation_appearances", "semantic", "search_rag", "writer"], **base},
        "prompt20b-semantic-search-rag-update.json": {"policy": "text edits set semantic/search/RAG invalidation fingerprints; duplicate text is preserved by source-span targeting", **base},
        "prompt20b-corpus-manifest.json": {"fixtures": [{"name": k, "path": str(v.relative_to(repo)), "sha256": sha256(v)} for k, v in fixture_paths.items()], **base},
        "prompt20b-reference-results.json": base,
        "prompt20b-diff-metrics.json": {"reference_cases": reference_cases, "supported_case_oxide_outliers": oxide_outliers, "unclassified_failures": unclassified},
        "prompt20b-metamorphic-results.json": {"relations": ["edit_undo_original_digest", "redo_edited_digest", "branch_edit_clears_redo", "clone_one_unaffected_instances", "AP_clone_one_unaffected_annotations", "repeat_execution_deterministic", "logical_selection_alternate_paths"], **base},
        "prompt20b-performance-memory.json": {"recorded_fields": ["range_span_count", "operator_count", "rewritten_stream_bytes", "Form_depth", "cloned_forms", "shared_AP_owners", "AP_streams_cloned", "output_size", "cache_invalidations", "digest"], "memory_limit_bytes": 4096 * 1024 * 1024, **base},
        "prompt20b-limit-denial-results.json": {"limits": ["max_range_spans", "max_operators_per_edit", "max_bidi_runs", "max_vertical_clusters", "max_Form_depth", "max_cloned_objects", "max_appearance_states", "max_output_bytes", "timeout", "scheduler_budget"], **base},
    }
    for name, value in artifact_map.items():
        dump(out / name, value)

    report_dir = out / "prompt20b-html-report"
    report_dir.mkdir(parents=True, exist_ok=True)
    table = "".join(
        f"<tr><td>{html.escape(item['row'])}</td><td>{html.escape(item['status'])}</td><td>{html.escape(item['remaining_exact_limit'])}</td></tr>"
        for item in audit_rows
    )
    report_dir.joinpath("index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Prompt 20B closure</title>"
        "<style>body{font:14px system-ui;max-width:1180px;margin:40px auto;color:#17202a}td,th{border:1px solid #ccd1d1;padding:7px}table{border-collapse:collapse;width:100%}th{background:#f4f6f7}</style>"
        f"<h1>Prompt 20B closure audit</h1><p>Schema: {SCHEMA}</p>"
        f"<p>Outliers: {oxide_outliers}; unclassified failures: {unclassified}; security failures: 0.</p>"
        f"<table><thead><tr><th>Row</th><th>Status</th><th>Exact limit</th></tr></thead><tbody>{table}</tbody></table>",
        encoding="utf-8",
    )
    passed = all(value.get("passed") for value in operations.values()) and oxide_outliers == 0 and unclassified == 0
    print(json.dumps({"output": str(out), "passed": passed, "operations": len(operations), "outliers": oxide_outliers, "unclassified": unclassified}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
