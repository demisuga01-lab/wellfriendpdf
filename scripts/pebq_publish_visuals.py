#!/usr/bin/env python3
"""Create readable PEBQ renderer comparison and difference sheets."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFont, ImageOps


ENGINES = ("wellfriendpdf", "mupdf", "pdfium", "poppler")
LABELS = {
    "wellfriendpdf": "Wellfriend PDF",
    "mupdf": "MuPDF",
    "pdfium": "PDFium",
    "poppler": "Poppler",
}
PANEL_WIDTH = 500
RENDER_HEIGHT = 650
DIFF_HEIGHT = 280
HEADER_HEIGHT = 72
GAP = 12


def safe_name(value: str) -> str:
    cleaned = "".join(character.lower() if character.isalnum() else "-" for character in value)
    return "-".join(part for part in cleaned.split("-") if part)[:90] or "pdf"


def fit(image: Image.Image, width: int, height: int, background: str = "white") -> Image.Image:
    panel = Image.new("RGB", (width, height), background)
    copy = ImageOps.contain(image.convert("RGB"), (width, height), Image.Resampling.LANCZOS)
    panel.paste(copy, ((width - copy.width) // 2, (height - copy.height) // 2))
    return panel


def amplified_difference(candidate: Image.Image, consensus: Image.Image) -> Image.Image:
    if candidate.size != consensus.size:
        panel = Image.new("RGB", candidate.size, (75, 0, 0))
        ImageDraw.Draw(panel).text((20, 20), "Dimension mismatch — no alignment applied", fill="white")
        return panel
    difference = ImageChops.difference(candidate.convert("RGB"), consensus.convert("RGB"))
    array = np.asarray(difference, dtype=np.uint16)
    magnitude = np.max(array, axis=2)
    heat = np.zeros((*magnitude.shape, 3), dtype=np.uint8)
    heat[..., 0] = np.clip(magnitude * 6, 0, 255).astype(np.uint8)
    heat[..., 1] = np.clip((magnitude - 16) * 4, 0, 255).astype(np.uint8)
    heat[..., 2] = np.clip((magnitude - 48) * 3, 0, 255).astype(np.uint8)
    return Image.fromarray(heat, "RGB")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    quality_rows = {}
    quality_path = args.results / "quality-pages.jsonl"
    if quality_path.exists():
        for line in quality_path.read_text(encoding="utf-8").splitlines():
            row = json.loads(line)
            quality_rows[row["relative_path"]] = row
    manifest = json.loads((args.results / "corpus-manifest.json").read_text(encoding="utf-8"))
    pages_dir = args.output / "pages"
    contacts_dir = args.output / "contacts"
    pages_dir.mkdir(parents=True, exist_ok=True)
    contacts_dir.mkdir(parents=True, exist_ok=True)
    font = ImageFont.load_default()
    sheet_paths: list[Path] = []

    for index, item in enumerate(manifest, start=1):
        relative = item["relative_path"]
        images: dict[str, Image.Image] = {}
        for engine in ENGINES:
            source = args.results / "rasters" / engine / (relative.replace("/", "__") + ".ppm")
            with Image.open(source) as image:
                images[engine] = image.convert("RGB").copy()

        sheet_width = len(ENGINES) * PANEL_WIDTH + (len(ENGINES) - 1) * GAP
        sheet_height = HEADER_HEIGHT + RENDER_HEIGHT + 42 + DIFF_HEIGHT
        sheet = Image.new("RGB", (sheet_width, sheet_height), "white")
        draw = ImageDraw.Draw(sheet)
        draw.text((12, 10), f"PEBQ {index:03d}/100 — {relative}", fill="black", font=font)
        draw.text(
            (12, 34),
            "Top: native 144-DPI raw RGB. Bottom: amplified absolute difference from leave-one-out consensus.",
            fill=(55, 55, 55),
            font=font,
        )
        metrics = quality_rows.get(relative, {}).get("engines", {})
        for column, engine in enumerate(ENGINES):
            left = column * (PANEL_WIDTH + GAP)
            image = images[engine]
            metric = metrics.get(engine)
            label = f"{LABELS[engine]} — {image.width}×{image.height}"
            if metric:
                label += f"  SSIM {metric['ssim']:.4f}  FLIP {metric['mean_flip']:.4f}"
            draw.text((left + 8, HEADER_HEIGHT - 22), label, fill="black", font=font)
            sheet.paste(fit(image, PANEL_WIDTH, RENDER_HEIGHT), (left, HEADER_HEIGHT))

            references = [np.asarray(other, dtype=np.uint8) for name, other in images.items() if name != engine]
            if len({other.size for other in images.values()}) == 1:
                consensus = Image.fromarray(np.median(np.stack(references), axis=0).astype(np.uint8), "RGB")
            else:
                consensus = Image.new("RGB", image.size, "white")
            difference = amplified_difference(image, consensus)
            sheet.paste(
                fit(difference, PANEL_WIDTH, DIFF_HEIGHT, "black"),
                (left, HEADER_HEIGHT + RENDER_HEIGHT + 42),
            )
            difference.close()
            consensus.close()

        target = pages_dir / f"{index:03d}-{safe_name(relative)}.webp"
        sheet.save(target, format="WEBP", quality=88, method=6)
        sheet_paths.append(target)
        sheet.close()
        for image in images.values():
            image.close()
        if index % 10 == 0:
            print(f"visual sheets: {index}/{len(manifest)}", flush=True)

    for start in range(0, len(sheet_paths), 25):
        group = sheet_paths[start : start + 25]
        thumb_width, thumb_height = 360, 190
        contact = Image.new("RGB", (thumb_width * 5, thumb_height * 5), "white")
        for offset, path in enumerate(group):
            with Image.open(path) as sheet:
                thumb = ImageOps.contain(sheet.convert("RGB"), (thumb_width, thumb_height), Image.Resampling.LANCZOS)
            contact.paste(thumb, ((offset % 5) * thumb_width, (offset // 5) * thumb_height))
        end = start + len(group)
        contact.save(
            contacts_dir / f"contact-{start + 1:03d}-{end:03d}.webp",
            format="WEBP",
            quality=84,
            method=6,
        )
        contact.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
