#!/usr/bin/env python3
"""Fixture-scale renderer visual-difference utility with active visual-reference normalization.

RB-15: Visual normalization for compact renderer reference comparisons.

Normalization pipeline:
  1. Channel order normalization (BGRA, ARGB, RGB, Gray -> canonical RGBA)
  2. Alpha/premultiplication handling (detect and unpremultiply)
  3. Dimensions/rotation metadata normalization (EXIF orientation)
  4. Canonical RGBA surface (unified 8-bit RGBA output)
  5. Expected-difference mask handling (load, validate, apply)
  6. Critical-region mismatch classification (severity tiers)

The utility intentionally accepts exactly two images per invocation. It does
not enumerate corpora, collect elapsed time, or rank renderers. It emits a
structured JSON result suitable for later manual adjudication.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import deque
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

from PIL import Image


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

SCHEMA_VERSION = 2

# Critical-region classification thresholds
CRITICAL_REGION_MIN_PIXELS = 16
CRITICAL_EDGE_WEIGHT_THRESHOLD = 30.0
CRITICAL_CHANGED_RATIO_THRESHOLD = 0.6

# Severity tiers for mismatch regions
class MismatchSeverity(str, Enum):
    """Classification severity for connected mismatch regions."""
    CRITICAL = "critical"
    SIGNIFICANT = "significant"
    MINOR = "minor"
    NEGLIGIBLE = "negligible"


# ---------------------------------------------------------------------------
# Channel order enumeration
# ---------------------------------------------------------------------------

class ChannelOrder(str, Enum):
    """Supported input channel orderings."""
    RGBA = "rgba"
    BGRA = "bgra"
    ARGB = "argb"
    RGB = "rgb"
    BGR = "bgr"
    GRAY = "gray"
    GRAY_ALPHA = "gray_alpha"


# ---------------------------------------------------------------------------
# Normalization metadata
# ---------------------------------------------------------------------------

@dataclass
class NormalizationReport:
    """Records what normalization steps were applied to an input image."""
    source_path: str = ""
    original_mode: str = ""
    original_size: tuple[int, int] = (0, 0)
    channel_order_applied: str = "none"
    unpremultiplied: bool = False
    exif_rotation_applied: int = 0
    final_mode: str = "RGBA"
    final_size: tuple[int, int] = (0, 0)

    def to_dict(self) -> dict:
        return {
            "source_path": self.source_path,
            "original_mode": self.original_mode,
            "original_size": list(self.original_size),
            "channel_order_applied": self.channel_order_applied,
            "unpremultiplied": self.unpremultiplied,
            "exif_rotation_applied": self.exif_rotation_applied,
            "final_mode": self.final_mode,
            "final_size": list(self.final_size),
        }


# ---------------------------------------------------------------------------
# Channel order normalization
# ---------------------------------------------------------------------------

def _reorder_channels(image: Image.Image, order: ChannelOrder) -> Image.Image:
    """Convert an image from a specified channel order to canonical RGBA."""
    if order == ChannelOrder.RGBA:
        return image.convert("RGBA")

    if order == ChannelOrder.BGRA:
        if image.mode != "RGBA":
            image = image.convert("RGBA")
        r, g, b, a = image.split()
        # Input is BGRA stored in RGBA slots: slot0=B, slot1=G, slot2=R, slot3=A
        return Image.merge("RGBA", (b, g, r, a))

    if order == ChannelOrder.ARGB:
        if image.mode != "RGBA":
            image = image.convert("RGBA")
        r, g, b, a = image.split()
        # Input is ARGB stored in RGBA slots: slot0=A, slot1=R, slot2=G, slot3=B
        return Image.merge("RGBA", (g, b, a, r))

    if order == ChannelOrder.RGB:
        if image.mode != "RGB":
            image = image.convert("RGB")
        r, g, b = image.split()
        a = Image.new("L", image.size, 255)
        return Image.merge("RGBA", (r, g, b, a))

    if order == ChannelOrder.BGR:
        if image.mode != "RGB":
            image = image.convert("RGB")
        r, g, b = image.split()
        # Input is BGR stored in RGB slots: slot0=B, slot1=G, slot2=R
        a = Image.new("L", image.size, 255)
        return Image.merge("RGBA", (b, g, r, a))

    if order == ChannelOrder.GRAY:
        gray = image.convert("L")
        r = g = b = gray
        a = Image.new("L", image.size, 255)
        return Image.merge("RGBA", (r, g, b, a))

    if order == ChannelOrder.GRAY_ALPHA:
        if image.mode != "LA":
            image = image.convert("LA")
        gray, a = image.split()
        return Image.merge("RGBA", (gray, gray, gray, a))

    raise ValueError(f"unsupported channel order: {order}")


def _load_raw_buffer(
    path: Path, order: ChannelOrder, width: int, height: int
) -> Image.Image:
    """Load a raw pixel buffer with explicit dimensions and channel order."""
    data = path.read_bytes()
    if order in (ChannelOrder.RGBA, ChannelOrder.BGRA, ChannelOrder.ARGB):
        bpp = 4
    elif order in (ChannelOrder.RGB, ChannelOrder.BGR):
        bpp = 3
    elif order == ChannelOrder.GRAY_ALPHA:
        bpp = 2
    elif order == ChannelOrder.GRAY:
        bpp = 1
    else:
        bpp = 4

    expected = width * height * bpp
    if len(data) != expected:
        raise ValueError(
            f"raw buffer length {len(data)} does not equal expected {expected} "
            f"({width}x{height}x{bpp})"
        )

    if order == ChannelOrder.BGRA:
        return Image.frombytes("RGBA", (width, height), data, "raw", "BGRA")
    elif order == ChannelOrder.ARGB:
        return Image.frombytes("RGBA", (width, height), data, "raw", "ARGB")
    elif order == ChannelOrder.RGBA:
        return Image.frombytes("RGBA", (width, height), data, "raw", "RGBA")
    elif order == ChannelOrder.RGB:
        return Image.frombytes("RGB", (width, height), data, "raw", "RGB")
    elif order == ChannelOrder.BGR:
        return Image.frombytes("RGB", (width, height), data, "raw", "BGR")
    elif order == ChannelOrder.GRAY:
        return Image.frombytes("L", (width, height), data, "raw", "L")
    elif order == ChannelOrder.GRAY_ALPHA:
        return Image.frombytes("LA", (width, height), data, "raw", "LA")
    else:
        raise ValueError(f"unsupported raw channel order: {order}")




# ---------------------------------------------------------------------------
# Alpha / premultiplication normalization
# ---------------------------------------------------------------------------

def _detect_premultiplied(image: Image.Image) -> bool:
    """Heuristic detection of premultiplied alpha.

    Checks a sample of pixels: if any color channel exceeds its alpha value,
    the image is NOT premultiplied. If all sampled color channels are <= alpha,
    and alpha < 255 in some pixels, it is likely premultiplied.
    """
    if image.mode != "RGBA":
        return False

    pixels = image.getdata()
    total = len(pixels)
    step = max(1, total // 1000)  # sample up to ~1000 pixels
    has_partial_alpha = False

    for i in range(0, total, step):
        r, g, b, a = pixels[i]
        if a == 0:
            continue
        if a < 255:
            has_partial_alpha = True
            if r > a or g > a or b > a:
                return False  # Definitely not premultiplied

    return has_partial_alpha


def _unpremultiply_alpha(image: Image.Image) -> Image.Image:
    """Convert premultiplied-alpha RGBA to straight-alpha RGBA."""
    if image.mode != "RGBA":
        image = image.convert("RGBA")

    pixels = list(image.getdata())
    result = []
    for r, g, b, a in pixels:
        if a == 0:
            result.append((0, 0, 0, 0))
        elif a == 255:
            result.append((r, g, b, a))
        else:
            scale = 255.0 / a
            result.append((
                min(255, int(r * scale + 0.5)),
                min(255, int(g * scale + 0.5)),
                min(255, int(b * scale + 0.5)),
                a,
            ))

    out = Image.new("RGBA", image.size)
    out.putdata(result)
    return out


# ---------------------------------------------------------------------------
# EXIF rotation / orientation normalization
# ---------------------------------------------------------------------------

# EXIF orientation tag -> (transpose operation sequence)
_EXIF_ORIENTATION_OPS = {
    2: [Image.Transpose.FLIP_LEFT_RIGHT],
    3: [Image.Transpose.ROTATE_180],
    4: [Image.Transpose.FLIP_TOP_BOTTOM],
    5: [Image.Transpose.FLIP_LEFT_RIGHT, Image.Transpose.ROTATE_90],
    6: [Image.Transpose.ROTATE_270],
    7: [Image.Transpose.FLIP_LEFT_RIGHT, Image.Transpose.ROTATE_270],
    8: [Image.Transpose.ROTATE_90],
}


def _apply_exif_orientation(image: Image.Image) -> tuple[Image.Image, int]:
    """Apply EXIF orientation and return (corrected_image, orientation_value_applied).

    Returns orientation 0 if no EXIF orientation was found or needed.
    """
    try:
        exif = image.getexif()
        if exif is None:
            return image, 0
    except (AttributeError, Exception):
        return image, 0

    orientation = exif.get(0x0112)  # EXIF Orientation tag
    if orientation is None or orientation == 1:
        return image, 0

    ops = _EXIF_ORIENTATION_OPS.get(orientation)
    if ops is None:
        return image, 0

    for op in ops:
        image = image.transpose(op)

    return image, orientation




# ---------------------------------------------------------------------------
# Canonical normalization pipeline
# ---------------------------------------------------------------------------

def normalize_to_canonical_rgba(
    path: Path,
    *,
    channel_order: ChannelOrder = ChannelOrder.RGBA,
    raw_mode: bool = False,
    width: int | None = None,
    height: int | None = None,
    assume_premultiplied: bool = False,
    apply_exif: bool = True,
) -> tuple[Image.Image, NormalizationReport]:
    """Full normalization pipeline: load -> channel reorder -> unpremultiply -> EXIF -> RGBA.

    Returns canonical 8-bit RGBA image and a report of applied steps.
    """
    report = NormalizationReport(source_path=str(path))

    # Step 1: Load
    if raw_mode:
        if width is None or height is None or width <= 0 or height <= 0:
            raise ValueError("raw mode requires positive width and height")
        image = _load_raw_buffer(path, channel_order, width, height)
    else:
        image = Image.open(path)

    report.original_mode = image.mode
    report.original_size = image.size

    # Step 2: EXIF orientation (before channel reorder, on the original load)
    exif_applied = 0
    if apply_exif and not raw_mode:
        image, exif_applied = _apply_exif_orientation(image)
        report.exif_rotation_applied = exif_applied

    # Step 3: Channel order normalization -> RGBA
    if raw_mode:
        # Raw buffers: _load_raw_buffer already decoded the channel order,
        # but we still need to convert to canonical RGBA
        image = _reorder_channels(image, channel_order)
        report.channel_order_applied = channel_order.value
    else:
        if channel_order != ChannelOrder.RGBA:
            image = _reorder_channels(image, channel_order)
            report.channel_order_applied = channel_order.value
        else:
            image = image.convert("RGBA")
            report.channel_order_applied = "auto_rgba"

    # Step 4: Alpha premultiplication handling
    if assume_premultiplied or _detect_premultiplied(image):
        image = _unpremultiply_alpha(image)
        report.unpremultiplied = True

    # Final state
    report.final_mode = image.mode
    report.final_size = image.size
    return image, report




# ---------------------------------------------------------------------------
# Pixel and comparison utilities
# ---------------------------------------------------------------------------

def _rgba_pixels(image: Image.Image) -> list[tuple[int, int, int, int]]:
    """Extract flat list of RGBA pixel tuples."""
    return list(image.getdata())


def _luminance(pixel: tuple[int, int, int, int]) -> float:
    """ITU-R BT.709 luminance from RGBA pixel."""
    r, g, b, _ = pixel
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def _edge_strength(
    pixels: list[tuple[int, int, int, int]], width: int, height: int, index: int
) -> float:
    """4-connected neighbor luminance gradient magnitude."""
    x = index % width
    y = index // width
    center = _luminance(pixels[index])
    total = 0.0
    count = 0
    for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
        if 0 <= nx < width and 0 <= ny < height:
            total += abs(center - _luminance(pixels[ny * width + nx]))
            count += 1
    return total / count if count else 0.0


# ---------------------------------------------------------------------------
# Expected-difference mask
# ---------------------------------------------------------------------------

def load_expected_mask(
    path: Path | None, width: int, height: int
) -> list[bool] | None:
    """Load an expected-difference mask (grayscale image where nonzero = ignored).

    Returns None if no mask path provided. Raises on dimension mismatch.
    """
    if path is None:
        return None
    with Image.open(path) as mask_img:
        mask = mask_img.convert("L")
        if mask.size != (width, height):
            raise ValueError(
                f"expected-difference mask dimensions {mask.size} do not match "
                f"input images ({width}, {height})"
            )
        return [value != 0 for value in mask.getdata()]




# ---------------------------------------------------------------------------
# Connected mismatch regions with severity classification
# ---------------------------------------------------------------------------

def _classify_region(
    region: dict,
    pixels_left: list[tuple[int, int, int, int]],
    pixels_right: list[tuple[int, int, int, int]],
    width: int,
    height: int,
) -> str:
    """Classify a connected mismatch region by severity.

    Severity tiers:
      - critical: large region (>=CRITICAL_REGION_MIN_PIXELS), high edge weight,
                  high fill ratio within bounding box
      - significant: moderate size or moderate edge weight
      - minor: small region with low structural impact
      - negligible: very small (< 4 pixels) or sub-threshold differences
    """
    changed_pixels = region["changed_pixels"]
    bbox_area = region["width"] * region["height"]
    fill_ratio = changed_pixels / bbox_area if bbox_area > 0 else 0.0

    if changed_pixels < 4:
        return MismatchSeverity.NEGLIGIBLE.value

    # Compute mean edge weight in the region
    rx, ry = region["x"], region["y"]
    rw, rh = region["width"], region["height"]
    edge_sum = 0.0
    edge_count = 0
    for dy in range(rh):
        for dx in range(rw):
            idx = (ry + dy) * width + (rx + dx)
            if idx < len(pixels_left):
                edge_sum += _edge_strength(pixels_left, width, height, idx)
                edge_count += 1
    mean_edge = edge_sum / edge_count if edge_count > 0 else 0.0

    if (
        changed_pixels >= CRITICAL_REGION_MIN_PIXELS
        and mean_edge >= CRITICAL_EDGE_WEIGHT_THRESHOLD
        and fill_ratio >= CRITICAL_CHANGED_RATIO_THRESHOLD
    ):
        return MismatchSeverity.CRITICAL.value

    if changed_pixels >= CRITICAL_REGION_MIN_PIXELS or mean_edge >= CRITICAL_EDGE_WEIGHT_THRESHOLD * 0.5:
        return MismatchSeverity.SIGNIFICANT.value

    if changed_pixels >= 4:
        return MismatchSeverity.MINOR.value

    return MismatchSeverity.NEGLIGIBLE.value


def _find_mismatch_regions(
    changed: list[bool], width: int, height: int
) -> list[dict]:
    """Find connected components of changed pixels via BFS flood fill."""
    seen = [False] * len(changed)
    regions: list[dict] = []

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

        regions.append({
            "x": min_x,
            "y": min_y,
            "width": max_x - min_x + 1,
            "height": max_y - min_y + 1,
            "changed_pixels": count,
        })

    return sorted(regions, key=lambda r: (-r["changed_pixels"], r["y"], r["x"]))




# ---------------------------------------------------------------------------
# Core comparison with normalization
# ---------------------------------------------------------------------------

def compare(
    left: Image.Image,
    right: Image.Image,
    tolerance: int,
    ignored: list[bool] | None,
) -> dict:
    """Compare two canonical RGBA images and return structured metrics.

    Both images must already be normalized to RGBA with matching dimensions.
    """
    if left.size != right.size:
        raise ValueError(f"dimension mismatch: {left.size} versus {right.size}")

    width, height = left.size
    left_pixels = _rgba_pixels(left)
    right_pixels = _rgba_pixels(right)

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
        weight = 1.0 + (
            _edge_strength(left_pixels, width, height, index)
            + _edge_strength(right_pixels, width, height, index)
        ) / 255.0
        edge_weighted += weight * max(deltas)
        lum_left.append(_luminance(a))
        lum_right.append(_luminance(b))

    if eligible == 0:
        raise ValueError("expected-difference mask excludes every pixel")

    sample_count = eligible * 4
    mae = channel_abs / sample_count
    mse = channel_sq / sample_count
    rmse = math.sqrt(mse)
    psnr = None if mse == 0 else 20.0 * math.log10(255.0) - 10.0 * math.log10(mse)

    # SSIM (global luminance)
    mean_left = sum(lum_left) / eligible
    mean_right = sum(lum_right) / eligible
    variance_left = sum((v - mean_left) ** 2 for v in lum_left) / eligible
    variance_right = sum((v - mean_right) ** 2 for v in lum_right) / eligible
    covariance = sum(
        (a - mean_left) * (b - mean_right) for a, b in zip(lum_left, lum_right)
    ) / eligible
    c1, c2 = 6.5025, 58.5225
    ssim = ((2 * mean_left * mean_right + c1) * (2 * covariance + c2)) / (
        (mean_left**2 + mean_right**2 + c1) * (variance_left + variance_right + c2)
    )

    # Connected regions with classification
    regions = _find_mismatch_regions(changed, width, height)
    classified_regions = []
    for region in regions:
        severity = _classify_region(region, left_pixels, right_pixels, width, height)
        classified_regions.append({**region, "severity": severity})

    # Summary classification counts
    severity_counts = {s.value: 0 for s in MismatchSeverity}
    for region in classified_regions:
        severity_counts[region["severity"]] += 1

    return {
        "schema_version": SCHEMA_VERSION,
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
        "connected_mismatch_regions": classified_regions,
        "severity_summary": severity_counts,
    }




# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Visual-reference comparison with active normalization (RB-15)."
    )
    parser.add_argument("--left", type=Path, required=True, help="Left/reference image path")
    parser.add_argument("--right", type=Path, required=True, help="Right/candidate image path")

    # Channel order
    parser.add_argument(
        "--left-channel-order", type=str, default="rgba",
        choices=[c.value for c in ChannelOrder],
        help="Channel order of left input (default: rgba)",
    )
    parser.add_argument(
        "--right-channel-order", type=str, default="rgba",
        choices=[c.value for c in ChannelOrder],
        help="Channel order of right input (default: rgba)",
    )

    # Raw mode
    parser.add_argument("--left-raw", action="store_true", help="Left is raw pixel buffer")
    parser.add_argument("--right-raw", action="store_true", help="Right is raw pixel buffer")
    parser.add_argument("--left-width", type=int, help="Width for raw left input")
    parser.add_argument("--left-height", type=int, help="Height for raw left input")
    parser.add_argument("--right-width", type=int, help="Width for raw right input")
    parser.add_argument("--right-height", type=int, help="Height for raw right input")

    # Premultiplication
    parser.add_argument(
        "--left-premultiplied", action="store_true",
        help="Assume left input has premultiplied alpha",
    )
    parser.add_argument(
        "--right-premultiplied", action="store_true",
        help="Assume right input has premultiplied alpha",
    )

    # EXIF
    parser.add_argument(
        "--no-exif", action="store_true",
        help="Skip EXIF orientation normalization",
    )

    # Legacy compat flags (mapped to channel-order)
    parser.add_argument("--left-bgra-raw", action="store_true", help="(Legacy) left is raw BGRA")
    parser.add_argument("--right-bgra-raw", action="store_true", help="(Legacy) right is raw BGRA")

    # Mask and tolerance
    parser.add_argument("--expected-difference-mask", type=Path, help="Grayscale mask (nonzero=ignore)")
    parser.add_argument("--tolerance", type=int, default=0, help="Per-channel tolerance 0..255")

    # Output
    parser.add_argument("--output", type=Path, required=True, help="JSON output path")

    args = parser.parse_args()

    if not 0 <= args.tolerance <= 255:
        parser.error("--tolerance must be in 0..255")

    # Handle legacy BGRA flags
    left_order = ChannelOrder(args.left_channel_order)
    right_order = ChannelOrder(args.right_channel_order)
    left_raw = args.left_raw
    right_raw = args.right_raw

    if args.left_bgra_raw:
        left_order = ChannelOrder.BGRA
        left_raw = True
    if args.right_bgra_raw:
        right_order = ChannelOrder.BGRA
        right_raw = True

    apply_exif = not args.no_exif

    # Normalize left
    left_img, left_report = normalize_to_canonical_rgba(
        args.left,
        channel_order=left_order,
        raw_mode=left_raw,
        width=args.left_width,
        height=args.left_height,
        assume_premultiplied=args.left_premultiplied,
        apply_exif=apply_exif,
    )

    # Normalize right
    right_img, right_report = normalize_to_canonical_rgba(
        args.right,
        channel_order=right_order,
        raw_mode=right_raw,
        width=args.right_width,
        height=args.right_height,
        assume_premultiplied=args.right_premultiplied,
        apply_exif=apply_exif,
    )

    # Load expected-difference mask
    ignored = load_expected_mask(args.expected_difference_mask, *left_img.size)

    # Compare
    result = compare(left_img, right_img, args.tolerance, ignored)

    # Attach metadata
    result["left"] = str(args.left)
    result["right"] = str(args.right)
    result["expected_difference_mask"] = (
        str(args.expected_difference_mask) if args.expected_difference_mask else None
    )
    result["normalization"] = {
        "left": left_report.to_dict(),
        "right": right_report.to_dict(),
    }

    # Write output
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
