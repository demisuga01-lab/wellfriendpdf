#!/usr/bin/env python3
"""Fixture-scale renderer visual-difference utility with active visual-reference normalization.

RB-15: Visual normalization for compact renderer reference comparisons.

Normalization pipeline:
  1. Channel order normalization (BGRA, ARGB, RGB, Gray -> canonical RGBA)
  2. Alpha semantics handling (straight, premultiplied, opaque)
  3. Dimensions/stride/rotation metadata normalization (raw stride, EXIF, render context)
  4. Render-context background/grayscale normalization and exact dimension checks
  5. Canonical RGBA surface (unified 8-bit RGBA output)
  6. Render-context normalization policy capture
  7. Expected-difference mask handling (load, validate, apply)
  8. Critical-region mismatch classification (severity tiers and likely causes)

The utility intentionally accepts exactly two images per invocation. It does
not enumerate corpora, collect elapsed time, or rank renderers. It emits a
structured JSON result suitable for later manual adjudication.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import deque
from dataclasses import dataclass, replace
from enum import Enum
from pathlib import Path

from PIL import Image


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

SCHEMA_VERSION = 3

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


class DifferenceClass(str, Enum):
    """Renderer-difference taxonomy used for later adjudication."""
    ANTIALIASING = "antialiasing"
    HINTING = "hinting"
    FONT_FALLBACK = "font_fallback"
    COLOR_PROFILE = "color_profile"
    MALFORMED_INPUT_POLICY = "malformed_input_policy"
    ANNOTATION_POLICY = "annotation_policy"
    FORM_POLICY = "form_policy"
    OPTIONAL_CONTENT_POLICY = "optional_content_policy"
    TRANSPARENCY_ROUNDING = "transparency_rounding"
    CHANNEL_LAYOUT = "channel_layout"
    DIMENSION_OR_TRANSFORM = "dimension_or_transform"
    BACKGROUND_POLICY = "background_policy"
    UNCLASSIFIED_PIXEL_DELTA = "unclassified_pixel_delta"


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


class AlphaSemantics(str, Enum):
    """Supported input alpha representations."""
    STRAIGHT = "straight"
    PREMULTIPLIED = "premultiplied"
    OPAQUE = "opaque"


class ByteOrder(str, Enum):
    """Supported packed raw-buffer byte-order interpretations."""
    NATIVE = "native"
    LITTLE = "little"
    BIG = "big"


# ---------------------------------------------------------------------------
# Render-context normalization helpers
# ---------------------------------------------------------------------------

def _optional_positive_int(value, field: str) -> int | None:
    if value is None:
        return None
    try:
        number = int(value)
    except (TypeError, ValueError) as error:
        raise ValueError(f"{field} must be an integer") from error
    if number <= 0:
        raise ValueError(f"{field} must be positive")
    return number


def _optional_float_list(value, length: int, field: str) -> list[float] | None:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) != length:
        raise ValueError(f"{field} must be a list of {length} numbers")
    try:
        return [float(item) for item in value]
    except (TypeError, ValueError) as error:
        raise ValueError(f"{field} must contain only numbers") from error


def _optional_rgba(value, field: str) -> list[int] | None:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) != 4:
        raise ValueError(f"{field} must be a list of 4 channel values")
    try:
        channels = [int(item) for item in value]
    except (TypeError, ValueError) as error:
        raise ValueError(f"{field} must contain integer channel values") from error
    if any(channel < 0 or channel > 255 for channel in channels):
        raise ValueError(f"{field} channel values must be in 0..255")
    return channels


def _optional_channel_order(value) -> str | None:
    if value is None:
        return None
    order = str(value).strip().lower()
    ChannelOrder(order)
    return order


def _normalize_alpha_semantics(value) -> str:
    if value is None:
        return AlphaSemantics.STRAIGHT.value
    semantics = str(value).strip().lower().replace("_", "-")
    if semantics in ("straight", "straight-alpha"):
        return AlphaSemantics.STRAIGHT.value
    if semantics in ("premultiplied", "premul", "premultiplied-alpha"):
        return AlphaSemantics.PREMULTIPLIED.value
    if semantics in ("opaque", "opaque-alpha"):
        return AlphaSemantics.OPAQUE.value
    allowed = ", ".join(alpha.value for alpha in AlphaSemantics)
    raise ValueError(f"alpha_semantics must be one of: {allowed}")


def _normalize_byte_order(value, *, strict: bool = False) -> str:
    if value is None:
        return ByteOrder.NATIVE.value
    order = str(value).strip().lower().replace("_", "-")
    if order in ("", "native", "host"):
        return ByteOrder.NATIVE.value
    if order in ("little", "little-endian", "le"):
        return ByteOrder.LITTLE.value
    if order in ("big", "big-endian", "be", "network"):
        return ByteOrder.BIG.value
    if strict:
        allowed = ", ".join(byte_order.value for byte_order in ByteOrder)
        raise ValueError(f"byte_order must be one of: {allowed}")
    return ByteOrder.NATIVE.value


def _normalize_font_environment_value(value):
    if value is None:
        return "unspecified"
    if isinstance(value, str):
        normalized = " ".join(value.replace("\\", "/").strip().lower().split())
        return normalized or "unspecified"
    if isinstance(value, bool) or isinstance(value, int):
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError("font_environment must not contain non-finite numbers")
        return value
    if isinstance(value, list):
        return [_normalize_font_environment_value(item) for item in value]
    if isinstance(value, dict):
        normalized = {}
        for key in sorted(value.keys(), key=lambda item: str(item).strip().lower()):
            normalized_key = str(key).replace("\\", "/").strip().lower()
            if not normalized_key:
                raise ValueError("font_environment keys must be non-empty")
            normalized[normalized_key] = _normalize_font_environment_value(value[key])
        return normalized
    return " ".join(str(value).replace("\\", "/").strip().lower().split()) or "unspecified"


def _normalize_font_environment(value) -> tuple[str, str]:
    normalized = _normalize_font_environment_value(value)
    if isinstance(normalized, str):
        canonical = normalized
    else:
        canonical = json.dumps(
            normalized,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=True,
        )
    fingerprint = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    return canonical, fingerprint


def _normalize_rotation(value) -> int:
    try:
        degrees = int(value)
    except (TypeError, ValueError) as error:
        raise ValueError("rotation_degrees must be an integer") from error
    degrees %= 360
    if degrees not in (0, 90, 180, 270):
        raise ValueError("rotation_degrees must normalize to 0, 90, 180, or 270")
    return degrees


def _bytes_per_pixel(order: ChannelOrder) -> int:
    if order in (ChannelOrder.RGBA, ChannelOrder.BGRA, ChannelOrder.ARGB):
        return 4
    if order in (ChannelOrder.RGB, ChannelOrder.BGR):
        return 3
    if order == ChannelOrder.GRAY_ALPHA:
        return 2
    if order == ChannelOrder.GRAY:
        return 1
    raise ValueError(f"unsupported channel order: {order}")


# ---------------------------------------------------------------------------
# Normalization metadata
# ---------------------------------------------------------------------------

@dataclass
class NormalizationReport:
    """Records what normalization steps were applied to an input image."""
    source_path: str = ""
    original_mode: str = ""
    original_size: tuple[int, int] = (0, 0)
    expected_size: tuple[int, int] | None = None
    raw_stride_bytes: int | None = None
    channel_order_applied: str = "none"
    alpha_semantics_applied: str = AlphaSemantics.STRAIGHT.value
    byte_order_applied: str = ByteOrder.NATIVE.value
    unpremultiplied: bool = False
    background_rgba_applied: list[int] | None = None
    grayscale_applied: bool = False
    exif_rotation_applied: int = 0
    render_rotation_applied: int = 0
    final_mode: str = "RGBA"
    final_size: tuple[int, int] = (0, 0)

    def to_dict(self) -> dict:
        return {
            "source_path": self.source_path,
            "original_mode": self.original_mode,
            "original_size": list(self.original_size),
            "expected_size": list(self.expected_size) if self.expected_size else None,
            "raw_stride_bytes": self.raw_stride_bytes,
            "channel_order_applied": self.channel_order_applied,
            "alpha_semantics_applied": self.alpha_semantics_applied,
            "byte_order_applied": self.byte_order_applied,
            "unpremultiplied": self.unpremultiplied,
            "background_rgba_applied": self.background_rgba_applied,
            "grayscale_applied": self.grayscale_applied,
            "exif_rotation_applied": self.exif_rotation_applied,
            "render_rotation_applied": self.render_rotation_applied,
            "final_mode": self.final_mode,
            "final_size": list(self.final_size),
        }


@dataclass
class RenderReferenceContext:
    """Normalized render-reference metadata carried beside a compact image.

    These fields describe the render contract that produced an image. The tool
    intentionally compares and classifies them; it does not run a renderer or a
    corpus campaign.
    """
    label: str = ""
    media_box: list[float] | None = None
    crop_box: list[float] | None = None
    rotation_degrees: int = 0
    matrix: list[float] | None = None
    width: int | None = None
    height: int | None = None
    stride_bytes: int | None = None
    channel_order: str | None = None
    alpha_semantics: str = "straight"
    background_rgba: list[int] | None = None
    annotations: str = "unspecified"
    forms: str = "unspecified"
    optional_content: str = "unspecified"
    grayscale: bool = False
    color_profile: str = "unspecified"
    display_profile: str = "unspecified"
    print_profile: str = "unspecified"
    byte_order: str = "native"
    text_smoothing: str = "unspecified"
    image_smoothing: str = "unspecified"
    path_smoothing: str = "unspecified"
    font_environment: str = "unspecified"
    font_environment_fingerprint: str = ""
    malformed_input_policy: str = "unspecified"

    @classmethod
    def from_json_file(cls, path: Path | None) -> "RenderReferenceContext":
        if path is None:
            return cls()
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError(f"render context must be a JSON object: {path}")
        return cls.from_dict(data, source_path=path)

    @classmethod
    def from_dict(
        cls,
        data: dict,
        *,
        source_path: Path | None = None,
    ) -> "RenderReferenceContext":
        dimensions = data.get("dimensions")
        width = data.get("width")
        height = data.get("height")
        if isinstance(dimensions, list) and len(dimensions) == 2:
            width = dimensions[0] if width is None else width
            height = dimensions[1] if height is None else height
        background = data.get("background_rgba", data.get("background"))
        alpha = data.get("alpha_semantics", data.get("alpha", "straight"))
        smoothing = data.get("smoothing", {})
        if smoothing is None:
            smoothing = {}
        if not isinstance(smoothing, dict):
            raise ValueError("smoothing must be a JSON object when present")
        font_environment, font_environment_fingerprint = _normalize_font_environment(
            data.get("font_environment", data.get("font_environment_manifest", "unspecified"))
        )

        context = cls(
            label=str(data.get("label", source_path or "")),
            media_box=_optional_float_list(data.get("media_box"), 4, "media_box"),
            crop_box=_optional_float_list(data.get("crop_box"), 4, "crop_box"),
            rotation_degrees=_normalize_rotation(data.get("rotation_degrees", data.get("rotation", 0))),
            matrix=_optional_float_list(data.get("matrix"), 6, "matrix"),
            width=_optional_positive_int(width, "width"),
            height=_optional_positive_int(height, "height"),
            stride_bytes=_optional_positive_int(
                data.get("stride_bytes", data.get("stride")), "stride"
            ),
            channel_order=_optional_channel_order(data.get("channel_order")),
            alpha_semantics=_normalize_alpha_semantics(alpha),
            background_rgba=_optional_rgba(background, "background"),
            annotations=str(data.get("annotations", "unspecified")).strip().lower(),
            forms=str(data.get("forms", "unspecified")).strip().lower(),
            optional_content=str(data.get("optional_content", "unspecified")).strip().lower(),
            grayscale=bool(data.get("grayscale", False)),
            color_profile=str(data.get("color_profile", "unspecified")).strip().lower(),
            display_profile=str(data.get("display_profile", "unspecified")).strip().lower(),
            print_profile=str(data.get("print_profile", "unspecified")).strip().lower(),
            byte_order=str(data.get("byte_order", "native")).strip().lower(),
            text_smoothing=str(smoothing.get("text", data.get("text_smoothing", "unspecified"))).strip().lower(),
            image_smoothing=str(smoothing.get("image", data.get("image_smoothing", "unspecified"))).strip().lower(),
            path_smoothing=str(smoothing.get("path", data.get("path_smoothing", "unspecified"))).strip().lower(),
            font_environment=font_environment,
            font_environment_fingerprint=font_environment_fingerprint,
            malformed_input_policy=str(data.get("malformed_input_policy", "unspecified")).strip().lower(),
        )
        context.validate()
        return context

    def validate(self) -> None:
        self.alpha_semantics = _normalize_alpha_semantics(self.alpha_semantics)
        if self.stride_bytes is not None and self.width is not None:
            bpp = _bytes_per_pixel(
                ChannelOrder(self.channel_order or ChannelOrder.RGBA.value)
            )
            min_stride = self.width * bpp
            if self.stride_bytes < min_stride:
                raise ValueError(
                    f"stride {self.stride_bytes} is smaller than row bytes {min_stride}"
                )
        if self.height is not None and self.height <= 0:
            raise ValueError("height must be positive")
        if self.width is not None and self.width <= 0:
            raise ValueError("width must be positive")
        if not self.font_environment_fingerprint:
            self.font_environment, self.font_environment_fingerprint = _normalize_font_environment(
                self.font_environment
            )

    def to_dict(self) -> dict:
        return {
            "label": self.label,
            "media_box": self.media_box,
            "crop_box": self.crop_box,
            "rotation_degrees": self.rotation_degrees,
            "matrix": self.matrix,
            "dimensions": [self.width, self.height],
            "stride_bytes": self.stride_bytes,
            "channel_order": self.channel_order,
            "alpha_semantics": self.alpha_semantics,
            "background_rgba": self.background_rgba,
            "annotations": self.annotations,
            "forms": self.forms,
            "optional_content": self.optional_content,
            "grayscale": self.grayscale,
            "color_profile": self.color_profile,
            "display_profile": self.display_profile,
            "print_profile": self.print_profile,
            "byte_order": self.byte_order,
            "smoothing": {
                "text": self.text_smoothing,
                "image": self.image_smoothing,
                "path": self.path_smoothing,
            },
            "font_environment": self.font_environment,
            "font_environment_fingerprint": self.font_environment_fingerprint,
            "malformed_input_policy": self.malformed_input_policy,
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


def _apply_raw_byte_order(data: bytes, order: ChannelOrder, byte_order: str) -> bytes:
    """Normalize packed raw-buffer byte order before channel interpretation."""
    normalized = _normalize_byte_order(byte_order, strict=True)
    if normalized != ByteOrder.LITTLE.value:
        return data

    bpp = _bytes_per_pixel(order)
    if bpp == 1:
        return data

    reversed_pixels = bytearray(len(data))
    for offset in range(0, len(data), bpp):
        reversed_pixels[offset:offset + bpp] = data[offset:offset + bpp][::-1]
    return bytes(reversed_pixels)


def _load_raw_buffer(
    path: Path,
    order: ChannelOrder,
    width: int,
    height: int,
    stride: int | None = None,
    byte_order: str = ByteOrder.NATIVE.value,
) -> Image.Image:
    """Load a raw pixel buffer with explicit dimensions, channel order, and stride."""
    data = path.read_bytes()
    bpp = _bytes_per_pixel(order)
    row_bytes = width * bpp
    stride = row_bytes if stride is None else stride
    if stride < row_bytes:
        raise ValueError(f"raw stride {stride} is smaller than row bytes {row_bytes}")
    expected = stride * height
    if len(data) != expected:
        raise ValueError(
            f"raw buffer length {len(data)} does not equal expected {expected} "
            f"({width}x{height}x{bpp}, stride {stride})"
        )
    if stride != row_bytes:
        rows = []
        for y in range(height):
            start = y * stride
            rows.append(data[start:start + row_bytes])
        data = b"".join(rows)
    data = _apply_raw_byte_order(data, order, byte_order)

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

def _flattened_data(image: Image.Image):
    """Return Pillow pixel data without relying on deprecated getdata()."""
    if hasattr(image, "get_flattened_data"):
        return image.get_flattened_data()
    return image.getdata()


def _detect_premultiplied(image: Image.Image) -> bool:
    """Heuristic detection of premultiplied alpha.

    Checks a sample of pixels: if any color channel exceeds its alpha value,
    the image is NOT premultiplied. If all sampled color channels are <= alpha,
    and alpha < 255 in some pixels, it is likely premultiplied.
    """
    if image.mode != "RGBA":
        return False

    pixels = _flattened_data(image)
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

    pixels = list(_flattened_data(image))
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


def _force_opaque_alpha(image: Image.Image) -> Image.Image:
    """Force alpha to 255 while preserving RGB channel values."""
    if image.mode != "RGBA":
        image = image.convert("RGBA")
    r, g, b, _ = image.split()
    a = Image.new("L", image.size, 255)
    return Image.merge("RGBA", (r, g, b, a))


def _apply_alpha_semantics(
    image: Image.Image,
    semantics: str,
) -> tuple[Image.Image, bool]:
    """Normalize an input alpha representation to canonical straight RGBA."""
    semantics = _normalize_alpha_semantics(semantics)
    if image.mode != "RGBA":
        image = image.convert("RGBA")
    if semantics == AlphaSemantics.PREMULTIPLIED.value:
        return _unpremultiply_alpha(image), True
    if semantics == AlphaSemantics.OPAQUE.value:
        return _force_opaque_alpha(image), False
    return image, False


def _composite_over_background(image: Image.Image, background_rgba: list[int]) -> Image.Image:
    """Composite a straight-alpha RGBA image over an explicit RGBA background."""
    if image.mode != "RGBA":
        image = image.convert("RGBA")
    background = Image.new("RGBA", image.size, tuple(background_rgba))
    return Image.alpha_composite(background, image)


def _apply_grayscale(image: Image.Image) -> Image.Image:
    """Convert RGB channels to canonical luma while preserving straight alpha."""
    if image.mode != "RGBA":
        image = image.convert("RGBA")
    r, g, b, a = image.split()
    gray = Image.merge("RGB", (r, g, b)).convert("L")
    return Image.merge("RGBA", (gray, gray, gray, a))


def _validate_expected_dimensions(
    image: Image.Image,
    expected_size: tuple[int, int] | None,
) -> None:
    if expected_size is None:
        return
    if image.size != expected_size:
        raise ValueError(
            f"normalized dimensions {image.size} do not match render context "
            f"dimensions {expected_size}"
        )


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


def _apply_render_context_rotation(image: Image.Image, degrees: int) -> Image.Image:
    """Normalize render-context rotation back to the canonical unrotated surface."""
    if degrees == 0:
        return image
    if degrees == 90:
        return image.transpose(Image.Transpose.ROTATE_90)
    if degrees == 180:
        return image.transpose(Image.Transpose.ROTATE_180)
    if degrees == 270:
        return image.transpose(Image.Transpose.ROTATE_270)
    raise ValueError("rotation_degrees must be 0, 90, 180, or 270")


def _context_dimension(value: int | None, fallback: int | None) -> int | None:
    return value if value is not None else fallback


def _effective_order(
    explicit_order: ChannelOrder,
    context: RenderReferenceContext,
) -> ChannelOrder:
    if (
        explicit_order == ChannelOrder.RGBA
        and context.channel_order is not None
    ):
        return ChannelOrder(context.channel_order)
    return explicit_order


def _compare_render_contexts(
    left: RenderReferenceContext,
    right: RenderReferenceContext,
) -> list[dict]:
    comparisons = [
        ("media_box", left.media_box, right.media_box, DifferenceClass.DIMENSION_OR_TRANSFORM),
        ("crop_box", left.crop_box, right.crop_box, DifferenceClass.DIMENSION_OR_TRANSFORM),
        ("rotation_degrees", left.rotation_degrees, right.rotation_degrees, DifferenceClass.DIMENSION_OR_TRANSFORM),
        ("matrix", left.matrix, right.matrix, DifferenceClass.DIMENSION_OR_TRANSFORM),
        ("dimensions", [left.width, left.height], [right.width, right.height], DifferenceClass.DIMENSION_OR_TRANSFORM),
        ("stride_bytes", left.stride_bytes, right.stride_bytes, DifferenceClass.CHANNEL_LAYOUT),
        ("channel_order", left.channel_order, right.channel_order, DifferenceClass.CHANNEL_LAYOUT),
        ("alpha_semantics", left.alpha_semantics, right.alpha_semantics, DifferenceClass.TRANSPARENCY_ROUNDING),
        ("background_rgba", left.background_rgba, right.background_rgba, DifferenceClass.BACKGROUND_POLICY),
        ("annotations", left.annotations, right.annotations, DifferenceClass.ANNOTATION_POLICY),
        ("forms", left.forms, right.forms, DifferenceClass.FORM_POLICY),
        ("optional_content", left.optional_content, right.optional_content, DifferenceClass.OPTIONAL_CONTENT_POLICY),
        ("grayscale", left.grayscale, right.grayscale, DifferenceClass.COLOR_PROFILE),
        ("color_profile", left.color_profile, right.color_profile, DifferenceClass.COLOR_PROFILE),
        ("display_profile", left.display_profile, right.display_profile, DifferenceClass.COLOR_PROFILE),
        ("print_profile", left.print_profile, right.print_profile, DifferenceClass.COLOR_PROFILE),
        ("byte_order", left.byte_order, right.byte_order, DifferenceClass.CHANNEL_LAYOUT),
        ("text_smoothing", left.text_smoothing, right.text_smoothing, DifferenceClass.HINTING),
        ("image_smoothing", left.image_smoothing, right.image_smoothing, DifferenceClass.ANTIALIASING),
        ("path_smoothing", left.path_smoothing, right.path_smoothing, DifferenceClass.ANTIALIASING),
        ("font_environment", left.font_environment, right.font_environment, DifferenceClass.FONT_FALLBACK),
        ("malformed_input_policy", left.malformed_input_policy, right.malformed_input_policy, DifferenceClass.MALFORMED_INPUT_POLICY),
    ]
    differences = []
    for field, left_value, right_value, category in comparisons:
        if left_value != right_value:
            differences.append({
                "field": field,
                "left": left_value,
                "right": right_value,
                "classification": category.value,
            })
    return differences


def _context_with_alpha_semantics(
    context: RenderReferenceContext,
    alpha_semantics: str | None,
) -> RenderReferenceContext:
    if alpha_semantics is None:
        return context
    return replace(
        context,
        alpha_semantics=_normalize_alpha_semantics(alpha_semantics),
    )


def _normalization_policy(
    left_context: RenderReferenceContext,
    right_context: RenderReferenceContext,
) -> dict:
    return {
        "canonical_surface": {
            "mode": "RGBA",
            "bit_depth": 8,
            "channel_order": "rgba",
            "alpha_semantics": "straight",
            "color_space": "srgb_bytes_unmanaged",
            "background_policy": "explicit_render_context_rgba_composited_into_canonical_surface",
            "grayscale_policy": "render_context_grayscale_converts_rgb_to_luma_preserving_alpha",
            "dimensions": "post_exif_and_render_context_rotation_exact_match_required",
            "dimension_policy": "declared_dimensions_are_validation_bounds_not_resize_hints",
        },
        "tracked_context_fields": [
            "media_box",
            "crop_box",
            "rotation_degrees",
            "matrix",
            "dimensions",
            "stride_bytes",
            "channel_order",
            "alpha_semantics",
            "background_rgba",
            "annotations",
            "forms",
            "optional_content",
            "grayscale",
            "color_profile",
            "display_profile",
            "print_profile",
            "byte_order",
            "text_smoothing",
            "image_smoothing",
            "path_smoothing",
            "font_environment",
            "font_environment_fingerprint",
            "malformed_input_policy",
        ],
        "left_context": left_context.to_dict(),
        "right_context": right_context.to_dict(),
        "context_differences": _compare_render_contexts(left_context, right_context),
    }




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
    stride: int | None = None,
    byte_order: str | None = None,
    assume_premultiplied: bool = False,
    alpha_semantics: str | None = None,
    apply_exif: bool = True,
    render_context: RenderReferenceContext | None = None,
) -> tuple[Image.Image, NormalizationReport]:
    """Full normalization pipeline: load -> channel reorder -> alpha semantics -> EXIF -> RGBA.

    Returns canonical 8-bit RGBA image and a report of applied steps.
    """
    report = NormalizationReport(source_path=str(path))
    render_context = render_context or RenderReferenceContext()
    channel_order = _effective_order(channel_order, render_context)
    width = _context_dimension(width, render_context.width)
    height = _context_dimension(height, render_context.height)
    stride = _context_dimension(stride, render_context.stride_bytes)
    raw_byte_order = _normalize_byte_order(
        byte_order if byte_order is not None else render_context.byte_order,
        strict=byte_order is not None,
    )
    expected_size = None
    if width is not None and height is not None:
        expected_size = (width, height)
        report.expected_size = expected_size
    if alpha_semantics is None:
        alpha_semantics = render_context.alpha_semantics
    alpha_semantics = _normalize_alpha_semantics(alpha_semantics)
    if assume_premultiplied:
        if alpha_semantics not in (AlphaSemantics.STRAIGHT.value, AlphaSemantics.PREMULTIPLIED.value):
            raise ValueError("assume_premultiplied conflicts with opaque alpha_semantics")
        alpha_semantics = AlphaSemantics.PREMULTIPLIED.value

    # Step 1: Load
    if raw_mode:
        if width is None or height is None or width <= 0 or height <= 0:
            raise ValueError("raw mode requires positive width and height")
        image = _load_raw_buffer(
            path,
            channel_order,
            width,
            height,
            stride,
            raw_byte_order,
        )
        report.raw_stride_bytes = stride
        report.byte_order_applied = raw_byte_order
    else:
        image = Image.open(path)

    report.original_mode = image.mode
    report.original_size = image.size

    # Step 2: EXIF orientation (before channel reorder, on the original load)
    exif_applied = 0
    if apply_exif and not raw_mode:
        image, exif_applied = _apply_exif_orientation(image)
        report.exif_rotation_applied = exif_applied

    # Step 2b: Render-contract rotation normalization from sidecar metadata.
    if render_context.rotation_degrees:
        image = _apply_render_context_rotation(image, render_context.rotation_degrees)
        report.render_rotation_applied = render_context.rotation_degrees

    # Step 3: Channel order normalization -> RGBA
    if raw_mode:
        # `_load_raw_buffer` decodes the declared raw layout directly into the
        # matching PIL mode (including BGRA/ARGB byte reordering), so only a
        # canonical RGBA conversion remains here.
        image = image.convert("RGBA")
        report.channel_order_applied = channel_order.value
    else:
        if channel_order != ChannelOrder.RGBA:
            image = _reorder_channels(image, channel_order)
            report.channel_order_applied = channel_order.value
        else:
            image = image.convert("RGBA")
            report.channel_order_applied = "auto_rgba"

    # Step 4: Alpha semantics handling. Automatic premultiplied detection is
    # intentionally advisory only: straight-alpha colors can satisfy the
    # r/g/b <= alpha heuristic, so mutating them would corrupt comparisons.
    image, report.unpremultiplied = _apply_alpha_semantics(image, alpha_semantics)
    report.alpha_semantics_applied = alpha_semantics

    # Step 5: Explicit render-context background and grayscale normalization.
    if render_context.background_rgba is not None:
        image = _composite_over_background(image, render_context.background_rgba)
        report.background_rgba_applied = render_context.background_rgba
    if render_context.grayscale:
        image = _apply_grayscale(image)
        report.grayscale_applied = True

    # Step 6: Enforce declared canonical dimensions without resampling pixels.
    _validate_expected_dimensions(image, expected_size)

    # Final state
    report.final_mode = image.mode
    report.final_size = image.size
    return image, report




# ---------------------------------------------------------------------------
# Pixel and comparison utilities
# ---------------------------------------------------------------------------

def _rgba_pixels(image: Image.Image) -> list[tuple[int, int, int, int]]:
    """Extract flat list of RGBA pixel tuples."""
    return list(_flattened_data(image))


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
        return [value != 0 for value in _flattened_data(mask)]




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


def _classify_region_causes(
    region: dict,
    pixels_left: list[tuple[int, int, int, int]],
    pixels_right: list[tuple[int, int, int, int]],
    width: int,
    height: int,
    context_differences: list[dict],
) -> list[str]:
    """Attach likely renderer-difference causes to a mismatch region."""
    classes = {item["classification"] for item in context_differences}
    rx, ry = region["x"], region["y"]
    rw, rh = region["width"], region["height"]
    alpha_abs = 0
    rgb_abs = 0
    edge_sum = 0.0
    samples = 0
    for dy in range(rh):
        for dx in range(rw):
            idx = (ry + dy) * width + (rx + dx)
            if idx >= len(pixels_left):
                continue
            left = pixels_left[idx]
            right = pixels_right[idx]
            alpha_abs += abs(left[3] - right[3])
            rgb_abs += (
                abs(left[0] - right[0])
                + abs(left[1] - right[1])
                + abs(left[2] - right[2])
            )
            edge_sum += _edge_strength(pixels_left, width, height, idx)
            samples += 1

    if samples == 0:
        return [DifferenceClass.UNCLASSIFIED_PIXEL_DELTA.value]

    mean_alpha = alpha_abs / samples
    mean_rgb = rgb_abs / (samples * 3)
    mean_edge = edge_sum / samples
    if mean_alpha > 0 and mean_alpha >= mean_rgb * 0.25:
        classes.add(DifferenceClass.TRANSPARENCY_ROUNDING.value)
    if mean_edge >= CRITICAL_EDGE_WEIGHT_THRESHOLD * 0.5:
        classes.add(DifferenceClass.ANTIALIASING.value)
    if not classes:
        classes.add(DifferenceClass.UNCLASSIFIED_PIXEL_DELTA.value)
    return sorted(classes)


def _classification_summary(regions: list[dict]) -> dict:
    summary = {item.value: 0 for item in DifferenceClass}
    for region in regions:
        for classification in region.get("classifications", []):
            summary[classification] = summary.get(classification, 0) + 1
    return summary


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
    context_differences: list[dict] | None = None,
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
    color_abs = 0
    color_sq = 0
    max_color_delta = 0
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
        color_deltas = deltas[:3]
        color_abs += sum(color_deltas)
        color_sq += sum(delta * delta for delta in color_deltas)
        max_color_delta = max(max_color_delta, *color_deltas)
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
    color_sample_count = eligible * 3
    mae = channel_abs / sample_count
    mse = channel_sq / sample_count
    rmse = math.sqrt(mse)
    color_mae = color_abs / color_sample_count
    color_rmse = math.sqrt(color_sq / color_sample_count)
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
    context_differences = context_differences or []
    regions = _find_mismatch_regions(changed, width, height)
    classified_regions = []
    for region in regions:
        severity = _classify_region(region, left_pixels, right_pixels, width, height)
        classifications = _classify_region_causes(
            region,
            left_pixels,
            right_pixels,
            width,
            height,
            context_differences,
        )
        classified_regions.append({
            **region,
            "severity": severity,
            "classifications": classifications,
        })

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
        "color_mae": color_mae,
        "color_rmse": color_rmse,
        "max_color_delta": max_color_delta,
        "psnr": "infinity" if psnr is None else psnr,
        "ssim_global_luminance": ssim,
        "alpha_mae": alpha_abs / eligible,
        "edge_weighted_difference": edge_weighted / eligible,
        "connected_mismatch_regions": classified_regions,
        "severity_summary": severity_counts,
        "classification_summary": _classification_summary(classified_regions),
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
    parser.add_argument("--left-stride", type=int, help="Stride in bytes for raw left input")
    parser.add_argument(
        "--left-byte-order",
        type=str,
        choices=[byte_order.value for byte_order in ByteOrder],
        help="Packed-pixel byte order for raw left input",
    )
    parser.add_argument("--right-width", type=int, help="Width for raw right input")
    parser.add_argument("--right-height", type=int, help="Height for raw right input")
    parser.add_argument("--right-stride", type=int, help="Stride in bytes for raw right input")
    parser.add_argument(
        "--right-byte-order",
        type=str,
        choices=[byte_order.value for byte_order in ByteOrder],
        help="Packed-pixel byte order for raw right input",
    )

    # Render contract sidecar metadata
    parser.add_argument(
        "--left-context-json",
        type=Path,
        help="JSON render-context sidecar for left/reference input",
    )
    parser.add_argument(
        "--right-context-json",
        type=Path,
        help="JSON render-context sidecar for right/candidate input",
    )

    # Premultiplication
    parser.add_argument(
        "--left-premultiplied", action="store_true",
        help="Legacy alias for --left-alpha-mode premultiplied",
    )
    parser.add_argument(
        "--right-premultiplied", action="store_true",
        help="Legacy alias for --right-alpha-mode premultiplied",
    )
    parser.add_argument(
        "--left-alpha-mode", type=str,
        choices=[alpha.value for alpha in AlphaSemantics],
        help="Alpha representation for left input: straight, premultiplied, or opaque",
    )
    parser.add_argument(
        "--right-alpha-mode", type=str,
        choices=[alpha.value for alpha in AlphaSemantics],
        help="Alpha representation for right input: straight, premultiplied, or opaque",
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

    left_alpha_semantics = args.left_alpha_mode
    right_alpha_semantics = args.right_alpha_mode
    if args.left_premultiplied:
        if left_alpha_semantics not in (None, AlphaSemantics.PREMULTIPLIED.value):
            parser.error("--left-premultiplied conflicts with --left-alpha-mode")
        left_alpha_semantics = AlphaSemantics.PREMULTIPLIED.value
    if args.right_premultiplied:
        if right_alpha_semantics not in (None, AlphaSemantics.PREMULTIPLIED.value):
            parser.error("--right-premultiplied conflicts with --right-alpha-mode")
        right_alpha_semantics = AlphaSemantics.PREMULTIPLIED.value

    apply_exif = not args.no_exif
    left_context = RenderReferenceContext.from_json_file(args.left_context_json)
    right_context = RenderReferenceContext.from_json_file(args.right_context_json)
    left_context = _context_with_alpha_semantics(left_context, left_alpha_semantics)
    right_context = _context_with_alpha_semantics(right_context, right_alpha_semantics)
    context_differences = _compare_render_contexts(left_context, right_context)

    # Normalize left
    left_img, left_report = normalize_to_canonical_rgba(
        args.left,
        channel_order=left_order,
        raw_mode=left_raw,
        width=args.left_width,
        height=args.left_height,
        stride=args.left_stride,
        byte_order=args.left_byte_order,
        alpha_semantics=left_alpha_semantics,
        apply_exif=apply_exif,
        render_context=left_context,
    )

    # Normalize right
    right_img, right_report = normalize_to_canonical_rgba(
        args.right,
        channel_order=right_order,
        raw_mode=right_raw,
        width=args.right_width,
        height=args.right_height,
        stride=args.right_stride,
        byte_order=args.right_byte_order,
        alpha_semantics=right_alpha_semantics,
        apply_exif=apply_exif,
        render_context=right_context,
    )

    # Load expected-difference mask
    ignored = load_expected_mask(args.expected_difference_mask, *left_img.size)

    # Compare
    result = compare(left_img, right_img, args.tolerance, ignored, context_differences)

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
    result["normalization_policy"] = _normalization_policy(left_context, right_context)

    # Write output
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
