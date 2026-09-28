#!/usr/bin/env python3
"""Compare same-sized renderer PNGs with reproducible pixel metrics."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image


def load_rgb(path: Path) -> np.ndarray:
    with Image.open(path) as image:
        return np.asarray(image.convert("RGB"), dtype=np.float64)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def compare(left: np.ndarray, right: np.ndarray, threshold: int) -> dict[str, object]:
    if left.shape != right.shape:
        raise ValueError(f"image dimensions differ: {left.shape} != {right.shape}")
    delta = np.abs(left - right)
    mse = float(np.mean(delta**2))
    changed = np.any(delta > threshold, axis=2)
    points = np.argwhere(changed)
    changed_bounds = None
    if points.size:
        y_min, x_min = points.min(axis=0)
        y_max, x_max = points.max(axis=0)
        changed_bounds = {
            "x": int(x_min),
            "y": int(y_min),
            "width": int(x_max - x_min + 1),
            "height": int(y_max - y_min + 1),
        }
    return {
        "identical": mse == 0.0,
        "psnr_db": None if mse == 0.0 else 10.0 * math.log10((255.0**2) / mse),
        "mean_absolute_channel_error": float(np.mean(delta)),
        "changed_pixel_percentage": float(np.mean(changed) * 100.0),
        "changed_pixel_bounds": changed_bounds,
        "maximum_channel_error": float(np.max(delta)),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("images", nargs="+", type=Path)
    parser.add_argument("--threshold", type=int, default=8)
    args = parser.parse_args()
    if len(args.images) < 2:
        parser.error("at least two images are required")
    if not 0 <= args.threshold <= 255:
        parser.error("--threshold must be in 0..=255")

    arrays = {str(path): load_rgb(path) for path in args.images}
    report: dict[str, object] = {
        "schema_version": "wellfriendpdf.render-image-comparison.v1",
        "threshold": args.threshold,
        "images": {
            name: {
                "sha256": sha256(Path(name)),
                "height": int(array.shape[0]),
                "width": int(array.shape[1]),
            }
            for name, array in arrays.items()
        },
        "comparisons": [],
    }
    names = list(arrays)
    comparisons = report["comparisons"]
    assert isinstance(comparisons, list)
    for left_index, left_name in enumerate(names):
        for right_name in names[left_index + 1 :]:
            comparisons.append(
                {
                    "left": left_name,
                    "right": right_name,
                    **compare(arrays[left_name], arrays[right_name], args.threshold),
                }
            )
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
