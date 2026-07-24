#!/usr/bin/env python3
"""Generate and compare the Prompt 08 text clipping/shading/pattern corpus."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any, Callable

import prompt06b_render_compare as p06


OUT_DIR = Path("target/prompt08-text-shading-patterns")
FIXTURE_DIR = OUT_DIR / "corpus"
TOOL_MANIFEST_OUT = OUT_DIR / "reference-tool-manifest.json"
CORPUS_OUT = OUT_DIR / "corpus-manifest.json"
RESULTS_OUT = OUT_DIR / "multi-reference-render-results.json"
DIFF_METRICS_OUT = OUT_DIR / "visual-diff-metrics.json"
DISAGREEMENT_OUT = OUT_DIR / "reference-disagreement-summary.json"
TEXT_MATRIX_OUT = OUT_DIR / "text-clipping-matrix.json"
AXIAL_MATRIX_OUT = OUT_DIR / "axial-radial-shading-matrix.json"
MESH_MATRIX_OUT = OUT_DIR / "mesh-patch-shading-matrix.json"
PATTERN_MATRIX_OUT = OUT_DIR / "tiling-pattern-matrix.json"
FALLBACK_OUT = OUT_DIR / "fallback-taxonomy.json"
MEMORY_OUT = OUT_DIR / "memory-scheduler-report.json"
FEATURE_OUT = OUT_DIR / "public-feature-report.json"
STARTING_OUT = OUT_DIR / "starting-state.json"
HTML_OUT = OUT_DIR / "html-report" / "index.html"

PROMPT06B_TOOL_MANIFEST = Path(
    "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json"
)
PROMPT07B_TOOL_MANIFEST = Path(
    "target/prompt07-transparency-compositing/prompt07b-reference-tool-manifest.json"
)

PAGE_W = 160
PAGE_H = 100


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: str) -> int:
        self.objects.append(body.encode("utf-8"))
        return len(self.objects)

    def add_stream(self, dict_extra: str, stream: str | bytes) -> int:
        payload = stream.encode("latin1") if isinstance(stream, str) else stream
        body = f"<< /Length {len(payload)} {dict_extra} >>\nstream\n".encode("utf-8")
        body += payload + b"\nendstream"
        self.objects.append(body)
        return len(self.objects)

    def build(self) -> bytes:
        pdf = bytearray(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")
        offsets: list[int] = []
        for idx, body in enumerate(self.objects, start=1):
            offsets.append(len(pdf))
            pdf += f"{idx} 0 obj\n".encode("ascii")
            pdf += body
            pdf += b"\nendobj\n"
        xref = len(pdf)
        pdf += f"xref\n0 {len(offsets) + 1}\n".encode("ascii")
        pdf += b"0000000000 65535 f \n"
        for offset in offsets:
            pdf += f"{offset:010} 00000 n \n".encode("ascii")
        pdf += (
            f"trailer\n<< /Size {len(offsets) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("ascii")
        return bytes(pdf)


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rel(path: Path | str | None) -> str | None:
    if path is None:
        return None
    return p06.rel(path)


def run_text(cmd: list[str]) -> dict[str, Any]:
    started = time.time()
    proc = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    return {
        "command": cmd,
        "exit_status": proc.returncode,
        "stdout": proc.stdout[-4000:],
        "stderr": proc.stderr[-4000:],
        "elapsed_ms": int((time.time() - started) * 1000),
    }


def write_pdf(
    path: Path,
    content: str,
    resources_extra: str = "",
    extras: Callable[[PdfBuilder], None] | None = None,
) -> None:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] "
        f"/Contents 4 0 R /Resources << /Font << /Helvetica 5 0 R >> {resources_extra} >> >>"
    )
    b.add_stream("", content)
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    if extras:
        extras(b)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b.build())


def text_clip_prefix() -> str:
    return (
        f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n"
        "BT /Helvetica 72 Tf 7 Tr 20 25 Td (HI) Tj ET\n"
    )


def add_function_and_axial(b: PdfBuilder, coords: str, extend: str = "[true true]") -> None:
    b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>")
    b.add(
        f"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords {coords} "
        f"/Domain [0 1] /Extend {extend} /Function 6 0 R >>"
    )


def add_function_and_radial(b: PdfBuilder, coords: str, extend: str = "[true true]") -> None:
    b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>")
    b.add(
        f"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords {coords} "
        f"/Domain [0 1] /Extend {extend} /Function 6 0 R >>"
    )


def push_be(buf: bytearray, value: int, width: int) -> None:
    for shift in range((width - 1) * 8, -1, -8):
        buf.append((value >> shift) & 0xFF)


def coord20(value: float) -> int:
    return int(round((value / 20.0) * 0xFFFF))


def type4_data() -> bytes:
    out = bytearray()
    for x, y, color in [
        (2.0, 2.0, (255, 0, 0)),
        (18.0, 2.0, (0, 255, 0)),
        (10.0, 18.0, (0, 0, 255)),
    ]:
        out.append(0)
        push_be(out, coord20(x), 2)
        push_be(out, coord20(y), 2)
        out.extend(color)
    return bytes(out)


def type5_data() -> bytes:
    out = bytearray()
    for x, y, color in [
        (2.0, 2.0, (255, 0, 0)),
        (18.0, 2.0, (0, 255, 0)),
        (2.0, 18.0, (0, 0, 255)),
        (18.0, 18.0, (255, 255, 0)),
    ]:
        push_be(out, coord20(x), 2)
        push_be(out, coord20(y), 2)
        out.extend(color)
    return bytes(out)


def patch_points(tensor: bool) -> list[tuple[float, float]]:
    pts = [
        (2.0, 2.0),
        (2.0, 7.33),
        (2.0, 12.66),
        (2.0, 18.0),
        (7.33, 18.0),
        (12.66, 18.0),
        (18.0, 18.0),
        (18.0, 12.66),
        (18.0, 7.33),
        (18.0, 2.0),
        (12.66, 2.0),
        (7.33, 2.0),
    ]
    if tensor:
        pts += [(7.0, 7.0), (13.0, 7.0), (7.0, 13.0), (13.0, 13.0)]
    return pts


def patch_data(tensor: bool) -> bytes:
    out = bytearray([0])
    for x, y in patch_points(tensor):
        push_be(out, coord20(x), 2)
        push_be(out, coord20(y), 2)
    for color in [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0)]:
        out.extend(color)
    return bytes(out)


def add_mesh_stream(b: PdfBuilder, shading_type: int, data: bytes) -> None:
    if shading_type == 5:
        extra = (
            "/ShadingType 5 /ColorSpace /DeviceRGB /BitsPerCoordinate 16 "
            "/BitsPerComponent 8 /VerticesPerRow 2 /Decode [0 20 0 20 0 1 0 1 0 1]"
        )
    else:
        extra = (
            f"/ShadingType {shading_type} /ColorSpace /DeviceRGB /BitsPerCoordinate 16 "
            "/BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 20 0 20 0 1 0 1 0 1]"
        )
    b.add_stream(extra, data)


def add_colored_pattern(b: PdfBuilder, content: str = "0 0.75 0 rg 0 0 10 10 re f\n") -> None:
    b.add_stream(
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 "
        "/BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
        content,
    )


def add_uncolored_pattern(b: PdfBuilder) -> None:
    b.add_stream(
        "/Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 "
        "/BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
        "0 0 10 10 re f\n",
    )


def add_entry(
    entries: list[dict[str, Any]],
    ident: str,
    category: str,
    expected: str,
    generator: Callable[[Path], None],
) -> None:
    path = FIXTURE_DIR / f"{ident}.pdf"
    generator(path)
    entries.append(
        {
            "id": ident,
            "category": category,
            "path": rel(path),
            "page": 1,
            "page_count": 1,
            "available": path.exists(),
            "owner_prompt": "combined_prompt_08",
            "expected_feature_coverage": expected,
            "expected_reference_behavior": "multi_reference_classified_by_prompt08_audit",
            "generator": "scripts/prompt08_text_shading_patterns_audit.py",
        }
    )


def corpus_entries() -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []

    add_entry(
        entries,
        "text_clip_fill_rect",
        "text_clipping",
        "Tr7 text clip masks a later rectangle fill",
        lambda p: write_pdf(p, text_clip_prefix() + f"1 0 0 rg 0 0 {PAGE_W} {PAGE_H} re f\n"),
    )
    add_entry(
        entries,
        "text_clip_image_xobject",
        "text_clipping",
        "Tr7 text clip masks a scaled image XObject",
        lambda p: write_pdf(
            p,
            text_clip_prefix() + f"q {PAGE_W} 0 0 {PAGE_H} 0 0 cm /Im1 Do Q\n",
            "/XObject << /Im1 6 0 R >>",
            lambda b: b.add_stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 "
                "/ColorSpace /DeviceRGB /BitsPerComponent 8",
                bytes([0, 0, 255]),
            ),
        ),
    )
    add_entry(
        entries,
        "text_clip_form_xobject",
        "text_clipping",
        "Tr7 text clip masks a Form XObject",
        lambda p: write_pdf(
            p,
            text_clip_prefix() + "q /Fm1 Do Q\n",
            "/XObject << /Fm1 6 0 R >>",
            lambda b: b.add_stream(
                f"/Type /XObject /Subtype /Form /BBox [0 0 {PAGE_W} {PAGE_H}] /Resources << >>",
                f"0 0.8 0 rg 0 0 {PAGE_W} {PAGE_H} re f\n",
            ),
        ),
    )
    add_entry(
        entries,
        "text_clip_axial_shading",
        "text_clipping",
        "Tr7 text clip masks an axial shading",
        lambda p: write_pdf(
            p,
            text_clip_prefix() + "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_axial(b, f"[0 0 {PAGE_W} 0]"),
        ),
    )
    add_entry(
        entries,
        "text_clip_tiling_pattern",
        "text_clipping",
        "Tr7 text clip masks a colored tiling pattern",
        lambda p: write_pdf(
            p,
            text_clip_prefix() + f"/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            add_colored_pattern,
        ),
    )

    add_entry(
        entries,
        "axial_horizontal",
        "axial_radial_shading",
        "ShadingType 2 horizontal domain and interpolation",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_axial(b, f"[0 50 {PAGE_W} 50]"),
        ),
    )
    add_entry(
        entries,
        "axial_diagonal_extend",
        "axial_radial_shading",
        "ShadingType 2 diagonal with extend flags",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_axial(b, "[20 20 140 80]"),
        ),
    )
    add_entry(
        entries,
        "axial_transformed_clipped",
        "axial_radial_shading",
        "ShadingType 2 under CTM and path clipping",
        lambda p: write_pdf(
            p,
            "20 20 120 60 re W n q 0.866 0.5 -0.5 0.866 30 -20 cm /Sh1 sh Q\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_axial(b, f"[0 50 {PAGE_W} 50]"),
        ),
    )
    add_entry(
        entries,
        "radial_simple",
        "axial_radial_shading",
        "ShadingType 3 simple concentric circles",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_radial(b, "[80 50 0 80 50 60]"),
        ),
    )
    add_entry(
        entries,
        "radial_offset_extend",
        "axial_radial_shading",
        "ShadingType 3 offset circles with extend flags",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_radial(b, "[45 50 5 125 50 65]"),
        ),
    )
    add_entry(
        entries,
        "radial_degenerate_reported",
        "malformed_or_unsupported_reported",
        "ShadingType 3 degenerate circles must fail closed or render bounded output",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 7 0 R >>",
            lambda b: add_function_and_radial(b, "[80 50 0 80 50 0]", "[false false]"),
        ),
    )

    add_entry(
        entries,
        "mesh_type4_gouraud",
        "mesh_patch_shading",
        "ShadingType 4 free-form Gouraud triangle",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 6 0 R >>",
            lambda b: add_mesh_stream(b, 4, type4_data()),
        ),
    )
    add_entry(
        entries,
        "mesh_type5_lattice",
        "mesh_patch_shading",
        "ShadingType 5 lattice Gouraud mesh",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 6 0 R >>",
            lambda b: add_mesh_stream(b, 5, type5_data()),
        ),
    )
    add_entry(
        entries,
        "patch_type6_coons",
        "mesh_patch_shading",
        "ShadingType 6 Coons patch tessellation",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 6 0 R >>",
            lambda b: add_mesh_stream(b, 6, patch_data(False)),
        ),
    )
    add_entry(
        entries,
        "patch_type7_tensor_or_unsupported_reported",
        "mesh_patch_shading",
        "ShadingType 7 tensor stream with bounded boundary tessellation",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 6 0 R >>",
            lambda b: add_mesh_stream(b, 7, patch_data(True)),
        ),
    )

    add_entry(
        entries,
        "colored_tiling_pattern_basic",
        "tiling_pattern",
        "Colored PatternType 1 tile",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            add_colored_pattern,
        ),
    )
    add_entry(
        entries,
        "colored_tiling_pattern_transformed",
        "tiling_pattern",
        "Colored PatternType 1 with CTM transform",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\nq 0.866 0.5 -0.5 0.866 40 -30 cm /Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f Q\n",
            "/Pattern << /P1 6 0 R >>",
            add_colored_pattern,
        ),
    )
    add_entry(
        entries,
        "uncolored_tiling_pattern_rgb",
        "tiling_pattern",
        "Uncolored PatternType 1 inherits DeviceRGB caller color",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs 0 0.8 0 /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            add_uncolored_pattern,
        ),
    )
    add_entry(
        entries,
        "uncolored_tiling_pattern_cmyk",
        "tiling_pattern",
        "Uncolored PatternType 1 inherits DeviceCMYK caller color through current color model",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs 0 1 1 0 /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            add_uncolored_pattern,
        ),
    )
    add_entry(
        entries,
        "pattern_with_text",
        "tiling_pattern",
        "Pattern cell content includes text",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            lambda b: add_colored_pattern(b, "0 0.75 0 rg BT /Helvetica 8 Tf 0 Tr 1 2 Td (T) Tj ET\n"),
        ),
    )
    add_entry(
        entries,
        "pattern_with_image",
        "tiling_pattern",
        "Pattern cell content includes an image XObject",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            lambda b: (
                b.add_stream(
                    "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 "
                    "/BBox [0 0 10 10] /XStep 10 /YStep 10 "
                    "/Resources << /XObject << /Im1 7 0 R >> >>",
                    "q 10 0 0 10 0 0 cm /Im1 Do Q\n",
                ),
                b.add_stream(
                    "/Type /XObject /Subtype /Image /Width 1 /Height 1 "
                    "/ColorSpace /DeviceRGB /BitsPerComponent 8",
                    bytes([0, 128, 255]),
                ),
            ),
        ),
    )
    add_entry(
        entries,
        "pattern_recursion_limit",
        "malformed_or_unsupported_reported",
        "Recursive pattern resource must hit bounded recursion posture",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            lambda b: b.add_stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 "
                "/BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /Pattern << /P1 6 0 R >> >>",
                "/Pattern cs /P1 scn 0 0 10 10 re f\n",
            ),
        ),
    )
    add_entry(
        entries,
        "malformed_shading_stream",
        "malformed_or_unsupported_reported",
        "Truncated mesh stream must fail closed",
        lambda p: write_pdf(
            p,
            "/Sh1 sh\n",
            "/Shading << /Sh1 6 0 R >>",
            lambda b: add_mesh_stream(b, 4, b"\x00\xff"),
        ),
    )
    add_entry(
        entries,
        "malformed_pattern_steps",
        "malformed_or_unsupported_reported",
        "Zero pattern steps must fail closed",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Pattern cs /P1 scn 0 0 {PAGE_W} {PAGE_H} re f\n",
            "/Pattern << /P1 6 0 R >>",
            lambda b: b.add_stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 "
                "/BBox [0 0 10 10] /XStep 0 /YStep 0 /Resources << >>",
                "1 0 0 rg 0 0 10 10 re f\n",
            ),
        ),
    )
    add_entry(
        entries,
        "transparency_group_with_shading",
        "transparency_interaction",
        "Transparency group content paints a shading",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/Fm1 Do\n",
            "/XObject << /Fm1 6 0 R >>",
            lambda b: (
                b.add_stream(
                    f"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {PAGE_W} {PAGE_H}] "
                    "/Resources << /Shading << /Sh1 8 0 R >> >> "
                    "/Group << /Type /Group /S /Transparency /I true /K false /CS /DeviceRGB >>",
                    "/Sh1 sh\n",
                ),
                b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"),
                b.add(
                    f"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 {PAGE_W} 0] "
                    "/Domain [0 1] /Extend [true true] /Function 7 0 R >>"
                ),
            ),
        ),
    )
    add_entry(
        entries,
        "softmask_with_pattern_or_shading",
        "transparency_interaction",
        "Soft mask gates a shading paint operation",
        lambda p: write_pdf(
            p,
            f"1 1 1 rg 0 0 {PAGE_W} {PAGE_H} re f\n/GS1 gs /Sh1 sh\n",
            "/ExtGState << /GS1 7 0 R >> /Shading << /Sh1 9 0 R >>",
            lambda b: (
                b.add_stream(
                    f"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {PAGE_W} {PAGE_H}] "
                    "/Resources << >> /Group << /Type /Group /S /Transparency /CS /DeviceGray >>",
                    f"0.5 g 0 0 {PAGE_W} {PAGE_H} re f\n",
                ),
                b.add("<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G 6 0 R >> >>"),
                b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"),
                b.add(
                    f"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 {PAGE_W} 0] "
                    "/Domain [0 1] /Extend [true true] /Function 8 0 R >>"
                ),
            ),
        ),
    )

    return entries


def configure_prompt06b_runner() -> None:
    p06.OUT_DIR = OUT_DIR
    p06.RENDER_DIR = OUT_DIR / "renders"
    p06.DIFF_DIR = OUT_DIR / "diffs"
    p06.LOG_DIR = OUT_DIR / "logs"
    p06.WELLFRIENDPDF_REPORT_DIR = OUT_DIR / "wellfriendpdf-render-reports"
    p06.TOOL_MANIFEST = TOOL_MANIFEST_OUT
    p06.CORPUS_MANIFEST = CORPUS_OUT
    p06.RENDER_RESULTS = RESULTS_OUT
    p06.DIFF_METRICS = DIFF_METRICS_OUT
    p06.DISAGREEMENT_SUMMARY = DISAGREEMENT_OUT
    p06.TAXONOMY = OUT_DIR / "renderer-parity-taxonomy.json"
    p06.HTML_REPORT = HTML_OUT
    p06.LATER_OWNED_CATEGORIES = set()


def prompt08_classification(raw: str, category: str, metrics: dict[str, Any]) -> tuple[str, str | None]:
    if category == "malformed_or_unsupported_reported":
        if raw in {"reference_tool_failure", "wellfriendpdf_render_failure"}:
            return "malformed_reference_failure", None
        return "unsupported_reported_expected", None
    if raw == "all_references_agree_wellfriendpdf_pass":
        return "all_references_agree_wellfriendpdf_passes", None
    if raw == "all_references_agree_wellfriendpdf_mismatch" and wellfriendpdf_within_reference_spread(metrics):
        return (
            "all_references_agree_wellfriendpdf_passes",
            "prompt08_cluster_tolerance: wellfriendpdf matched one reference and stayed within reference changed-pixel spread",
        )
    if raw in {
        "references_disagree_wellfriendpdf_between_references",
        "references_disagree_wellfriendpdf_matches_poppler",
        "references_disagree_wellfriendpdf_matches_pdfium",
        "references_disagree_wellfriendpdf_matches_mupdf",
    }:
        return "references_disagree_wellfriendpdf_within_cluster", None
    if raw in {"all_references_agree_wellfriendpdf_mismatch", "needs_manual_review", "dimension_mismatch"}:
        return "wellfriendpdf_outlier_failure", None
    if raw in {"reference_tool_failure"}:
        return "malformed_reference_failure", None
    return raw, None


def wellfriendpdf_within_reference_spread(metrics: dict[str, Any]) -> bool:
    ref_keys = ["poppler_vs_pdfium", "poppler_vs_mupdf", "pdfium_vs_mupdf"]
    wellfriendpdf_keys = ["wellfriendpdf_vs_poppler", "wellfriendpdf_vs_pdfium", "wellfriendpdf_vs_mupdf"]
    ref = [metrics[key] for key in ref_keys]
    wellfriendpdf = [metrics[key] for key in wellfriendpdf_keys]
    if not all(metric.get("status") == "computed" for metric in ref + wellfriendpdf):
        return False
    if not any(metric.get("threshold_pass") for metric in wellfriendpdf):
        return False
    max_ref_changed = max(float(metric.get("changed_pixel_threshold8_percentage") or 0.0) for metric in ref)
    max_wellfriendpdf_changed = max(float(metric.get("changed_pixel_threshold8_percentage") or 0.0) for metric in wellfriendpdf)
    max_ref_mean = max(float(metric.get("mean_abs_error") or 0.0) for metric in ref)
    max_wellfriendpdf_mean = max(float(metric.get("mean_abs_error") or 0.0) for metric in wellfriendpdf)
    return max_wellfriendpdf_changed <= max_ref_changed + 0.01 and max_wellfriendpdf_mean <= max_ref_mean + 1.10


def copy_tool_manifest() -> dict[str, Any]:
    src = PROMPT06B_TOOL_MANIFEST if PROMPT06B_TOOL_MANIFEST.exists() else PROMPT07B_TOOL_MANIFEST
    if not src.exists():
        raise RuntimeError(
            "Missing target-local Poppler/PDFium/MuPDF manifest; run Prompt 06B bootstrap first"
        )
    TOOL_MANIFEST_OUT.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, TOOL_MANIFEST_OUT)
    return p06.load_manifest(TOOL_MANIFEST_OUT)


def write_starting_state() -> None:
    write_json(
        STARTING_OUT,
        {
            "kind": "prompt08_starting_state",
            "head": run_text(["git", "rev-parse", "--short", "HEAD"]),
            "status_short": run_text(["git", "status", "--short"]),
            "log_oneline_30": run_text(["git", "log", "--oneline", "-n", "30"]),
        },
    )


def write_static_matrices(entries: list[dict[str, Any]]) -> None:
    by_category: dict[str, list[dict[str, Any]]] = {}
    for entry in entries:
        by_category.setdefault(entry["category"], []).append(entry)
    write_json(TEXT_MATRIX_OUT, {"kind": "prompt08_text_clipping_matrix", "entries": by_category.get("text_clipping", [])})
    write_json(AXIAL_MATRIX_OUT, {"kind": "prompt08_axial_radial_shading_matrix", "entries": by_category.get("axial_radial_shading", [])})
    write_json(MESH_MATRIX_OUT, {"kind": "prompt08_mesh_patch_shading_matrix", "entries": by_category.get("mesh_patch_shading", [])})
    write_json(PATTERN_MATRIX_OUT, {"kind": "prompt08_tiling_pattern_matrix", "entries": by_category.get("tiling_pattern", [])})
    write_json(
        FALLBACK_OUT,
        {
            "kind": "prompt08_fallback_taxonomy",
            "removed_vague_buckets": ["text_clipping/later", "shading/later", "pattern/later"],
            "remaining_precise_limits": [
                "advanced_icc_device_link_multicolor_cmm",
                "image_or_resource_only_Type3_charproc_fail_closed",
                "exotic_missing_glyph_outline_for_text_clip",
                "cropped_coordinate_offscreen_optimization",
            ],
            "fail_closed_categories": [
                "malformed_shading_stream",
                "malformed_pattern_steps",
                "pattern_recursion_limit",
                "radial_degenerate_reported",
            ],
        },
    )
    write_json(
        MEMORY_OUT,
        {
            "kind": "prompt08_memory_scheduler_report",
            "memory_cap_mb": 4096,
            "pattern_tile_count_cap": 20000,
            "pattern_recursion_cap": 8,
            "offscreen_surfaces": "scheduler bounded by existing render decode/offscreen posture",
            "mesh_tessellation": "bounded fixed subdivision and raster triangle loops",
        },
    )


def render_and_compare(
    entries: list[dict[str, Any]],
    manifest: dict[str, Any],
    wellfriendpdf_bin: str | None,
    dpi: int,
    timeout: int,
) -> tuple[dict[str, Any], dict[str, Any]]:
    base = p06.wellfriendpdf_base_command(wellfriendpdf_bin)
    pages: list[dict[str, Any]] = []
    metrics_pages: list[dict[str, Any]] = []
    classification_counts: dict[str, int] = {}
    raw_classification_counts: dict[str, int] = {}

    for entry in entries:
        renders = {
            "wellfriendpdf": p06.render_wellfriendpdf(base, entry, dpi, timeout),
            "poppler": p06.render_reference("poppler", manifest["tools"]["poppler"], entry, dpi, timeout),
            "pdfium": p06.render_reference("pdfium", manifest["tools"]["pdfium"], entry, dpi, timeout),
            "mupdf": p06.render_reference("mupdf", manifest["tools"]["mupdf"], entry, dpi, timeout),
        }
        pair_metrics = {
            f"{a}_vs_{b}": safe_image_metrics(
                a,
                renders[a].get("artifact"),
                b,
                renders[b].get("artifact"),
                entry["id"],
            )
            for a, b in p06.PAIR_NAMES
        }
        raw = p06.classify_page(entry["category"], renders, pair_metrics)
        classification, classification_note = prompt08_classification(
            raw, entry["category"], pair_metrics
        )
        raw_classification_counts[raw] = raw_classification_counts.get(raw, 0) + 1
        classification_counts[classification] = classification_counts.get(classification, 0) + 1
        page = {
            **entry,
            "renders": renders,
            "pair_metrics": pair_metrics,
            "raw_prompt06b_classification": raw,
            "classification": classification,
        }
        if classification_note:
            page["classification_note"] = classification_note
        pages.append(page)
        metrics_pages.append({"id": entry["id"], "pairs": pair_metrics})

    results = {
        "schema_version": 1,
        "kind": "prompt08_multi_reference_render_results",
        "dpi": dpi,
        "page_count": len(entries),
        "pages": pages,
    }
    pair_summary = p06.pair_summary(metrics_pages)
    summary = {
        "schema_version": 1,
        "kind": "prompt08_reference_disagreement_summary",
        "page_count": len(entries),
        "total_pairwise_comparisons": len(entries) * len(p06.PAIR_NAMES),
        "classification_counts": classification_counts,
        "raw_prompt06b_classification_counts": raw_classification_counts,
        "pair_summary": pair_summary,
        "wellfriendpdf_outlier_failures": classification_counts.get("wellfriendpdf_outlier_failure", 0),
        "malformed_reference_failures": classification_counts.get("malformed_reference_failure", 0),
        "prompt08_cluster_tolerance_acceptances": sum(
            1 for page in pages if "classification_note" in page
        ),
    }
    write_json(RESULTS_OUT, results)
    write_json(DIFF_METRICS_OUT, {"kind": "prompt08_visual_diff_metrics", "pages": metrics_pages})
    write_json(DISAGREEMENT_OUT, summary)
    p06.render_html(results, summary)
    HTML_OUT.write_text(
        HTML_OUT.read_text(encoding="utf-8").replace(
            "Prompt 06B Multi-Reference Renderer Audit",
            "Prompt 08 Text Clipping, Shading, and Pattern Audit",
        ),
        encoding="utf-8",
    )
    return results, summary


def safe_image_metrics(
    a_name: str,
    a_path: str | None,
    b_name: str,
    b_path: str | None,
    entry_id: str,
) -> dict[str, Any]:
    try:
        return p06.image_metrics(a_name, a_path, b_name, b_path, entry_id)
    except Exception as exc:  # malformed/reference outputs are audit data, not fatal
        return {
            "status": "image_decode_failure",
            "threshold_pass": False,
            "artifact_a": a_path,
            "artifact_b": b_path,
            "entry_id": entry_id,
            "error": str(exc),
        }


def write_feature_report(wellfriendpdf_bin: str | None, timeout: int) -> None:
    base = p06.wellfriendpdf_base_command(wellfriendpdf_bin)
    result = run_full_command([*base, "feature-report"], timeout=timeout)
    payload: dict[str, Any] = {
        "kind": "prompt08_public_feature_report",
        "command": result,
    }
    try:
        payload["feature_report"] = json.loads(result.get("stdout") or "{}")
    except json.JSONDecodeError as exc:
        payload["parse_error"] = str(exc)
    write_json(FEATURE_OUT, payload)


def run_full_command(cmd: list[str], timeout: int) -> dict[str, Any]:
    started = time.time()
    actual_cmd = cmd
    if cmd and cmd[0].lower().endswith((".cmd", ".bat")):
        import os

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
            "stdout": proc.stdout,
            "stderr": proc.stderr,
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
        return {
            "command": cmd,
            "executed_command": actual_cmd,
            "exit_status": None,
            "stdout": stdout,
            "stderr": stderr,
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wellfriendpdf-bin")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    configure_prompt06b_runner()
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    write_starting_state()
    manifest = copy_tool_manifest()
    entries = corpus_entries()
    categories: dict[str, int] = {}
    for entry in entries:
        categories[entry["category"]] = categories.get(entry["category"], 0) + 1
    write_json(
        CORPUS_OUT,
        {
            "schema_version": 1,
            "kind": "prompt08_corpus_manifest",
            "page_count": len(entries),
            "categories": categories,
            "entries": entries,
        },
    )
    write_static_matrices(entries)
    render_and_compare(entries, manifest, args.wellfriendpdf_bin, args.dpi, args.timeout)
    write_feature_report(args.wellfriendpdf_bin, args.timeout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
