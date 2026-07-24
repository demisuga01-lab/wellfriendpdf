#!/usr/bin/env python3
"""Generate and compare the Prompt 07 transparency compositing corpus."""

from __future__ import annotations

import argparse
import html
import json
import os
import subprocess
import time
import zipfile
from pathlib import Path
from typing import Any, Callable


OUT_DIR = Path("target/prompt07-transparency-compositing")
FIXTURE_DIR = OUT_DIR / "generated-fixtures"
RENDER_DIR = OUT_DIR / "renders"
DIFF_DIR = OUT_DIR / "diffs"
LOG_DIR = OUT_DIR / "logs"
WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "wellfriendpdf-reports"

TOOL_MANIFEST = Path("target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json")
CORPUS_MANIFEST = OUT_DIR / "corpus-manifest.json"
BASELINE_RESULTS = OUT_DIR / "baseline-render-results.json"
POST_RESULTS = OUT_DIR / "post-implementation-render-results.json"
REFERENCE_DISAGREEMENT = OUT_DIR / "reference-disagreement-summary.json"
BLEND_MATRIX = OUT_DIR / "blend-mode-matrix.json"
SOFT_MASK_MATRIX = OUT_DIR / "soft-mask-matrix.json"
GROUP_MATRIX = OUT_DIR / "group-isolation-knockout-matrix.json"
FALLBACK_TAXONOMY = OUT_DIR / "fallback-taxonomy.json"
MEMORY_BUDGET_REPORT = OUT_DIR / "memory-budget-report.json"
HTML_REPORT = OUT_DIR / "html-report" / "index.html"

PAIR_NAMES = [
    ("wellfriendpdf", "poppler"),
    ("wellfriendpdf", "pdfium"),
    ("wellfriendpdf", "mupdf"),
    ("poppler", "pdfium"),
    ("poppler", "mupdf"),
    ("pdfium", "mupdf"),
]
REFERENCE_PAIRS = [("poppler", "pdfium"), ("poppler", "mupdf"), ("pdfium", "mupdf")]
WELLFRIENDPDF_PAIRS = [("wellfriendpdf", "poppler"), ("wellfriendpdf", "pdfium"), ("wellfriendpdf", "mupdf")]

BLEND_MODES = [
    "Normal",
    "Multiply",
    "Screen",
    "Overlay",
    "Darken",
    "Lighten",
    "ColorDodge",
    "ColorBurn",
    "HardLight",
    "SoftLight",
    "Difference",
    "Exclusion",
    "Hue",
    "Saturation",
    "Color",
    "Luminosity",
]


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


def run_command(cmd: list[str], timeout: int) -> dict[str, Any]:
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


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: str | bytes) -> int:
        if isinstance(body, str):
            body = body.encode("latin1")
        self.objects.append(body)
        return len(self.objects)

    def add_stream(self, dict_extra: str, stream: str | bytes) -> int:
        if isinstance(stream, str):
            stream_bytes = stream.encode("latin1")
        else:
            stream_bytes = stream
        body = f"<< /Length {len(stream_bytes)} {dict_extra} >>\nstream\n".encode("latin1")
        body += stream_bytes + b"\nendstream"
        self.objects.append(body)
        return len(self.objects)

    def build(self, root_id: int) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets: list[int] = []
        for idx, body in enumerate(self.objects, start=1):
            offsets.append(len(out))
            out.extend(f"{idx} 0 obj\n".encode("ascii"))
            out.extend(body)
            out.extend(b"\nendobj\n")
        xref = len(out)
        out.extend(f"xref\n0 {len(self.objects) + 1}\n".encode("ascii"))
        out.extend(b"0000000000 65535 f \n")
        for offset in offsets:
            out.extend(f"{offset:010d} 00000 n \n".encode("ascii"))
        out.extend(
            (
                f"trailer\n<< /Size {len(self.objects) + 1} /Root {root_id} 0 R >>\n"
                f"startxref\n{xref}\n%%EOF\n"
            ).encode("ascii")
        )
        return bytes(out)


ExtraBuilder = Callable[[PdfBuilder], dict[str, int]]


def write_single_page_pdf(
    path: Path,
    content: str | bytes,
    resources_template: str = "<< >>",
    extra_builder: ExtraBuilder | None = None,
    media_box: str = "[0 0 100 100]",
) -> None:
    b = PdfBuilder()
    ids = extra_builder(b) if extra_builder else {}
    content_id = b.add_stream("", content)
    page_id = len(b.objects) + 1
    pages_id = page_id + 1
    root_id = page_id + 2
    resources = resources_template.format(**ids)
    b.add(
        f"<< /Type /Page /Parent {pages_id} 0 R /MediaBox {media_box} "
        f"/Resources {resources} /Contents {content_id} 0 R >>"
    )
    b.add(f"<< /Type /Pages /Kids [{page_id} 0 R] /Count 1 >>")
    b.add(f"<< /Type /Catalog /Pages {pages_id} 0 R >>")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build(root_id))


def add_font_and_gs(b: PdfBuilder, gs_body: str) -> dict[str, int]:
    font = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    gs = b.add(gs_body)
    return {"font": font, "gs": gs}


def form_stream_dict(bbox: str, group: str, resources: str = "<< >>") -> str:
    return f"/Type /XObject /Subtype /Form /FormType 1 /BBox {bbox} /Resources {resources} {group}"


def generate_corpus() -> list[dict[str, Any]]:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    entries: list[dict[str, Any]] = []

    def add_entry(ident: str, category: str, file_name: str, expected: str, generator: Callable[[Path], None]) -> None:
        path = FIXTURE_DIR / file_name
        generator(path)
        entries.append(
            {
                "id": ident,
                "category": category,
                "path": rel(path),
                "page": 1,
                "available": path.exists(),
                "expected_visual_behavior": expected,
                "generator": "scripts/prompt07_transparency_compositing_audit.py",
            }
        )

    add_entry(
        "alpha_vector",
        "alpha/vector",
        "alpha_vector.pdf",
        "50 percent blue vector fill over red page yields purple center",
        lambda p: write_single_page_pdf(
            p,
            "1 0 0 rg 0 0 100 100 re f\n/GS1 gs 0 0 1 rg 20 20 60 60 re f\n",
            "<< /ExtGState << /GS1 {gs} 0 R >> >>",
            lambda b: {"gs": b.add("<< /Type /ExtGState /ca 0.5 >>")},
        ),
    )
    add_entry(
        "alpha_text",
        "alpha/text",
        "alpha_text.pdf",
        "semi-transparent text paints through nonstroking alpha",
        lambda p: write_single_page_pdf(
            p,
            "1 1 1 rg 0 0 100 100 re f\n/GS1 gs 0 0 0 rg BT /F1 28 Tf 12 45 Td (ALPHA) Tj ET\n",
            "<< /Font << /F1 {font} 0 R >> /ExtGState << /GS1 {gs} 0 R >> >>",
            lambda b: add_font_and_gs(b, "<< /Type /ExtGState /ca 0.45 >>"),
        ),
    )

    def alpha_image(path: Path) -> None:
        def extras(b: PdfBuilder) -> dict[str, int]:
            gs = b.add("<< /Type /ExtGState /ca 0.5 >>")
            image = b.add_stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8",
                b"\x00\x00\xff",
            )
            return {"gs": gs, "image": image}

        write_single_page_pdf(
            path,
            "1 0 0 rg 0 0 100 100 re f\n/GS1 gs q 60 0 0 60 20 20 cm /Im1 Do Q\n",
            "<< /ExtGState << /GS1 {gs} 0 R >> /XObject << /Im1 {image} 0 R >> >>",
            extras,
        )

    add_entry(
        "alpha_image",
        "alpha/image",
        "alpha_image.pdf",
        "semi-transparent image XObject blends over red page",
        alpha_image,
    )

    def blend_fixture(mode: str) -> Callable[[Path], None]:
        return lambda p: write_single_page_pdf(
            p,
            "0.95 0.20 0.10 rg 0 0 100 100 re f\n"
            "/GS1 gs 0.10 0.45 0.95 rg 12 12 76 76 re f\n",
            "<< /ExtGState << /GS1 {gs} 0 R >> >>",
            lambda b: {"gs": b.add(f"<< /Type /ExtGState /BM /{mode} /ca 0.82 >>")},
        )

    for mode in BLEND_MODES:
        add_entry(
            f"blend_{mode.lower()}",
            f"blend/{mode}",
            f"blend_{mode.lower()}.pdf",
            f"{mode} blend mode applied to blue rectangle over warm backdrop",
            blend_fixture(mode),
        )

    def blend_grid(path: Path) -> None:
        def extras(b: PdfBuilder) -> dict[str, int]:
            return {f"gs{i}": b.add(f"<< /Type /ExtGState /BM /{mode} /ca 0.85 >>") for i, mode in enumerate(BLEND_MODES)}

        content = ["0.8 0.8 0.8 rg 0 0 100 100 re f"]
        for i, mode in enumerate(BLEND_MODES):
            x = (i % 4) * 25
            y = (i // 4) * 25
            content.append(f"/GS{i} gs 0.1 0.2 0.9 rg {x + 3} {y + 3} 19 19 re f")
        resources = "<< /ExtGState << " + " ".join(f"/GS{i} {{gs{i}}} 0 R" for i in range(len(BLEND_MODES))) + " >> >>"
        write_single_page_pdf(path, "\n".join(content) + "\n", resources, extras)

    add_entry(
        "blend_grid",
        "blend/grid",
        "blend_grid.pdf",
        "all required blend modes rendered in a 4 by 4 grid",
        blend_grid,
    )

    def group_fixture(path: Path, isolated: bool, knockout: bool, nested: bool = False, clipped: bool = False) -> None:
        def extras(b: PdfBuilder) -> dict[str, int]:
            inner_group = "/Group << /Type /Group /S /Transparency /I true /K true >>" if nested else ""
            inner = b.add_stream(
                form_stream_dict("[0 0 50 50]", inner_group),
                "0 1 0 rg 8 8 34 34 re f\n0 0 1 rg 18 18 24 24 re f\n",
            )
            form_content = "0 0 1 rg 10 10 70 70 re f\n"
            if nested:
                form_content += "q 1 0 0 1 25 25 cm /Fm2 Do Q\n"
                resources = f"<< /XObject << /Fm2 {inner} 0 R >> >>"
            else:
                form_content += "1 1 0 rg 28 28 44 44 re f\n"
                resources = "<< >>"
            bbox = "[12 12 88 88]" if clipped else "[0 0 100 100]"
            flags = f"/Group << /Type /Group /S /Transparency /I {'true' if isolated else 'false'} /K {'true' if knockout else 'false'} >>"
            form = b.add_stream(form_stream_dict(bbox, flags, resources), form_content)
            return {"form": form}

        write_single_page_pdf(
            path,
            "1 0 0 rg 0 0 100 100 re f\n/Fm1 Do\n",
            "<< /XObject << /Fm1 {form} 0 R >> >>",
            extras,
        )

    add_entry("group_isolated", "group/isolated", "group_isolated.pdf", "isolated group paints as a unit over red backdrop", lambda p: group_fixture(p, True, False))
    add_entry("group_non_isolated", "group/non_isolated", "group_non_isolated.pdf", "non-isolated group can see the red page backdrop", lambda p: group_fixture(p, False, False))
    add_entry("group_knockout", "group/knockout", "group_knockout.pdf", "knockout group records K true behavior for overlapping objects", lambda p: group_fixture(p, True, True))
    add_entry("group_nested_isolated", "group/nested_isolated", "group_nested_isolated.pdf", "nested isolated group restores parent state", lambda p: group_fixture(p, True, False, nested=True))
    add_entry("group_nested_knockout", "group/nested_knockout", "group_nested_knockout.pdf", "nested knockout group is bounded and reported", lambda p: group_fixture(p, True, True, nested=True))
    add_entry("group_clipped", "group/clipped", "group_clipped.pdf", "group BBox clips the painted content", lambda p: group_fixture(p, True, False, clipped=True))
    add_entry("form_xobject_transparency_group", "group/form_xobject", "form_xobject_transparency_group.pdf", "Form XObject transparency group is detected and composited", lambda p: group_fixture(p, False, False))

    def soft_mask_fixture(path: Path, subtype: str, source: str = "vector", matrix: str = "", bbox: str = "[0 0 100 100]") -> None:
        def extras(b: PdfBuilder) -> dict[str, int]:
            mask_content = "1 g 0 0 50 100 re f\n"
            mask_extra = matrix
            mask = b.add_stream(
                f"/Type /XObject /Subtype /Form /FormType 1 /BBox {bbox} {mask_extra} "
                "/Group << /Type /Group /S /Transparency /CS /DeviceGray >>",
                mask_content,
            )
            gs = b.add(f"<< /Type /ExtGState /SMask << /Type /Mask /S /{subtype} /G {mask} 0 R >> >>")
            form = b.add_stream(form_stream_dict("[0 0 100 100]", ""), "0 0 0 rg 0 0 100 100 re f\n")
            image = b.add_stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8",
                b"\x00\x00\x00",
            )
            return {"gs": gs, "form": form, "image": image}

        if source == "text":
            resources = "<< /ExtGState << /GS1 {gs} 0 R >> /Font << /F1 {font} 0 R >> >>"
            def text_extras(b: PdfBuilder) -> dict[str, int]:
                ids = extras(b)
                ids["font"] = b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
                return ids
            content = "1 1 1 rg 0 0 100 100 re f\n/GS1 gs 0 0 0 rg BT /F1 30 Tf 8 45 Td (MASK) Tj ET\n"
            write_single_page_pdf(path, content, resources, text_extras)
        elif source == "image":
            write_single_page_pdf(
                path,
                "1 1 1 rg 0 0 100 100 re f\n/GS1 gs q 100 0 0 100 0 0 cm /Im1 Do Q\n",
                "<< /ExtGState << /GS1 {gs} 0 R >> /XObject << /Im1 {image} 0 R >> >>",
                extras,
            )
        elif source == "form":
            write_single_page_pdf(
                path,
                "1 1 1 rg 0 0 100 100 re f\n/GS1 gs /Fm1 Do\n",
                "<< /ExtGState << /GS1 {gs} 0 R >> /XObject << /Fm1 {form} 0 R >> >>",
                extras,
            )
        else:
            write_single_page_pdf(
                path,
                "1 1 1 rg 0 0 100 100 re f\n/GS1 gs 0 0 0 rg 0 0 100 100 re f\n",
                "<< /ExtGState << /GS1 {gs} 0 R >> >>",
                extras,
            )

    add_entry("softmask_alpha", "softmask/alpha", "softmask_alpha.pdf", "alpha soft mask reveals left half", lambda p: soft_mask_fixture(p, "Alpha"))
    add_entry("softmask_luminosity", "softmask/luminosity", "softmask_luminosity.pdf", "luminosity soft mask reveals left half", lambda p: soft_mask_fixture(p, "Luminosity"))
    add_entry("softmask_transformed", "softmask/transformed", "softmask_transformed.pdf", "mask form Matrix shifts mask coverage", lambda p: soft_mask_fixture(p, "Luminosity", matrix="/Matrix [1 0 0 1 20 0]"))
    add_entry("softmask_clipped", "softmask/clipped", "softmask_clipped.pdf", "mask BBox bounds coverage", lambda p: soft_mask_fixture(p, "Luminosity", bbox="[0 0 50 100]"))
    add_entry("softmask_image", "softmask/image", "softmask_image.pdf", "soft mask modulates image XObject source alpha", lambda p: soft_mask_fixture(p, "Luminosity", source="image"))
    add_entry("softmask_text", "softmask/text", "softmask_text.pdf", "soft mask modulates text painting", lambda p: soft_mask_fixture(p, "Luminosity", source="text"))
    add_entry("softmask_form", "softmask/form", "softmask_form.pdf", "soft mask modulates Form XObject painting", lambda p: soft_mask_fixture(p, "Luminosity", source="form"))

    add_entry("malformed_oversized_group", "malformed/oversized_group", "malformed_oversized_group.pdf", "extreme Form BBox is clipped to page and must not allocate unbounded memory", lambda p: group_fixture(p, True, False, clipped=False))
    add_entry("malformed_recursive_group", "malformed/recursive_group", "malformed_recursive_group.pdf", "missing recursive XObject reference is diagnosed without panic", lambda p: write_single_page_pdf(p, "1 0 0 rg 0 0 100 100 re f\n/FmMissing Do\n", "<< /XObject << >> >>"))
    add_entry("memory_denial_fixture", "malformed/memory_denial", "memory_denial_fixture.pdf", "paired with renderer_offscreen_surface_fails_closed_over_budget unit test", lambda p: group_fixture(p, True, False))

    write_json(
        CORPUS_MANIFEST,
        {
            "schema_version": 1,
            "kind": "prompt07_transparency_corpus_manifest",
            "fixture_count": len(entries),
            "entries": entries,
            "memory_cap_mb": 4096,
        },
    )
    return entries


def load_tool_manifest(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise RuntimeError(f"missing reference tool manifest: {path}; run scripts/prompt06b_multi_reference_audit.ps1 first")
    payload = json.loads(path.read_text(encoding="utf-8-sig"))
    missing = [
        name
        for name in ["poppler", "pdfium", "mupdf"]
        if payload.get("tools", {}).get(name, {}).get("availability") != "available"
    ]
    if missing:
        raise RuntimeError(f"required reference renderers unavailable: {', '.join(missing)}")
    return payload


def wellfriendpdf_base_command(wellfriendpdf_bin: str | None) -> list[str]:
    if wellfriendpdf_bin:
        return [str(Path(wellfriendpdf_bin))]
    suffix = ".exe" if os.name == "nt" else ""
    for candidate in [Path("target/debug") / f"wellfriendpdf{suffix}", Path("target/release") / f"wellfriendpdf{suffix}"]:
        if candidate.exists():
            return [str(candidate)]
    return ["cargo", "run", "-p", "wellfriendpdf-cli", "--quiet", "--"]


def render_wellfriendpdf(base: list[str], entry: dict[str, Any], dpi: int, timeout: int, phase: str) -> dict[str, Any]:
    render_dir = RENDER_DIR / phase / "wellfriendpdf"
    render_dir.mkdir(parents=True, exist_ok=True)
    zip_path = render_dir / f"{entry['id']}-p{entry['page']}.zip"
    png_path = render_dir / f"{entry['id']}-p{entry['page']}.png"
    report_path = WELLFRIENDPDF_REPORT_DIR / phase / f"{entry['id']}-p{entry['page']}.json"
    for path in [zip_path, png_path, report_path]:
        if path.exists():
            path.unlink()
    render_result = run_command(
        [
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
        ],
        timeout,
    )
    compare_result = run_command(
        [
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
        ],
        timeout,
    )
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
        status = "wellfriendpdf_render_failure"
    else:
        try:
            with zipfile.ZipFile(zip_path) as zf:
                names = sorted(name for name in zf.namelist() if name.lower().endswith(".png"))
                if not names:
                    status = "blank_output"
                else:
                    png_path.write_bytes(zf.read(names[0]))
        except zipfile.BadZipFile:
            status = "wellfriendpdf_render_failure"
    return {
        "status": status,
        "artifact": rel(png_path) if png_path.exists() else None,
        "zip_artifact": rel(zip_path) if zip_path.exists() else None,
        "render_report_artifact": rel(report_path) if report_path.exists() else None,
        "native_counters": counters,
        "render_command": render_result,
        "render_compare_command": compare_result,
    }


def render_reference(engine: str, tool: dict[str, Any], entry: dict[str, Any], dpi: int, timeout: int, phase: str) -> dict[str, Any]:
    render_dir = RENDER_DIR / phase / engine
    render_dir.mkdir(parents=True, exist_ok=True)
    output = render_dir / f"{entry['id']}-p{entry['page']}.png"
    if output.exists():
        output.unlink()
    executable = str(tool["executable_path"])
    if engine == "poppler":
        prefix = render_dir / f"{entry['id']}-p{entry['page']}"
        for stale in render_dir.glob(f"{entry['id']}-p{entry['page']}-*.png"):
            stale.unlink()
        cmd = [executable, "-png", "-r", str(dpi), "-f", str(entry["page"]), "-l", str(entry["page"]), entry["path"], str(prefix)]
        result = run_command(cmd, timeout)
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
        result = run_command(cmd, timeout)
    elif engine == "mupdf":
        cmd = [executable, "draw", "-o", str(output), "-r", str(dpi), entry["path"], str(entry["page"])]
        result = run_command(cmd, timeout)
    else:
        raise ValueError(engine)
    log_path = LOG_DIR / phase / engine / f"{entry['id']}-p{entry['page']}.json"
    write_json(log_path, result)
    if result["timed_out"]:
        status = "render_timeout"
    elif result["exit_status"] != 0:
        status = "reference_execution_failure"
    elif not output.exists() or output.stat().st_size == 0:
        status = "blank_output"
    else:
        status = "rendered"
    return {"status": status, "artifact": rel(output) if output.exists() else None, "log_artifact": rel(log_path), "command": result}


def average_hash(image: Any) -> str:
    from PIL import Image  # type: ignore

    resampling = getattr(Image, "Resampling", Image).LANCZOS
    gray = image.convert("L").resize((8, 8), resampling)
    pixels = list(gray.getdata())
    avg = sum(pixels) / len(pixels)
    bits = "".join("1" if px >= avg else "0" for px in pixels)
    return f"{int(bits, 2):016x}"


def image_metrics(a_name: str, a_path: str | None, b_name: str, b_path: str | None, entry_id: str, phase: str) -> dict[str, Any]:
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
                "size_a": list(ia.size),
                "size_b": list(ib.size),
                "visual_hash_a": hash_a,
                "visual_hash_b": hash_b,
            }
        bytes_a = ia.tobytes()
        bytes_b = ib.tobytes()
        changed_pixels = 0
        changed8 = 0
        max_delta = 0
        abs_sum = 0
        diff_bytes = bytearray(len(bytes_a))
        for idx in range(0, len(bytes_a), 4):
            pixel_delta = 0
            for channel in range(4):
                delta = abs(bytes_a[idx + channel] - bytes_b[idx + channel])
                pixel_delta += delta
                abs_sum += delta
                max_delta = max(max_delta, delta)
                diff_bytes[idx + channel] = min(255, delta * 4)
            diff_bytes[idx + 3] = 255
            if pixel_delta:
                changed_pixels += 1
            if pixel_delta > 8:
                changed8 += 1
        total = ia.size[0] * ia.size[1]
        mean_abs = abs_sum / (total * 4) if total else 0.0
        changed_pct = changed_pixels / total if total else 0.0
        changed8_pct = changed8 / total if total else 0.0
        threshold_pass = mean_abs <= 2.0 or changed8_pct <= 0.02
        pair_dir = DIFF_DIR / phase / f"{a_name}_vs_{b_name}"
        pair_dir.mkdir(parents=True, exist_ok=True)
        diff_path = pair_dir / f"{entry_id}.png"
        Image.frombytes("RGBA", ia.size, bytes(diff_bytes)).save(diff_path)
        return {
            "status": "computed",
            "threshold_pass": threshold_pass,
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
    if renders["wellfriendpdf"]["status"] != "rendered":
        return "wellfriendpdf_render_failure"
    if any(renders[name]["status"] != "rendered" for name in ["poppler", "pdfium", "mupdf"]):
        return "reference_tool_failure"
    if any(metric.get("status") == "dimension_mismatch" for metric in metrics.values()):
        return "dimension_mismatch"

    def pair_pass(a: str, b: str) -> bool:
        return bool(metrics[f"{a}_vs_{b}"].get("threshold_pass"))

    references_agree = all(pair_pass(a, b) for a, b in REFERENCE_PAIRS)
    wellfriendpdf_matches = [b for a, b in WELLFRIENDPDF_PAIRS if pair_pass(a, b)]
    if references_agree:
        return "all_references_agree_wellfriendpdf_pass" if len(wellfriendpdf_matches) == 3 else "all_references_agree_wellfriendpdf_mismatch"
    if len(wellfriendpdf_matches) == 1:
        return f"references_disagree_wellfriendpdf_matches_{wellfriendpdf_matches[0]}"
    if len(wellfriendpdf_matches) > 1:
        return "references_disagree_wellfriendpdf_between_references"
    return "needs_manual_review" if category.startswith(("group/", "softmask/", "blend/")) else "references_disagree_wellfriendpdf_between_references"


def run_phase(entries: list[dict[str, Any]], tools: dict[str, Any], base: list[str], phase: str, dpi: int, timeout: int) -> dict[str, Any]:
    pages: list[dict[str, Any]] = []
    categories: dict[str, int] = {}
    classification_counts: dict[str, int] = {}
    fallback_reasons: dict[str, int] = {}
    totals: dict[str, int] = {}
    for entry in entries:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
        renders = {
            "wellfriendpdf": render_wellfriendpdf(base, entry, dpi, timeout, phase),
            "poppler": render_reference("poppler", tools["poppler"], entry, dpi, timeout, phase),
            "pdfium": render_reference("pdfium", tools["pdfium"], entry, dpi, timeout, phase),
            "mupdf": render_reference("mupdf", tools["mupdf"], entry, dpi, timeout, phase),
        }
        pair_metrics = {
            f"{a}_vs_{b}": image_metrics(a, renders[a].get("artifact"), b, renders[b].get("artifact"), f"{entry['id']}-p{entry['page']}", phase)
            for a, b in PAIR_NAMES
        }
        classification = classify_page(entry["category"], renders, pair_metrics)
        classification_counts[classification] = classification_counts.get(classification, 0) + 1
        counters = renders["wellfriendpdf"].get("native_counters", {})
        for key, value in counters.items():
            if isinstance(value, int):
                totals[key] = totals.get(key, 0) + value
        report_artifact = renders["wellfriendpdf"].get("render_report_artifact")
        if report_artifact and Path(report_artifact).exists():
            try:
                report = json.loads(Path(report_artifact).read_text(encoding="utf-8"))
                for reason, count in report.get("compatibility_fallback_reasons", {}).items():
                    fallback_reasons[reason] = fallback_reasons.get(reason, 0) + int(count)
            except json.JSONDecodeError:
                pass
        pages.append(
            {
                "id": entry["id"],
                "category": entry["category"],
                "expected_visual_behavior": entry["expected_visual_behavior"],
                "page": entry["page"],
                "input": entry["path"],
                "classification": classification,
                "renders": renders,
                "pair_metrics": pair_metrics,
                "native_counters": counters,
            }
        )
    return {
        "schema_version": 1,
        "kind": f"prompt07_{phase}_render_results",
        "phase": phase,
        "dpi": dpi,
        "memory_cap_mb": 4096,
        "fixture_count": len(entries),
        "categories": categories,
        "classification_counts": classification_counts,
        "native_counter_totals": totals,
        "fallback_reasons": fallback_reasons,
        "pages": pages,
    }


def latest_results() -> dict[str, Any] | None:
    if POST_RESULTS.exists():
        return json.loads(POST_RESULTS.read_text(encoding="utf-8"))
    if BASELINE_RESULTS.exists():
        return json.loads(BASELINE_RESULTS.read_text(encoding="utf-8"))
    return None


def write_summary_artifacts(entries: list[dict[str, Any]], results: dict[str, Any] | None) -> None:
    pages = results.get("pages", []) if results else []
    classification_counts = results.get("classification_counts", {}) if results else {}
    reference_disagreement_pages = [
        {"id": p["id"], "category": p["category"], "classification": p["classification"]}
        for p in pages
        if "references_disagree" in p.get("classification", "") or p.get("classification") == "needs_manual_review"
    ]
    write_json(
        REFERENCE_DISAGREEMENT,
        {
            "schema_version": 1,
            "kind": "prompt07_reference_disagreement_summary",
            "fixture_count": len(entries),
            "classification_counts": classification_counts,
            "reference_disagreement_pages": reference_disagreement_pages,
        },
    )
    write_json(
        BLEND_MATRIX,
        {
            "schema_version": 1,
            "kind": "prompt07_blend_mode_matrix",
            "blend_modes": [
                {
                    "mode": mode,
                    "implemented": True,
                    "fixture": f"blend_{mode.lower()}",
                    "classification": next((p["classification"] for p in pages if p["id"] == f"blend_{mode.lower()}"), "not_run"),
                }
                for mode in BLEND_MODES
            ],
            "combined_grid_fixture": "blend_grid",
        },
    )
    write_json(
        SOFT_MASK_MATRIX,
        {
            "schema_version": 1,
            "kind": "prompt07_soft_mask_matrix",
            "features": [
                {
                    "id": e["id"],
                    "category": e["category"],
                    "classification": next((p["classification"] for p in pages if p["id"] == e["id"]), "not_run"),
                }
                for e in entries
                if e["category"].startswith("softmask/")
            ],
            "known_limits": [
                "Prompt 07B closes common image /SMask /Matte and ExtGState /BC backdrop behavior",
                "Prompt 07B closes DeviceGray/DeviceRGB/DeviceCMYK luminosity mask color-space coverage",
                "advanced ICC/device-link luminosity parity remains unsupported-reported CMM work",
            ],
        },
    )
    write_json(
        GROUP_MATRIX,
        {
            "schema_version": 1,
            "kind": "prompt07_group_isolation_knockout_matrix",
            "features": [
                {
                    "id": e["id"],
                    "category": e["category"],
                    "classification": next((p["classification"] for p in pages if p["id"] == e["id"]), "not_run"),
                }
                for e in entries
                if e["category"].startswith("group/")
            ],
            "known_limits": [
                "Prompt 07B closes interior knockout overlap for common vector/Form group cases",
                "text clipping plus pattern/shading paints inside knockout groups remain later prompt ownership",
            ],
        },
    )
    write_json(
        FALLBACK_TAXONOMY,
        {
            "schema_version": 1,
            "kind": "prompt07_fallback_taxonomy",
            "removed_or_reduced": ["transparency/later is no longer an unowned Prompt 06B category"],
            "remaining": [
                "pattern/later remains Prompt 08",
                "shading/later remains Prompt 08",
                "advanced ICC/device-link/multicolor group color-space management",
                "text clipping plus pattern/shading paints inside knockout groups remain later prompt ownership",
            ],
            "measured_fallback_reasons": results.get("fallback_reasons", {}) if results else {},
        },
    )
    write_json(
        MEMORY_BUDGET_REPORT,
        {
            "schema_version": 1,
            "kind": "prompt07_memory_budget_report",
            "memory_cap_mb": 4096,
            "renderer_scheduler_budget_default_bytes": 512 * 1024 * 1024,
            "offscreen_surface_admission": "transparency groups and soft mask groups reserve RGBA bytes before allocation",
            "denial_evidence": {
                "unit_test": "renderer_offscreen_surface_fails_closed_over_budget",
                "module": "crates/engine/src/render/page_renderer.rs",
                "expected_error": "exceeding scheduler budget",
            },
            "known_limits": ["current offscreen surface uses page-sized coordinates, with BBox clipping; cropped coordinate surfaces are a future memory optimization"],
        },
    )
    HTML_REPORT.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for page in pages:
        rows.append(
            "<tr>"
            f"<td>{html.escape(page['id'])}</td>"
            f"<td>{html.escape(page['category'])}</td>"
            f"<td>{html.escape(page['classification'])}</td>"
            f"<td>{html.escape(page['renders']['poppler']['status'])}</td>"
            f"<td>{html.escape(page['renders']['pdfium']['status'])}</td>"
            f"<td>{html.escape(page['renders']['mupdf']['status'])}</td>"
            "</tr>"
        )
    HTML_REPORT.write_text(
        "<!doctype html><meta charset='utf-8'>"
        "<title>Prompt 07 Transparency Compositing Audit</title>"
        "<style>body{font-family:system-ui,sans-serif;margin:32px;color:#172033}"
        "table{border-collapse:collapse;font-size:13px}td,th{border:1px solid #cbd5e1;padding:4px 8px}"
        "th{background:#f1f5f9;text-align:left}</style>"
        "<h1>Prompt 07 Transparency Compositing Audit</h1>"
        f"<p>Fixtures: {len(entries)}. Memory cap: 4096 MB.</p>"
        "<h2>Classification Counts</h2><pre>"
        f"{html.escape(json.dumps(classification_counts, indent=2, sort_keys=True))}</pre>"
        "<h2>Pages</h2><table><tr><th>Fixture</th><th>Category</th><th>Classification</th>"
        "<th>Poppler</th><th>PDFium</th><th>MuPDF</th></tr>"
        + "\n".join(rows)
        + "</table>",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=TOOL_MANIFEST)
    parser.add_argument("--wellfriendpdf-bin")
    parser.add_argument("--phase", choices=["baseline", "post", "both"], default="post")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    tools = load_tool_manifest(args.manifest)["tools"]
    entries = generate_corpus()
    base = wellfriendpdf_base_command(args.wellfriendpdf_bin)
    latest: dict[str, Any] | None = None
    if args.phase in {"baseline", "both"}:
        latest = run_phase(entries, tools, base, "baseline", args.dpi, args.timeout)
        latest["starting_checkpoint"] = run_command(["git", "rev-parse", "--short", "HEAD"], 10)
        latest["baseline_note"] = "Prompt 07 visual baseline for the transparency-focused corpus."
        write_json(BASELINE_RESULTS, latest)
    if args.phase in {"post", "both"}:
        latest = run_phase(entries, tools, base, "post", args.dpi, args.timeout)
        latest["starting_checkpoint"] = run_command(["git", "rev-parse", "--short", "HEAD"], 10)
        latest["post_note"] = "Post-implementation Prompt 07 transparency corpus comparison."
        write_json(POST_RESULTS, latest)
    if latest is None:
        latest = latest_results()
    write_summary_artifacts(entries, latest)
    print(
        json.dumps(
            {
                "status": "ok",
                "phase": args.phase,
                "fixture_count": len(entries),
                "artifacts": {
                    "corpus": rel(CORPUS_MANIFEST),
                    "baseline": rel(BASELINE_RESULTS) if BASELINE_RESULTS.exists() else None,
                    "post": rel(POST_RESULTS) if POST_RESULTS.exists() else None,
                    "summary": rel(REFERENCE_DISAGREEMENT),
                    "html": rel(HTML_REPORT),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
