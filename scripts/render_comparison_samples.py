#!/usr/bin/env python3
"""Create representative four-renderer comparison sheets from measured rows."""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any

from PIL import Image, ImageDraw, ImageFont

from render_stream_benchmark import command_for, read_ppm_frame


ENGINES = ("wellfriend", "pdfium", "poppler", "mupdf")
LABELS = {
    "wellfriend": "Wellfriend SDK",
    "pdfium": "PDFium",
    "poppler": "Poppler",
    "mupdf": "MuPDF",
}


def font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    path = Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    return ImageFont.truetype(str(path), size) if path.exists() else ImageFont.load_default()


def measured_pages(path: Path) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, int], list[dict[str, Any]]] = defaultdict(list)
    with path.open("r", encoding="utf-8") as stream:
        for line in stream:
            row = json.loads(line)
            if row.get("phase") != "quality-page" or row.get("status") != "pass":
                continue
            ssim = row.get("metrics", {}).get("thumbnail", {}).get("ssim")
            if not isinstance(ssim, (int, float)) or int(row.get("page_count") or 0) > 60:
                continue
            grouped[(str(row["relative_path"]), int(row["page_number"]))].append(row)

    pages: list[dict[str, Any]] = []
    for (relative_path, page_number), rows in grouped.items():
        if {str(row.get("reference")) for row in rows} != {"pdfium", "poppler", "mupdf"}:
            continue
        pages.append(
            {
                "relative_path": relative_path,
                "page_number": page_number,
                "page_count": int(rows[0]["page_count"]),
                "mean_thumbnail_ssim": statistics.fmean(
                    float(row["metrics"]["thumbnail"]["ssim"]) for row in rows
                ),
                "metrics": {
                    str(row["reference"]): row["metrics"]["thumbnail"] for row in rows
                },
            }
        )
    return sorted(pages, key=lambda row: row["mean_thumbnail_ssim"])


def select_pages(pages: list[dict[str, Any]]) -> list[tuple[str, dict[str, Any]]]:
    if not pages:
        return []
    requested = (
        ("worst", 0.0),
        ("lower-tail", 0.05),
        ("median", 0.50),
        ("upper-tail", 0.95),
    )
    selected: list[tuple[str, dict[str, Any]]] = []
    used_documents: set[str] = set()
    for label, quantile in requested:
        origin = round((len(pages) - 1) * quantile)
        candidates = sorted(range(len(pages)), key=lambda index: abs(index - origin))
        index = next(
            (candidate for candidate in candidates if pages[candidate]["relative_path"] not in used_documents),
            candidates[0],
        )
        selected.append((label, pages[index]))
        used_documents.add(pages[index]["relative_path"])
    return selected


def render_page(
    engine: str,
    adapter_dir: Path,
    pdf: Path,
    dpi: int,
    page_number: int,
) -> Image.Image:
    process = subprocess.Popen(
        command_for(engine, adapter_dir, pdf, dpi),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdout is not None
    selected: Image.Image | None = None
    try:
        index = 0
        while True:
            frame = read_ppm_frame(process.stdout)
            if frame is None:
                break
            index += 1
            if index == page_number:
                width, height, pixels = frame
                selected = Image.frombytes("RGB", (width, height), pixels)
        _, stderr = process.communicate(timeout=1800)
    except Exception:
        process.kill()
        process.communicate()
        raise
    if process.returncode != 0 or selected is None:
        message = stderr.decode("utf-8", "replace")[-800:]
        raise RuntimeError(f"{engine} failed to provide page {page_number}: {message}")
    return selected


def compose(
    selection_label: str,
    page: dict[str, Any],
    frames: dict[str, Image.Image],
    output: Path,
) -> None:
    tile_width, tile_height = 720, 930
    header_height, footer_height = 118, 82
    canvas = Image.new("RGB", (tile_width * 2, header_height + tile_height * 2 + footer_height), "#eef1f4")
    draw = ImageDraw.Draw(canvas)
    draw.rectangle((0, 0, canvas.width, header_height), fill="#17212b")
    draw.text((24, 16), f"{selection_label}: {page['relative_path']}", font=font(25), fill="white")
    draw.text(
        (24, 57),
        f"page {page['page_number']}/{page['page_count']} | mean thumbnail SSIM {page['mean_thumbnail_ssim']:.6f} | 72 DPI RGB",
        font=font(20),
        fill="#d8e4ef",
    )

    for index, engine in enumerate(ENGINES):
        column, row = index % 2, index // 2
        left, top = column * tile_width, header_height + row * tile_height
        draw.rectangle((left + 8, top + 8, left + tile_width - 8, top + tile_height - 8), fill="white", outline="#93a4b4", width=2)
        draw.text((left + 24, top + 18), LABELS[engine], font=font(24), fill="#17212b")
        image = frames[engine].copy()
        image.thumbnail((tile_width - 48, tile_height - 88), Image.Resampling.LANCZOS)
        image_left = left + (tile_width - image.width) // 2
        image_top = top + 58 + (tile_height - 76 - image.height) // 2
        canvas.paste(image, (image_left, image_top))

    metrics = page["metrics"]
    footer_top = header_height + tile_height * 2
    draw.rectangle((0, footer_top, canvas.width, canvas.height), fill="#17212b")
    text = "  |  ".join(
        f"WF vs {LABELS[reference]}: SSIM {metrics[reference]['ssim']:.6f}, PSNR {metrics[reference]['psnr_db']:.2f} dB, MAE {metrics[reference]['mae']:.3f}"
        for reference in ("pdfium", "poppler", "mupdf")
    )
    draw.text((18, footer_top + 25), text, font=font(16), fill="white")
    output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(output, format="WEBP", quality=92, method=6)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--quality-jsonl", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dpi", type=int, default=72)
    args = parser.parse_args()

    selected = select_pages(measured_pages(args.quality_jsonl))
    evidence: list[dict[str, Any]] = []
    for index, (label, page) in enumerate(selected, start=1):
        pdf = args.corpus / page["relative_path"]
        frames = {
            engine: render_page(engine, args.adapter_dir, pdf, args.dpi, page["page_number"])
            for engine in ENGINES
        }
        filename = f"{index:02d}-{label}.webp"
        compose(label, page, frames, args.output / filename)
        evidence.append({"selection": label, "image": filename, **page})
        print(f"sample {index}/{len(selected)} {label} {page['relative_path']} page={page['page_number']}", flush=True)
    (args.output / "visual-samples.json").write_text(
        json.dumps({"contract": "distribution-selected successful paired pages; four engines; 72 DPI RGB", "samples": evidence}, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
