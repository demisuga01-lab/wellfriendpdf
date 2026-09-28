//! OpenType **variable font** (OTVar) instance selection.
//!
//! A variable font stores a continuum of styles in one program; a concrete
//! *instance* is chosen by coordinates along design axes (`wght`, `wdth`,
//! `slnt`, `ital`, `opsz`, or custom axes), and glyph outlines are interpolated
//! from per-master deltas. This module decides **which instance** to render and
//! applies it to a [`ttf_parser::Face`]. The face supplies gvar outlines,
//! avar-normalized coordinates and supported variation-adjusted metrics. CFF2
//! outlines use the SDK's bounded, per-glyph-dictionary decoder through
//! `fonts::sfnt_outline`. These are rendering coordinates, not a persisted
//! whole-font static instancing operation.
//!
//! # How the instance is selected
//!
//! PDF has **no** standard channel to pass arbitrary axis coordinates to an
//! embedded font program, so in practice the instance comes from one of:
//!
//! 1. **Pre-instanced** — the producer flattened the variable font to a static
//!    instance before embedding (no `fvar` table). This is overwhelmingly the
//!    common case. Such fonts are not variable and use the static path.
//! 2. **Default instance** — a true variable font embedded as-is. `ttf-parser`
//!    initializes coordinates to the font's default (all normalized to 0), so
//!    no additional axis selection is needed. This does not establish complete
//!    rendering support for every variable-font table.
//! 3. **PDF-descriptor-selected** — the `FontDescriptor` carries `/FontWeight`
//!    (100–900) and/or `/FontStretch` (a name like `/Condensed`). When the
//!    embedded font is variable and exposes a matching `wght`/`wdth` axis, those
//!    descriptor values select the instance — the one case where Wellfriend can
//!    honor a *non-default* instance from PDF metadata. This is the genuine
//!    correctness win this module adds.
//!
//! Determinism: identical (font bytes, request) → identical coordinates →
//! identical outline. Validation uses bounded axis metadata; all
//! coordinate values are clamped to each axis's `[min, max]` by `ttf-parser`.

use ttf_parser::{Face, Tag};

/// `wght` axis (CSS weight 1–1000; PDF `/FontWeight` 100–900).
pub const AXIS_WGHT: Tag = Tag::from_bytes(b"wght");
/// `wdth` axis (width percentage; 100 = normal).
pub const AXIS_WDTH: Tag = Tag::from_bytes(b"wdth");

/// One axis coordinate to pin (user-space value, not normalized).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisValue {
    pub tag: Tag,
    pub value: f32,
}

/// The intended variable-font instance: a small set of axis coordinates to pin.
/// An empty request means "render the font's default instance" (the no-op case).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VariationRequest {
    axes: Vec<AxisValue>,
}

impl VariationRequest {
    /// A request that pins nothing — the default instance.
    pub fn none() -> Self {
        Self { axes: Vec::new() }
    }

    /// True if this request pins no axes (the default instance / no-op).
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty()
    }

    /// Pin an axis to a user-space value (overwrites a prior value for the tag).
    pub fn with_axis(mut self, tag: Tag, value: f32) -> Self {
        if let Some(existing) = self.axes.iter_mut().find(|a| a.tag == tag) {
            existing.value = value;
        } else {
            self.axes.push(AxisValue { tag, value });
        }
        self
    }

    /// The pinned axes.
    pub fn axes(&self) -> &[AxisValue] {
        &self.axes
    }

    /// A stable, order-independent hash of the pinned coordinates, for use as
    /// part of a glyph-cache key (so two instances of the same font program do
    /// not collide). `0` for the empty request (the default instance), keeping
    /// the cache key identical to the pre-variation behaviour for static fonts.
    pub fn cache_hash(&self) -> u64 {
        if self.axes.is_empty() {
            return 0;
        }
        // Sort by tag so the hash is independent of insertion order.
        let mut sorted = self.axes.clone();
        sorted.sort_by_key(|a| a.tag.0);
        // FNV-1a over (tag, value-bits) pairs.
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |x: u64| {
            for byte in x.to_le_bytes() {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for av in &sorted {
            mix(u64::from(av.tag.0));
            mix(u64::from(av.value.to_bits()));
        }
        h
    }

    /// Build a request from PDF `FontDescriptor` values: `/FontWeight` → `wght`,
    /// `/FontStretch` → `wdth`. Returns an empty request when neither descriptor
    /// value is present. Normal CSS/PDF values are not
    /// necessarily the selected font's defaults and must not be discarded.
    pub fn from_descriptor(font_weight: Option<f64>, font_stretch: Option<&str>) -> Self {
        let mut req = Self::none();
        // Keep explicit normal values too: a font may default to wght=700.
        if let Some(w) = font_weight {
            if w.is_finite() && (1.0..=1000.0).contains(&w) {
                req = req.with_axis(AXIS_WGHT, w as f32);
            }
        }
        if let Some(stretch) = font_stretch {
            if let Some(pct) = font_stretch_percent(stretch) {
                req = req.with_axis(AXIS_WDTH, pct);
            }
        }
        req
    }
}

/// Map a PDF `/FontStretch` name to a `wdth`-axis percentage (per the OpenType
/// `wdth` axis registration / CSS `font-stretch` keyword table).
pub fn font_stretch_percent(name: &str) -> Option<f32> {
    let n = name.trim_start_matches('/');
    Some(match n {
        "UltraCondensed" => 50.0,
        "ExtraCondensed" => 62.5,
        "Condensed" => 75.0,
        "SemiCondensed" => 87.5,
        "Normal" => 100.0,
        "SemiExpanded" => 112.5,
        "Expanded" => 125.0,
        "ExtraExpanded" => 150.0,
        "UltraExpanded" => 200.0,
        _ => return None,
    })
}

/// Whether the font program is an OpenType variable font (has an `fvar` table
/// with at least one axis). Bare-CFF / Type1 programs and pre-instanced static
/// fonts return `false`.
pub fn is_variable(font_bytes: &[u8]) -> bool {
    Face::parse(font_bytes, 0)
        .map(|f| f.is_variable())
        .unwrap_or(false)
}

/// The font's variation axes as `(tag, min, default, max)`, empty when not
/// variable. Useful for diagnostics and for clamping a request to real axes.
pub fn axes(font_bytes: &[u8]) -> Vec<(Tag, f32, f32, f32)> {
    let Ok(face) = Face::parse(font_bytes, 0) else {
        return Vec::new();
    };
    face.variation_axes()
        .into_iter()
        .map(|a| (a.tag, a.min_value, a.def_value, a.max_value))
        .collect()
}

/// Apply a [`VariationRequest`] to a mutable face. Only axes the font actually
/// exposes are set (others are ignored). Returns `true` if at least one axis was
/// applied (which need not differ from the font's default).
///
/// `set_variation` clamps each value to the axis `[min, max]` and applies `avar`
/// normalization internally, so callers may pass raw user-space values.
pub fn apply_request(face: &mut Face, request: &VariationRequest) -> bool {
    apply_request_checked(face, request).unwrap_or(false)
}

/// Atomic checked selection for render/instance paths. Unknown request axes
/// remain ignorable for descriptor-based fallback fonts, but malformed tables,
/// non-finite values and backend limits are not silently treated as defaults.
pub fn apply_request_checked(face: &mut Face, request: &VariationRequest) -> crate::Result<bool> {
    use crate::WellfriendError;
    crate::cancel::check_current_cancel("font instance coordinates")?;
    if request.axes.len() > 64 {
        return Err(WellfriendError::ResourceLimit(
            "font coordinate request exceeds 64 axes".into(),
        ));
    }
    if request.axes.iter().any(|axis| !axis.value.is_finite()) {
        return Err(WellfriendError::invalid_input(
            "non-finite font axis coordinate",
        ));
    }
    if request.is_empty() {
        return Ok(false);
    }
    if super::instance_coordinates::validate(face)? == 0 {
        return Ok(false);
    }
    let tags = face
        .variation_axes()
        .into_iter()
        .map(|a| a.tag.0)
        .collect::<std::collections::BTreeSet<_>>();
    let mut selected = face.clone();
    let mut applied = false;
    for av in request.axes() {
        if tags.contains(&av.tag.0) {
            if selected.set_variation(av.tag, av.value).is_none() {
                return Err(WellfriendError::UnsupportedFeature(
                    "font axis count exceeds this parser's coordinate capacity".into(),
                ));
            }
            applied = true;
        }
    }
    crate::cancel::check_current_cancel("font coordinate publication")?;
    *face = selected;
    Ok(applied)
}

#[cfg(test)]
#[path = "instance_coordinate_tests.rs"]
mod coordinate_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_normal_values_are_explicit_not_assumed_font_defaults() {
        let req = VariationRequest::from_descriptor(Some(400.0), Some("Normal"));
        assert_eq!(
            req.axes(),
            &[
                AxisValue {
                    tag: AXIS_WGHT,
                    value: 400.
                },
                AxisValue {
                    tag: AXIS_WDTH,
                    value: 100.
                }
            ]
        );
    }

    #[test]
    fn descriptor_bold_pins_wght() {
        let req = VariationRequest::from_descriptor(Some(700.0), None);
        assert_eq!(
            req.axes(),
            &[AxisValue {
                tag: AXIS_WGHT,
                value: 700.0
            }]
        );
    }

    #[test]
    fn descriptor_condensed_pins_wdth() {
        let req = VariationRequest::from_descriptor(None, Some("Condensed"));
        assert_eq!(
            req.axes(),
            &[AxisValue {
                tag: AXIS_WDTH,
                value: 75.0
            }]
        );
    }

    #[test]
    fn descriptor_bold_condensed_pins_both() {
        let req = VariationRequest::from_descriptor(Some(800.0), Some("/SemiCondensed"));
        assert_eq!(
            req.axes(),
            &[
                AxisValue {
                    tag: AXIS_WGHT,
                    value: 800.0
                },
                AxisValue {
                    tag: AXIS_WDTH,
                    value: 87.5
                },
            ]
        );
    }

    #[test]
    fn out_of_range_weight_ignored() {
        assert!(VariationRequest::from_descriptor(Some(0.0), None).is_empty());
        assert!(VariationRequest::from_descriptor(Some(5000.0), None).is_empty());
        assert!(VariationRequest::from_descriptor(Some(f64::NAN), None).is_empty());
    }

    #[test]
    fn unknown_stretch_ignored() {
        assert!(VariationRequest::from_descriptor(None, Some("Wonky")).is_empty());
    }

    #[test]
    fn with_axis_overwrites() {
        let req = VariationRequest::none()
            .with_axis(AXIS_WGHT, 300.0)
            .with_axis(AXIS_WGHT, 700.0);
        assert_eq!(
            req.axes(),
            &[AxisValue {
                tag: AXIS_WGHT,
                value: 700.0
            }]
        );
    }

    #[test]
    fn non_variable_bytes_report_false() {
        // A non-font byte blob is not variable.
        assert!(!is_variable(b"not a font"));
        assert!(axes(b"not a font").is_empty());
    }
}
