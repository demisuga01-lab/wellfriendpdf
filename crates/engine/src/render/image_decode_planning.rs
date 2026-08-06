//! Image decode planning: metadata-first culling and complete cache identity.
//!
//! This module implements the technically actionable portion of RB-06/RB-13:
//! metadata-first image culling and cache-identity completeness for the active
//! renderer. Before decoding any image XObject, the renderer inspects image
//! metadata (dimensions, object identity) and the conservative device bounds
//! (computed from the current CTM and the unit-square image domain) to determine
//! whether the decoded pixels would intersect the active tile/viewport. Images
//! that are entirely outside the viewport are skipped without invoking the
//! decode pipeline or occupying cache slots.
//!
//! Additionally, the image cache key is extended with fields that are relevant
//! for source-region selection (if the decoder supported it), reduction level
//! selection, and render-contract state. These fields ensure that a cache entry
//! produced for one viewport/transform/reduction state is never incorrectly
//! reused when the decode contract changes, even though the underlying image
//! decoders (JPEG, JBIG2, JPEG2000 via `jpeg2000`/`jpegxr` crate) cannot
//! currently perform true region-of-interest or resolution-tier decode.
//!
//! # Upstream decoder limitations (as of this implementation)
//!
//! - **JPEG (via `zune-jpeg` / `image` crate)**: No region decode. No reduced
//!   resolution decode (DCT IDCT scaling is not exposed). Full image must be
//!   decoded to obtain any pixels.
//!
//! - **JPEG 2000 / JPX (via `jpeg2000` crate or fallback)**: The `jpeg2000`
//!   crate does not expose OpenJPEG region-of-interest or resolution-tier
//!   selection through its Rust API. The codec is architecturally capable of
//!   both features (via `opj_set_decode_area` and resolution reduction), but
//!   the safe Rust binding does not surface them. Full codestream decode is
//!   required for any pixel access.
//!
//! - **JBIG2 (via `jbig2dec` FFI or fallback)**: No region decode API. The
//!   segment model theoretically supports stripe-based decode, but available
//!   Rust wrappers do not expose partial decoding.
//!
//! - **CCITT Fax (internal)**: Row-based decoding is possible but the current
//!   implementation decodes the entire strip. Region skipping would require a
//!   row-range filter that is not yet implemented.
//!
//! - **Flate/LZW (internal)**: These are generic stream filters, not image
//!   codecs. No spatial awareness. Full decompression is required before
//!   predictor application.
//!
//! Because none of the upstream decoders currently support partial decode, the
//! `source_region` and `reduction_level` fields in [`ImageDecodePlan`] and
//! [`ImageDecodeCacheKey`] are populated conservatively (full region, no
//! reduction) and function as cache-identity placeholders. When a future
//! decoder upgrade exposes partial APIs, these fields will drive actual partial
//! decode without changing cache semantics.

use crate::render::display_list::RenderBounds;
use crate::render::transform::{Transform2D, Viewport};

// ---------------------------------------------------------------------------
// Image metadata extracted from the dictionary before decode.
// ---------------------------------------------------------------------------

/// Lightweight image metadata extracted from the PDF image XObject dictionary
/// without triggering any stream decompression or pixel decode.
#[derive(Debug, Clone)]
pub(crate) struct ImageMetadata {
    /// PDF object number (0 for inline images).
    pub object_number: u32,
    /// PDF generation number.
    pub generation_number: u16,
    /// Image width in samples.
    pub width: u32,
    /// Image height in samples.
    pub height: u32,
    /// Bits per component (1..16).
    pub bits_per_component: u8,
    /// Canonical color space family name.
    pub color_space: String,
    /// Filter chain names.
    pub filters: Vec<String>,
    /// Whether the image is a stencil mask.
    pub is_mask: bool,
    /// Whether this is an inline image.
    pub is_inline: bool,
}

// ---------------------------------------------------------------------------
// Source region / reduction / contract fields for cache identity.
// ---------------------------------------------------------------------------

/// Describes the source region of interest for an image decode operation.
///
/// Currently always `Full` because upstream decoders cannot do ROI decode.
/// Included in the cache key so that a future partial-decode upgrade will
/// automatically invalidate stale cache entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageSourceRegion {
    /// Full image decode (the only mode currently supported).
    Full,
    /// Future: a sub-rectangle in image-sample coordinates.
    SubRect {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

/// Resolution reduction tier for the image decode.
///
/// Currently always `None` because upstream decoders don't expose resolution
/// levels. The field is included in cache identity for forward compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageReductionLevel {
    /// Full resolution decode (the only mode currently supported).
    None,
    /// Future: power-of-two reduction (e.g., 1 = half, 2 = quarter).
    PowerOfTwo(u8),
}

/// Contract-relevant rendering state that affects how an image should be
/// decoded or post-processed. Changes to these fields mean the decoded image
/// cannot be reused from cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageContractState {
    /// Quantized device-space target width (0 when not axis-aligned or unknown).
    pub target_width: u32,
    /// Quantized device-space target height (0 when not axis-aligned or unknown).
    pub target_height: u32,
    /// Whether high-quality interpolation is active.
    pub high_quality: bool,
    /// Whether the image filter is JPXDecode (affects smoothing decisions).
    pub is_jpx: bool,
    /// Source region for this decode operation.
    pub source_region: ImageSourceRegion,
    /// Reduction level for this decode operation.
    pub reduction_level: ImageReductionLevel,
}

// ---------------------------------------------------------------------------
// Complete image decode cache key.
// ---------------------------------------------------------------------------

/// Extended cache key for image XObject decode results.
///
/// This combines the existing object-identity fields with the new
/// source-region, reduction, and contract-relevant state. Two entries with
/// the same object identity but different contract state are cached separately.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageDecodeCacheKey {
    /// Base identity: object number, generation, dimensions, BPC, filters.
    pub base_key: String,
    /// Contract state that modifies decode/post-process semantics.
    pub contract: ImageContractState,
}

impl ImageDecodeCacheKey {
    /// Build the complete cache key string. This extends the base key with
    /// contract-relevant fields so that lookups remain string-based (matching
    /// the existing `HashMap<String, Arc<RawImage>>` cache).
    pub fn to_cache_string(&self) -> String {
        // For the current implementation where source_region is always Full
        // and reduction_level is always None, we append only the target
        // dimensions, quality mode, and JPX flag. This ensures the key differs
        // when the same image is rendered at different device sizes.
        format!(
            "{}:dp:{}x{}:{}:{}:sr{:?}:rl{:?}",
            self.base_key,
            self.contract.target_width,
            self.contract.target_height,
            if self.contract.high_quality {
                "hq"
            } else {
                "compat"
            },
            if self.contract.is_jpx { "jpx" } else { "std" },
            self.contract.source_region,
            self.contract.reduction_level,
        )
    }
}

// ---------------------------------------------------------------------------
// Image decode plan.
// ---------------------------------------------------------------------------

/// The decision made by the image decode planner for a particular image XObject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageDecodePlanDecision {
    /// Image is entirely outside the active viewport/tile. Decode is skipped.
    SkipOutsideViewport,
    /// Image intersects the viewport. Full decode should proceed.
    DecodeRequired,
}

/// Complete image decode plan produced by the planner.
#[derive(Debug, Clone)]
pub(crate) struct ImageDecodePlan {
    /// The planning decision.
    pub decision: ImageDecodePlanDecision,
    /// Cache key incorporating contract state.
    pub cache_key: ImageDecodeCacheKey,
}

// ---------------------------------------------------------------------------
// Planner: compute device bounds and decide whether to decode.
// ---------------------------------------------------------------------------

/// Compute conservative device bounds for an image XObject.
///
/// PDF images occupy the unit square [0,0]–[1,1] in user space, transformed
/// by the current CTM to device space. This function transforms the four
/// corners of that unit square through the CTM and viewport mapping to
/// produce axis-aligned device-pixel bounds.
pub(crate) fn image_device_bounds(ctm: &Transform2D, viewport: &Viewport) -> Option<RenderBounds> {
    RenderBounds::from_unit_square(ctm, viewport, 1.0)
}

/// Compute the device-space target dimensions for axis-aligned images.
/// Returns (width, height) or (0, 0) if the image is not axis-aligned.
pub(crate) fn image_device_target_dimensions(ctm: &Transform2D, viewport: &Viewport) -> (u32, u32) {
    // Check axis-alignment: an axis-aligned CTM has b==0 and c==0.
    if ctm.b.abs() > 1e-10 || ctm.c.abs() > 1e-10 {
        return (0, 0);
    }
    // Compute device extent from unit square
    let combined = ctm.concat(&viewport.to_transform());
    let p0 = combined.transform_point(0.0, 0.0);
    let p1 = combined.transform_point(1.0, 1.0);
    let w = (p1.0 - p0.0).abs();
    let h = (p1.1 - p0.1).abs();
    if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 {
        return (0, 0);
    }
    (w.round() as u32, h.round() as u32)
}

/// Produce the complete image decode plan for an image XObject.
///
/// This is the main entry point called from the renderer before invoking
/// `scheduled_decode_image`. If the plan decision is `SkipOutsideViewport`,
/// the caller must not invoke the decoder.
pub(crate) fn plan_image_decode(
    metadata: &ImageMetadata,
    ctm: &Transform2D,
    viewport: &Viewport,
    base_cache_key: &str,
    high_quality: bool,
) -> ImageDecodePlan {
    let device_bounds = image_device_bounds(ctm, viewport);
    let intersects = if viewport.origin_x_px != 0 || viewport.origin_y_px != 0 {
        // A tile viewport carries a local device transform. Until the planner
        // tracks both global and tile-local bounds, fail open here rather than
        // incorrectly skipping an image that crosses this tile.
        true
    } else {
        device_bounds
            .as_ref()
            .map(|bounds| {
                bounds.x1 > 0
                    && bounds.x0 < viewport.width_px as i32
                    && bounds.y1 > 0
                    && bounds.y0 < viewport.height_px as i32
            })
            .unwrap_or(true)
    };

    let decision = if intersects {
        ImageDecodePlanDecision::DecodeRequired
    } else {
        ImageDecodePlanDecision::SkipOutsideViewport
    };

    let is_jpx = metadata.filters.iter().any(|f| f == "JPXDecode");
    let (target_width, target_height) = image_device_target_dimensions(ctm, viewport);

    let source_region = ImageSourceRegion::Full;
    let reduction_level = ImageReductionLevel::None;

    let contract = ImageContractState {
        target_width,
        target_height,
        high_quality,
        is_jpx,
        source_region,
        reduction_level,
    };

    let cache_key = ImageDecodeCacheKey {
        base_key: format!(
            "{base_cache_key}:obj:{}:{}:src:{}x{}:{}:cs:{}:mask:{}:inline:{}",
            metadata.object_number,
            metadata.generation_number,
            metadata.width,
            metadata.height,
            metadata.bits_per_component,
            metadata.color_space,
            metadata.is_mask,
            metadata.is_inline,
        ),
        contract,
    };

    ImageDecodePlan {
        decision,
        cache_key,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_viewport() -> Viewport {
        // A 100x100 pixel viewport at 72 DPI for a [0 0 100 100] media box.
        Viewport::new([0.0, 0.0, 100.0, 100.0], 72)
    }

    fn small_tile_viewport() -> Viewport {
        // A viewport representing a small tile: 50x50 pixels at offset (0,0)
        // for a [0 0 100 100] media box at 72 DPI, then restricted.
        let mut vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        vp.width_px = 50;
        vp.height_px = 50;
        vp.origin_x_px = 0;
        vp.origin_y_px = 0;
        vp
    }

    fn metadata_for_test() -> ImageMetadata {
        ImageMetadata {
            object_number: 5,
            generation_number: 0,
            width: 200,
            height: 200,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filters: vec!["FlateDecode".to_string()],
            is_mask: false,
            is_inline: false,
        }
    }

    #[test]
    fn image_inside_viewport_requires_decode() {
        let vp = test_viewport();
        // CTM places image at [10,10] with size 50x50 in page space
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn image_outside_viewport_skips_decode() {
        let vp = small_tile_viewport();
        // CTM places image entirely at x=[70..90], y=[70..90] in page space.
        // With a 50x50 pixel viewport at origin (0,0), the device pixels
        // for this image will be beyond pixel 50, hence outside.
        let ctm = Transform2D::new(20.0, 0.0, 0.0, 20.0, 70.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);
        assert_eq!(plan.decision, ImageDecodePlanDecision::SkipOutsideViewport);
    }

    #[test]
    fn cache_key_differs_for_different_target_dimensions() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";

        // Small CTM
        let ctm_small = Transform2D::new(30.0, 0.0, 0.0, 30.0, 10.0, 10.0);
        let plan_small = plan_image_decode(&meta, &ctm_small, &vp, base, false);

        // Large CTM
        let ctm_large = Transform2D::new(80.0, 0.0, 0.0, 80.0, 10.0, 10.0);
        let plan_large = plan_image_decode(&meta, &ctm_large, &vp, base, false);

        let key_small = plan_small.cache_key.to_cache_string();
        let key_large = plan_large.cache_key.to_cache_string();
        assert_ne!(
            key_small, key_large,
            "cache keys must differ for different target sizes"
        );
    }

    #[test]
    fn cache_key_differs_for_quality_mode() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let plan_compat = plan_image_decode(&meta, &ctm, &vp, base, false);
        let plan_hq = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_ne!(
            plan_compat.cache_key.to_cache_string(),
            plan_hq.cache_key.to_cache_string(),
            "cache keys must differ for different quality modes"
        );
    }

    #[test]
    fn cache_key_differs_for_jpx_filter() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let meta_flate = metadata_for_test();
        let mut meta_jpx = metadata_for_test();
        meta_jpx.filters = vec!["JPXDecode".to_string()];

        let plan_flate = plan_image_decode(&meta_flate, &ctm, &vp, base, false);
        let plan_jpx = plan_image_decode(&meta_jpx, &ctm, &vp, base, false);

        assert_ne!(
            plan_flate.cache_key.to_cache_string(),
            plan_jpx.cache_key.to_cache_string(),
            "cache keys must differ for JPX vs non-JPX"
        );
    }

    #[test]
    fn cache_key_includes_source_region_and_reduction() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        let key_str = plan.cache_key.to_cache_string();

        // Verify the key contains the source region and reduction markers.
        assert!(
            key_str.contains("sr"),
            "key must contain source region marker"
        );
        assert!(
            key_str.contains("rl"),
            "key must contain reduction level marker"
        );
    }

    #[test]
    fn degenerate_ctm_fails_open_allows_decode() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        // Zero CTM — degenerate, produces no valid bounds
        let ctm = Transform2D::new(0.0, 0.0, 0.0, 0.0, 50.0, 50.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        // Degenerate CTM should fail open (allow decode) rather than incorrectly
        // cull the image.
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn non_axis_aligned_ctm_gives_zero_target_dimensions() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        // Rotated CTM (b != 0, c != 0)
        let ctm = Transform2D::new(30.0, 20.0, -20.0, 30.0, 10.0, 10.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert_eq!(plan.cache_key.contract.target_width, 0);
        assert_eq!(plan.cache_key.contract.target_height, 0);
        // Still requires decode since it intersects
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn invisible_image_does_not_produce_cache_entry() {
        // This test verifies the contract: when plan_image_decode returns
        // SkipOutsideViewport, the caller knows NOT to invoke the decoder
        // or insert anything into the cache.
        let vp = small_tile_viewport(); // 50x50 at origin
                                        // Image entirely at x=[80..100], outside the viewport
        let ctm = Transform2D::new(20.0, 0.0, 0.0, 20.0, 80.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);

        assert_eq!(
            plan.decision,
            ImageDecodePlanDecision::SkipOutsideViewport,
            "image at x=[80..100] must be outside 50px-wide viewport"
        );
        // In the real renderer, this decision means scheduled_decode_image
        // is never called, so no cache entry is created and no decode work
        // is performed.
    }

    #[test]
    fn same_image_different_transform_produces_different_cache_keys() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";

        let ctm_a = Transform2D::new(40.0, 0.0, 0.0, 40.0, 5.0, 5.0);
        let ctm_b = Transform2D::new(60.0, 0.0, 0.0, 60.0, 5.0, 5.0);

        let plan_a = plan_image_decode(&meta, &ctm_a, &vp, base, false);
        let plan_b = plan_image_decode(&meta, &ctm_b, &vp, base, false);

        assert_ne!(
            plan_a.cache_key.to_cache_string(),
            plan_b.cache_key.to_cache_string(),
            "same image at different scales must have different cache keys"
        );
    }
}
