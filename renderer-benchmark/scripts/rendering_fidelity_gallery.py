#!/usr/bin/env python3
"""Create side-by-side visual galleries from renderer benchmark output.

The 0A renderer benchmark stores Poppler PNGs and Oxide's render ZIP per file.
This helper extracts the worst failed pages and writes contact sheets:

    Oxide | Poppler | amplified diff

It is intentionally a reporting helper only; it does not run either renderer.
"""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path
from typing import Any

try:
    from PIL import Image, ImageChops, ImageDraw
except ImportError as exc:  # pragma: no cover - environment/setup failure
    raise SystemExit("Pillow is required: python -m pip install pillow") from exc


def parse_page_num(name: str, fallback: int) -> int:
    match = re.search(r"(\d+)(?=\.[^.]+$)", name)
    return int(match.group(1)) if match else fallback


def page_rank(page: dict[str, Any]) -> tuple[float, float, float, float]:
    reasons = set(page.get("reasons") or [page.get("reason")])
    severity = 0.0
    if "rendered_page_missing" in reasons:
        severity += 1000.0
    if "blank_page_mismatch" in reasons:
        severity += 800.0
    if "large_region_difference" in reasons:
        severity += 500.0
    if "edge_or_text_shift" in reasons:
        severity += 250.0
    if "pixel_difference" in reasons:
        severity += 100.0
    return (
        severity,
        float(page.get("mae") or 0.0),
        float(page.get("large_region_score") or 0.0),
        float(page.get("different_pixel_percent") or 0.0),
    )


def load_failed_pages(results_dir: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for path in sorted((results_dir / "files").glob("*.json")):
        result = json.loads(path.read_text(encoding="utf-8"))
        for page in result.get("visual_compare", {}).get("failed_pages", []):
            if page.get("reason") == "rendered_page_missing":
                continue
            row = dict(page)
            row["id"] = result["id"]
            row["file"] = result.get("file")
            row["category"] = result.get("category")
            rows.append(row)
    rows.sort(key=page_rank, reverse=True)
    return rows


def oxide_page_image(artifact_dir: Path, page_number: int) -> Image.Image | None:
    zip_path = artifact_dir / "oxide.zip"
    if not zip_path.exists():
        return None
    with zipfile.ZipFile(zip_path) as archive:
        names = sorted(name for name in archive.namelist() if name.lower().endswith(".png"))
        by_page = {parse_page_num(name, idx): name for idx, name in enumerate(names, start=1)}
        name = by_page.get(page_number)
        if name is None:
            return None
        with archive.open(name) as handle:
            return Image.open(handle).convert("RGB")


def poppler_page_image(artifact_dir: Path, page_number: int) -> Image.Image | None:
    paths = sorted(artifact_dir.glob("poppler_page-*.png"))
    by_page = {parse_page_num(path.name, idx): path for idx, path in enumerate(paths, start=1)}
    path = by_page.get(page_number)
    if path is None:
        return None
    return Image.open(path).convert("RGB")


def write_sheet(
    oxide: Image.Image,
    poppler: Image.Image,
    out_path: Path,
    label: str,
    diff_scale: int,
    max_width: int,
) -> None:
    width = min(oxide.width, poppler.width)
    height = min(oxide.height, poppler.height)
    oxide = oxide.crop((0, 0, width, height))
    poppler = poppler.crop((0, 0, width, height))
    diff = ImageChops.difference(oxide, poppler).point(lambda value: min(255, value * diff_scale))

    thumb_h = max(1, int(height * max_width / max(1, width)))
    label_h = 32
    sheet = Image.new("RGB", (max_width * 3, thumb_h + label_h), "white")
    draw = ImageDraw.Draw(sheet)
    for idx, (image, title) in enumerate(
        [(oxide, "Oxide"), (poppler, "Poppler"), (diff, f"Diff x{diff_scale}")]
    ):
        x = idx * max_width
        sheet.paste(image.resize((max_width, thumb_h)), (x, label_h))
        draw.text((x + 5, 8), f"{title} - {label}", fill=(0, 0, 0))
    out_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out_path)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results-dir", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--limit", type=int, default=12)
    parser.add_argument("--diff-scale", type=int, default=4)
    parser.add_argument("--max-width", type=int, default=420)
    args = parser.parse_args()

    rows = load_failed_pages(args.results_dir)[: args.limit]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    index_lines = [
        "# Rendering Fidelity Gallery",
        "",
        f"Source results: `{args.results_dir}`",
        "",
        "| rank | id | page | category | reason | sheet |",
        "| ---: | --- | ---: | --- | --- | --- |",
    ]
    for rank, row in enumerate(rows, start=1):
        page_number = int(row.get("page") or 1)
        artifact_dir = args.results_dir / "artifacts" / row["id"]
        oxide = oxide_page_image(artifact_dir, page_number)
        poppler = poppler_page_image(artifact_dir, page_number)
        if oxide is None or poppler is None:
            continue
        safe_reason = re.sub(r"[^A-Za-z0-9_.-]+", "_", str(row.get("reason") or "diff"))
        out_name = f"{rank:02d}_{row['id']}_p{page_number}_{safe_reason}.png"
        out_path = args.output_dir / out_name
        label = f"{row['id']} p{page_number}"
        write_sheet(oxide, poppler, out_path, label, args.diff_scale, args.max_width)
        index_lines.append(
            f"| {rank} | `{row['id']}` | {page_number} | {row.get('category')} | "
            f"{row.get('reason')} | [{out_name}]({out_name}) |"
        )
    (args.output_dir / "index.md").write_text("\n".join(index_lines) + "\n", encoding="utf-8")
    print(f"Wrote {args.output_dir / 'index.md'}")


if __name__ == "__main__":
    main()
