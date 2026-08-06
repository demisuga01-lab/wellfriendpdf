#!/usr/bin/env python3
"""Fixture-scale renderer visual-difference utility.

The utility intentionally accepts exactly two images per invocation. It does
not enumerate corpora, collect elapsed time, or rank renderers. It emits a
structured JSON result suitable for later manual adjudication.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import deque
from pathlib import Path
from typing import Iterable

from PIL import Image


def load_rgba(path: Path, raw_bgra: bool, width: int | None, height: int | None) -> Image.Image:
    if raw_bgra:
        if width is None or height is None or width <= 0 or height <= 0:
            raise ValueError("raw BGRA input requires positive --width and --height")
        data = path.read_bytes()
        expected = width * height * 4
        if len(data) != expected:
            raise ValueError(f"raw BGRA length {len(data)} does not equal {expected}")
        return Image.frombytes("RGBA", (width, height), data, "raw", "BGRA")
    with Image.open(path) as image:
        return image.convert("RGBA")


def rgba_pixels(image: Image.Image) -> list[tuple[int, int, int, int]]:
    return list(image.getdata())


def luminance(pixel: tuple[int, int, int, int]) -> float:
    r, g, b, _ = pixel
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def edge_strength(pixels: list[tuple[int, int, int, int]], width: int, height: int, index: int) -> float:
    x = index % width
    y = index // width
    center = luminance(pixels[index])
    total = 0.0
    count = 0
    for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
        if 0 <= nx < width and 0 <= ny < height:
            total += abs(center - luminance(pixels[ny * width + nx]))
            count += 1
    return total / count if count else 0.0


def mismatch_regions(changed: list[bool], width: int, height: int) -> list[dict[str, int]]:
    seen = [False] * len(changed)
    regions: list[dict[str, int]] = []
    for start, is_changed in enumerate(changed):
        if not is_changed or seen[start]:
            continue
        queue: deque[int] = deque([start])
        seen[start] = True
        min_x = max_x = start % width
        min_y = max_y = start // width
        count = 0
        while queue:
            index = queue.popleft()
            x, y = index % width, index // width
            count += 1
            min_x, max_x = min(min_x, x), max(max_x, x)
            min_y, max_y = min(min_y, y), max(max_y, y)
            for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                if not (0 <= nx < width and 0 <= ny < height):
                    continue
                neighbor = ny * width + nx
                if changed[neighbor] and not seen[neighbor]:
                    seen[neighbor] = True
                    queue.append(neighbor)
        regions.append(
            {
                "x": min_x,
                "y": min_y,
                "width": max_x - min_x + 1,
                "height": max_y - min_y + 1,
                "changed_pixels": count,
            }
        )
    return sorted(regions, key=lambda region: (-region["changed_pixels"], region["y"], region["x"]))


def expected_mask(path: Path | None, width: int, height: int) -> list[bool] | None:
    if path is None:
        return None
    with Image.open(path) as image:
        mask = image.convert("L")
        if mask.size != (width, height):
            raise ValueError("expected-difference mask dimensions do not match input images")
        return [value != 0 for value in mask.getdata()]


def compare(
    left: Image.Image,
    right: Image.Image,
    tolerance: int,
    ignored: list[bool] | None,
) -> dict[str, object]:
    if left.size != right.size:
        raise ValueError(f"dimension mismatch: {left.size} versus {right.size}")
    width, height = left.size
    left_pixels = rgba_pixels(left)
    right_pixels = rgba_pixels(right)
    changed: list[bool] = []
    count = 0
    channel_abs = 0
    channel_sq = 0
    alpha_abs = 0
    edge_weighted = 0.0
    eligible = 0
    lum_left: list[float] = []
    lum_right: list[float] = []

    for index, (a, b) in enumerate(zip(left_pixels, right_pixels)):
        if ignored is not None and ignored[index]:
            changed.append(False)
            continue
        eligible += 1
        deltas = [abs(int(x) - int(y)) for x, y in zip(a, b)]
        is_changed = max(deltas) > tolerance
        changed.append(is_changed)
        if is_changed:
            count += 1
        channel_abs += sum(deltas)
        channel_sq += sum(delta * delta for delta in deltas)
        alpha_abs += deltas[3]
        weight = 1.0 + (edge_strength(left_pixels, width, height, index) + edge_strength(right_pixels, width, height, index)) / 255.0
        edge_weighted += weight * max(deltas)
        lum_left.append(luminance(a))
        lum_right.append(luminance(b))

    if eligible == 0:
        raise ValueError("expected-difference mask excludes every pixel")
    sample_count = eligible * 4
    mae = channel_abs / sample_count
    mse = channel_sq / sample_count
    rmse = math.sqrt(mse)
    psnr = None if mse == 0 else 20.0 * math.log10(255.0) - 10.0 * math.log10(mse)
    mean_left = sum(lum_left) / eligible
    mean_right = sum(lum_right) / eligible
    variance_left = sum((value - mean_left) ** 2 for value in lum_left) / eligible
    variance_right = sum((value - mean_right) ** 2 for value in lum_right) / eligible
    covariance = sum((a - mean_left) * (b - mean_right) for a, b in zip(lum_left, lum_right)) / eligible
    c1, c2 = 6.5025, 58.5225
    ssim = ((2 * mean_left * mean_right + c1) * (2 * covariance + c2)) / (
        (mean_left * mean_left + mean_right * mean_right + c1) * (variance_left + variance_right + c2)
    )

    return {
        "schema_version": 1,
        "width": width,
        "height": height,
        "eligible_pixels": eligible,
        "tolerance": tolerance,
        "changed_pixel_count": count,
        "changed_pixel_percent": count * 100.0 / eligible,
        "mae": mae,
        "rmse": rmse,
        "psnr": "infinity" if psnr is None else psnr,
        "ssim_global_luminance": ssim,
        "alpha_mae": alpha_abs / eligible,
        "edge_weighted_difference": edge_weighted / eligible,
        "connected_mismatch_regions": mismatch_regions(changed, width, height),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left", type=Path, required=True)
    parser.add_argument("--right", type=Path, required=True)
    parser.add_argument("--left-bgra-raw", action="store_true")
    parser.add_argument("--right-bgra-raw", action="store_true")
    parser.add_argument("--left-width", type=int)
    parser.add_argument("--left-height", type=int)
    parser.add_argument("--right-width", type=int)
    parser.add_argument("--right-height", type=int)
    parser.add_argument("--expected-difference-mask", type=Path)
    parser.add_argument("--tolerance", type=int, default=0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 0 <= args.tolerance <= 255:
        parser.error("--tolerance must be in 0..255")

    left = load_rgba(args.left, args.left_bgra_raw, args.left_width, args.left_height)
    right = load_rgba(args.right, args.right_bgra_raw, args.right_width, args.right_height)
    ignored = expected_mask(args.expected_difference_mask, *left.size)
    result = compare(left, right, args.tolerance, ignored)
    result["left"] = str(args.left)
    result["right"] = str(args.right)
    result["expected_difference_mask"] = (
        str(args.expected_difference_mask) if args.expected_difference_mask else None
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
