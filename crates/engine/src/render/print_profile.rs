//! Active PrintProfile semantics for annotation visibility, cache identity,
//! and prepress/proof CMM routing.
//!
//! PDF annotation flags (ISO 32000-2, Table 167) define per-annotation
//! visibility for Display vs Print contexts:
//!
//! - Bit 3 (Print, value 4): When set, the annotation SHALL be printed when
//!   the page is printed. When clear, it SHALL NOT be printed.
//! - Bit 4 (NoZoom, value 8) and Bit 5 (NoRotate, value 16) do not affect
//!   visibility but affect appearance transforms — not modeled here.
//! - Bit 2 (Hidden, value 2): When set, the annotation SHALL NOT be displayed
//!   or printed, regardless of other flags.
//! - Bit 1 (Invisible, value 1): Deprecated non-standard invisibility.
//! - Bit 6 (NoView, value 32): The annotation SHALL NOT be displayed on
//!   screen. If Print is also set, it is printed but not viewed.
//! - Bit 7 (ReadOnly, value 64): does not affect visibility.
//! - Bit 8 (Locked, value 128): does not affect visibility.
//! - Bit 9 (ToggleNoView, value 256): content visible but inverted w.r.t. NoView.
//!
//! Display mode shows annotations that are not Hidden, not Invisible (legacy),
//! and not NoView (the existing renderer baseline behavior).
//!
//! Print mode shows annotations that are not Hidden and have the Print flag
//! set. NoView annotations with Print set ARE shown in print mode.
//!
//! Proof mode uses Print visibility (same as Print) plus routes color through
//! output-intent proof CMM transforms when the native CMM backend is available
//! and the rendering intent/profile shape is supported.

use crate::render::buffer::PixelBuffer;
use crate::render::cmm;
use crate::render::contract::{
    ColorManagementPolicy, HalftonePolicy, OverprintPolicy, PrintProfile,
};

/// PDF annotation flag bits (ISO 32000-2, Table 167).
pub(crate) mod annotation_flags {
    pub const INVISIBLE: i64 = 1 << 0;
    pub const HIDDEN: i64 = 1 << 1;
    pub const PRINT: i64 = 1 << 2;
    pub const NO_VIEW: i64 = 1 << 5;
}

/// Determines whether an annotation with the given /F flags value should be
/// rendered for the active `PrintProfile`.
///
/// Returns `true` if the annotation is *visible* in the given profile;
/// `false` if it should be excluded from rendering.
pub(crate) fn annotation_visible_for_profile(flags: i64, profile: PrintProfile) -> bool {
    // Hidden overrides everything regardless of profile.
    if flags & annotation_flags::HIDDEN != 0 {
        return false;
    }
    match profile {
        PrintProfile::Display => {
            // Display mode: exclude Invisible (legacy) and NoView annotations.
            if flags & annotation_flags::INVISIBLE != 0 {
                return false;
            }
            if flags & annotation_flags::NO_VIEW != 0 {
                return false;
            }
            true
        }
        PrintProfile::Print | PrintProfile::Proof => {
            // Print/Proof mode: show only annotations with the Print flag.
            // NoView does NOT exclude in print — only the absence of Print does.
            // Invisible (legacy bit) also does not exclude if Print is set,
            // per ISO 32000-2 where Print flag is the definitive print-visibility
            // control for well-formed annotations.
            flags & annotation_flags::PRINT != 0
        }
    }
}

/// Returns a short stable string label for the `PrintProfile` suitable for
/// inclusion in cache identity fingerprints.
pub(crate) fn print_profile_cache_label(profile: PrintProfile) -> &'static str {
    match profile {
        PrintProfile::Display => "display",
        PrintProfile::Print => "print",
        PrintProfile::Proof => "proof",
    }
}

/// Typed refusal for unsupported prepress semantics that cannot be silently
/// ignored without producing incorrect output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrintProfileRefusal {
    pub profile: String,
    pub reason: String,
    pub category: PrintProfileRefusalCategory,
}

/// Categories of unsupported prepress semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintProfileRefusalCategory {
    /// Halftone screening cannot be combined with separation-preserving output.
    UnsupportedHalftone,
    /// Separation-preserving overprint requested for a raster target that cannot
    /// retain process and named plates as output channels.
    UnsupportedOverprintSeparations,
    /// Proof profile CMM routing requires native lcms2 backend.
    UnsupportedProofCmm,
    /// NativeLittleCms was requested but this build/target has no native CMM.
    UnsupportedNativeCmmBackend,
    /// Halftone+Overprint combined semantics are not modeled.
    UnsupportedHalftoneOverprint,
}

/// Output surface family used when validating print/prepress policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintOutputSurfaceKind {
    /// Normal render contracts that return bounded RGB/BGRA/RGBA/BGR/gray rows.
    RgbGrayRaster,
    /// The prepress `SeparationFramebuffer` N-channel plate surface.
    SeparationFramebuffer,
}

/// Validate that the requested print profile + prepress policy combination is
/// implementable by the current engine configuration. Returns `Ok(())` when
/// the combination is supported, or a typed refusal describing why the
/// combination cannot be honored correctly.
///
/// This is called during contract compilation so that unsupported semantics
/// produce explicit errors rather than silently rendering with incorrect color.
pub(crate) fn validate_print_profile_prepress(
    profile: PrintProfile,
    halftone: HalftonePolicy,
    overprint: OverprintPolicy,
    cmm_policy: ColorManagementPolicy,
) -> Result<(), PrintProfileRefusal> {
    validate_print_profile_prepress_for_surface(
        profile,
        halftone,
        overprint,
        cmm_policy,
        PrintOutputSurfaceKind::RgbGrayRaster,
    )
}

/// Validate print/prepress policy for a specific output surface family.
///
/// The regular render-contract surface is RGB/gray and therefore cannot accept
/// `PreserveSeparations`. The prepress plate report path owns a bounded
/// `SeparationFramebuffer` with deterministic N-channel plane labels, so it can
/// preserve process and named plate samples without pretending to be an RGB
/// raster render.
pub(crate) fn validate_print_profile_prepress_for_surface(
    profile: PrintProfile,
    halftone: HalftonePolicy,
    overprint: OverprintPolicy,
    cmm_policy: ColorManagementPolicy,
    surface: PrintOutputSurfaceKind,
) -> Result<(), PrintProfileRefusal> {
    // RGB ordered-screen halftoning is implemented for raster surfaces. It must
    // not be used for PreserveSeparations because that policy requires keeping
    // ink channels semantically separate.
    if halftone == HalftonePolicy::Screen && overprint == OverprintPolicy::PreserveSeparations {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "HalftonePolicy::Screen cannot be combined with \
                     OverprintPolicy::PreserveSeparations because the active \
                     raster screen is RGB/page-surface based, not separation \
                     preserving"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedHalftone,
        });
    }

    // PreserveSeparations cannot be represented by RGB/gray raster outputs.
    // The separate prepress N-channel framebuffer is the source path that can
    // retain process and named plates as semantic output channels.
    if overprint == OverprintPolicy::PreserveSeparations
        && surface == PrintOutputSurfaceKind::RgbGrayRaster
    {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "OverprintPolicy::PreserveSeparations requires a separation-preserving \
                     output surface; active render contracts produce bounded RGB/gray raster \
                     output and expose Separation/DeviceN plate data through the prepress \
                     plate report instead"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedOverprintSeparations,
        });
    }

    // A contract that explicitly selects NativeLittleCms must not pass
    // validation in default/wasm builds that can only execute qcms fallback.
    if cmm_policy == ColorManagementPolicy::NativeLittleCms && !cmm::native_cmm_status().available {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "ColorManagementPolicy::NativeLittleCms requires the native lcms2 backend, \
                     but the active build/target reports no available native CMM"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedNativeCmmBackend,
        });
    }

    // Proof profile requires native output-intent proofing. Portable qcms and
    // deterministic fallback cannot produce the active proof transform.
    if profile == PrintProfile::Proof && cmm_policy != ColorManagementPolicy::NativeLittleCms {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "PrintProfile::Proof requires ColorManagementPolicy::NativeLittleCms; \
                     portable/deterministic CMM cannot honor output-intent proofing"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedProofCmm,
        });
    }

    Ok(())
}

pub(crate) fn apply_ordered_halftone_screen(
    buffer: &mut PixelBuffer,
    page_origin_x: u32,
    page_origin_y: u32,
) {
    const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let width = buffer.width as usize;
    let origin_x = page_origin_x as usize;
    let origin_y = page_origin_y as usize;
    for (index, px) in buffer.rgba_bytes_mut().chunks_exact_mut(4).enumerate() {
        let x = index % width;
        let y = index / width;
        let threshold = u16::from(BAYER_4X4[(origin_y + y) & 3][(origin_x + x) & 3]) * 16 + 8;
        px[0] = screen_channel(px[0], threshold);
        px[1] = screen_channel(px[1], threshold);
        px[2] = screen_channel(px[2], threshold);
    }
}

#[inline]
fn screen_channel(channel: u8, threshold: u16) -> u8 {
    if u16::from(channel) > threshold {
        255
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_annotation_excluded_in_all_profiles() {
        let flags = annotation_flags::HIDDEN | annotation_flags::PRINT;
        assert!(!annotation_visible_for_profile(
            flags,
            PrintProfile::Display
        ));
        assert!(!annotation_visible_for_profile(flags, PrintProfile::Print));
        assert!(!annotation_visible_for_profile(flags, PrintProfile::Proof));
    }

    #[test]
    fn display_excludes_no_view_and_invisible() {
        assert!(!annotation_visible_for_profile(
            annotation_flags::NO_VIEW,
            PrintProfile::Display
        ));
        assert!(!annotation_visible_for_profile(
            annotation_flags::INVISIBLE,
            PrintProfile::Display
        ));
        // NoView + Print is hidden on display
        assert!(!annotation_visible_for_profile(
            annotation_flags::NO_VIEW | annotation_flags::PRINT,
            PrintProfile::Display
        ));
    }

    #[test]
    fn display_shows_normal_and_print_flagged_annotations() {
        // Normal annotation with no flags set: visible on display
        assert!(annotation_visible_for_profile(0, PrintProfile::Display));
        // Annotation with Print flag: also visible on display
        assert!(annotation_visible_for_profile(
            annotation_flags::PRINT,
            PrintProfile::Display
        ));
    }

    #[test]
    fn print_requires_print_flag() {
        // No Print flag: excluded from print
        assert!(!annotation_visible_for_profile(0, PrintProfile::Print));
        // Print flag set: included
        assert!(annotation_visible_for_profile(
            annotation_flags::PRINT,
            PrintProfile::Print
        ));
        // NoView + Print: visible in print (print-only annotation)
        assert!(annotation_visible_for_profile(
            annotation_flags::NO_VIEW | annotation_flags::PRINT,
            PrintProfile::Print
        ));
    }

    #[test]
    fn proof_uses_same_visibility_as_print() {
        assert!(!annotation_visible_for_profile(0, PrintProfile::Proof));
        assert!(annotation_visible_for_profile(
            annotation_flags::PRINT,
            PrintProfile::Proof
        ));
        assert!(annotation_visible_for_profile(
            annotation_flags::NO_VIEW | annotation_flags::PRINT,
            PrintProfile::Proof
        ));
    }

    #[test]
    fn cache_labels_are_distinct() {
        let labels: Vec<_> = [
            PrintProfile::Display,
            PrintProfile::Print,
            PrintProfile::Proof,
        ]
        .iter()
        .map(|p| print_profile_cache_label(*p))
        .collect();
        assert_eq!(labels.len(), 3);
        assert!(labels[0] != labels[1]);
        assert!(labels[1] != labels[2]);
        assert!(labels[0] != labels[2]);
    }

    #[test]
    fn halftone_screen_is_supported_without_separation_preservation() {
        for profile in [PrintProfile::Display, PrintProfile::Print] {
            let result = validate_print_profile_prepress(
                profile,
                HalftonePolicy::Screen,
                OverprintPolicy::Disabled,
                ColorManagementPolicy::PortableQcms,
            );
            assert!(result.is_ok());
        }
        let proof = validate_print_profile_prepress(
            PrintProfile::Proof,
            HalftonePolicy::Screen,
            OverprintPolicy::Disabled,
            ColorManagementPolicy::PortableQcms,
        );
        assert_eq!(
            proof
                .expect_err("Proof still requires native output-intent CMM")
                .category,
            PrintProfileRefusalCategory::UnsupportedProofCmm
        );
    }

    #[test]
    fn halftone_screen_refuses_preserve_separations() {
        let result = validate_print_profile_prepress(
            PrintProfile::Print,
            HalftonePolicy::Screen,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::NativeLittleCms,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().category,
            PrintProfileRefusalCategory::UnsupportedHalftone
        );
    }

    #[test]
    fn ordered_halftone_screen_is_deterministic_and_preserves_alpha() {
        let mut buffer = PixelBuffer::new_filled(4, 4, [128, 96, 64, 37]);
        apply_ordered_halftone_screen(&mut buffer, 0, 0);
        let pixels = buffer.rgba_bytes();
        assert!(pixels.chunks_exact(4).any(|px| px[0] == 0));
        assert!(pixels.chunks_exact(4).any(|px| px[0] == 255));
        assert!(pixels.chunks_exact(4).all(|px| px[3] == 37));
    }

    #[test]
    fn preserve_separations_requires_separation_preserving_output_surface() {
        let result = validate_print_profile_prepress(
            PrintProfile::Print,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::PortableQcms,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().category,
            PrintProfileRefusalCategory::UnsupportedOverprintSeparations
        );

        let result = validate_print_profile_prepress(
            PrintProfile::Print,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::NativeLittleCms,
        );
        let refusal =
            result.expect_err("native CMM alone must not imply separation-preserving output");
        assert_eq!(
            refusal.category,
            PrintProfileRefusalCategory::UnsupportedOverprintSeparations
        );
        assert!(refusal.reason.contains("separation-preserving output"));
    }

    #[test]
    fn separation_framebuffer_surface_accepts_preserve_separations() {
        let result = validate_print_profile_prepress_for_surface(
            PrintProfile::Print,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::PortableQcms,
            PrintOutputSurfaceKind::SeparationFramebuffer,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn separation_framebuffer_surface_still_refuses_rgb_halftone_preserve_separations() {
        let result = validate_print_profile_prepress_for_surface(
            PrintProfile::Print,
            HalftonePolicy::Screen,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::PortableQcms,
            PrintOutputSurfaceKind::SeparationFramebuffer,
        );
        assert_eq!(
            result.unwrap_err().category,
            PrintProfileRefusalCategory::UnsupportedHalftone
        );
    }

    #[test]
    fn proof_separation_framebuffer_keeps_native_cmm_requirement() {
        let result = validate_print_profile_prepress_for_surface(
            PrintProfile::Proof,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::PortableQcms,
            PrintOutputSurfaceKind::SeparationFramebuffer,
        );
        assert_eq!(
            result.unwrap_err().category,
            PrintProfileRefusalCategory::UnsupportedProofCmm
        );
    }

    #[test]
    fn native_littlecms_policy_requires_available_backend() {
        let result = validate_print_profile_prepress(
            PrintProfile::Display,
            HalftonePolicy::Disabled,
            OverprintPolicy::Disabled,
            ColorManagementPolicy::NativeLittleCms,
        );
        if cmm::native_cmm_status().available {
            assert!(result.is_ok());
        } else {
            let refusal = result.expect_err("unavailable NativeLittleCms must be refused");
            assert_eq!(
                refusal.category,
                PrintProfileRefusalCategory::UnsupportedNativeCmmBackend
            );
            assert!(refusal.reason.contains("native lcms2 backend"));
        }
    }

    #[test]
    fn proof_refuses_deterministic_fallback_cmm() {
        for policy in [
            ColorManagementPolicy::PortableQcms,
            ColorManagementPolicy::DeterministicFallback,
        ] {
            let result = validate_print_profile_prepress(
                PrintProfile::Proof,
                HalftonePolicy::Disabled,
                OverprintPolicy::Disabled,
                policy,
            );
            assert!(result.is_err(), "Proof must refuse {policy:?}");
            assert_eq!(
                result.unwrap_err().category,
                PrintProfileRefusalCategory::UnsupportedProofCmm
            );
        }
    }

    #[test]
    fn valid_combinations_pass() {
        // Display with defaults: always valid
        assert!(validate_print_profile_prepress(
            PrintProfile::Display,
            HalftonePolicy::Disabled,
            OverprintPolicy::Disabled,
            ColorManagementPolicy::PortableQcms,
        )
        .is_ok());

        // Print with overprint preview (not PreserveSeparations): valid
        assert!(validate_print_profile_prepress(
            PrintProfile::Print,
            HalftonePolicy::Disabled,
            OverprintPolicy::Preview,
            ColorManagementPolicy::PortableQcms,
        )
        .is_ok());

        // PreserveSeparations remains invalid even with NativeLittleCms because
        // the active render target is RGB/gray raster output, not n-channel ink.
        let native_result = validate_print_profile_prepress(
            PrintProfile::Proof,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::NativeLittleCms,
        );
        assert_eq!(
            native_result
                .expect_err("PreserveSeparations must fail closed")
                .category,
            PrintProfileRefusalCategory::UnsupportedOverprintSeparations
        );
    }
}
