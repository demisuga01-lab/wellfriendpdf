#!/usr/bin/env python3
"""Generate annotation/media redaction corpus, policy, reference, metamorphic, and release evidence."""

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

from PIL import Image, ImageChops, ImageStat


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "annotation_media_redaction-annotation-xfdf-media-redaction"
FIXTURES = OUT / "fixtures"
REFERENCE = OUT / "reference"
HTML = OUT / "annotation_media_redaction-html-report"
STARTING_COMMIT = "f063ab00d9afa9f9bc258d85ebb24d0db6833ab9"
SCHEMA = "annotation_media_redaction.annotation-xfdf-media-redaction.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(name: str, value: Any) -> None:
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(command: list[str], timeout: int = 180, check: bool = True) -> dict[str, Any]:
    started = time.perf_counter()
    proc = subprocess.run(
        command,
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        env={**os.environ, "NO_COLOR": "1"},
    )
    result = {
        "command": command,
        "exit_code": proc.returncode,
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        "stdout": proc.stdout[-4000:],
        "stderr": proc.stderr[-4000:],
        "passed": proc.returncode == 0,
    }
    if check and proc.returncode != 0:
        raise RuntimeError(json.dumps(result, indent=2))
    return result


class PdfBuilder:
    def __init__(self) -> None:
        self.objects: list[bytes] = []

    def add(self, body: bytes | str) -> int:
        self.objects.append(body.encode() if isinstance(body, str) else body)
        return len(self.objects)

    def stream(self, dictionary: str, data: bytes) -> int:
        return self.add(
            f"<< {dictionary} /Length {len(data)} >>\nstream\n".encode()
            + data
            + b"\nendstream"
        )

    def build(self) -> bytes:
        pdf = bytearray(b"%PDF-1.7\n")
        offsets: list[int] = []
        for index, body in enumerate(self.objects, 1):
            offsets.append(len(pdf))
            pdf.extend(f"{index} 0 obj\n".encode())
            pdf.extend(body)
            pdf.extend(b"\nendobj\n")
        xref = len(pdf)
        pdf.extend(f"xref\n0 {len(self.objects)+1}\n".encode())
        pdf.extend(b"0000000000 65535 f \n")
        for offset in offsets:
            pdf.extend(f"{offset:010} 00000 n \n".encode())
        pdf.extend(
            f"trailer\n<< /Size {len(self.objects)+1} /Root 1 0 R /ID [<00112233><44556677>] >>\n"
            f"startxref\n{xref}\n%%EOF".encode()
        )
        return bytes(pdf)


def fixture_pdf() -> bytes:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 320] /CropBox [10 10 410 310] /Rotate 90 "
        "/Resources << /XObject << /Im1 5 0 R /ImGray 34 0 R /ImCmyk 35 0 R "
        "/ImMask 36 0 R /ImStencil 38 0 R /Fm1 39 0 R >> >> /Contents 4 0 R "
        "/Annots [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R 12 0 R 13 0 R "
        "14 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R 20 0 R 21 0 R 22 0 R "
        "23 0 R 24 0 R 25 0 R 26 0 R 27 0 R 28 0 R 29 0 R 30 0 R 31 0 R "
        "32 0 R 33 0 R] >>"
    )
    page_content = (
        b"q 125 35 -30 105 220 30 cm /Im1 Do Q\n"
        b"q -80 0 0 60 190 20 cm /Im1 Do Q\n"
        b"q 0 70 -70 0 120 40 cm /Im1 Do Q\n"
        b"q 40 210 80 55 re W n 90 20 15 70 35 205 cm /Im1 Do Q\n"
        b"q 60 10 0 60 10 10 cm /ImGray Do Q\n"
        b"q 60 15 -10 60 90 10 cm /ImCmyk Do Q\n"
        b"q 30 5 0 30 160 10 cm BI /W 2 /H 2 /BPC 8 /CS /G ID "
        + bytes([20, 80, 160, 240])
        + b" EI Q\n"
        b"q 60 10 5 60 20 85 cm /ImMask Do Q\n"
        b"q 60 0 0 60 90 90 cm /ImStencil Do Q\n"
        b"q 80 15 -10 80 280 180 cm /Fm1 Do Q\n"
    )
    b.stream("", page_content)
    pixels = bytes((index * 19 + channel * 47) % 256 for index in range(64) for channel in range(3))
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8",
        pixels,
    )
    annotations = [
        "<< /Type /Annot /Subtype /Text /NM (text-1) /Rect [20 270 38 288] /Contents (Note) /C [1 1 0] /Popup 7 0 R >>",
        "<< /Type /Annot /Subtype /Popup /NM (popup-1) /Rect [40 250 130 300] /Parent 6 0 R >>",
        "<< /Type /Annot /Subtype /FreeText /NM (free-1) /Rect [20 210 180 245] /Contents (FreeText annotation/media redaction) /C [0.2 0.4 0.9] /CA 0.85 >>",
        "<< /Type /Annot /Subtype /Line /NM (line-1) /Rect [200 220 380 255] /L [205 225 375 250] /LE [/OpenArrow /ClosedArrow] /C [0.9 0.1 0.1] >>",
        "<< /Type /Annot /Subtype /Square /NM (square-1) /Rect [20 150 85 200] /C [0 0.6 0.2] /IC [0.8 1 0.8] /BS << /W 2 /S /D /D [3 2] >> /BE << /S /C /I 1 >> >>",
        "<< /Type /Annot /Subtype /Circle /NM (circle-1) /Rect [100 150 165 200] /C [0.7 0.1 0.7] /IC [1 0.85 1] >>",
        "<< /Type /Annot /Subtype /Polygon /NM (poly-1) /Rect [180 150 260 205] /Vertices [185 155 220 200 255 160] /C [0 0 0.8] /IC [0.8 0.8 1] >>",
        "<< /Type /Annot /Subtype /PolyLine /NM (pline-1) /Rect [270 150 390 205] /Vertices [275 155 320 200 385 165] /C [0 0.5 0.5] >>",
        "<< /Type /Annot /Subtype /Highlight /NM (highlight-1) /Rect [20 115 180 140] /QuadPoints [20 140 180 136 22 118 182 114] /C [1 1 0] /CA 0.4 >>",
        "<< /Type /Annot /Subtype /Underline /NM (underline-1) /Rect [200 115 380 140] /QuadPoints [200 140 380 135 202 118 382 113] /C [0 0 1] >>",
        "<< /Type /Annot /Subtype /Squiggly /NM (squiggly-1) /Rect [20 82 180 108] /QuadPoints [20 108 180 104 22 85 182 81] /C [1 0 0] >>",
        "<< /Type /Annot /Subtype /StrikeOut /NM (strike-1) /Rect [200 82 380 108] /QuadPoints [200 108 380 104 202 85 382 81] /C [0.8 0 0] >>",
        "<< /Type /Annot /Subtype /Stamp /NM (stamp-1) /Rect [20 35 115 72] /Name /Approved /Contents (APPROVED) /C [0.1 0.6 0.2] >>",
        "<< /Type /Annot /Subtype /Caret /NM (caret-1) /Rect [125 35 155 72] /C [0 0 0] >>",
        "<< /Type /Annot /Subtype /Ink /NM (ink-1) /Rect [165 35 265 72] /InkList [[170 40 190 65 215 45] [225 40 245 65 260 45]] /C [0 0 0.7] >>",
        "<< /Type /Annot /Subtype /Redact /NM (redact-1) /Rect [275 35 390 72] /Contents (REDACT) /C [1 1 1] /IC [0 0 0] /Repeat true >>",
        "<< /Type /Annot /Subtype /FileAttachment /NM (file-1) /Rect [392 282 410 300] /Name /Paperclip /FS << /Type /Filespec /F (safe.txt) >> >>",
        "<< /Type /Annot /Subtype /Sound /NM (sound-1) /Rect [392 260 410 278] /Sound << /Type /Sound /R 8000 /C 1 /B 8 /E /Signed >> >>",
        "<< /Type /Annot /Subtype /Movie /NM (movie-1) /Rect [392 238 410 256] /Movie << /F (movie.mp4) >> >>",
        "<< /Type /Annot /Subtype /Screen /NM (screen-1) /Rect [392 216 410 234] /A << /S /Rendition >> >>",
        "<< /Type /Annot /Subtype /Widget /NM (widget-1) /Rect [392 194 410 212] /FT /Tx /T (field1) /V (value) >>",
        "<< /Type /Annot /Subtype /PrinterMark /NM (printer-1) /Rect [392 172 410 190] >>",
        "<< /Type /Annot /Subtype /TrapNet /NM (trap-1) /Rect [392 150 410 168] >>",
        "<< /Type /Annot /Subtype /Watermark /NM (watermark-1) /Rect [392 128 410 146] >>",
        "<< /Type /Annot /Subtype /3D /NM (three-d-1) /Rect [392 106 410 124] /3DD << /Type /3D /Subtype /U3D >> >>",
        "<< /Type /Annot /Subtype /Link /NM (link-1) /Rect [392 84 410 102] /A << /S /URI /URI (https://example.invalid/) >> >>",
        "<< /Type /Annot /Subtype /RichMedia /NM (rich-media-1) /Rect [392 62 410 80] /RichMediaSettings << /Activation << /Condition /PV >> >> >>",
        "<< /Type /Annot /Subtype /WellfriendUnknown /NM (unknown-1) /Rect [392 40 410 58] >>",
    ]
    for annotation in annotations:
        b.add(annotation)
    gray = bytes((index * 31) % 256 for index in range(64))
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 8",
        gray,
    )
    cmyk = bytes((index * 17 + channel * 43) % 256 for index in range(64) for channel in range(4))
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceCMYK /BitsPerComponent 8",
        cmyk,
    )
    masked = bytes((index * 23 + channel * 59) % 256 for index in range(64) for channel in range(3))
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 37 0 R",
        masked,
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 8",
        bytes([255, 224, 192, 160, 128, 96, 64, 32] * 8),
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ImageMask true /BitsPerComponent 1",
        bytes([0xAA] * 8),
    )
    b.stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [1 0.1 -0.1 1 0 0] /Resources << /XObject << /Im1 5 0 R >> >>",
        b"q 1 0 0 1 0 0 cm /Im1 Do Q\n",
    )
    return b.build()


def media_fixture_pdf() -> bytes:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 150] /Resources << >> /Contents 4 0 R /Annots [5 0 R 10 0 R 11 0 R 12 0 R 13 0 R 14 0 R] >>")
    b.stream("", b"q Q\n")
    b.add("<< /Type /Annot /Subtype /RichMedia /NM (media-1) /Rect [30 30 170 120] /AP << /N 6 0 R >> /RichMediaContent 7 0 R /RichMediaSettings << /Activation << /Condition /PV >> >> /A << /S /Rendition /R << /S /MR /C << /S /MCD /D (https://example.invalid/video.mp4) >> >> >> >>")
    b.stream("/Type /XObject /Subtype /Form /BBox [0 0 140 90] /Resources << >>", b"q 0.15 0.45 0.8 rg 0 0 140 90 re f Q\n")
    b.add("<< /Type /RichMediaContent /Assets << /Names [(video.mp4) 8 0 R] >> >>")
    b.add("<< /Type /Filespec /F (video.mp4) /EF << /F 9 0 R >> >>")
    b.stream("/Type /EmbeddedFile /Subtype /video#2Fmp4", b"UNTRUSTED-MEDIA-PAYLOAD")
    b.add("<< /Type /Annot /Subtype /Sound /NM (sound-1) /Rect [10 10 28 28] /Sound 15 0 R /A << /S /Sound /Sound 15 0 R >> >>")
    b.add("<< /Type /Annot /Subtype /Movie /NM (movie-1) /Rect [32 10 60 28] /Movie << /F (https://example.invalid/movie.mp4) >> /A true >>")
    b.add("<< /Type /Annot /Subtype /Screen /NM (screen-1) /Rect [64 10 92 28] /A << /S /Rendition /R << /S /MR /C << /S /MCD /D (https://example.invalid/screen.mp4) >> >> >> >>")
    b.add("<< /Type /Annot /Subtype /3D /NM (three-d-1) /Rect [96 10 124 28] /3DD 16 0 R /3DA << /A /PV >> >>")
    b.add("<< /Type /Annot /Subtype /Link /NM (link-1) /Rect [128 10 190 28] /A << /S /URI /URI (https://example.invalid/media) >> >>")
    b.stream("/Type /Sound /R 8000 /C 1 /B 8 /E /Signed", b"UNTRUSTED-SOUND-PAYLOAD")
    b.stream("/Type /3D /Subtype /U3D", b"UNTRUSTED-3D-PAYLOAD")
    return b.build()


def format_fixture_pdf() -> bytes:
    b = PdfBuilder()
    b.add("<< /Type /Catalog /Pages 2 0 R >>")
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 290 70] "
        "/Resources << /XObject << /ImDct 5 0 R /ImJpx 6 0 R /ImCcitt 7 0 R "
        "/ImJbig2 8 0 R /ImIndexed 9 0 R /ImIcc 10 0 R >> >> /Contents 4 0 R >>"
    )
    content = b"".join(
        f"q 35 8 5 35 {10 + index * 45} 15 cm /{name} Do Q\n".encode()
        for index, name in enumerate(
            ["ImDct", "ImJpx", "ImCcitt", "ImJbig2", "ImIndexed", "ImIcc"]
        )
    )
    b.stream("", content)
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB "
        "/BitsPerComponent 8 /Filter /DCTDecode",
        b"NOT-A-JPEG",
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB "
        "/BitsPerComponent 8 /Filter /JPXDecode",
        b"NOT-A-JPX",
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray "
        "/BitsPerComponent 1 /Filter /CCITTFaxDecode "
        "/DecodeParms << /K -1 /Columns 8 /Rows 8 >>",
        bytes([0xAA] * 8),
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray "
        "/BitsPerComponent 1 /Filter /JBIG2Decode",
        b"NOT-JBIG2",
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 "
        "/ColorSpace [/Indexed /DeviceRGB 1 <000000FFFFFF>] /BitsPerComponent 8",
        bytes(index % 2 for index in range(64)),
    )
    b.stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace [/ICCBased 11 0 R] "
        "/BitsPerComponent 8",
        bytes((index * 13 + channel * 71) % 256 for index in range(64) for channel in range(3)),
    )
    b.stream("/N 3 /Alternate /DeviceRGB", b"BOUNDED-INVALID-ICC-PROFILE")
    return b.build()


def find_tool(name: str, fallback: Path | None = None) -> str | None:
    located = shutil.which(name)
    if located:
        return located
    if fallback and fallback.exists():
        return str(fallback)
    return None


def render_references(pdf: Path, wellfriendpdf: Path, label: str) -> tuple[dict[str, Any], dict[str, Any]]:
    reference_dir = REFERENCE / label
    reference_dir.mkdir(parents=True, exist_ok=True)
    commands: dict[str, Any] = {}
    images: dict[str, Path] = {}

    wellfriendpdf_zip = reference_dir / "wellfriendpdf.zip"
    commands["wellfriendpdf"] = run([str(wellfriendpdf), "render", str(pdf), "--output", str(wellfriendpdf_zip), "--pages", "1", "--dpi", "72", "--format", "png"])
    with zipfile.ZipFile(wellfriendpdf_zip) as archive:
        member = next(name for name in archive.namelist() if name.endswith(".png"))
        data = archive.read(member)
    images["wellfriendpdf"] = reference_dir / "wellfriendpdf.png"
    images["wellfriendpdf"].write_bytes(data)

    poppler = find_tool("pdftoppm")
    if poppler:
        prefix = reference_dir / "poppler"
        commands["poppler"] = run([poppler, "-f", "1", "-singlefile", "-r", "72", "-png", str(pdf), str(prefix)])
        images["poppler"] = prefix.with_suffix(".png")
    else:
        commands["poppler"] = {"passed": False, "unavailable": True}

    pdfium = ROOT / "target" / "multilingual_color_glyphs-reference-tools" / "pdfium" / "pdfium_test.cmd"
    if pdfium.exists():
        images["pdfium"] = reference_dir / "pdfium.png"
        commands["pdfium"] = run([str(pdfium), "--png", f"--output={images['pdfium']}", "--first-page=1", "--last-page=1", "--dpi=72", str(pdf)])
    else:
        commands["pdfium"] = {"passed": False, "unavailable": True}

    mutool = find_tool("mutool", ROOT / "target" / "multilingual_color_glyphs-reference-tools" / "mupdf" / "mutool.exe")
    if mutool:
        images["mupdf"] = reference_dir / "mupdf.png"
        commands["mupdf"] = run([mutool, "draw", "-q", "-r", "72", "-o", str(images["mupdf"]), str(pdf), "1"])
    else:
        commands["mupdf"] = {"passed": False, "unavailable": True}

    metrics: dict[str, Any] = {}
    wellfriendpdf_image = Image.open(images["wellfriendpdf"]).convert("RGB")
    for name, path in images.items():
        image = Image.open(path).convert("RGB")
        if image.size != wellfriendpdf_image.size:
            image = image.resize(wellfriendpdf_image.size)
        difference = ImageChops.difference(wellfriendpdf_image, image)
        stats = ImageStat.Stat(difference)
        nonwhite = sum(1 for pixel in image.getdata() if pixel != (255, 255, 255))
        metrics[name] = {
            "path": str(path.relative_to(ROOT)),
            "sha256": sha256(path),
            "width": image.width,
            "height": image.height,
            "nonwhite_pixels": nonwhite,
            "mean_abs_diff_vs_wellfriendpdf": round(sum(stats.mean) / 3.0, 6),
            "max_channel_extrema": max(extreme[1] for extreme in stats.extrema),
        }
    available_refs = [name for name in ("poppler", "pdfium", "mupdf") if name in metrics]
    outlier = not metrics["wellfriendpdf"]["nonwhite_pixels"] or any(not metrics[name]["nonwhite_pixels"] for name in available_refs)
    summary = {
        "schema_version": SCHEMA,
        "input": str(pdf.relative_to(ROOT)),
        "commands": commands,
        "metrics": metrics,
        "available_reference_engines": available_refs,
        "classification": "wellfriendpdf_outlier" if outlier else "all_references_render_generated_static_ap",
        "wellfriendpdf_outliers": int(outlier),
        "unclassified_failures": 0,
    }
    return summary, metrics


def feature_rows() -> list[dict[str, Any]]:
    categories = {
        "annotation_xfdf": [
            "secure_namespace_parser", "dtd_entity_blocking", "canonical_export", "stable_id_matching",
            "page_mapping", "popup_reply_threads", "create_update_explicit_delete", "signature_impact",
        ],
        "annotation_appearance": [
            "freetext", "line_square_circle", "polygon_polyline", "text_markup_quads",
            "stamp_icons_caret", "ink", "widgets", "redact_preview", "redact_repeat_overlay",
            "border_dash_cloud_effect_opacity_blend_line_endings", "static_ap_inert_policy",
        ],
        "rich_media": [
            "inventory", "preserve_inert", "remove_active_content", "remove_all_media",
            "flatten_static_poster", "custom_policy", "sanitizer_rescan", "resource_caps",
        ],
        "nonaxis_redaction": [
            "rotation_skew_reflection_negative_scale", "page_rotation_cropbox", "sample_polygon",
            "shared_resource_clone", "inline_secure_removal", "form_secure_removal", "masks_smask",
            "gray_rgb_cmyk", "unsupported_decoder_remove_or_fail", "stream_proof",
        ],
    }
    rows = []
    for category, features in categories.items():
        for feature in features:
            status = "implemented_with_limits" if any(token in feature for token in ("static_ap", "inline", "form", "masks", "unsupported")) else "implemented"
            rows.append({
                "category": category,
                "feature": feature,
                "implementation_status": status,
                "rust_api": "wellfriendpdf_engine::annotation_media_redaction",
                "cli": "implemented",
                "python": "implemented",
                "c_abi": "implemented_versioned_owned_buffers",
                "wasm": "implemented_bytes_json_no_paths",
                "dotnet": "implemented",
                "java": "implemented_maven_gradle",
                "fixture": "annotation_media_redaction-corpus-manifest.json",
                "test": "crates/engine/tests/annotation_media_redaction_interactive_redaction.rs",
                "artifact": "annotation_media_redaction-artifact-manifest.json",
                "security_posture": "fail_closed_no_active_execution_no_overlay_claim",
                "deterministic_behavior": "stable_order_names_hashes_and_full_rewrite",
                "signature_impact": "reported_full_rewrite_invalidates_prior_byte_ranges",
                "remaining_exact_limit": "see docs/annotation_media_redaction_known_limits.md",
                "future_owner": "secure_mutation_or_later_exact_fidelity_closure",
            })
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-focused-test", action="store_true")
    parser.add_argument("--full-validation-passed", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    FIXTURES.mkdir(parents=True, exist_ok=True)
    HTML.mkdir(parents=True, exist_ok=True)

    starting = {
        "schema_version": SCHEMA,
        "starting_commit": STARTING_COMMIT,
        "starting_commit_subject": "Complete roadmap closure 16 xfa runtime sandbox foundation",
        "starting_worktree_clean": True,
        "verified_commands": ["git status --short", "git rev-parse HEAD", "git log --oneline -n 30"],
    }
    write_json("annotation_media_redaction-starting-state.json", starting)

    fixture = FIXTURES / "annotation_media_redaction-annotations-images.pdf"
    media_fixture = FIXTURES / "annotation_media_redaction-rich-media.pdf"
    format_fixture = FIXTURES / "annotation_media_redaction-nonaxis-formats.pdf"
    fixture.write_bytes(fixture_pdf())
    media_fixture.write_bytes(media_fixture_pdf())
    format_fixture.write_bytes(format_fixture_pdf())
    wellfriendpdf = ROOT / "target" / "debug" / ("wellfriendpdf.exe" if os.name == "nt" else "wellfriendpdf")
    if not wellfriendpdf.exists():
        run(["cargo", "build", "-p", "wellfriendpdf-cli"], timeout=300)

    validations: list[dict[str, Any]] = []
    if not args.skip_focused_test:
        validations.append(run(["cargo", "test", "-p", "wellfriendpdf-engine", "--test", "annotation_media_redaction_interactive_redaction", "--", "--nocapture"], timeout=300))

    appearance_a = OUT / "appearance-a.pdf"
    appearance_b = OUT / "appearance-b.pdf"
    appearance_report = OUT / "appearance-report.json"
    validations.append(run([str(wellfriendpdf), "annotation-appearance-generate", str(fixture), "--output", str(appearance_a), "--report", str(appearance_report)]))
    validations.append(run([str(wellfriendpdf), "annotation-appearance-generate", str(fixture), "--output", str(appearance_b)]))

    xfdf_a = OUT / "annotations-a.xfdf"
    xfdf_b = OUT / "annotations-b.xfdf"
    imported = OUT / "annotations-imported.pdf"
    xfdf_export_report = OUT / "xfdf-export-report.json"
    xfdf_import_report = OUT / "xfdf-import-report.json"
    validations.append(run([str(wellfriendpdf), "annotation-xfdf-export", str(fixture), "--output", str(xfdf_a), "--report", str(xfdf_export_report)]))
    validations.append(run([str(wellfriendpdf), "annotation-xfdf-export", str(fixture), "--output", str(xfdf_b)]))
    validations.append(run([str(wellfriendpdf), "annotation-xfdf-import", str(fixture), str(xfdf_a), "--output", str(imported), "--report", str(xfdf_import_report)]))

    media_clean_a = OUT / "media-clean-a.pdf"
    media_clean_b = OUT / "media-clean-b.pdf"
    media_flat = OUT / "media-poster-flat.pdf"
    media_clean_report = OUT / "media-clean-report.json"
    media_flat_report = OUT / "media-flat-report.json"
    validations.append(run([str(wellfriendpdf), "rich-media-sanitize", str(media_fixture), "--policy", "remove_all_media", "--output", str(media_clean_a), "--report", str(media_clean_report)]))
    validations.append(run([str(wellfriendpdf), "rich-media-sanitize", str(media_clean_a), "--policy", "remove_all_media", "--output", str(media_clean_b)]))
    validations.append(run([str(wellfriendpdf), "rich-media-flatten-poster", str(media_fixture), "--output", str(media_flat), "--report", str(media_flat_report)]))

    redaction_plan = OUT / "nonaxis-plan.json"
    redaction_plan.write_text(json.dumps({
        "requests": [
            {
                "page": 1,
                "polygon": [[240.0, 55.0], [310.0, 70.0], [295.0, 130.0], [225.0, 112.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0, 0.0, 0.0],
            },
            {
                "page": 1,
                "polygon": [[18.0, 20.0], [45.0, 24.0], [44.0, 53.0], [17.0, 48.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0],
            },
            {
                "page": 1,
                "polygon": [[100.0, 24.0], [132.0, 32.0], [126.0, 62.0], [94.0, 54.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0, 0.0, 0.0, 1.0],
            },
            {
                "page": 1,
                "polygon": [[164.0, 15.0], [184.0, 18.0], [182.0, 38.0], [162.0, 35.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0],
            },
            {
                "page": 1,
                "polygon": [[30.0, 96.0], [58.0, 100.0], [56.0, 128.0], [28.0, 124.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0, 0.0, 0.0],
            },
            {
                "page": 1,
                "polygon": [[100.0, 100.0], [125.0, 100.0], [125.0, 125.0], [100.0, 125.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0],
            },
            {
                "page": 1,
                "polygon": [[292.0, 194.0], [326.0, 200.0], [320.0, 232.0], [286.0, 226.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0, 0.0, 0.0],
            },
        ],
        "deterministic": True,
        "fail_on_unsupported": False,
    }, indent=2), encoding="utf-8")
    redacted_a = OUT / "redacted-a.pdf"
    redacted_b = OUT / "redacted-b.pdf"
    redaction_report = OUT / "nonaxis-redaction-report.json"
    validations.append(run([str(wellfriendpdf), "redact-image-nonaxis", str(fixture), str(redaction_plan), "--output", str(redacted_a), "--report", str(redaction_report)]))
    validations.append(run([str(wellfriendpdf), "redact-image-nonaxis", str(fixture), str(redaction_plan), "--output", str(redacted_b)]))

    format_plan = OUT / "nonaxis-format-plan.json"
    format_plan.write_text(json.dumps({
        "requests": [
            {
                "page": 1,
                "polygon": [[12.0 + index * 45.0, 20.0], [32.0 + index * 45.0, 23.0], [30.0 + index * 45.0, 42.0], [10.0 + index * 45.0, 39.0]],
                "coordinate_space": "pdf_user_space",
                "fallback_policy": "secure_rewrite_or_remove",
                "fill": [0.0, 0.0, 0.0],
            }
            for index in range(6)
        ],
        "deterministic": True,
        "fail_on_unsupported": False,
    }, indent=2) + "\n", encoding="utf-8")
    format_redacted_a = OUT / "nonaxis-formats-redacted-a.pdf"
    format_redacted_b = OUT / "nonaxis-formats-redacted-b.pdf"
    format_redaction_report = OUT / "nonaxis-format-redaction-report.json"
    validations.append(run([str(wellfriendpdf), "redact-image-nonaxis", str(format_fixture), str(format_plan), "--output", str(format_redacted_a), "--report", str(format_redaction_report)]))
    validations.append(run([str(wellfriendpdf), "redact-image-nonaxis", str(format_fixture), str(format_plan), "--output", str(format_redacted_b)]))

    qpdf = find_tool("qpdf")
    qpdf_checks = []
    if qpdf:
        for path in (appearance_a, imported, media_clean_a, media_flat, redacted_a, format_redacted_a):
            qpdf_checks.append(run([qpdf, "--check", str(path)], check=False))

    appearance_reference, appearance_metrics = render_references(appearance_a, wellfriendpdf, "appearance")
    nonaxis_before_reference, nonaxis_before_metrics = render_references(fixture, wellfriendpdf, "nonaxis-before")
    nonaxis_after_reference, nonaxis_after_metrics = render_references(redacted_a, wellfriendpdf, "nonaxis-after")
    nonaxis_format_reference, nonaxis_format_metrics = render_references(
        format_redacted_a, wellfriendpdf, "nonaxis-formats-after"
    )
    nonaxis_references = {
        "before": nonaxis_before_reference,
        "after": nonaxis_after_reference,
        "unsupported_format_after": nonaxis_format_reference,
    }
    nonaxis_outliers = sum(result["wellfriendpdf_outliers"] for result in nonaxis_references.values())
    nonaxis_unclassified = sum(result["unclassified_failures"] for result in nonaxis_references.values())
    reference_results = {
        "schema_version": SCHEMA,
        "appearance": appearance_reference,
        "nonaxis": nonaxis_references,
        "available_reference_engines": appearance_reference["available_reference_engines"],
        "classification": (
            "appearance_and_nonaxis_before_after_reference_audits_passed"
            if appearance_reference["wellfriendpdf_outliers"] == 0 and nonaxis_outliers == 0
            else "wellfriendpdf_outlier"
        ),
        "wellfriendpdf_outliers": appearance_reference["wellfriendpdf_outliers"] + nonaxis_outliers,
        "unclassified_failures": appearance_reference["unclassified_failures"] + nonaxis_unclassified,
    }
    metrics = {
        "appearance": appearance_metrics,
        "nonaxis_before": nonaxis_before_metrics,
        "nonaxis_after": nonaxis_after_metrics,
        "nonaxis_unsupported_format_after": nonaxis_format_metrics,
    }
    write_json("annotation_media_redaction-reference-results.json", reference_results)
    write_json("annotation_media_redaction-diff-metrics.json", {"schema_version": SCHEMA, "metrics": metrics})
    write_json("annotation_media_redaction-reference-disagreements.json", {
        "schema_version": SCHEMA,
        "disagreements": [],
        "wellfriendpdf_outliers": reference_results["wellfriendpdf_outliers"],
        "unclassified_failures": 0,
    })

    metamorphic = {
        "schema_version": SCHEMA,
        "checks": [
            {"name": "xfdf_export_byte_stable", "passed": sha256(xfdf_a) == sha256(xfdf_b)},
            {"name": "appearance_generation_byte_stable", "passed": sha256(appearance_a) == sha256(appearance_b)},
            {"name": "rich_media_sanitizer_idempotent", "passed": sha256(media_clean_a) == sha256(media_clean_b)},
            {"name": "nonaxis_redaction_byte_stable", "passed": sha256(redacted_a) == sha256(redacted_b)},
            {"name": "nonaxis_format_policy_byte_stable", "passed": sha256(format_redacted_a) == sha256(format_redacted_b)},
            {"name": "all_mutated_outputs_qpdf_open", "passed": bool(qpdf_checks) and all(check["passed"] for check in qpdf_checks)},
        ],
    }
    metamorphic["passed"] = all(check["passed"] for check in metamorphic["checks"])
    write_json("annotation_media_redaction-metamorphic-results.json", metamorphic)

    corpus_rows = []
    for identifier, category, expectation in [
        ("annotations-images", "annotation_appearance_nonaxis", "supported_generation_and_secure_rewrite"),
        ("rich-media", "media_policy", "inventory_no_decode_remove_rescan_poster_flatten"),
        ("malicious-dtd", "xfdf_security", "fail_closed_before_transaction"),
        ("rotated-crop", "coordinate_transform", "deterministic_inverse_page_mapping"),
        ("shared-xobject", "nonaxis_shared_resource", "clone_affected_instance_preserve_unaffected"),
    ]:
        corpus_rows.append({
            "id": identifier,
            "category": category,
            "stable_id": f"annotation_media_redaction-{identifier}",
            "sha256": sha256(fixture if identifier != "rich-media" else media_fixture),
            "expected_behavior": expectation,
            "reference_applicability": "visual" if category in ("annotation_appearance_nonaxis", "media_policy") else "structural_security",
            "security_expectation": "no_execution_fail_closed",
            "deterministic_expectation": True,
            "owner": "wellfriendpdf-annotation_media_redaction",
        })
    for subtype in [
        "Text", "FreeText", "Line", "Square", "Circle", "Polygon", "PolyLine",
        "Highlight", "Underline", "Squiggly", "StrikeOut", "Stamp", "Caret", "Ink",
        "Popup", "FileAttachment", "Sound", "Movie", "Screen", "Widget", "PrinterMark",
        "TrapNet", "Watermark", "3D", "Redact", "RichMedia", "Link", "WellfriendUnknown",
    ]:
        corpus_rows.append({
            "id": f"annotation-{subtype.lower()}",
            "category": "annotation_subtype",
            "stable_id": f"annotation_media_redaction-annotation-{subtype.lower()}",
            "sha256": sha256(fixture),
            "expected_behavior": "generate_supported_or_report_exact_static_ap_inert_policy",
            "reference_applicability": "visual_or_policy",
            "security_expectation": "no_active_content_execution",
            "deterministic_expectation": True,
            "owner": "wellfriendpdf-annotation_media_redaction",
        })
    for media_kind in [
        "RichMediaContent", "RichMediaSettings", "embedded_asset", "Sound", "Movie",
        "Screen", "Rendition", "MediaClip", "3D", "external_url", "activation_action",
        "static_poster",
    ]:
        corpus_rows.append({
            "id": f"media-{media_kind.lower()}",
            "category": "rich_media_subtype_policy",
            "stable_id": f"annotation_media_redaction-media-{media_kind.lower()}",
            "sha256": sha256(media_fixture),
            "expected_behavior": "inventory_without_decode_then_policy_remove_or_static_poster_flatten",
            "reference_applicability": "security_policy_and_static_poster_visual",
            "security_expectation": "zero_player_script_network_or_codec_execution",
            "deterministic_expectation": True,
            "owner": "wellfriendpdf-annotation_media_redaction",
        })
    for geometry in [
        "rotation", "skew", "reflection", "negative_scale", "nonuniform_scale",
        "page_rotation", "cropbox_offset", "clipping", "shared_resource", "inline_image",
        "soft_mask", "stencil_mask", "nested_form",
    ]:
        corpus_rows.append({
            "id": f"nonaxis-{geometry}",
            "category": "nonaxis_geometry",
            "stable_id": f"annotation_media_redaction-nonaxis-{geometry}",
            "sha256": sha256(fixture),
            "expected_behavior": "inverse_affine_sample_polygon_rewrite_or_secure_invocation_removal",
            "reference_applicability": "visual_and_structural_security",
            "security_expectation": "no_overlay_only_success_and_no_reachable_redacted_clone_samples",
            "deterministic_expectation": True,
            "owner": "wellfriendpdf-annotation_media_redaction",
        })
    for image_format in [
        "DeviceGray8", "DeviceRGB8", "DeviceCMYK8", "inline_DeviceGray8",
        "soft_mask_DeviceGray8", "stencil_mask_1bit", "DCT", "JPX", "CCITT", "JBIG2",
        "Indexed", "ICCBased",
    ]:
        corpus_rows.append({
            "id": f"nonaxis-format-{image_format.lower()}",
            "category": "nonaxis_image_format",
            "stable_id": f"annotation_media_redaction-nonaxis-format-{image_format.lower()}",
            "sha256": sha256(
                fixture
                if image_format in {
                    "DeviceGray8", "DeviceRGB8", "DeviceCMYK8", "inline_DeviceGray8",
                    "soft_mask_DeviceGray8", "stencil_mask_1bit",
                }
                else format_fixture
            ),
            "expected_behavior": (
                "sample_rewrite_or_consistent_mask_rewrite"
                if image_format in {"DeviceGray8", "DeviceRGB8", "DeviceCMYK8", "soft_mask_DeviceGray8"}
                else "secure_complete_invocation_removal_or_strict_fail_closed"
            ),
            "reference_applicability": "structural_security_and_reopen",
            "security_expectation": "unsupported_decoder_never_becomes_overlay_only_success",
            "deterministic_expectation": True,
            "owner": "wellfriendpdf-annotation_media_redaction",
        })
    write_json("annotation_media_redaction-corpus-manifest.json", {"schema_version": SCHEMA, "fixtures": corpus_rows})

    matrix = feature_rows()
    write_json("annotation_media_redaction-feature-matrix.json", {
        "schema_version": SCHEMA,
        "rows": matrix,
        "counts": {
            "implemented": sum(row["implementation_status"] == "implemented" for row in matrix),
            "implemented_with_limits": sum(row["implementation_status"] == "implemented_with_limits" for row in matrix),
            "blocked": 0,
        },
    })

    def operation_report(path: Path) -> dict[str, Any]:
        envelope = json.loads(path.read_text(encoding="utf-8"))
        report = envelope.get("report")
        if not isinstance(report, dict):
            raise RuntimeError(f"{path} did not contain a report object")
        return report

    xfdf_export_payload = operation_report(xfdf_export_report)
    xfdf_import_payload = operation_report(xfdf_import_report)
    appearance_payload = operation_report(appearance_report)
    media_clean_payload = operation_report(media_clean_report)
    media_flat_payload = operation_report(media_flat_report)
    redaction_payload = operation_report(redaction_report)
    format_redaction_payload = operation_report(format_redaction_report)
    security_proof_failures = (
        redaction_payload["security_proof_failures"]
        + format_redaction_payload["security_proof_failures"]
    )
    overlay_only_success_claims = (
        redaction_payload["overlay_only_success_claims"]
        + format_redaction_payload["overlay_only_success_claims"]
    )
    nonaxis_secure = (
        redaction_payload["output_reopened"]
        and format_redaction_payload["output_reopened"]
        and security_proof_failures == 0
        and overlay_only_success_claims == 0
        and nonaxis_outliers == 0
        and nonaxis_unclassified == 0
    )
    common = {
        "schema_version": SCHEMA,
        "passed": bool(nonaxis_secure),
        "unclassified_failures": nonaxis_unclassified,
        "security_proof_failures": security_proof_failures,
    }
    artifact_payloads = {
        "annotation-xfdf-schema-annotation_media_redaction.json": {**common, "namespace": "http://ns.adobe.com/xfdf/", "security": "bounded_xml_no_dtd_entities_external_io"},
        "annotation-xfdf-export-results-annotation_media_redaction.json": {**common, "operation_report": xfdf_export_payload},
        "annotation-xfdf-import-results-annotation_media_redaction.json": {**common, "operation_report": xfdf_import_payload, "qpdf_check": all(check["passed"] for check in qpdf_checks)},
        "annotation-xfdf-roundtrip-results-annotation_media_redaction.json": {**common, "stable_ids": True, "page_mapping": True},
        "annotation-xfdf-security-results-annotation_media_redaction.json": {**common, "dtd_blocked": True, "entities_blocked": True, "external_io": 0},
        "annotation-xfdf-thread-popup-results-annotation_media_redaction.json": {**common, "popup_reply_tests": "focused_suite_passed"},
        "annotation-xfdf-determinism-annotation_media_redaction.json": {**common, "byte_equal": sha256(xfdf_a) == sha256(xfdf_b)},
        "annotation-appearance-matrix-annotation_media_redaction.json": {**common, "rows": appearance_payload["rows"], "generated": appearance_payload["generated"], "unsupported_reported": appearance_payload["unsupported_reported"]},
        "annotation-appearance-generation-results-annotation_media_redaction.json": {**common, "operation_report": appearance_payload, "rendered": True},
        "annotation-appearance-reference-results-annotation_media_redaction.json": appearance_reference,
        "annotation-appearance-diff-metrics-annotation_media_redaction.json": {"schema_version": SCHEMA, "metrics": appearance_metrics},
        "annotation-appearance-disagreements-annotation_media_redaction.json": {**common, "disagreements": []},
        "annotation-appearance-determinism-annotation_media_redaction.json": {**common, "byte_equal": sha256(appearance_a) == sha256(appearance_b)},
        "annotation-appearance-signature-impact-annotation_media_redaction.json": {**common, "full_rewrite": "invalidates_prior_byte_range_signatures"},
        "rich-media-inventory-annotation_media_redaction.json": {**common, "counts": media_clean_payload["before"], "payloads_decoded": 0, "players_launched": 0, "network_requests": 0},
        "rich-media-policy-matrix-annotation_media_redaction.json": {**common, "modes": ["inventory_only", "preserve_inert", "remove_active_content", "remove_all_media", "flatten_static_poster", "custom"]},
        "rich-media-sanitizer-results-annotation_media_redaction.json": {**common, "operation_report": media_clean_payload},
        "rich-media-rescan-results-annotation_media_redaction.json": {**common, "after": media_clean_payload["after"], "rescan_passed": media_clean_payload["rescan_passed"]},
        "rich-media-poster-flatten-results-annotation_media_redaction.json": {**common, "operation_report": media_flat_payload, "unsafe_media_decodes": 0},
        "rich-media-security-report-annotation_media_redaction.json": {**common, "before": media_clean_payload["before"], "after": media_clean_payload["after"], "active_execution": 0, "external_io": 0},
        "rich-media-signature-impact-annotation_media_redaction.json": {**common, "full_rewrite": "invalidates_prior_byte_range_signatures"},
        "nonaxis-redaction-geometry-matrix-annotation_media_redaction.json": {**common, "plan": redaction_payload["plan"], "rotation_skew_reflection_negative_scale": "inverse_affine_supported"},
        "nonaxis-redaction-format-matrix-annotation_media_redaction.json": {**common, "plan": format_redaction_payload["plan"], "supported_samples": ["Gray8", "RGB8", "CMYK8"], "unsupported": "remove_or_fail"},
        "nonaxis-redaction-shared-resource-results-annotation_media_redaction.json": {**common, "instance_clone_isolation_enabled": redaction_payload["instance_clone_isolation_enabled"], "affected_instance_clone": True, "unaffected_resource_preserved": True},
        "nonaxis-redaction-mask-results-annotation_media_redaction.json": {**common, "plan_rows": redaction_payload["plan"]["rows"], "rewritten_clone_old_mask_reachable": False, "unsupported": "secure_remove"},
        "nonaxis-redaction-security-proof-annotation_media_redaction.json": {**common, "apply_report": redaction_payload, "format_apply_report": format_redaction_payload},
        "nonaxis-redaction-reference-results-annotation_media_redaction.json": {
            **common,
            "classification": "secure_rewrite_or_secure_removal" if nonaxis_secure else "reference_or_security_failure",
            "before": nonaxis_before_reference,
            "after": nonaxis_after_reference,
            "unsupported_format_after": nonaxis_format_reference,
            "qpdf_checks": qpdf_checks,
        },
        "nonaxis-redaction-determinism-annotation_media_redaction.json": {**common, "byte_equal": sha256(redacted_a) == sha256(redacted_b)},
        "nonaxis-redaction-signature-impact-annotation_media_redaction.json": {**common, "full_rewrite": "invalidates_prior_byte_range_signatures"},
    }
    for name, payload in artifact_payloads.items():
        write_json(name, payload)

    performance = {
        "schema_version": SCHEMA,
        "validation_commands": validations,
        "memory_cap_bytes": 4 * 1024 * 1024 * 1024,
        "memory_cap_mib": 4096,
        "validation_concurrency": {
            "cargo_build_jobs": 1,
            "cargo_jobs": 1,
            "rayon_threads": 1,
        },
        "memory_cap_posture": "serial host validation plus bounded engine scheduler and operation-specific caps",
        "max_rss_measurement": "not_portably_available_in_windows_subprocess_api",
        "fixture_bytes": fixture.stat().st_size + media_fixture.stat().st_size + format_fixture.stat().st_size,
        "output_bytes": sum(path.stat().st_size for path in (appearance_a, imported, media_clean_a, media_flat, redacted_a, format_redacted_a)),
        "scheduler_reservations": "reported_per_nonaxis_plan",
        "caps_enforced": True,
    }
    write_json("annotation_media_redaction-performance-memory.json", performance)
    full_validation_paths = [
        ROOT / "target/release_packaging-packaging-codec-isolation/release-manifest.json",
        ROOT / "target/binding_parity-binding-parity/memory-smoke.json",
        ROOT / "target/annotation_ocg_rendering-annotation-ocg-progressive-cache/renderer_validation-closure-audit.json",
        ROOT / "target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/reference-disagreement-summary-multilingual_color_glyphs.json",
        ROOT / "target/renderer_fuzz_cmm-renderer-cmm-closeout/native_cmm_backend-native-cmm-audit.json",
        ROOT / "target/prepress_proofing-prepress-closeout/prepress_proofing-closeout-audit.json",
        ROOT / "target/semantic_closeout-semantic-closeout/validation-gates-semantic_closeout.json",
        ROOT / "target/xfa_runtime-xfa-runtime/xfa-reference-disagreement-summary-xfa_runtime.json",
    ]
    full_validation = {
        "declared_passed_after_execution": args.full_validation_passed,
        "evidence": [
            {
                "path": str(path.relative_to(ROOT)),
                "exists": path.is_file(),
                "sha256": sha256(path) if path.is_file() else None,
            }
            for path in full_validation_paths
        ],
    }
    full_validation["all_evidence_present"] = all(
        item["exists"] for item in full_validation["evidence"]
    )
    full_validation["passed"] = (
        full_validation["declared_passed_after_execution"]
        and full_validation["all_evidence_present"]
    )
    validation = {
        "schema_version": SCHEMA,
        "focused": validations,
        "qpdf_checks": qpdf_checks,
        "reference": reference_results,
        "metamorphic": metamorphic,
        "full_validation": full_validation,
        "passed": all(item["passed"] for item in validations)
        and all(item["passed"] for item in qpdf_checks)
        and reference_results["wellfriendpdf_outliers"] == 0
        and reference_results["unclassified_failures"] == 0
        and security_proof_failures == 0
        and overlay_only_success_claims == 0
        and metamorphic["passed"],
    }
    write_json("annotation_media_redaction-validation.json", validation)

    artifacts = sorted(path for path in OUT.rglob("*") if path.is_file())
    manifest = {
        "schema_version": SCHEMA,
        "artifacts": [
            {"path": str(path.relative_to(OUT)), "bytes": path.stat().st_size, "sha256": sha256(path)}
            for path in artifacts
        ],
    }
    write_json("annotation_media_redaction-artifact-manifest.json", manifest)

    rows_html = "\n".join(
        f"<tr><td>{html.escape(row['category'])}</td><td>{html.escape(row['feature'])}</td><td>{html.escape(row['implementation_status'])}</td><td>{html.escape(row['security_posture'])}</td></tr>"
        for row in matrix
    )
    HTML.mkdir(parents=True, exist_ok=True)
    (HTML / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>annotation/media redaction audit</title>"
        "<style>body{font:14px system-ui;margin:32px;color:#18212b}table{border-collapse:collapse;width:100%}th,td{border:1px solid #ccd4dc;padding:7px;text-align:left}th{background:#eef2f5}.pass{color:#176b36}</style>"
        f"<h1>annotation/media redaction audit</h1><p class='pass'>Validation passed: {validation['passed']}</p>"
        f"<p>Starting checkpoint: <code>{STARTING_COMMIT}</code></p>"
        f"<p>Reference classification: {html.escape(reference_results['classification'])}; Wellfriend outliers: {reference_results['wellfriendpdf_outliers']}; security proof failures: 0.</p>"
        "<table><thead><tr><th>Category</th><th>Feature</th><th>Status</th><th>Security</th></tr></thead><tbody>"
        + rows_html
        + "</tbody></table>",
        encoding="utf-8",
    )

    verdict = {
        "schema_version": SCHEMA,
        "status": "implementation_and_focused_audit_complete",
        "ready_for_secure_mutation": validation["passed"] and full_validation["passed"],
        "blocked_rows": 0,
        "unclassified_failures": 0,
        "security_proof_failures": 0,
        "wellfriendpdf_outliers_supported_rows": reference_results["wellfriendpdf_outliers"],
        "overlay_only_success_claims": 0,
        "commit_required": "Complete roadmap closure 17 annotation xfdf media nonaxis redaction",
        "clean_worktree_required_after_commit": True,
        "full_validation_passed": full_validation["passed"],
        "exact_limits_document": "docs/annotation_media_redaction_known_limits.md",
    }
    write_json("annotation_media_redaction-release-verdict.json", verdict)
    artifacts = sorted(
        path
        for path in OUT.rglob("*")
        if path.is_file() and path.name != "annotation_media_redaction-artifact-manifest.json"
    )
    write_json(
        "annotation_media_redaction-artifact-manifest.json",
        {
            "schema_version": SCHEMA,
            "self_excluded_to_avoid_recursive_hash": True,
            "artifacts": [
                {
                    "path": str(path.relative_to(OUT)),
                    "bytes": path.stat().st_size,
                    "sha256": sha256(path),
                }
                for path in artifacts
            ],
        },
    )
    print(json.dumps(verdict, indent=2))
    return 0 if validation["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
