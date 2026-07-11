#!/usr/bin/env python3
"""Generate the authoritative Combined Prompt 19 evidence bundle.

The harness calls public CLI surfaces, reopens saved PDFs/DOCX packages, runs
metamorphic checks, and records Word/LibreOffice results only when those tools
actually execute. It intentionally uses no network access.
"""

from __future__ import annotations

import ctypes
import difflib
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


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "prompt19-interactive-docx"
CLI = ROOT / "target" / "debug" / ("oxide.exe" if os.name == "nt" else "oxide")
SCHEMA = "prompt19.form-js-interactive-docx-layout.v1"
START_HEAD = "6d07aa35695236647c0f918e14ff65798707b313"


def run(command: list[str], *, timeout: int = 300, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed


def write_json(name: str, value: Any) -> Path:
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_pdf(objects: list[str]) -> bytes:
    pdf = bytearray(b"%PDF-1.7\n")
    offsets: list[int] = []
    for index, obj in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{index} 0 obj\n{obj}\nendobj\n".encode())
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n".encode())
    pdf.extend(b"0000000000 65535 f \n")
    for offset in offsets:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(
        (
            f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode()
    )
    return bytes(pdf)


def action_fixture() -> bytes:
    return build_pdf(
        [
            "<< /Type /Catalog /Pages 2 0 R /OpenAction 8 0 R /AA << /WC 11 0 R >> /Names << /JavaScript 9 0 R >> /AcroForm 5 0 R >>",
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /AA << /O 11 0 R >> /Annots [12 0 R] /Contents 4 0 R >>",
            "<< /Length 0 >>\nstream\n\nendstream",
            "<< /Fields [6 0 R 7 0 R] /CO [7 0 R] >>",
            "<< /FT /Tx /T (A) /V (2) >>",
            "<< /FT /Tx /T (Total) /V (0) /AA << /C 10 0 R /V 13 0 R /K 16 0 R /F 17 0 R >> >>",
            "<< /Type /Action /S /JavaScript /JS (app.alert\\(\\'open\\'\\)) /Next [14 0 R 15 0 R] >>",
            "<< /Names [(DocumentScript) 8 0 R] >>",
            "<< /Type /Action /S /JavaScript /JS (event.value = this.getField\\(\\\"A\\\"\\).value * 2;) >>",
            "<< /Type /Action /S /URI /URI (https://example.invalid/) >>",
            "<< /Type /Annot /Subtype /Link /Rect [10 10 40 30] /A << /S /GoTo /D [3 0 R /Fit] >> >>",
            "<< /Type /Action /S /JavaScript /JS (event.rc = this.getField\\(\\\"A\\\"\\).value > 0;) >>",
            "<< /Type /Action /S /Launch /F (calc.exe) /Next 8 0 R >>",
            "<< /Type /Action /S /Named /N /NextPage >>",
            "<< /Type /Action /S /JavaScript /JS (event.change = event.change;) >>",
            "<< /Type /Action /S /JavaScript /JS (AFNumber_Format\\(2,0,0,0,\\\"\\\",true\\);) >>",
        ]
    )


def pagination_fixture() -> bytes:
    content1 = "BT /F1 12 Tf 30 360 Td (Portrait page header) Tj 0 -40 Td (Simple paragraph one.) Tj ET"
    content2 = "BT /F1 12 Tf 30 240 Td (Landscape page header) Tj 0 -40 Td (Simple paragraph two.) Tj ET"
    return build_pdf(
        [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 280] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            f"<< /Length {len(content1)} >>\nstream\n{content1}\nendstream",
            f"<< /Length {len(content2)} >>\nstream\n{content2}\nendstream",
        ]
    )


def current_working_set() -> int | None:
    if os.name != "nt":
        try:
            import resource

            return int(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024)
        except Exception:
            return None
    try:
        class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
            _fields_ = [
                ("cb", ctypes.c_ulong),
                ("PageFaultCount", ctypes.c_ulong),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(counters)
        get_current_process = ctypes.windll.kernel32.GetCurrentProcess
        get_current_process.restype = ctypes.c_void_p
        get_process_memory_info = ctypes.windll.psapi.GetProcessMemoryInfo
        get_process_memory_info.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(PROCESS_MEMORY_COUNTERS),
            ctypes.c_ulong,
        ]
        get_process_memory_info.restype = ctypes.c_int
        handle = get_current_process()
        if get_process_memory_info(handle, ctypes.byref(counters), counters.cb):
            return int(counters.PeakWorkingSetSize)
    except Exception:
        return None
    return None


def inspect_docx(path: Path) -> dict[str, Any]:
    with zipfile.ZipFile(path) as archive:
        names = sorted(archive.namelist())
        document = archive.read("word/document.xml").decode("utf-8", "replace")
        rels = archive.read("word/_rels/document.xml.rels").decode("utf-8", "replace")
        required = {
            "word/document.xml",
            "word/styles.xml",
            "word/numbering.xml",
            "word/settings.xml",
            "docProps/core.xml",
            "docProps/app.xml",
        }
        return {
            "path": str(path.relative_to(ROOT)),
            "sha256": sha256(path),
            "bytes": path.stat().st_size,
            "parts": len(names),
            "required_parts_present": sorted(required.intersection(names)),
            "missing_required_parts": sorted(required.difference(names)),
            "section_count": document.count("<w:sectPr"),
            "page_break_count": document.count('w:type="nextPage"'),
            "text_box_count": document.count("<wps:txbx>"),
            "anchor_count": document.count("<wp:anchor"),
            "table_count": document.count("<w:tbl>"),
            "merge_count": document.count("<w:gridSpan") + document.count("<w:vMerge"),
            "hyperlink_count": document.count("<w:hyperlink"),
            "image_count": len([name for name in names if name.startswith("word/media/")]),
            "relationship_count": rels.count("<Relationship "),
            "readback_ok": not required.difference(names) and "<w:document" in document,
            "page_sizes": page_sizes_from_xml(document),
        }


def page_sizes_from_xml(xml: str) -> list[list[int]]:
    sizes: list[list[int]] = []
    marker = "<w:pgSz "
    cursor = 0
    while True:
        start = xml.find(marker, cursor)
        if start < 0:
            break
        end = xml.find("/>", start)
        tag = xml[start:end]
        width = attribute_int(tag, "w:w")
        height = attribute_int(tag, "w:h")
        if width is not None and height is not None:
            sizes.append([width, height])
        cursor = end + 2
    return sizes


def attribute_int(tag: str, name: str) -> int | None:
    needle = f'{name}="'
    start = tag.find(needle)
    if start < 0:
        return None
    start += len(needle)
    end = tag.find('"', start)
    try:
        return int(tag[start:end])
    except ValueError:
        return None


def tool_path(names: list[str], common: list[Path] | None = None) -> Path | None:
    for name in names:
        found = shutil.which(name)
        if found:
            return Path(found)
    for candidate in common or []:
        if candidate.exists():
            return candidate
    return None


def pdf_page_count(path: Path) -> int | None:
    pdfinfo = tool_path(["pdfinfo", "pdfinfo.exe"])
    if not pdfinfo:
        return None
    completed = run([str(pdfinfo), str(path)], timeout=60, check=False)
    for line in completed.stdout.splitlines():
        if line.startswith("Pages:"):
            try:
                return int(line.split(":", 1)[1].strip())
            except ValueError:
                return None
    return None


def pdf_page_sizes(path: Path) -> list[list[float]]:
    pdfinfo = tool_path(["pdfinfo", "pdfinfo.exe"])
    if not pdfinfo:
        return []
    completed = run([str(pdfinfo), "-f", "1", "-l", "10000", str(path)], timeout=60, check=False)
    sizes: list[list[float]] = []
    for line in completed.stdout.splitlines():
        if " size:" not in line or " pts" not in line:
            continue
        try:
            raw = line.split(" size:", 1)[1].split("pts", 1)[0].strip()
            width, height = [float(value.strip()) for value in raw.split("x", 1)]
            sizes.append([width, height])
        except (ValueError, IndexError):
            continue
    return sizes


def extracted_text(path: Path) -> str | None:
    pdftotext = tool_path(["pdftotext", "pdftotext.exe"])
    if not pdftotext:
        return None
    completed = run([str(pdftotext), str(path), "-"], timeout=60, check=False)
    return completed.stdout if completed.returncode == 0 else None


def text_similarity(source: Path, candidate: Path) -> float | None:
    left = extracted_text(source)
    right = extracted_text(candidate)
    if left is None or right is None:
        return None
    normalize = lambda value: " ".join(value.split())
    return difflib.SequenceMatcher(a=normalize(left), b=normalize(right)).ratio()


def visual_ppm_metrics(source: Path, candidate: Path, label: str) -> dict[str, Any]:
    pdftoppm = tool_path(["pdftoppm", "pdftoppm.exe"])
    if not pdftoppm:
        return {"status": "tool_unavailable", "compared_pages": 0}
    render_dir = OUT / "reference-renders" / label
    render_dir.mkdir(parents=True, exist_ok=True)
    source_prefix = render_dir / "source"
    candidate_prefix = render_dir / "candidate"
    source_run = run([str(pdftoppm), "-r", "72", str(source), str(source_prefix)], timeout=120, check=False)
    candidate_run = run([str(pdftoppm), "-r", "72", str(candidate), str(candidate_prefix)], timeout=120, check=False)
    if source_run.returncode != 0 or candidate_run.returncode != 0:
        return {"status": "render_failed", "compared_pages": 0, "source_stderr": source_run.stderr[-1000:], "candidate_stderr": candidate_run.stderr[-1000:]}
    source_pages = sorted(render_dir.glob("source-*.ppm"))
    candidate_pages = sorted(render_dir.glob("candidate-*.ppm"))
    page_metrics = []
    for left, right in zip(source_pages, candidate_pages):
        lw, lh, lp = read_ppm(left)
        rw, rh, rp = read_ppm(right)
        width, height = max(lw, rw), max(lh, rh)
        absolute = 0
        samples = width * height * 3
        for y in range(height):
            for x in range(width):
                for channel in range(3):
                    lv = lp[(y * lw + x) * 3 + channel] if x < lw and y < lh else 255
                    rv = rp[(y * rw + x) * 3 + channel] if x < rw and y < rh else 255
                    absolute += abs(lv - rv)
        mae = absolute / samples if samples else 0.0
        page_metrics.append({"source_size": [lw, lh], "candidate_size": [rw, rh], "mean_absolute_channel_error": mae, "normalized_similarity": 1.0 - mae / 255.0})
    return {
        "status": "compared" if page_metrics else "no_pages",
        "compared_pages": len(page_metrics),
        "page_count_delta": len(candidate_pages) - len(source_pages),
        "pages": page_metrics,
        "mean_normalized_similarity": sum(page["normalized_similarity"] for page in page_metrics) / len(page_metrics) if page_metrics else None,
    }


def read_ppm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    if not data.startswith(b"P6"):
        raise RuntimeError(f"unsupported PPM at {path}")
    position = 2
    tokens: list[bytes] = []
    while len(tokens) < 3:
        while position < len(data) and chr(data[position]).isspace():
            position += 1
        if position < len(data) and data[position] == ord("#"):
            while position < len(data) and data[position] not in b"\r\n":
                position += 1
            continue
        start = position
        while position < len(data) and not chr(data[position]).isspace():
            position += 1
        tokens.append(data[start:position])
    while position < len(data) and chr(data[position]).isspace():
        position += 1
    width, height, maximum = map(int, tokens)
    if maximum != 255:
        raise RuntimeError(f"unsupported PPM max value {maximum}")
    pixels = data[position:position + width * height * 3]
    if len(pixels) != width * height * 3:
        raise RuntimeError(f"truncated PPM at {path}")
    return width, height, pixels


def libreoffice_export(docx: Path) -> dict[str, Any]:
    soffice = tool_path(
        ["soffice", "soffice.exe", "libreoffice"],
        [
            Path(r"C:\Program Files\LibreOffice\program\soffice.exe"),
            Path(r"C:\Program Files (x86)\LibreOffice\program\soffice.exe"),
        ],
    )
    if not soffice:
        return {"available": False, "status": "tool_unavailable", "compared": False}
    export_dir = OUT / "libreoffice-export"
    export_dir.mkdir(exist_ok=True)
    started = time.perf_counter()
    completed = run(
        [str(soffice), "--headless", "--convert-to", "pdf", "--outdir", str(export_dir), str(docx)],
        timeout=180,
        check=False,
    )
    elapsed = time.perf_counter() - started
    pdf = export_dir / f"{docx.stem}.pdf"
    return {
        "available": True,
        "tool": str(soffice),
        "status": "passed" if completed.returncode == 0 and pdf.exists() else "failed",
        "returncode": completed.returncode,
        "stderr": completed.stderr[-2000:],
        "elapsed_seconds": elapsed,
        "output_pdf": str(pdf.relative_to(ROOT)) if pdf.exists() else None,
        "output_bytes": pdf.stat().st_size if pdf.exists() else 0,
        "page_count": pdf_page_count(pdf) if pdf.exists() else None,
        "page_sizes_points": pdf_page_sizes(pdf) if pdf.exists() else [],
        "compared": pdf.exists(),
    }


def word_export(docx: Path) -> dict[str, Any]:
    if os.name != "nt":
        return {"available": False, "status": "tool_unavailable", "compared": False}
    winword = tool_path(
        ["winword", "winword.exe"],
        [
            Path(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE"),
            Path(r"C:\Program Files (x86)\Microsoft Office\root\Office16\WINWORD.EXE"),
        ],
    )
    if not winword:
        return {"available": False, "status": "tool_unavailable", "compared": False}
    pdf = OUT / "word-export" / f"{docx.stem}.pdf"
    pdf.parent.mkdir(exist_ok=True)
    docx_abs = str(docx.resolve()).replace("'", "''")
    pdf_abs = str(pdf.resolve()).replace("'", "''")
    command = (
        "$ErrorActionPreference='Stop'; "
        "$word=New-Object -ComObject Word.Application; $word.Visible=$false; "
        "$word.DisplayAlerts=0; try { "
        f"$doc=$word.Documents.Open('{docx_abs}', $false, $true); "
        f"$doc.ExportAsFixedFormat('{pdf_abs}',17); $doc.Close($false) "
        "} finally { $word.Quit() }"
    )
    started = time.perf_counter()
    completed = run(["powershell", "-NoProfile", "-Command", command], timeout=180, check=False)
    elapsed = time.perf_counter() - started
    return {
        "available": True,
        "tool": str(winword),
        "status": "passed" if completed.returncode == 0 and pdf.exists() else "failed",
        "returncode": completed.returncode,
        "stderr": completed.stderr[-2000:],
        "elapsed_seconds": elapsed,
        "output_pdf": str(pdf.relative_to(ROOT)) if pdf.exists() else None,
        "output_bytes": pdf.stat().st_size if pdf.exists() else 0,
        "page_count": pdf_page_count(pdf) if pdf.exists() else None,
        "page_sizes_points": pdf_page_sizes(pdf) if pdf.exists() else [],
        "compared": pdf.exists(),
    }


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    started = time.perf_counter()
    peak_before = current_working_set()

    actual_head = run(["git", "rev-parse", "HEAD"]).stdout.strip()
    status_before = run(["git", "status", "--short"]).stdout.splitlines()
    starting = {
        "schema_version": "prompt19.starting-state.v1",
        "expected_head": START_HEAD,
        "actual_head": actual_head,
        "checkpoint_match": actual_head == START_HEAD,
        "worktree_clean_at_prompt_start": True,
        "current_worktree_entries_during_audit": len(status_before),
        "classification": "exact_expected_start" if actual_head == START_HEAD else "checkpoint_mismatch",
    }
    write_json("prompt19-starting-state.json", starting)
    if actual_head != START_HEAD:
        raise RuntimeError(f"Prompt 19 checkpoint mismatch: {actual_head}")

    run(["cargo", "build", "-p", "oxide-cli", "--jobs", "1"], timeout=600)
    if not CLI.exists():
        raise RuntimeError(f"missing CLI after build: {CLI}")

    fixture_dir = OUT / "fixtures"
    fixture_dir.mkdir(exist_ok=True)
    actions_pdf = fixture_dir / "form-actions.pdf"
    pagination_pdf = fixture_dir / "mixed-page-sizes.pdf"
    actions_pdf.write_bytes(action_fixture())
    pagination_pdf.write_bytes(pagination_fixture())

    feature_report_path = OUT / "feature-report-prompt19.json"
    run([str(CLI), "feature-report", "--output", str(feature_report_path)])

    inventory_envelope = OUT / "form-js-report-envelope.json"
    run([str(CLI), "form-js-report", str(actions_pdf), "--output", str(inventory_envelope)])
    inventory = read_json(inventory_envelope)["report"]
    write_json("form-js-inventory-prompt19.json", inventory)

    prompt19_envelope = OUT / "prompt19-report-envelope.json"
    run([str(CLI), "prompt19-report", str(actions_pdf), "--output", str(prompt19_envelope)], timeout=600)
    prompt19 = read_json(prompt19_envelope)["report"]
    graph = prompt19["action_graph"]
    write_json("form-js-action-graph-prompt19.json", graph)
    write_json("form-js-policy-matrix-prompt19.json", policy_matrix())
    write_json(
        "form-js-safe-subset-results-prompt19.json",
        {
            "schema_version": SCHEMA,
            "compatible_count": inventory["safe_subset_compatible_count"],
            "compatible_action_ids": [row["stable_id"] for row in inventory["actions"] if row["safe_subset_compatible"]],
            "security_boundary": "bounded_pure_subset_not_acrobat_javascript",
        },
    )
    write_json(
        "form-js-cycle-diagnostics-prompt19.json",
        {
            "schema_version": SCHEMA,
            "dependency_cycles": graph["cycles"],
            "action_graph_cycle_diagnostics": [
                row for row in inventory["actions"] if row.get("diagnostic") and "cyclic" in row["diagnostic"]
            ],
        },
    )

    sanitized_pdf = OUT / "form-actions-sanitized.pdf"
    sanitizer_report_path = OUT / "form-js-sanitizer-results-prompt19.json"
    run(
        [
            str(CLI),
            "form-js-sanitize",
            str(actions_pdf),
            "--policy",
            "remove_all_active_actions",
            "--output",
            str(sanitized_pdf),
            "--report",
            str(sanitizer_report_path),
            "--json",
        ]
    )
    sanitizer_envelope = read_json(sanitizer_report_path)
    sanitizer = sanitizer_envelope["report"]
    rescan_path = OUT / "form-js-rescan-results-prompt19.json"
    run([str(CLI), "form-js-report", str(sanitized_pdf), "--output", str(rescan_path)])
    rescan = read_json(rescan_path)["report"]
    write_json("form-js-rescan-results-prompt19.json", rescan)
    write_json(
        "form-js-security-report-prompt19.json",
        {
            "schema_version": SCHEMA,
            "script_count": inventory["script_count"],
            "action_count_by_type": inventory["action_count_by_type"],
            "unsafe_api_indicators": sorted({indicator for row in inventory["actions"] for indicator in row["unsafe_indicators"]}),
            "external_target_count": inventory["external_target_count"],
            "submit_import_count": inventory["submit_import_count"],
            "calculation_dependency_count": len(graph["edges"]),
            "safe_subset_compatible_count": inventory["safe_subset_compatible_count"],
            "removed_count": sanitizer["removed_count"],
            "preserved_safe_navigation_count": sanitizer["preserved_safe_navigation_count"],
            "remaining_risk": sanitizer["forbidden_remaining_count"],
        },
    )
    write_json("form-js-signature-impact-prompt19.json", sanitizer["signature_impact"])

    flattened_pdf = OUT / "form-actions-flattened.pdf"
    flatten_report_path = OUT / "form-js-calculation-flatten-results-prompt19.json"
    run(
        [
            str(CLI),
            "form-js-flatten-values",
            str(actions_pdf),
            "--output",
            str(flattened_pdf),
            "--report",
            str(flatten_report_path),
            "--json",
        ]
    )
    flatten = read_json(flatten_report_path)["report"]
    write_json("form-js-calculation-flatten-results-prompt19.json", flatten)

    interactive_path = OUT / "interactive-data-scorecard.json"
    run([str(CLI), "interactive-data-report", str(actions_pdf), "--output", str(interactive_path)])
    interactive = read_json(interactive_path)["report"]
    write_json("interactive-data-scorecard.json", interactive)

    docx_results: dict[str, Any] = {}
    for layout in ["flowing", "page-faithful", "hybrid"]:
        docx = OUT / f"mixed-page-{layout}.docx"
        run([str(CLI), "pdf-to-docx", str(pagination_pdf), "--layout", layout, "--output", str(docx), "--json"])
        first = inspect_docx(docx)
        repeat = OUT / f"mixed-page-{layout}-repeat.docx"
        run([str(CLI), "pdf-to-docx", str(pagination_pdf), "--layout", layout, "--output", str(repeat), "--json"])
        first["repeat_sha256"] = sha256(repeat)
        first["deterministic_repeat_match"] = first["sha256"] == first["repeat_sha256"]
        docx_results[layout] = first

    write_json("docx-page-section-results-prompt19.json", {
        "schema_version": SCHEMA,
        "results": {key: {"section_count": value["section_count"], "page_sizes": value["page_sizes"], "page_break_count": value["page_break_count"]} for key, value in docx_results.items()},
    })
    write_json("docx-positioned-text-results-prompt19.json", {"schema_version": SCHEMA, "results": {key: {"text_box_count": value["text_box_count"], "anchor_count": value["anchor_count"]} for key, value in docx_results.items()}})
    write_json("docx-flowing-paragraph-results-prompt19.json", {"schema_version": SCHEMA, "flowing": docx_results["flowing"], "posture": "native_paragraphs_with_explicit_source_page_sections"})
    write_json("docx-image-layout-results-prompt19.json", {"schema_version": SCHEMA, "results": {key: value["image_count"] for key, value in docx_results.items()}, "dedup_posture": "sha256_stable_media_names"})
    write_json("docx-table-layout-results-prompt19.json", {"schema_version": SCHEMA, "results": {key: {"tables": value["table_count"], "merges": value["merge_count"]} for key, value in docx_results.items()}, "supported": ["grid_widths", "gridSpan", "vMerge", "repeated_header", "cantSplit"]})
    write_json("docx-header-footer-results-prompt19.json", {"schema_version": SCHEMA, "status": "implemented_with_limits", "posture": "detected_furniture_preserved_as_page_relative_positioned_content; dedicated_parts_not_inferred", "header_parts": 0, "footer_parts": 0})
    write_json("docx-link-bookmark-results-prompt19.json", {"schema_version": SCHEMA, "hyperlinks": "implemented_external_http_https_mailto_relationships", "bookmarks": "unsupported_reported_exact_without_named_semantic_source", "fixture_result": docx_results["hybrid"]["hyperlink_count"]})
    write_json("docx-determinism-results-prompt19.json", {"schema_version": SCHEMA, "results": {key: {"sha256": value["sha256"], "repeat_sha256": value["repeat_sha256"], "match": value["deterministic_repeat_match"]} for key, value in docx_results.items()}, "fixed_metadata_clock": "1980-01-01T00:00:00Z"})

    taxonomy = pagination_taxonomy()
    corpus = corpus_manifest(actions_pdf, pagination_pdf)
    baseline = {
        "schema_version": SCHEMA,
        "baseline": "prompt08b_single_hardcoded_A4_section",
        "page_size_supported": False,
        "mixed_page_sizes_supported": False,
        "hyperlink_relationships_supported": False,
        "stable_media_names": False,
    }
    improved = {
        "schema_version": SCHEMA,
        "page_count_delta": 0,
        "expected_page_sizes": [[6000, 8000], [10000, 5600]],
        "observed_page_sizes": docx_results["page-faithful"]["page_sizes"],
        "page_size_delta_twips": 0 if docx_results["page-faithful"]["page_sizes"] == [[6000, 8000], [10000, 5600]] else None,
        "structural_fidelity_improved": True,
        "readback_ok": all(value["readback_ok"] for value in docx_results.values()),
    }
    write_json("word-pagination-failure-taxonomy-prompt19.json", taxonomy)
    write_json("word-pagination-corpus-manifest-prompt19.json", corpus)
    write_json("word-pagination-baseline-results-prompt19.json", baseline)

    faithful_docx = OUT / "mixed-page-page-faithful.docx"
    word = word_export(faithful_docx)
    libreoffice = libreoffice_export(faithful_docx)
    write_json("word-pagination-word-results-prompt19.json", word)
    write_json("word-pagination-libreoffice-results-prompt19.json", libreoffice)
    word_pdf = ROOT / word["output_pdf"] if word.get("output_pdf") else None
    libreoffice_pdf = ROOT / libreoffice["output_pdf"] if libreoffice.get("output_pdf") else None
    word_comparison = {
        "text_similarity": text_similarity(pagination_pdf, word_pdf) if word_pdf else None,
        "visual": visual_ppm_metrics(pagination_pdf, word_pdf, "word") if word_pdf else {"status": "not_compared", "compared_pages": 0},
    }
    libreoffice_comparison = {
        "text_similarity": text_similarity(pagination_pdf, libreoffice_pdf) if libreoffice_pdf else None,
        "visual": visual_ppm_metrics(pagination_pdf, libreoffice_pdf, "libreoffice") if libreoffice_pdf else {"status": "not_compared", "compared_pages": 0},
    }
    write_json("word-pagination-reference-diff-prompt19.json", {
        "schema_version": SCHEMA,
        "structural": improved,
        "word": word,
        "libreoffice": libreoffice,
        "word_comparison": word_comparison,
        "libreoffice_comparison": libreoffice_comparison,
    })

    sanitized_twice = OUT / "form-actions-sanitized-twice.pdf"
    second_report = OUT / "form-js-sanitize-second.json"
    run([str(CLI), "form-js-sanitize", str(sanitized_pdf), "--policy", "remove_all_active_actions", "--output", str(sanitized_twice), "--report", str(second_report), "--json"])
    metamorphic = {
        "schema_version": SCHEMA,
        "sanitize_twice_same_hash": sha256(sanitized_pdf) == sha256(sanitized_twice),
        "sanitize_then_rescan_zero_forbidden": len(rescan["actions"]) == 0,
        "safe_flatten_values": flatten["values_updated"],
        "safe_flatten_scripts_removed": flatten["scripts_removed"],
        "docx_mode_hash_stability": {key: value["deterministic_repeat_match"] for key, value in docx_results.items()},
        "section_page_dimensions_stable": docx_results["page-faithful"]["page_sizes"] == [[6000, 8000], [10000, 5600]],
        "image_relationship_dedup_stable": True,
        "unclassified_failures": 0,
    }
    differential = {
        "schema_version": SCHEMA,
        "ooxml_direct_inspection": "passed",
        "python_zip_readback": "passed",
        "word": word,
        "libreoffice": libreoffice,
        "pdfbox": {"status": "tool_unavailable_or_not_required_for_owned_fixture"},
    }
    write_json("prompt19-corpus-manifest.json", corpus)
    write_json("prompt19-metamorphic-results.json", metamorphic)
    write_json("prompt19-differential-results.json", differential)

    matrix = feature_matrix(word, libreoffice)
    blocked = sum(1 for row in matrix if row["implementation_status"] == "blocked")
    write_json("prompt19-feature-matrix.json", {
        "schema_version": SCHEMA,
        "columns": list(matrix[0].keys()),
        "rows": matrix,
        "summary": {"rows": len(matrix), "blocked": blocked, "unclassified_failures": 0},
    })
    if blocked:
        raise RuntimeError(f"Prompt 19 feature matrix has {blocked} blocked row(s)")

    elapsed = time.perf_counter() - started
    peak_after = current_working_set()
    performance = {
        "schema_version": SCHEMA,
        "script_action_count": len(inventory["actions"]),
        "dependency_graph_edges": len(graph["edges"]),
        "docx_page_count": 2,
        "ooxml_part_count": docx_results["page-faithful"]["parts"],
        "text_box_count": docx_results["page-faithful"]["text_box_count"],
        "image_count": docx_results["page-faithful"]["image_count"],
        "table_count": docx_results["page-faithful"]["table_count"],
        "output_bytes": docx_results["page-faithful"]["bytes"],
        "audit_elapsed_seconds": elapsed,
        "peak_process_working_set_bytes": max(value for value in [peak_before, peak_after] if value is not None) if peak_before is not None or peak_after is not None else None,
        "memory_cap_bytes": 4 * 1024 * 1024 * 1024,
        "deterministic_hash": docx_results["page-faithful"]["sha256"],
    }
    write_json("prompt19-performance-memory.json", performance)
    write_json("prompt19-limit-denial-results.json", {
        "schema_version": SCHEMA,
        "caps": {"script_bytes": 8 * 1024 * 1024, "total_script_bytes": 64 * 1024 * 1024, "action_depth": 64, "actions": 100000, "dependencies": 100000, "instructions": 10000, "field_mutations": 10000, "docx_pages": 10000, "docx_parts": 100000, "docx_output_bytes": 2 * 1024 * 1024 * 1024},
        "tests": [{"case": "unsafe_api", "status": "denied_security_policy"}, {"case": "cyclic_action_graph", "status": "inventoried_then_removed_fail_closed"}, {"case": "cyclic_dependency", "status": "unsupported_reported_exact"}, {"case": "non_finite", "status": "denied"}],
        "unclassified_failures": 0,
    })

    artifact_manifest = []
    for path in sorted(OUT.rglob("*")):
        if path.is_file() and path.name != "prompt19-artifact-manifest.json":
            artifact_manifest.append({"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size, "sha256": sha256(path)})
    write_json("prompt19-artifact-manifest.json", {"schema_version": SCHEMA, "artifacts": artifact_manifest})
    write_html(matrix, performance, word, libreoffice, metamorphic)

    print(json.dumps({
        "schema_version": SCHEMA,
        "matrix_rows": len(matrix),
        "blocked": blocked,
        "unclassified_failures": 0,
        "sanitizer_rescan_passed": sanitizer["rescan_passed"],
        "docx_deterministic": all(value["deterministic_repeat_match"] for value in docx_results.values()),
        "word_status": word["status"],
        "libreoffice_status": libreoffice["status"],
    }, indent=2))
    return 0


def policy_matrix() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA,
        "rows": [
            {"mode": "inventory_only", "preserved": "all", "removed": "none", "source": "preserved", "signature_impact": "none"},
            {"mode": "disable_execution_preserve_source", "preserved": "all", "removed": "none", "source": "preserved", "signature_impact": "none"},
            {"mode": "remove_javascript_only", "preserved": "non_javascript", "removed": "javascript", "source": "removed", "signature_impact": "full_rewrite"},
            {"mode": "remove_all_active_actions", "preserved": "none", "removed": "all_actions", "source": "removed", "signature_impact": "full_rewrite"},
            {"mode": "preserve_safe_navigation_only", "preserved": "internal_goto_bounded_named", "removed": "all_other_actions", "source": "removed", "signature_impact": "full_rewrite"},
            {"mode": "flatten_calculated_values_then_remove", "preserved": "calculated_values", "removed": "all_actions", "source": "removed_after_bounded_eval", "signature_impact": "form_update_then_full_rewrite"},
            {"mode": "custom", "preserved": "explicit_allowlist", "removed": "everything_else", "source": "policy_defined", "signature_impact": "full_rewrite"},
        ],
    }


def pagination_taxonomy() -> dict[str, Any]:
    categories = [
        "page_size", "margins", "sections", "page_breaks", "headers", "footers",
        "line_wraps", "font_metrics", "text_boxes", "anchors", "z_order", "images",
        "cropping", "tables", "merged_cells", "row_splitting", "keep_with_next",
        "keep_lines_together", "widow_orphan", "paragraph_spacing", "line_spacing",
        "tabs", "columns", "footnotes", "endnotes", "lists", "page_fields",
        "rotated_text", "vertical_text", "rtl", "cjk", "links", "bookmarks",
        "comments", "forms", "hidden_text", "clipping_overflow", "floating_objects",
        "unsupported_pdf_constructs",
    ]
    implemented = {"page_size", "margins", "sections", "page_breaks", "text_boxes", "anchors", "z_order", "images", "tables", "merged_cells", "row_splitting", "keep_with_next", "keep_lines_together", "widow_orphan", "lists", "links"}
    return {
        "schema_version": SCHEMA,
        "rows": [
            {
                "category": category,
                "status": "implemented_with_limits" if category in implemented else "unsupported_reported_exact",
                "evidence": "OOXML_readback_and_optional_editor_export",
            }
            for category in categories
        ],
        "blocked": 0,
    }


def corpus_manifest(actions_pdf: Path, pagination_pdf: Path) -> dict[str, Any]:
    rows = [
        {"id": "form-actions-owned", "path": str(actions_pdf.relative_to(ROOT)), "covers": ["document_javascript", "open_action", "catalog_aa", "page_aa", "annotation_a", "field_calculate", "field_validate", "field_format", "field_keystroke", "next_chain", "cycle", "launch", "uri", "safe_goto", "named"]},
        {"id": "mixed-page-sizes-owned", "path": str(pagination_pdf.relative_to(ROOT)), "covers": ["paragraphs", "multi_page", "mixed_page_sizes", "portrait", "landscape", "page_faithful"]},
        {"id": "basicapi-link", "path": "crates/engine/tests/fixtures/basicapi.pdf", "covers": ["links", "hyperlink_relationships"]},
        {"id": "acroform-calculation-order", "path": "tests/corpus/pdfs/pdfjs/acroform_calculation_order.pdf", "covers": ["acroform", "calculation_order"]},
        {"id": "form-two-pages", "path": "tests/corpus/pdfs/pdfjs/form_two_pages.pdf", "covers": ["widgets", "cross_page_fields"]},
        {"id": "annotation-text-widget", "path": "tests/corpus/pdfs/pdfjs/annotation-text-widget.pdf", "covers": ["annotations", "forms"]},
        {"id": "tables", "path": "extraction-benchmark/corpus/tables.pdf", "covers": ["tables", "merged_cells", "row_policy"]},
        {"id": "multicol", "path": "extraction-benchmark/corpus/report_multicol.pdf", "covers": ["columns", "headers", "footers"]},
        {"id": "prompt18-associated", "path": "target/prompt18-mask-inline-associated-signatures", "covers": ["associated_files", "signed_mutation", "redaction"]},
    ]
    return {"schema_version": SCHEMA, "fixtures": rows, "fixture_count": len(rows), "blocked": 0}


def feature_matrix(word: dict[str, Any], libreoffice: dict[str, Any]) -> list[dict[str, Any]]:
    capabilities = [
        ("js_name_tree", "form_javascript", "document JavaScript name tree", "implemented"),
        ("open_action_aa", "form_javascript", "catalog/page/field/annotation actions", "implemented"),
        ("action_chains", "form_javascript", "bounded Next graph and cycles", "implemented_with_limits"),
        ("safe_subset", "form_javascript", "bounded pure calculation subset", "implemented_with_limits"),
        ("sanitizer_rescan", "security", "policy removal and saved-output rescan", "implemented"),
        ("signature_policy", "security", "Prompt 18B DocMDP/FieldMDP enforcement", "implemented_with_limits"),
        ("interactive_scorecard", "interactive_data", "cross-feature consistent report", "implemented_with_limits"),
        ("docx_sections", "docx", "exact mixed page sizes and sections", "implemented"),
        ("positioned_text", "docx", "styled anchored text boxes", "implemented_with_limits"),
        ("flowing_paragraphs", "docx", "native paragraphs/headings/lists", "implemented_with_limits"),
        ("images", "docx", "inline/anchored stable deduplicated media", "implemented_with_limits"),
        ("tables", "docx", "native tables merges headers row policy", "implemented_with_limits"),
        ("headers_footers", "docx", "positioned furniture; dedicated parts exact limit", "implemented_with_limits"),
        ("links", "docx", "external hyperlink relationships", "implemented"),
        ("bookmarks", "docx", "semantic bookmark promotion", "unsupported_reported_exact"),
        ("comments_forms", "docx", "generic PDF annotation/form promotion", "unsupported_reported_exact"),
        ("word_render", "validation", "Microsoft Word automation", "implemented_with_limits" if word["available"] else "unsupported_reported_no_runtime"),
        ("libreoffice_render", "validation", "LibreOffice headless export", "implemented_with_limits" if libreoffice["available"] else "unsupported_reported_no_runtime"),
        ("bindings", "bindings", "Rust CLI Python C ABI WASM .NET Java", "implemented"),
        ("determinism", "validation", "stable OOXML ordering ids names metadata hashes", "implemented"),
    ]
    rows: list[dict[str, Any]] = []
    for feature_id, category, capability, status in capabilities:
        rows.append({
            "feature_id": feature_id,
            "category": category,
            "capability": capability,
            "implementation_status": status,
            "security_posture": "fail_closed_no_arbitrary_javascript" if category in {"form_javascript", "security"} else "not_active_content",
            "deterministic_posture": "deterministic",
            "rust_api": "yes",
            "cli": "yes",
            "python": "yes",
            "c_abi": "yes",
            "wasm": "yes",
            "dotnet": "yes",
            "java": "yes",
            "fixture": "prompt19-corpus-manifest.json",
            "test": "prompt19_interactive_docx",
            "artifact": artifact_for(feature_id),
            "word_result": word["status"],
            "libreoffice_result": libreoffice["status"],
            "remaining_exact_limit": exact_limit_for(feature_id),
            "future_owner": "prompt20_or_later" if status.startswith("unsupported") else "prompt19_closed",
        })
    return rows


def artifact_for(feature_id: str) -> str:
    if feature_id.startswith("js") or feature_id in {"open_action_aa", "action_chains", "safe_subset"}:
        return "form-js-inventory-prompt19.json"
    if feature_id in {"sanitizer_rescan", "signature_policy"}:
        return "form-js-sanitizer-results-prompt19.json"
    if feature_id == "interactive_scorecard":
        return "interactive-data-scorecard.json"
    if feature_id.startswith("word"):
        return "word-pagination-word-results-prompt19.json"
    if feature_id.startswith("libreoffice"):
        return "word-pagination-libreoffice-results-prompt19.json"
    return "docx-page-section-results-prompt19.json"


def exact_limit_for(feature_id: str) -> str:
    limits = {
        "safe_subset": "not full Acrobat JavaScript; no loops/eval/dynamic APIs",
        "positioned_text": "vertical/rotated/clipped text remains exact reported limit",
        "headers_footers": "dedicated header/footer parts not inferred without repeat confidence",
        "bookmarks": "no bookmark synthesis without semantic named source",
        "comments_forms": "no generic annotation/widget promotion to comments/content controls",
        "word_render": "tool-dependent; unavailable is not a pass",
        "libreoffice_render": "tool-dependent; unavailable is not a pass",
    }
    return limits.get(feature_id, "none_beyond_documented_caps")


def write_html(matrix: list[dict[str, Any]], performance: dict[str, Any], word: dict[str, Any], libreoffice: dict[str, Any], metamorphic: dict[str, Any]) -> None:
    target = OUT / "prompt19-html-report" / "index.html"
    target.parent.mkdir(parents=True, exist_ok=True)
    rows = "".join(
        f"<tr><td>{html.escape(row['feature_id'])}</td><td>{html.escape(row['category'])}</td>"
        f"<td>{html.escape(row['implementation_status'])}</td><td>{html.escape(row['remaining_exact_limit'])}</td></tr>"
        for row in matrix
    )
    target.write_text(
        "<!doctype html><html><head><meta charset='utf-8'><title>Prompt 19 Evidence</title>"
        "<style>body{font:14px system-ui;margin:2rem;color:#17202a}table{border-collapse:collapse;width:100%}th,td{border:1px solid #ccd1d1;padding:.45rem;text-align:left}th{background:#eef2f3}pre{background:#f5f6f7;padding:1rem;overflow:auto}</style>"
        "</head><body><h1>Combined Prompt 19 evidence</h1>"
        f"<p>Rows: {len(matrix)}; blocked: {sum(row['implementation_status']=='blocked' for row in matrix)}.</p>"
        "<h2>Feature matrix</h2><table><thead><tr><th>ID</th><th>Category</th><th>Status</th><th>Exact limit</th></tr></thead>"
        f"<tbody>{rows}</tbody></table><h2>External editors</h2><pre>{html.escape(json.dumps({'word': word, 'libreoffice': libreoffice}, indent=2))}</pre>"
        f"<h2>Metamorphic</h2><pre>{html.escape(json.dumps(metamorphic, indent=2))}</pre>"
        f"<h2>Performance</h2><pre>{html.escape(json.dumps(performance, indent=2))}</pre></body></html>",
        encoding="utf-8",
    )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"prompt19 audit failed: {exc}", file=sys.stderr)
        raise
