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
//! proof CMM transforms when the native CMM backend is available and the
//! rendering intent/profile shape is supported.

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
    /// Halftone screening is requested but not implemented.
    UnsupportedHalftone,
    /// Overprint semantics (PreserveSeparations) requested without native CMM.
    UnsupportedOverprintSeparations,
    /// Proof profile CMM routing requires native lcms2 backend.
    UnsupportedProofCmm,
    /// Halftone+Overprint combined semantics are not modeled.
    UnsupportedHalftoneOverprint,
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
    // Halftone screening is not implemented in any profile.
    if halftone == HalftonePolicy::Screen {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "halftone screening (HalftonePolicy::Screen) is not implemented; \
                     requesting it would produce incorrect raster output"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedHalftone,
        });
    }

    // PreserveSeparations overprint requires native CMM for correct ink behavior.
    if overprint == OverprintPolicy::PreserveSeparations
        && cmm_policy != ColorManagementPolicy::NativeLittleCms
    {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "OverprintPolicy::PreserveSeparations requires NativeLittleCms \
                     color management for correct ink separation behavior; current CMM \
                     policy cannot honor separation-preserving overprint"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedOverprintSeparations,
        });
    }

    // Proof profile with DeterministicFallback CMM cannot produce correct
    // proof rendering because it lacks ICC transform fidelity.
    if profile == PrintProfile::Proof && cmm_policy == ColorManagementPolicy::DeterministicFallback
    {
        return Err(PrintProfileRefusal {
            profile: print_profile_cache_label(profile).to_string(),
            reason: "PrintProfile::Proof requires at minimum PortableQcms color management; \
                     DeterministicFallback cannot produce correct proof-intent color transforms"
                .to_string(),
            category: PrintProfileRefusalCategory::UnsupportedProofCmm,
        });
    }

    Ok(())
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
    fn halftone_screen_is_refused_in_all_profiles() {
        for profile in [
            PrintProfile::Display,
            PrintProfile::Print,
            PrintProfile::Proof,
        ] {
            let result = validate_print_profile_prepress(
                profile,
                HalftonePolicy::Screen,
                OverprintPolicy::Disabled,
                ColorManagementPolicy::PortableQcms,
            );
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().category,
                PrintProfileRefusalCategory::UnsupportedHalftone
            );
        }
    }

    #[test]
    fn preserve_separations_requires_native_cmm() {
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

        // NativeLittleCms allows PreserveSeparations
        let result = validate_print_profile_prepress(
            PrintProfile::Print,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::NativeLittleCms,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn proof_refuses_deterministic_fallback_cmm() {
        let result = validate_print_profile_prepress(
            PrintProfile::Proof,
            HalftonePolicy::Disabled,
            OverprintPolicy::Disabled,
            ColorManagementPolicy::DeterministicFallback,
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().category,
            PrintProfileRefusalCategory::UnsupportedProofCmm
        );
    }

    #[test]
    fn proof_intent_override_requires_native_cmm() {
        assert_eq!(
            proof_intent_override(PrintProfile::Proof, true),
            Some(crate::render::contract::RenderingIntent::AbsoluteColorimetric)
        );
        assert_eq!(proof_intent_override(PrintProfile::Proof, false), None);
        assert_eq!(proof_intent_override(PrintProfile::Display, true), None);
        assert_eq!(proof_intent_override(PrintProfile::Print, true), None);
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

        // Proof with PortableQcms: valid (qcms is sufficient for proof)
        assert!(validate_print_profile_prepress(
            PrintProfile::Proof,
            HalftonePolicy::Disabled,
            OverprintPolicy::Disabled,
            ColorManagementPolicy::PortableQcms,
        )
        .is_ok());

        // Proof with NativeLittleCms and PreserveSeparations: valid
        assert!(validate_print_profile_prepress(
            PrintProfile::Proof,
            HalftonePolicy::Disabled,
            OverprintPolicy::PreserveSeparations,
            ColorManagementPolicy::NativeLittleCms,
        )
        .is_ok());
    }
}
