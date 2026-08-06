//! Shared vector output fallback classifier for SVG and PostScript sinks.
//!
//! This module decides whether a page must be fully rasterized, can be emitted
//! as pure vector, or can use a **regional fallback** where simple axis-aligned
//! Image XObject `Do` operations are embedded as bounded raster regions while
//! surrounding vector content (paths, text, clips) is preserved natively.
//!
//! # Regional fallback scope
//!
//! Regional image embedding is permitted only when:
//! - The `Do` operand names an Image XObject (not a Form XObject).
//! - The current transform matrix at the point of the `Do` is axis-aligned
//!   (no rotation/skew), so the image occupies a simple rectangular region.
//! - The image bounds are resolvable from the CTM and the image dimensions.
//!
//! # Whole-page fallback is retained when:
//! - The page uses Form XObjects (`Do` with Subtype Form).
//! - Named shadings (`sh`), inline images, or pattern colour spaces appear.
//! - An ExtGState operator (`gs`) is present (soft masks, blend modes).
//! - Semantic ordering makes regional embedding unsafe (e.g., overlapping
//!   transparency groups interleaving image and vector content).
//! - Any image XObject has a non-axis-aligned CTM at invocation.

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::GraphicsState;
use crate::engine::PageResources;

/// Maximum text-showing operations before the SVG sibling renderer falls back
/// to whole-page rasterization (bounded vector-text scope).
pub const MAX_VECTOR_TEXT_SHOWING_OPS: usize = 64;

/// Classification of a page's content for vector output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorFallbackDecision {
    /// Page is pure vector: no images, no unsupported operations.
    PureVector,
    /// Page contains simple axis-aligned image XObjects that can be regionally
    /// embedded while preserving surrounding vector content natively.
    RegionalImageFallback {
        /// Names of Image XObjects that will be regionally embedded.
        image_names: Vec<String>,
    },
    /// Page must be fully rasterized (unsupported constructs present).
    WholePageRaster {
        /// Reason the page requires full rasterization.
        reason: &'static str,
    },
}

/// Result of checking a single `Do` operation for regional-embed eligibility.
#[derive(Debug, Clone)]
pub struct ImageDoClassification {
    /// The XObject resource name from the `Do` operand.
    pub name: String,
    /// Whether this is a simple axis-aligned Image XObject eligible for
    /// regional embedding.
    pub eligible: bool,
    /// If eligible, the device-space bounding box [x, y, width, height] where
    /// the image will be placed.
    pub device_rect: Option<[f64; 4]>,
}

/// Classify a page's operations for vector output, using resource information
/// to distinguish Image XObjects from Form XObjects.
///
/// This is the authoritative fallback decision shared by SVG and PostScript
/// sinks. It replaces the older per-sink `needs_raster_fallback` functions.
pub fn classify_page_for_vector_output(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
) -> VectorFallbackDecision {
    let mut image_do_ops: Vec<ImageDoClassification> = Vec::new();
    let mut gs = GraphicsState::default();
    let mut text_showing_ops = 0usize;

    for op in ops {
        match op.operator.as_str() {
            // Inline images: whole-page fallback (complex data interleaved with ops).
            "BI" | "ID" | "EI" | "inline_image_data" => {
                return VectorFallbackDecision::WholePageRaster {
                    reason: "inline image",
                };
            }
            // Named shading fill: whole-page fallback.
            "sh" => {
                return VectorFallbackDecision::WholePageRaster {
                    reason: "named shading (sh)",
                };
            }
            // ExtGState: may carry soft mask, blend mode, transfer function —
            // the vector sinks don't resolve the dictionary, so whole-page.
            "gs" => {
                return VectorFallbackDecision::WholePageRaster {
                    reason: "ExtGState (gs)",
                };
            }
            // Pattern fill/stroke: whole-page fallback.
            "scn" | "SCN" if op.operands.iter().any(|o| matches!(o, Operand::Name(_))) => {
                return VectorFallbackDecision::WholePageRaster {
                    reason: "pattern colour space",
                };
            }
            // XObject invocation: classify Image vs Form.
            "Do" => {
                let name = match op.operands.first() {
                    Some(Operand::Name(n)) => n.clone(),
                    _ => {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "Do without resolvable name",
                        };
                    }
                };
                // Check subtype from resources.
                let subtype = resources.xobject_subtypes.get(&name);
                match subtype.map(|s| s.as_str()) {
                    Some("Image") => {
                        // Check axis-alignment of the current CTM.
                        let classification =
                            classify_image_do(&name, &gs, resources, viewport_scale);
                        if !classification.eligible {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "non-axis-aligned or unresolvable image XObject",
                            };
                        }
                        image_do_ops.push(classification);
                    }
                    Some("Form") | Some("PS") => {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "Form XObject",
                        };
                    }
                    _ => {
                        // Unknown subtype or missing — fail closed.
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "XObject with unknown or missing Subtype",
                        };
                    }
                }
            }
            // Text operations: bounded scope for vector sinks.
            "Tj" | "TJ" | "'" | "\"" => {
                text_showing_ops = text_showing_ops.saturating_add(1);
                if text_showing_ops > MAX_VECTOR_TEXT_SHOWING_OPS {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "text-showing operations exceed vector scope",
                    };
                }
            }
            _ => {}
        }
        // Track graphics state for CTM analysis.
        gs.process(op);
    }

    if image_do_ops.is_empty() {
        VectorFallbackDecision::PureVector
    } else {
        VectorFallbackDecision::RegionalImageFallback {
            image_names: image_do_ops.into_iter().map(|c| c.name).collect(),
        }
    }
}

/// Check whether a `Do` invocation of a named Image XObject is eligible for
/// regional embedding: the CTM must be axis-aligned (no rotation/skew) and the
/// image dimensions must be resolvable so we can compute the device rectangle.
fn classify_image_do(
    name: &str,
    gs: &GraphicsState,
    _resources: &PageResources,
    viewport_scale: f64,
) -> ImageDoClassification {
    let ctm = gs.ctm;
    // PDF image XObjects are drawn in the unit square [0,0]-[1,1] mapped by the
    // CTM. An axis-aligned placement has ctm[1] ≈ 0 and ctm[2] ≈ 0 (no shear/rotation).
    let shear_threshold = 1e-6;
    let is_axis_aligned = ctm[1].abs() < shear_threshold && ctm[2].abs() < shear_threshold;

    if !is_axis_aligned {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    }

    // The CTM maps the unit square to the image placement on the page.
    // In PDF, images paint into [0,0]-[1,1] which the CTM transforms.
    // For axis-aligned: width = |ctm[0]|, height = |ctm[3]|, origin = (ctm[4], ctm[5]).
    let w = ctm[0].abs() * viewport_scale;
    let h = ctm[3].abs() * viewport_scale;
    let x = ctm[4] * viewport_scale;
    // PDF y-axis is bottom-up; the device y needs adjustment by the page height,
    // but the caller handles that. We store in page-user-space scaled to device pixels.
    let y = ctm[5] * viewport_scale;

    // Sanity: image must have positive dimensions.
    if w < 1.0 || h < 1.0 || !w.is_finite() || !h.is_finite() || !x.is_finite() || !y.is_finite() {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    }

    ImageDoClassification {
        name: name.to_string(),
        eligible: true,
        device_rect: Some([x, y, w, h]),
    }
}

/// Compute the device-space rectangle for an axis-aligned image `Do` operation,
/// accounting for the PDF bottom-up to device top-down coordinate flip.
///
/// Returns `(x, y, width, height)` in device-pixel coordinates (top-left origin,
/// y-down — the same space both the SVG and PS sinks emit geometry in).
pub fn image_device_rect(
    gs: &GraphicsState,
    viewport_scale: f64,
    page_height_px: f64,
) -> Option<[f64; 4]> {
    let ctm = gs.ctm;
    let shear_threshold = 1e-6;
    if ctm[1].abs() >= shear_threshold || ctm[2].abs() >= shear_threshold {
        return None;
    }

    let w = ctm[0].abs() * viewport_scale;
    let h = ctm[3].abs() * viewport_scale;

    // The origin in PDF user space (bottom-left up). ctm[4], ctm[5] give the
    // bottom-left corner of the image placement in user space.
    // In device space (y-down from top-left), we flip:
    //   device_y = page_height_px - (user_y + image_height_in_device)
    // But we need to be careful: ctm[3] can be negative (common for images),
    // meaning the image is flipped vertically in PDF user space.
    let (user_x, user_y_bottom) = (ctm[4] * viewport_scale, ctm[5] * viewport_scale);

    // If ctm[3] < 0, the image is painted "upside down" from the PDF origin,
    // which is the normal convention (images have origin at top-left of their
    // data, ctm[3] is negative to flip them into the page's bottom-up space).
    let device_y_top = if ctm[3] < 0.0 {
        // Normal case: image data top = user_y_bottom (which is actually the
        // top of the image in device-y-down space after page flip).
        page_height_px - user_y_bottom
    } else {
        // Unusual: positive ctm[3] means the image is not flipped. The top of
        // the image in user space is user_y_bottom + h/viewport_scale * viewport_scale = user_y_bottom + h.
        page_height_px - user_y_bottom - h
    };

    let device_x = if ctm[0] < 0.0 { user_x - w } else { user_x };

    if w < 1.0 || h < 1.0 || !w.is_finite() || !h.is_finite() {
        return None;
    }

    Some([device_x, device_y_top, w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_do_op(name: &str) -> ContentOperation {
        ContentOperation::new("Do", vec![Operand::Name(name.to_string())])
    }

    fn make_path_ops() -> Vec<ContentOperation> {
        vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(100.0)]),
            ContentOperation::new("f", vec![]),
        ]
    }

    fn resources_with_image(name: &str) -> PageResources {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert(name.to_string(), "Image".to_string());
        r.xobjects.insert(name.to_string(), (1, 0));
        r
    }

    fn resources_with_form(name: &str) -> PageResources {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert(name.to_string(), "Form".to_string());
        r.xobjects.insert(name.to_string(), (2, 0));
        r
    }

    #[test]
    fn pure_vector_ops_classified_as_pure_vector() {
        let ops = make_path_ops();
        let r = PageResources::default();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert_eq!(decision, VectorFallbackDecision::PureVector);
    }

    #[test]
    fn image_xobject_with_axis_aligned_ctm_is_regional() {
        // Set up a CTM that places a 200x100 image at (50, 300) — axis-aligned.
        let mut ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(200.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(-100.0), // negative = normal image flip
                    Operand::Real(50.0),
                    Operand::Real(400.0),
                ],
            ),
            make_do_op("Im0"),
        ];
        // Add some vector ops before.
        let mut full_ops = make_path_ops();
        full_ops.extend(ops);
        let r = resources_with_image("Im0");
        let decision = classify_page_for_vector_output(&full_ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { image_names } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn form_xobject_triggers_whole_page() {
        let ops = vec![make_do_op("Fm0")];
        let r = resources_with_form("Fm0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn rotated_image_triggers_whole_page() {
        // 45-degree rotation: ctm[1] and ctm[2] are non-zero.
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(141.0),
                    Operand::Real(141.0),  // non-zero b = rotation
                    Operand::Real(-141.0), // non-zero c = rotation
                    Operand::Real(141.0),
                    Operand::Real(50.0),
                    Operand::Real(300.0),
                ],
            ),
            make_do_op("Im0"),
        ];
        let r = resources_with_image("Im0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn inline_image_triggers_whole_page() {
        let ops = vec![ContentOperation::new("BI", vec![])];
        let r = PageResources::default();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn gs_operator_triggers_whole_page() {
        let ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        let r = PageResources::default();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn image_device_rect_computes_correct_coordinates() {
        let mut gs = GraphicsState::default();
        // Typical image placement: 200pt wide, 100pt tall, at (50, 400) with negative ctm[3].
        gs.ctm = [200.0, 0.0, 0.0, -100.0, 50.0, 400.0];
        let scale = 1.0;
        let page_h = 800.0;
        let rect = image_device_rect(&gs, scale, page_h).unwrap();
        // device_x = 50, device_y_top = 800 - 400 = 400, w = 200, h = 100
        assert!((rect[0] - 50.0).abs() < 0.01);
        assert!((rect[1] - 400.0).abs() < 0.01);
        assert!((rect[2] - 200.0).abs() < 0.01);
        assert!((rect[3] - 100.0).abs() < 0.01);
    }
}
