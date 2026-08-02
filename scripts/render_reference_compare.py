#!/usr/bin/env python3
"""Compare Wellfriend raster output with local reference renderers.

The script intentionally stores aggregate metrics and path hashes only. It does
not retain rendered page images or source PDF content in the report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import warnings
import zipfile
from pathlib import Path

from PIL import Image, ImageChops

warnings.filterwarnings("ignore", category=DeprecationWarning)

REFERENCE_RENDER_TIMEOUT_SEC = 120


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8", "replace")).hexdigest()


def collect_pdfs(root: Path, limit: int | None) -> list[Path]:
    files = sorted(path for path in root.rglob("*.pdf") if path.is_file())
    return files[:limit] if limit else files


def load_wellfriend_png(
    wellfriend_bin: Path,
    pdf: Path,
    dpi: int,
    tmp: Path,
    render_quality: str,
    timeout_sec: int,
) -> Image.Image:
    out_zip = tmp / "wellfriend.zip"
    subprocess.run(
        [
            str(wellfriend_bin),
            "render",
            str(pdf),
            "--output",
            str(out_zip),
            "--pages",
            "1",
            "--dpi",
            str(dpi),
            "--format",
            "png",
            "--render-quality",
            render_quality,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec,
    )
    with zipfile.ZipFile(out_zip) as zf:
        names = [name for name in zf.namelist() if name.lower().endswith(".png")]
        if not names:
            raise RuntimeError("wellfriend render zip contained no PNG")
        with zf.open(names[0]) as handle:
            with Image.open(handle) as image:
                return image.convert("RGBA")


def render_pdfium_png_to_path(pdf: Path, dpi: int, output: Path) -> None:
    import pypdfium2 as pdfium

    doc = pdfium.PdfDocument(str(pdf))
    try:
        page = doc[0]
        try:
            bitmap = page.render(scale=dpi / 72.0)
            try:
                image = bitmap.to_pil().convert("RGBA")
                try:
                    image.save(output)
                finally:
                    image.close()
            finally:
                close = getattr(bitmap, "close", None)
                if close is not None:
                    close()
        finally:
            close = getattr(page, "close", None)
            if close is not None:
                close()
    finally:
        close = getattr(doc, "close", None)
        if close is not None:
            close()


def render_mupdf_png_to_path(pdf: Path, dpi: int, output: Path) -> None:
    import fitz

    with fitz.open(str(pdf)) as doc:
        page = doc[0]
        pix = page.get_pixmap(matrix=fitz.Matrix(dpi / 72.0, dpi / 72.0), alpha=False)
        image = Image.frombytes("RGB", [pix.width, pix.height], pix.samples).convert("RGBA")
        try:
            image.save(output)
        finally:
            image.close()


def load_reference_png(kind: str, pdf: Path, dpi: int, tmp: Path, timeout_sec: int) -> Image.Image:
    out_png = tmp / f"{kind}.png"
    subprocess.run(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "--reference-helper",
            kind,
            "--pdf",
            str(pdf),
            "--dpi",
            str(dpi),
            "--png-output",
            str(out_png),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec,
    )
    with Image.open(out_png) as image:
        return image.convert("RGBA")


def load_poppler_png(pdf: Path, dpi: int, tmp: Path) -> Image.Image:
    prefix = tmp / "poppler"
    subprocess.run(
        ["pdftoppm", "-f", "1", "-l", "1", "-r", str(dpi), "-png", str(pdf), str(prefix)],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=120,
    )
    images = sorted(tmp.glob("poppler-*.png"))
    if not images:
        raise RuntimeError("pdftoppm produced no PNG")
    with Image.open(images[0]) as image:
        return image.convert("RGBA")


def diff_metric(a: Image.Image, b: Image.Image) -> dict[str, object]:
    if a.size != b.size:
        return {"same_size": False, "size_a": list(a.size), "size_b": list(b.size)}
    with ImageChops.difference(a, b) as diff:
        pixels = diff.getdata()
        total = a.size[0] * a.size[1]
        changed = 0
        max_delta = 0
        for pixel in pixels:
            local = max(pixel)
            if local > 8:
                changed += 1
            if local > max_delta:
                max_delta = local
    return {
        "same_size": True,
        "changed_pixel_threshold8_percentage": round((changed / total) * 100.0, 6)
        if total
        else 0.0,
        "max_channel_delta": max_delta,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference-helper", choices=["pdfium", "mupdf"])
    parser.add_argument("--pdf", type=Path)
    parser.add_argument("--png-output", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--wellfriend-bin", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--limit", type=int, default=25)
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--wellfriend-render-quality", default="compat")
    parser.add_argument("--wellfriend-timeout-sec", type=int, default=120)
    parser.add_argument("--reference-timeout-sec", type=int, default=REFERENCE_RENDER_TIMEOUT_SEC)
    args = parser.parse_args()

    if args.reference_helper:
        if args.pdf is None or args.png_output is None:
            return 2
        if args.reference_helper == "pdfium":
            render_pdfium_png_to_path(args.pdf, args.dpi, args.png_output)
        else:
            render_mupdf_png_to_path(args.pdf, args.dpi, args.png_output)
        return 0

    if args.corpus is None or args.wellfriend_bin is None or args.output is None:
        return 2

    tools = {
        "pdfium": "available",
        "mupdf": "available",
        "poppler": "available" if shutil.which("pdftoppm") else "unavailable",
    }
    pages = []
    failures = 0
    reference_failures = 0
    start = time.time()
    pdfs = collect_pdfs(args.corpus, args.limit)
    progress_path = args.output.with_suffix(args.output.suffix + ".progress.json")
    for index, pdf in enumerate(pdfs, start=1):
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            wellfriend: Image.Image | None = None
            refs: dict[str, Image.Image] = {}
            try:
                wellfriend = load_wellfriend_png(
                    args.wellfriend_bin,
                    pdf,
                    args.dpi,
                    tmp,
                    args.wellfriend_render_quality,
                    args.wellfriend_timeout_sec,
                )
                reference_errors = {}
                for ref_name in ("pdfium", "mupdf"):
                    try:
                        refs[ref_name] = load_reference_png(
                            ref_name, pdf, args.dpi, tmp, args.reference_timeout_sec
                        )
                    except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                        reference_errors[ref_name] = exc.__class__.__name__
                        reference_failures += 1
                if tools["poppler"] == "available":
                    try:
                        refs["poppler"] = load_poppler_png(pdf, args.dpi, tmp)
                    except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                        reference_errors["poppler"] = exc.__class__.__name__
                        reference_failures += 1
                comparisons = {name: diff_metric(wellfriend, image) for name, image in refs.items()}
                ok = True
            except Exception as exc:  # noqa: BLE001 - report exact class without content.
                comparisons = {}
                reference_errors = {}
                ok = False
                failures += 1
                error = exc.__class__.__name__
            else:
                error = None
            finally:
                for image in refs.values():
                    image.close()
                if wellfriend is not None:
                    wellfriend.close()
            pages.append(
                {
                    "path_sha256": sha256_text(str(pdf)),
                    "ok": ok,
                    "error_class": error,
                    "reference_errors": reference_errors,
                    "comparisons": comparisons,
                }
            )
            if index == len(pdfs) or index % 100 == 0:
                progress_path.write_text(
                    json.dumps(
                        {
                            "schema_version": "wellfriend.reference_render_compare.progress.v1",
                            "attempted": index,
                            "total": len(pdfs),
                            "failures": failures,
                            "reference_failures": reference_failures,
                            "elapsed_sec": round(time.time() - start, 3),
                        },
                        indent=2,
                    ),
                    encoding="utf-8",
                )

    report = {
        "schema_version": "wellfriend.reference_render_compare.v1",
        "dpi": args.dpi,
        "limit": args.limit,
        "wellfriend_render_quality": args.wellfriend_render_quality,
        "files_attempted": len(pages),
        "failures": failures,
        "reference_failures": reference_failures,
        "duration_sec": round(time.time() - start, 3),
        "tools": tools,
        "pages": pages,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
