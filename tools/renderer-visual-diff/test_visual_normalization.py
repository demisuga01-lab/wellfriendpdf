#!/usr/bin/env python3
"""Compact self-contained tests for RB-15 visual normalization.

These tests use synthetic in-memory images (no external corpus or fixtures required).
Run with: python -m pytest tools/renderer-visual-diff/test_visual_normalization.py -v
"""

from __future__ import annotations

import json
import math
from pathlib import Path

import pytest
from PIL import Image

# Import the module under test
import sys
sys.path.insert(0, str(Path(__file__).parent))
import visual_diff


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_solid_rgba(width: int, height: int, color: tuple[int, int, int, int]) -> Image.Image:
    """Create a solid-color RGBA image."""
    img = Image.new("RGBA", (width, height), color)
    return img


def _save_png(img: Image.Image, path: Path) -> None:
    img.save(path, "PNG")


def _save_raw(img: Image.Image, path: Path, raw_format: str = "RGBA") -> None:
    """Save raw pixel bytes in the specified format."""
    data = img.tobytes("raw", raw_format)
    path.write_bytes(data)


# ---------------------------------------------------------------------------
# Test: Channel order normalization
# ---------------------------------------------------------------------------

class TestChannelOrderNormalization:
    """Verify that various channel orders are correctly converted to canonical RGBA."""

    def test_rgba_passthrough(self, tmp_path: Path):
        """RGBA input passes through unchanged."""
        img = _make_solid_rgba(4, 4, (200, 100, 50, 255))
        path = tmp_path / "rgba.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, channel_order=visual_diff.ChannelOrder.RGBA
        )
        assert result.mode == "RGBA"
        assert result.size == (4, 4)
        r, g, b, a = result.getpixel((0, 0))
        assert (r, g, b, a) == (200, 100, 50, 255)
        assert report.channel_order_applied == "auto_rgba"

    def test_bgra_raw_normalization(self, tmp_path: Path):
        """Raw BGRA buffer is reordered to RGBA correctly."""
        # Create RGBA image, save as raw BGRA
        img = _make_solid_rgba(4, 4, (200, 100, 50, 255))
        path = tmp_path / "bgra.raw"
        _save_raw(img, path, "BGRA")

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            channel_order=visual_diff.ChannelOrder.BGRA,
            raw_mode=True,
            width=4,
            height=4,
        )
        assert result.mode == "RGBA"
        r, g, b, a = result.getpixel((0, 0))
        assert (r, g, b, a) == (200, 100, 50, 255)
        assert report.channel_order_applied == "bgra"

    def test_raw_stride_padding_is_removed(self, tmp_path: Path):
        """Raw stride padding is validated and stripped row by row."""
        path = tmp_path / "stride_bgra.raw"
        row0 = bytes([30, 20, 10, 255, 60, 50, 40, 255, 0, 0, 0, 0])
        row1 = bytes([90, 80, 70, 255, 120, 110, 100, 255, 0, 0, 0, 0])
        path.write_bytes(row0 + row1)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            channel_order=visual_diff.ChannelOrder.BGRA,
            raw_mode=True,
            width=2,
            height=2,
            stride=12,
        )

        assert report.raw_stride_bytes == 12
        assert result.getpixel((0, 0)) == (10, 20, 30, 255)
        assert result.getpixel((1, 1)) == (100, 110, 120, 255)

    def test_raw_stride_too_small_raises(self, tmp_path: Path):
        """A stride smaller than the row byte count is rejected."""
        path = tmp_path / "bad_stride.raw"
        path.write_bytes(bytes(16))
        with pytest.raises(ValueError, match="stride"):
            visual_diff.normalize_to_canonical_rgba(
                path,
                channel_order=visual_diff.ChannelOrder.RGBA,
                raw_mode=True,
                width=2,
                height=2,
                stride=4,
            )

    def test_raw_little_endian_rgba_word_normalization(self, tmp_path: Path):
        """Packed little-endian RGBA words are byte-swapped per pixel."""
        path = tmp_path / "little_rgba.raw"
        path.write_bytes(bytes([4, 3, 2, 1, 40, 30, 20, 10]))

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            channel_order=visual_diff.ChannelOrder.RGBA,
            raw_mode=True,
            width=2,
            height=1,
            byte_order="little",
        )

        assert result.getpixel((0, 0)) == (1, 2, 3, 4)
        assert result.getpixel((1, 0)) == (10, 20, 30, 40)
        assert report.byte_order_applied == "little"

    def test_context_byte_order_drives_raw_normalization(self, tmp_path: Path):
        """Render-context byte_order participates in raw-buffer normalization."""
        path = tmp_path / "context_little.raw"
        path.write_bytes(bytes([90, 80, 70, 60]))
        context = visual_diff.RenderReferenceContext.from_dict({
            "dimensions": [1, 1],
            "channel_order": "rgba",
            "byte_order": "little",
        })

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            raw_mode=True,
            render_context=context,
        )

        assert result.getpixel((0, 0)) == (60, 70, 80, 90)
        assert report.channel_order_applied == "rgba"
        assert report.byte_order_applied == "little"

    def test_argb_normalization(self, tmp_path: Path):
        """ARGB channel order is correctly reordered to RGBA."""
        # Create an image where we manually set ARGB byte order
        # ARGB in RGBA slots: slot0=A, slot1=R, slot2=G, slot3=B
        width, height = 2, 2
        # We want final RGBA = (100, 150, 200, 255)
        # In ARGB layout stored as RGBA image: R=A=255, G=R=100, B=G=150, A=B=200
        argb_img = Image.new("RGBA", (width, height), (255, 100, 150, 200))
        path = tmp_path / "argb.png"
        _save_png(argb_img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, channel_order=visual_diff.ChannelOrder.ARGB
        )
        r, g, b, a = result.getpixel((0, 0))
        assert (r, g, b, a) == (100, 150, 200, 255)

    def test_rgb_adds_opaque_alpha(self, tmp_path: Path):
        """RGB input gets alpha=255 appended."""
        img = Image.new("RGB", (3, 3), (80, 160, 240))
        path = tmp_path / "rgb.png"
        img.save(path, "PNG")

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, channel_order=visual_diff.ChannelOrder.RGB
        )
        r, g, b, a = result.getpixel((0, 0))
        assert (r, g, b, a) == (80, 160, 240, 255)
        assert report.channel_order_applied == "rgb"

    def test_gray_expands_to_rgba(self, tmp_path: Path):
        """Grayscale input expands to RGBA with equal R=G=B and alpha=255."""
        img = Image.new("L", (5, 5), 128)
        path = tmp_path / "gray.png"
        img.save(path, "PNG")

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, channel_order=visual_diff.ChannelOrder.GRAY
        )
        r, g, b, a = result.getpixel((0, 0))
        assert r == g == b == 128
        assert a == 255

    def test_gray_alpha_expands(self, tmp_path: Path):
        """Gray+Alpha input expands to RGBA with R=G=B=gray and given alpha."""
        img = Image.new("LA", (3, 3), (100, 200))
        path = tmp_path / "gray_alpha.png"
        img.save(path, "PNG")

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, channel_order=visual_diff.ChannelOrder.GRAY_ALPHA
        )
        r, g, b, a = result.getpixel((0, 0))
        assert r == g == b == 100
        assert a == 200


# ---------------------------------------------------------------------------
# Test: Alpha / premultiplication
# ---------------------------------------------------------------------------

class TestAlphaPremultiplication:
    """Verify premultiplied alpha detection and unpremultiplication."""

    def test_detect_premultiplied(self):
        """Image with all channels <= alpha is detected as premultiplied."""
        img = Image.new("RGBA", (4, 4))
        pixels = [(64, 64, 64, 128)] * 16  # All channels <= alpha
        img.putdata(pixels)
        assert visual_diff._detect_premultiplied(img) is True

    def test_detect_not_premultiplied(self):
        """Image with channels > alpha is NOT premultiplied."""
        img = Image.new("RGBA", (4, 4))
        pixels = [(200, 150, 100, 128)] * 16  # R=200 > alpha=128
        img.putdata(pixels)
        assert visual_diff._detect_premultiplied(img) is False

    def test_opaque_not_premultiplied(self):
        """Fully opaque image is not flagged as premultiplied."""
        img = Image.new("RGBA", (4, 4), (200, 100, 50, 255))
        assert visual_diff._detect_premultiplied(img) is False

    def test_unpremultiply_restores_values(self):
        """Unpremultiplication correctly scales channels back."""
        img = Image.new("RGBA", (1, 1))
        # Premultiplied: original (200, 100, 50, 128) -> stored as (100, 50, 25, 128)
        img.putdata([(100, 50, 25, 128)])
        result = visual_diff._unpremultiply_alpha(img)
        r, g, b, a = result.getpixel((0, 0))
        # 100 * 255/128 ≈ 199, 50 * 255/128 ≈ 100, 25 * 255/128 ≈ 50
        assert a == 128
        assert abs(r - 199) <= 1
        assert abs(g - 100) <= 1
        assert abs(b - 50) <= 1

    def test_unpremultiply_zero_alpha(self):
        """Zero alpha pixels become (0,0,0,0)."""
        img = Image.new("RGBA", (1, 1))
        img.putdata([(0, 0, 0, 0)])
        result = visual_diff._unpremultiply_alpha(img)
        assert result.getpixel((0, 0)) == (0, 0, 0, 0)

    def test_normalize_with_premultiplied_flag(self, tmp_path: Path):
        """assume_premultiplied triggers unpremultiplication."""
        img = Image.new("RGBA", (2, 2))
        img.putdata([(64, 32, 16, 128)] * 4)
        path = tmp_path / "premul.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, assume_premultiplied=True
        )
        assert report.unpremultiplied is True
        r, g, b, a = result.getpixel((0, 0))
        assert a == 128
        # 64 * 255/128 ≈ 127
        assert abs(r - 127) <= 1

    def test_normalize_with_premultiplied_alpha_mode(self, tmp_path: Path):
        """alpha_semantics=premultiplied triggers unpremultiplication."""
        img = Image.new("RGBA", (2, 2))
        img.putdata([(64, 32, 16, 128)] * 4)
        path = tmp_path / "premul_mode.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, alpha_semantics="premultiplied"
        )

        assert report.alpha_semantics_applied == "premultiplied"
        assert report.unpremultiplied is True
        r, g, b, a = result.getpixel((0, 0))
        assert a == 128
        assert abs(r - 127) <= 1
        assert abs(g - 64) <= 1
        assert abs(b - 32) <= 1

    def test_normalize_with_opaque_alpha_mode_forces_alpha(self, tmp_path: Path):
        """alpha_semantics=opaque preserves RGB and forces alpha to 255."""
        img = Image.new("RGBA", (2, 2), (200, 100, 50, 17))
        path = tmp_path / "opaque_mode.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, alpha_semantics="opaque"
        )

        assert report.alpha_semantics_applied == "opaque"
        assert report.unpremultiplied is False
        assert result.getpixel((0, 0)) == (200, 100, 50, 255)

    def test_render_context_opaque_alpha_mode_forces_alpha(self, tmp_path: Path):
        """render-context alpha_semantics=opaque drives canonical alpha output."""
        img = Image.new("RGBA", (2, 2), (10, 20, 30, 64))
        path = tmp_path / "context_opaque.png"
        _save_png(img, path)
        context = visual_diff.RenderReferenceContext.from_dict({
            "alpha_semantics": "opaque",
            "dimensions": [2, 2],
        })

        result, report = visual_diff.normalize_to_canonical_rgba(
            path, render_context=context
        )

        assert context.alpha_semantics == "opaque"
        assert report.alpha_semantics_applied == "opaque"
        assert result.getpixel((0, 0)) == (10, 20, 30, 255)

    def test_invalid_alpha_semantics_raises(self):
        """Unknown alpha modes are rejected before comparison."""
        with pytest.raises(ValueError, match="alpha_semantics"):
            visual_diff.RenderReferenceContext.from_dict({
                "alpha_semantics": "associated",
            })


# ---------------------------------------------------------------------------
# Test: EXIF rotation normalization
# ---------------------------------------------------------------------------

class TestExifRotation:
    """Verify EXIF orientation metadata is applied during normalization."""

    def test_no_exif_no_change(self, tmp_path: Path):
        """Image without EXIF stays unchanged."""
        img = _make_solid_rgba(4, 6, (100, 100, 100, 255))
        path = tmp_path / "no_exif.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(path)
        assert result.size == (4, 6)
        assert report.exif_rotation_applied == 0

    def test_exif_rotation_applied(self, tmp_path: Path):
        """Image with EXIF orientation 6 (90° CW) is rotated."""
        # Create a non-square image so rotation changes dimensions
        img = Image.new("RGB", (4, 8), (255, 0, 0))
        path = tmp_path / "rotated.jpg"
        # Build minimal EXIF with orientation=6 using Pillow's built-in support
        from PIL.Image import Exif
        exif = Exif()
        exif[0x0112] = 6  # Orientation tag = Rotate 270
        img.save(path, "JPEG", exif=exif.tobytes())

        result, report = visual_diff.normalize_to_canonical_rgba(path)
        # After orientation-6 rotation, 4x8 becomes 8x4
        assert result.size == (8, 4)
        assert report.exif_rotation_applied == 6

    def test_no_exif_flag_skips(self, tmp_path: Path):
        """apply_exif=False skips orientation correction."""
        img = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        path = tmp_path / "skip_exif.png"
        _save_png(img, path)

        result, report = visual_diff.normalize_to_canonical_rgba(path, apply_exif=False)
        assert report.exif_rotation_applied == 0


# ---------------------------------------------------------------------------
# Test: Expected-difference mask
# ---------------------------------------------------------------------------

class TestExpectedDifferenceMask:
    """Verify mask loading and application in comparison."""

    def test_mask_excludes_pixels(self, tmp_path: Path):
        """Pixels marked in mask are excluded from comparison."""
        # Two images that differ everywhere
        left = _make_solid_rgba(4, 4, (255, 0, 0, 255))
        right = _make_solid_rgba(4, 4, (0, 255, 0, 255))
        left_path = tmp_path / "left.png"
        right_path = tmp_path / "right.png"
        _save_png(left, left_path)
        _save_png(right, right_path)

        # Mask that marks ALL pixels as expected-different (white = ignore)
        mask = Image.new("L", (4, 4), 255)
        mask_path = tmp_path / "mask.png"
        mask.save(mask_path)

        left_norm, _ = visual_diff.normalize_to_canonical_rgba(left_path)
        right_norm, _ = visual_diff.normalize_to_canonical_rgba(right_path)
        ignored = visual_diff.load_expected_mask(mask_path, 4, 4)

        # All pixels ignored -> should raise (no eligible pixels)
        with pytest.raises(ValueError, match="excludes every pixel"):
            visual_diff.compare(left_norm, right_norm, 0, ignored)

    def test_partial_mask(self, tmp_path: Path):
        """Partial mask excludes only marked pixels."""
        left = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        # Make one pixel different in right
        right.putpixel((0, 0), (200, 100, 100, 255))

        left_path = tmp_path / "left.png"
        right_path = tmp_path / "right.png"
        _save_png(left, left_path)
        _save_png(right, right_path)

        # Mask that ignores only pixel (0,0)
        mask = Image.new("L", (4, 4), 0)
        mask.putpixel((0, 0), 255)
        mask_path = tmp_path / "mask.png"
        mask.save(mask_path)

        left_norm, _ = visual_diff.normalize_to_canonical_rgba(left_path)
        right_norm, _ = visual_diff.normalize_to_canonical_rgba(right_path)
        ignored = visual_diff.load_expected_mask(mask_path, 4, 4)

        result = visual_diff.compare(left_norm, right_norm, 0, ignored)
        # The differing pixel is masked, so no changes detected
        assert result["changed_pixel_count"] == 0

    def test_mask_dimension_mismatch_raises(self, tmp_path: Path):
        """Mask with wrong dimensions raises ValueError."""
        mask = Image.new("L", (10, 10), 0)
        mask_path = tmp_path / "wrong_mask.png"
        mask.save(mask_path)

        with pytest.raises(ValueError, match="do not match"):
            visual_diff.load_expected_mask(mask_path, 4, 4)

    def test_none_mask_returns_none(self):
        """None path returns None."""
        assert visual_diff.load_expected_mask(None, 4, 4) is None


# ---------------------------------------------------------------------------
# Test: Critical-region mismatch classification
# ---------------------------------------------------------------------------

class TestCriticalRegionClassification:
    """Verify severity classification of mismatch regions."""

    def test_negligible_small_region(self, tmp_path: Path):
        """Very small regions (< 4 pixels) are classified as negligible."""
        left = _make_solid_rgba(10, 10, (100, 100, 100, 255))
        right = _make_solid_rgba(10, 10, (100, 100, 100, 255))
        # 2 adjacent differing pixels
        right.putpixel((5, 5), (255, 0, 0, 255))
        right.putpixel((6, 5), (255, 0, 0, 255))

        result = visual_diff.compare(left, right, 0, None)
        regions = result["connected_mismatch_regions"]
        assert len(regions) == 1
        assert regions[0]["severity"] == "negligible"
        assert regions[0]["changed_pixels"] < 4

    def test_minor_medium_region(self, tmp_path: Path):
        """Small but >= 4 pixel regions are at least minor."""
        left = _make_solid_rgba(10, 10, (100, 100, 100, 255))
        right = _make_solid_rgba(10, 10, (100, 100, 100, 255))
        # 4 adjacent differing pixels (small change)
        for i in range(4):
            right.putpixel((3 + i, 5), (110, 100, 100, 255))

        result = visual_diff.compare(left, right, 0, None)
        regions = result["connected_mismatch_regions"]
        assert len(regions) == 1
        assert regions[0]["changed_pixels"] >= 4
        assert regions[0]["severity"] in ("minor", "significant", "critical")

    def test_severity_summary_present(self, tmp_path: Path):
        """Comparison result includes severity_summary."""
        left = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right = _make_solid_rgba(4, 4, (200, 200, 200, 255))

        result = visual_diff.compare(left, right, 0, None)
        assert "severity_summary" in result
        assert set(result["severity_summary"].keys()) == {
            "critical", "significant", "minor", "negligible"
        }

    def test_identical_images_no_regions(self):
        """Identical images produce zero mismatch regions."""
        img = _make_solid_rgba(8, 8, (50, 100, 150, 255))
        result = visual_diff.compare(img, img.copy(), 0, None)
        assert result["changed_pixel_count"] == 0
        assert result["connected_mismatch_regions"] == []
        assert result["severity_summary"] == {
            "critical": 0, "significant": 0, "minor": 0, "negligible": 0
        }


# ---------------------------------------------------------------------------
# Test: Render-context normalization metadata and taxonomy
# ---------------------------------------------------------------------------

class TestRenderContextNormalization:
    """Verify render-context sidecars drive policy reporting and classifications."""

    def test_context_json_records_normalization_policy(self, tmp_path: Path):
        """Render context fields are normalized and compared."""
        left_path = tmp_path / "left_context.json"
        right_path = tmp_path / "right_context.json"
        left_path.write_text(
            """{
              "label": "left",
              "media_box": [0, 0, 612, 792],
              "crop_box": [0, 0, 612, 792],
              "rotation_degrees": 0,
              "matrix": [1, 0, 0, 1, 0, 0],
              "dimensions": [2, 2],
              "stride": 8,
              "channel_order": "rgba",
              "alpha_semantics": "straight",
              "background": [255, 255, 255, 255],
              "annotations": "render",
              "forms": "render",
              "optional_content": "default",
              "grayscale": false,
              "color_profile": "srgb",
              "display_profile": "display",
              "print_profile": "none",
              "byte_order": "rgba",
              "smoothing": {"text": "subpixel", "image": "linear", "path": "aa"},
              "font_environment": "bundled-v1",
              "malformed_input_policy": "recover"
            }""",
            encoding="utf-8",
        )
        right_path.write_text(
            """{
              "label": "right",
              "media_box": [0, 0, 612, 792],
              "crop_box": [0, 0, 612, 792],
              "rotation_degrees": 90,
              "matrix": [1, 0, 0, 1, 0, 0],
              "dimensions": [2, 2],
              "stride": 8,
              "channel_order": "bgra",
              "alpha_semantics": "premultiplied",
              "background": [255, 255, 255, 255],
              "annotations": "skip",
              "forms": "render",
              "optional_content": "default",
              "grayscale": false,
              "color_profile": "display-p3",
              "display_profile": "display",
              "print_profile": "none",
              "byte_order": "bgra",
              "smoothing": {"text": "gray", "image": "linear", "path": "aa"},
              "font_environment": "system-v2",
              "malformed_input_policy": "recover"
            }""",
            encoding="utf-8",
        )

        left = visual_diff.RenderReferenceContext.from_json_file(left_path)
        right = visual_diff.RenderReferenceContext.from_json_file(right_path)
        policy = visual_diff._normalization_policy(left, right)
        classes = {
            item["classification"]
            for item in policy["context_differences"]
        }

        assert policy["canonical_surface"]["channel_order"] == "rgba"
        assert (
            policy["canonical_surface"]["background_policy"]
            == "explicit_render_context_rgba_composited_into_canonical_surface"
        )
        assert (
            policy["canonical_surface"]["grayscale_policy"]
            == "render_context_grayscale_converts_rgb_to_luma_preserving_alpha"
        )
        assert (
            policy["canonical_surface"]["dimension_policy"]
            == "declared_dimensions_are_validation_bounds_not_resize_hints"
        )
        assert "dimension_or_transform" in classes
        assert "channel_layout" in classes
        assert "transparency_rounding" in classes
        assert "annotation_policy" in classes
        assert "color_profile" in classes
        assert "font_fallback" in classes
        assert "font_environment_fingerprint" in policy["tracked_context_fields"]
        assert len(policy["left_context"]["font_environment_fingerprint"]) == 64

    def test_structured_font_environment_is_canonicalized(self):
        """Structured font manifests normalize key order, path separators, and case."""
        left = visual_diff.RenderReferenceContext.from_dict({
            "font_environment": {
                "provider": "System",
                "fonts": [
                    {
                        "family": "Arial",
                        "path": "C:\\Windows\\Fonts\\Arial.ttf",
                        "sha256": "ABC123",
                    }
                ],
                "fallbacks": ["Bundled-v1"],
            }
        })
        right = visual_diff.RenderReferenceContext.from_dict({
            "font_environment": {
                "fallbacks": ["bundled-v1"],
                "fonts": [
                    {
                        "sha256": "abc123",
                        "path": "c:/windows/fonts/arial.ttf",
                        "family": "arial",
                    }
                ],
                "provider": "system",
            }
        })

        assert left.font_environment == right.font_environment
        assert left.font_environment_fingerprint == right.font_environment_fingerprint
        assert left.to_dict()["font_environment_fingerprint"] == left.font_environment_fingerprint

    def test_font_environment_difference_is_classified_after_canonicalization(self):
        """A real normalized font-environment change remains a font-fallback cause."""
        left = visual_diff.RenderReferenceContext.from_dict({
            "font_environment": {
                "provider": "bundled",
                "fallbacks": ["LiberationSans"],
            }
        })
        right = visual_diff.RenderReferenceContext.from_dict({
            "font_environment": {
                "fallbacks": ["DejaVuSans"],
                "provider": "bundled",
            }
        })

        differences = visual_diff._compare_render_contexts(left, right)
        assert {
            item["classification"] for item in differences
        } == {"font_fallback"}
        assert differences[0]["field"] == "font_environment"

    def test_context_differences_classify_regions(self):
        """Context differences are attached to connected mismatch regions."""
        left = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right.putpixel((1, 1), (120, 120, 120, 128))
        context_differences = [{
            "field": "color_profile",
            "left": "srgb",
            "right": "display-p3",
            "classification": "color_profile",
        }]

        result = visual_diff.compare(left, right, 0, None, context_differences)
        classifications = result["connected_mismatch_regions"][0]["classifications"]

        assert "color_profile" in classifications
        assert "transparency_rounding" in classifications
        assert result["classification_summary"]["color_profile"] == 1

    def test_background_context_composites_to_canonical_surface(self, tmp_path: Path):
        """Explicit background metadata composites transparent pixels."""
        img = Image.new("RGBA", (2, 2), (100, 50, 0, 128))
        path = tmp_path / "transparent.png"
        _save_png(img, path)
        context = visual_diff.RenderReferenceContext.from_dict({
            "background": [10, 20, 30, 255],
            "dimensions": [2, 2],
        })

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            render_context=context,
        )

        expected = Image.alpha_composite(
            Image.new("RGBA", (2, 2), (10, 20, 30, 255)),
            img,
        )
        assert result.getpixel((0, 0)) == expected.getpixel((0, 0))
        assert result.getpixel((0, 0))[3] == 255
        assert report.background_rgba_applied == [10, 20, 30, 255]
        assert report.expected_size == (2, 2)

    def test_grayscale_context_normalizes_rgb_channels(self, tmp_path: Path):
        """grayscale=true converts RGB to canonical luma and preserves alpha."""
        img = Image.new("RGBA", (1, 1), (120, 60, 30, 200))
        path = tmp_path / "color.png"
        _save_png(img, path)
        context = visual_diff.RenderReferenceContext.from_dict({
            "grayscale": True,
            "dimensions": [1, 1],
        })

        result, report = visual_diff.normalize_to_canonical_rgba(
            path,
            render_context=context,
        )

        gray = Image.new("RGB", (1, 1), (120, 60, 30)).convert("L").getpixel((0, 0))
        assert result.getpixel((0, 0)) == (gray, gray, gray, 200)
        assert report.grayscale_applied is True

    def test_context_dimensions_are_enforced_after_normalization(self, tmp_path: Path):
        """Declared dimensions are exact comparison dimensions, not resize hints."""
        img = Image.new("RGBA", (2, 2), (0, 0, 0, 255))
        path = tmp_path / "wrong_size.png"
        _save_png(img, path)
        context = visual_diff.RenderReferenceContext.from_dict({
            "dimensions": [3, 2],
        })

        with pytest.raises(ValueError, match="normalized dimensions"):
            visual_diff.normalize_to_canonical_rgba(path, render_context=context)


# ---------------------------------------------------------------------------
# Test: Full normalization pipeline integration
# ---------------------------------------------------------------------------

class TestFullPipelineIntegration:
    """End-to-end normalization pipeline tests."""

    def test_identical_after_normalization(self, tmp_path: Path):
        """Same image through different formats normalizes to identical comparison."""
        img = _make_solid_rgba(6, 6, (120, 80, 40, 255))

        # Save as PNG (RGBA)
        png_path = tmp_path / "img.png"
        _save_png(img, png_path)

        # Save as raw BGRA
        raw_path = tmp_path / "img.raw"
        _save_raw(img, raw_path, "BGRA")

        # Normalize both
        left, _ = visual_diff.normalize_to_canonical_rgba(png_path)
        right, _ = visual_diff.normalize_to_canonical_rgba(
            raw_path,
            channel_order=visual_diff.ChannelOrder.BGRA,
            raw_mode=True,
            width=6,
            height=6,
        )

        result = visual_diff.compare(left, right, 0, None)
        assert result["changed_pixel_count"] == 0
        assert result["psnr"] == "infinity"
        assert result["ssim_global_luminance"] == 1.0

    def test_normalization_report_populated(self, tmp_path: Path):
        """Normalization report captures all steps."""
        img = _make_solid_rgba(3, 3, (10, 20, 30, 255))
        path = tmp_path / "test.png"
        _save_png(img, path)

        _, report = visual_diff.normalize_to_canonical_rgba(path)
        d = report.to_dict()
        assert d["original_mode"] == "RGBA"
        assert d["original_size"] == [3, 3]
        assert d["final_mode"] == "RGBA"
        assert d["final_size"] == [3, 3]
        assert d["alpha_semantics_applied"] == "straight"
        assert d["byte_order_applied"] == "native"
        assert d["unpremultiplied"] is False
        assert d["expected_size"] is None
        assert d["background_rgba_applied"] is None
        assert d["grayscale_applied"] is False
        assert d["exif_rotation_applied"] == 0

    def test_schema_version_v3(self, tmp_path: Path):
        """Compare output uses schema version 3."""
        img = _make_solid_rgba(2, 2, (100, 100, 100, 255))
        result = visual_diff.compare(img, img.copy(), 0, None)
        assert result["schema_version"] == 3

    def test_color_difference_metrics_are_rgb_only(self):
        """Color metrics report RGB deltas separately from alpha deltas."""
        left = _make_solid_rgba(1, 1, (10, 20, 30, 128))
        right = _make_solid_rgba(1, 1, (20, 20, 60, 0))

        result = visual_diff.compare(left, right, 0, None)

        assert result["color_mae"] == pytest.approx(40 / 3)
        assert result["color_rmse"] == pytest.approx(math.sqrt(1000 / 3))
        assert result["max_color_delta"] == 30
        assert result["alpha_mae"] == 128

    def test_cli_alpha_mode_overrides_normalization_policy(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        capsys: pytest.CaptureFixture[str],
    ):
        """CLI alpha-mode flags normalize pixels and update context policy."""
        left = Image.new("RGBA", (1, 1), (64, 32, 16, 128))
        right = Image.new("RGBA", (1, 1), (10, 20, 30, 64))
        left_path = tmp_path / "left.png"
        right_path = tmp_path / "right.png"
        output_path = tmp_path / "out.json"
        _save_png(left, left_path)
        _save_png(right, right_path)

        monkeypatch.setattr(sys, "argv", [
            "visual_diff.py",
            "--left", str(left_path),
            "--right", str(right_path),
            "--left-alpha-mode", "premultiplied",
            "--right-alpha-mode", "opaque",
            "--output", str(output_path),
        ])

        assert visual_diff.main() == 0
        capsys.readouterr()
        data = json.loads(output_path.read_text(encoding="utf-8"))

        assert data["normalization"]["left"]["alpha_semantics_applied"] == "premultiplied"
        assert data["normalization"]["right"]["alpha_semantics_applied"] == "opaque"
        policy = data["normalization_policy"]
        assert policy["left_context"]["alpha_semantics"] == "premultiplied"
        assert policy["right_context"]["alpha_semantics"] == "opaque"
        assert policy["context_differences"][0]["field"] == "alpha_semantics"

    def test_cli_byte_order_normalizes_raw_input(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        capsys: pytest.CaptureFixture[str],
    ):
        """CLI byte-order flags normalize packed raw input."""
        left_path = tmp_path / "left.raw"
        right_path = tmp_path / "right.png"
        output_path = tmp_path / "byte_order.json"
        left_path.write_bytes(bytes([4, 3, 2, 1]))
        _save_png(Image.new("RGBA", (1, 1), (1, 2, 3, 4)), right_path)

        monkeypatch.setattr(sys, "argv", [
            "visual_diff.py",
            "--left", str(left_path),
            "--right", str(right_path),
            "--left-raw",
            "--left-width", "1",
            "--left-height", "1",
            "--left-channel-order", "rgba",
            "--left-byte-order", "little",
            "--output", str(output_path),
        ])

        assert visual_diff.main() == 0
        capsys.readouterr()
        data = json.loads(output_path.read_text(encoding="utf-8"))

        assert data["changed_pixel_count"] == 0
        assert data["normalization"]["left"]["byte_order_applied"] == "little"

    def test_dimension_mismatch_raises(self, tmp_path: Path):
        """Mismatched dimensions after normalization raise ValueError."""
        left = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right = _make_solid_rgba(5, 5, (100, 100, 100, 255))
        with pytest.raises(ValueError, match="dimension mismatch"):
            visual_diff.compare(left, right, 0, None)

    def test_tolerance_suppresses_small_changes(self):
        """Tolerance parameter suppresses sub-threshold differences."""
        left = _make_solid_rgba(4, 4, (100, 100, 100, 255))
        right = _make_solid_rgba(4, 4, (102, 100, 100, 255))

        # Without tolerance: detected
        result_strict = visual_diff.compare(left, right, 0, None)
        assert result_strict["changed_pixel_count"] == 16

        # With tolerance=5: suppressed
        result_tolerant = visual_diff.compare(left, right, 5, None)
        assert result_tolerant["changed_pixel_count"] == 0
