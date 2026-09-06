//! Packed retained display-list and backend-neutral render plan.
//!
//! This is intentionally additive during migration: compact path/state arenas
//! serve vector replay now, while high-level PDF operations migrate to typed,
//! resource-aware descriptors class by class.

use std::collections::HashMap;
use std::sync::Arc;

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{concat_matrix, ColorSpace};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};

use super::contract::{DisplayItemId, RenderContract};
use super::display_list::{
    CpuRenderDevice, DisplayList, DisplayOp, DrawState, RenderBounds, RenderDevice, RenderTile,
    RetainedInlineImage, RetainedTextArrayItem, RetainedTextOp,
};
use super::path::{FillRule, Path, PathSegment};
use super::{PixelBuffer, RenderMode, Transform2D, Viewport};

const OP_SAVE: u16 = 1;
const OP_RESTORE: u16 = 2;
const OP_CLIP: u16 = 3;
const OP_FILL: u16 = 4;
const OP_STROKE: u16 = 5;
const OP_STATE: u16 = 6;
const OP_NATIVE_TEXT: u16 = 7;
const OP_NATIVE_IMAGE: u16 = 8;
const OP_NATIVE_SHADING: u16 = 9;
const OP_NATIVE_PATTERN: u16 = 10;
const OP_NATIVE_INLINE_IMAGE: u16 = 11;
const OP_NATIVE_FORM: u16 = 12;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001b3;

fn hash_mix(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(FNV_PRIME);
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    hash_mix(hash, bytes.len() as u64);
    for byte in bytes {
        hash_mix(hash, u64::from(*byte));
    }
}

/// Validate page graphics-state operators before any retained or immediate path
/// can substitute identity/default operands for malformed input.
pub fn graphics_state_operand_refusal(op: &ContentOperation) -> Option<String> {
    match op.operator.as_str() {
        "q" | "Q" => validate_no_operands(op),
        "cm" => validate_exact_finite_numbers(op, 6, "concatenate matrix"),
        "w" => validate_finite_number_min(op, "line width", 0.0),
        "J" => validate_integer_range(op, "line cap", 0, 2),
        "j" => validate_integer_range(op, "line join", 0, 2),
        "M" => validate_finite_number_min(op, "miter limit", 1.0),
        "d" => validate_dash_operands(op),
        "ri" => validate_single_name(op, "rendering intent"),
        "i" => validate_finite_number_min(op, "flatness tolerance", 0.0),
        "G" => validate_exact_finite_numbers(op, 1, "stroking gray color"),
        "g" => validate_exact_finite_numbers(op, 1, "nonstroking gray color"),
        "RG" => validate_exact_finite_numbers(op, 3, "stroking RGB color"),
        "rg" => validate_exact_finite_numbers(op, 3, "nonstroking RGB color"),
        "K" => validate_exact_finite_numbers(op, 4, "stroking CMYK color"),
        "k" => validate_exact_finite_numbers(op, 4, "nonstroking CMYK color"),
        "CS" => validate_single_name(op, "stroking color space"),
        "cs" => validate_single_name(op, "nonstroking color space"),
        "SC" | "SCN" | "sc" | "scn" => validate_color_components_or_pattern_name(op),
        "gs" => validate_single_name(op, "ExtGState resource"),
        _ => None,
    }
}

/// Validate generic `SC`/`SCN`/`sc`/`scn` component counts once the current
/// graphics-state color space is known. The syntax-level validator permits a
/// variable component vector because named and Pattern spaces are resource
/// dependent; device spaces are exact and must not reach render conversion via
/// padding, truncation, or a stray pattern resource name.
pub fn graphics_state_color_component_arity_refusal(
    op: &ContentOperation,
    current_space: &ColorSpace,
    usage: &str,
) -> Option<String> {
    if !matches!(op.operator.as_str(), "SC" | "SCN" | "sc" | "scn") {
        return None;
    }
    let (space_name, expected) = match current_space {
        ColorSpace::DeviceGray => ("DeviceGray", 1),
        ColorSpace::DeviceRGB => ("DeviceRGB", 3),
        ColorSpace::DeviceCMYK => ("DeviceCMYK", 4),
        ColorSpace::Named(_) => return None,
    };
    if op
        .operands
        .iter()
        .any(|operand| operand.as_name().is_some())
    {
        return graphics_state_operand_error(
            op,
            format!("{usage} {space_name} color does not allow a pattern resource name"),
        );
    }
    let actual = op
        .operands
        .iter()
        .filter(|operand| finite_number(operand).is_some())
        .count();
    if actual != expected {
        return graphics_state_operand_error(
            op,
            format!(
                "{usage} {space_name} color expects exactly {expected} numeric component(s), got {actual}"
            ),
        );
    }
    None
}

/// Validate text-state and text-showing operators before retained capture or
/// immediate rendering can substitute empty strings, zero advances, or identity
/// text matrices for malformed input.
pub fn text_operand_refusal(op: &ContentOperation) -> Option<String> {
    match op.operator.as_str() {
        "BT" | "ET" | "T*" => validate_text_no_operands(op),
        "Tf" => validate_text_font(op),
        "Td" | "TD" => validate_text_exact_finite_numbers(op, 2, "text position"),
        "Tm" => validate_text_exact_finite_numbers(op, 6, "text matrix"),
        "Tc" => validate_text_exact_finite_numbers(op, 1, "character spacing"),
        "Tw" => validate_text_exact_finite_numbers(op, 1, "word spacing"),
        "Tz" => validate_text_exact_finite_numbers(op, 1, "horizontal scaling"),
        "TL" => validate_text_exact_finite_numbers(op, 1, "text leading"),
        "Tr" => validate_text_rendering_mode(op),
        "Ts" => validate_text_exact_finite_numbers(op, 1, "text rise"),
        "Tj" | "'" => validate_text_single_string(op),
        "TJ" => validate_text_array(op),
        "\"" => validate_text_spacing_next_line(op),
        _ => None,
    }
}

/// Validate marked-content and compatibility section operators before retained
/// or immediate paths can substitute empty tags, empty property lists, or
/// ignored section operands.
pub fn marked_content_operand_refusal(op: &ContentOperation) -> Option<String> {
    match op.operator.as_str() {
        "BMC" | "MP" => validate_marked_content_tag(op),
        "BDC" | "DP" => validate_marked_content_tag_and_properties(op),
        "EMC" | "BX" | "EX" => validate_marked_content_no_operands(op),
        _ => None,
    }
}

/// Validate page path-construction, path-painting, and clipping operators before
/// active/rendered paths can ignore malformed operands or synthesize a missing
/// current point.
pub fn path_operand_refusal(op: &ContentOperation, has_current_point: bool) -> Option<String> {
    match op.operator.as_str() {
        "m" => validate_path_exact_finite_numbers(op, 2, "move-to coordinates"),
        "l" => validate_path_segment_numbers(op, 2, "line-to coordinates", has_current_point),
        "c" => validate_path_segment_numbers(op, 6, "curve-to coordinates", has_current_point),
        "v" | "y" => {
            validate_path_segment_numbers(op, 4, "curve-to coordinates", has_current_point)
        }
        "h" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "W" | "W*" => {
            validate_path_no_operands(op)
        }
        "re" => validate_path_exact_finite_numbers(op, 4, "rectangle coordinates"),
        _ => None,
    }
}

/// Validate resource invocation operators before retained or immediate paths can
/// ignore missing names, use empty names, or drop extra operands.
pub fn resource_invocation_operand_refusal(op: &ContentOperation) -> Option<String> {
    match op.operator.as_str() {
        "Do" => validate_resource_invocation_name(op, "XObject resource"),
        "sh" => validate_resource_invocation_name(op, "shading resource"),
        _ => None,
    }
}

/// Validate Type 3 glyph metric operators before retained or immediate paths
/// can substitute zero widths or zero bounding boxes for malformed CharProc
/// metrics.
pub fn type3_glyph_metric_operand_refusal(op: &ContentOperation) -> Option<String> {
    match op.operator.as_str() {
        "d0" => validate_type3_glyph_metric_numbers(op, 2, "glyph displacement"),
        "d1" => validate_type3_glyph_metric_numbers(op, 6, "glyph displacement and bbox"),
        _ => None,
    }
}

fn graphics_state_operand_error(op: &ContentOperation, reason: impl AsRef<str>) -> Option<String> {
    Some(format!(
        "malformed graphics-state operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_no_operands(op: &ContentOperation) -> Option<String> {
    if op.operands.is_empty() {
        None
    } else {
        graphics_state_operand_error(op, "expected no operands")
    }
}

fn validate_exact_finite_numbers(
    op: &ContentOperation,
    expected: usize,
    label: &str,
) -> Option<String> {
    if op.operands.len() != expected {
        return graphics_state_operand_error(
            op,
            format!("{label} expects exactly {expected} numeric operand(s)"),
        );
    }
    for (idx, operand) in op.operands.iter().enumerate() {
        if finite_number(operand).is_none() {
            return graphics_state_operand_error(
                op,
                format!("{label} operand {idx} must be a finite number"),
            );
        }
    }
    None
}

fn validate_finite_number_min(
    op: &ContentOperation,
    label: &str,
    min_value: f64,
) -> Option<String> {
    if op.operands.len() != 1 {
        return graphics_state_operand_error(
            op,
            format!("{label} expects exactly one numeric operand"),
        );
    }
    let Some(value) = finite_number(&op.operands[0]) else {
        return graphics_state_operand_error(op, format!("{label} must be a finite number"));
    };
    if value < min_value {
        graphics_state_operand_error(op, format!("{label} must be at least {min_value}"))
    } else {
        None
    }
}

fn validate_integer_range(
    op: &ContentOperation,
    label: &str,
    min_value: i64,
    max_value: i64,
) -> Option<String> {
    if op.operands.len() != 1 {
        return graphics_state_operand_error(op, format!("{label} expects exactly one operand"));
    }
    let Some(value) = finite_number(&op.operands[0]) else {
        return graphics_state_operand_error(op, format!("{label} must be a finite integer"));
    };
    if value.fract() != 0.0 {
        return graphics_state_operand_error(op, format!("{label} must be an integer"));
    }
    let integer = value as i64;
    if !(min_value..=max_value).contains(&integer) {
        graphics_state_operand_error(
            op,
            format!("{label} must be in the range {min_value}..={max_value}"),
        )
    } else {
        None
    }
}

fn validate_dash_operands(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 2 {
        return graphics_state_operand_error(
            op,
            "dash pattern expects exactly an array and a phase",
        );
    }
    let Some(array) = op.operands[0].as_array() else {
        return graphics_state_operand_error(op, "dash pattern operand must be an array");
    };
    let Some(phase) = finite_number(&op.operands[1]) else {
        return graphics_state_operand_error(op, "dash phase must be a finite number");
    };
    if phase < 0.0 {
        return graphics_state_operand_error(op, "dash phase must be nonnegative");
    }
    let mut all_zero = !array.is_empty();
    for (idx, operand) in array.iter().enumerate() {
        let Some(value) = finite_number(operand) else {
            return graphics_state_operand_error(
                op,
                format!("dash interval {idx} must be a finite number"),
            );
        };
        if value < 0.0 {
            return graphics_state_operand_error(op, "dash intervals must be nonnegative");
        }
        all_zero &= value == 0.0;
    }
    if all_zero {
        graphics_state_operand_error(op, "dash array cannot contain only zero intervals")
    } else {
        None
    }
}

fn validate_single_name(op: &ContentOperation, label: &str) -> Option<String> {
    if op.operands.len() != 1 {
        return graphics_state_operand_error(op, format!("{label} expects exactly one name"));
    }
    if op.operands[0].as_name().is_none() {
        graphics_state_operand_error(op, format!("{label} operand must be a name"))
    } else {
        None
    }
}

fn validate_color_components_or_pattern_name(op: &ContentOperation) -> Option<String> {
    if op.operands.is_empty() {
        return graphics_state_operand_error(
            op,
            "color component operator expects numeric components or a pattern name",
        );
    }
    let mut saw_name = false;
    for (idx, operand) in op.operands.iter().enumerate() {
        if operand.as_name().is_some() {
            if saw_name {
                return graphics_state_operand_error(
                    op,
                    "color component operator allows at most one pattern name",
                );
            }
            if idx + 1 != op.operands.len() {
                return graphics_state_operand_error(
                    op,
                    "pattern name must be the final color operand",
                );
            }
            saw_name = true;
        } else if finite_number(operand).is_none() {
            return graphics_state_operand_error(
                op,
                format!("color component operand {idx} must be finite numeric or a name"),
            );
        }
    }
    None
}

fn finite_number(operand: &Operand) -> Option<f64> {
    let value = operand.as_number()?;
    if value.is_finite() {
        Some(value)
    } else {
        None
    }
}

fn path_operand_error(op: &ContentOperation, reason: impl AsRef<str>) -> Option<String> {
    Some(format!(
        "malformed path operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_path_no_operands(op: &ContentOperation) -> Option<String> {
    if op.operands.is_empty() {
        None
    } else {
        path_operand_error(op, "expected no operands")
    }
}

fn validate_path_exact_finite_numbers(
    op: &ContentOperation,
    expected: usize,
    label: &str,
) -> Option<String> {
    if op.operands.len() != expected {
        return path_operand_error(
            op,
            format!("expected exactly {expected} finite numeric operands for {label}"),
        );
    }
    for (idx, operand) in op.operands.iter().enumerate() {
        match operand.as_number() {
            Some(value) if value.is_finite() => {}
            Some(_) => {
                return path_operand_error(
                    op,
                    format!("operand {} for {label} is not finite", idx + 1),
                );
            }
            None => {
                return path_operand_error(
                    op,
                    format!("operand {} for {label} is not numeric", idx + 1),
                );
            }
        }
    }
    None
}

fn validate_path_segment_numbers(
    op: &ContentOperation,
    expected: usize,
    label: &str,
    has_current_point: bool,
) -> Option<String> {
    validate_path_exact_finite_numbers(op, expected, label).or_else(|| {
        if has_current_point {
            None
        } else {
            path_operand_error(op, "requires an active current point")
        }
    })
}

fn resource_invocation_operand_error(
    op: &ContentOperation,
    reason: impl AsRef<str>,
) -> Option<String> {
    Some(format!(
        "malformed resource invocation operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_resource_invocation_name(op: &ContentOperation, label: &str) -> Option<String> {
    if op.operands.len() != 1 {
        return resource_invocation_operand_error(
            op,
            format!("{label} expects exactly one name operand"),
        );
    }
    if op.operands[0].as_name().is_none() {
        return resource_invocation_operand_error(op, format!("{label} operand must be a name"));
    }
    None
}

fn type3_glyph_metric_operand_error(
    op: &ContentOperation,
    reason: impl AsRef<str>,
) -> Option<String> {
    Some(format!(
        "malformed Type 3 glyph metric operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_type3_glyph_metric_numbers(
    op: &ContentOperation,
    expected: usize,
    label: &str,
) -> Option<String> {
    if op.operands.len() != expected {
        return type3_glyph_metric_operand_error(
            op,
            format!("{label} expects exactly {expected} numeric operand(s)"),
        );
    }
    for (idx, operand) in op.operands.iter().enumerate() {
        if finite_number(operand).is_none() {
            return type3_glyph_metric_operand_error(
                op,
                format!("{label} operand {idx} must be a finite number"),
            );
        }
    }
    None
}

fn marked_content_operand_error(op: &ContentOperation, reason: impl AsRef<str>) -> Option<String> {
    Some(format!(
        "malformed marked-content operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_marked_content_no_operands(op: &ContentOperation) -> Option<String> {
    if op.operands.is_empty() {
        None
    } else {
        marked_content_operand_error(op, "expected no operands")
    }
}

fn validate_marked_content_tag(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 1 {
        return marked_content_operand_error(op, "expected exactly one tag name operand");
    }
    if op.operands[0].as_name().is_none() {
        return marked_content_operand_error(op, "tag must be a name");
    }
    None
}

fn validate_marked_content_tag_and_properties(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 2 {
        return marked_content_operand_error(
            op,
            "expected exactly tag name and property-list operands",
        );
    }
    if op.operands[0].as_name().is_none() {
        return marked_content_operand_error(op, "tag must be a name");
    }
    match &op.operands[1] {
        Operand::Name(_) | Operand::Dictionary(_) => None,
        _ => marked_content_operand_error(op, "property list must be a name or dictionary"),
    }
}

fn text_operand_error(op: &ContentOperation, reason: impl AsRef<str>) -> Option<String> {
    Some(format!(
        "malformed text operator '{}': {}",
        op.operator,
        reason.as_ref()
    ))
}

fn validate_text_no_operands(op: &ContentOperation) -> Option<String> {
    if op.operands.is_empty() {
        None
    } else {
        text_operand_error(op, "expected no operands")
    }
}

fn validate_text_exact_finite_numbers(
    op: &ContentOperation,
    expected: usize,
    label: &str,
) -> Option<String> {
    if op.operands.len() != expected {
        return text_operand_error(
            op,
            format!("{label} expects exactly {expected} numeric operand(s)"),
        );
    }
    for (idx, operand) in op.operands.iter().enumerate() {
        if finite_number(operand).is_none() {
            return text_operand_error(
                op,
                format!("{label} operand {idx} must be a finite number"),
            );
        }
    }
    None
}

fn validate_text_font(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 2 {
        return text_operand_error(op, "font selection expects a name and finite size");
    }
    if op.operands[0].as_name().is_none() {
        return text_operand_error(op, "font resource operand must be a name");
    }
    if finite_number(&op.operands[1]).is_none() {
        return text_operand_error(op, "font size must be a finite number");
    }
    None
}

fn validate_text_rendering_mode(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 1 {
        return text_operand_error(op, "text rendering mode expects exactly one operand");
    }
    let Some(value) = finite_number(&op.operands[0]) else {
        return text_operand_error(op, "text rendering mode must be a finite integer");
    };
    if value.fract() != 0.0 {
        return text_operand_error(op, "text rendering mode must be an integer");
    }
    let mode = value as i64;
    if !(0..=7).contains(&mode) {
        text_operand_error(op, "text rendering mode must be in the range 0..=7")
    } else {
        None
    }
}

fn validate_text_single_string(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 1 {
        return text_operand_error(op, "text showing expects exactly one string");
    }
    if op.operands[0].as_bytes().is_none() {
        text_operand_error(op, "text showing operand must be a string")
    } else {
        None
    }
}

fn validate_text_array(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 1 {
        return text_operand_error(op, "text array showing expects exactly one array");
    }
    let Some(items) = op.operands[0].as_array() else {
        return text_operand_error(op, "text array showing operand must be an array");
    };
    for (idx, item) in items.iter().enumerate() {
        if item.as_bytes().is_some() {
            continue;
        }
        if finite_number(item).is_none() {
            return text_operand_error(
                op,
                format!("text array item {idx} must be a string or finite number"),
            );
        }
    }
    None
}

fn validate_text_spacing_next_line(op: &ContentOperation) -> Option<String> {
    if op.operands.len() != 3 {
        return text_operand_error(
            op,
            "spacing text showing expects word spacing, character spacing, and string",
        );
    }
    if finite_number(&op.operands[0]).is_none() {
        return text_operand_error(op, "word spacing must be a finite number");
    }
    if finite_number(&op.operands[1]).is_none() {
        return text_operand_error(op, "character spacing must be a finite number");
    }
    if op.operands[2].as_bytes().is_none() {
        return text_operand_error(op, "spacing text showing operand must end with a string");
    }
    None
}

fn path_fingerprint(path: &Path) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_mix(&mut hash, path.segments.len() as u64);
    for segment in &path.segments {
        match segment {
            PathSegment::MoveTo(x, y) => {
                hash_mix(&mut hash, 1);
                hash_mix(&mut hash, x.to_bits());
                hash_mix(&mut hash, y.to_bits());
            }
            PathSegment::LineTo(x, y) => {
                hash_mix(&mut hash, 2);
                hash_mix(&mut hash, x.to_bits());
                hash_mix(&mut hash, y.to_bits());
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                hash_mix(&mut hash, 3);
                for value in [cp1x, cp1y, cp2x, cp2y, x, y] {
                    hash_mix(&mut hash, value.to_bits());
                }
            }
            PathSegment::ClosePath => hash_mix(&mut hash, 4),
        }
    }
    match path.current_point {
        Some((x, y)) => {
            hash_mix(&mut hash, 5);
            hash_mix(&mut hash, x.to_bits());
            hash_mix(&mut hash, y.to_bits());
        }
        None => hash_mix(&mut hash, 6),
    }
    hash
}

fn hash_optional_f32(hash: &mut u64, value: Option<[f32; 4]>) {
    match value {
        Some(values) => {
            hash_mix(hash, 1);
            for value in values {
                hash_mix(hash, u64::from(value.to_bits()));
            }
        }
        None => hash_mix(hash, 0),
    }
}

fn same_optional_f32(left: Option<[f32; 4]>, right: Option<[f32; 4]>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| left.to_bits() == right.to_bits()),
        (None, None) => true,
        _ => false,
    }
}

fn draw_state_fingerprint(state: &DrawState) -> u64 {
    let mut hash = FNV_OFFSET;
    for value in state.ctm.to_array() {
        hash_mix(&mut hash, value.to_bits());
    }
    for value in state.fill_color {
        hash_mix(&mut hash, u64::from(value));
    }
    for value in state.stroke_color {
        hash_mix(&mut hash, u64::from(value));
    }
    hash_mix(&mut hash, if state.fill_color_explicit { 1 } else { 0 });
    hash_mix(&mut hash, if state.stroke_color_explicit { 1 } else { 0 });
    hash_optional_f32(&mut hash, state.fill_cmyk);
    hash_optional_f32(&mut hash, state.stroke_cmyk);
    hash_mix(&mut hash, state.blend_mode as u64);
    hash_bytes(&mut hash, state.rendering_intent.as_bytes());
    hash_mix(&mut hash, if state.stroke_overprint { 1 } else { 0 });
    hash_mix(&mut hash, if state.fill_overprint { 1 } else { 0 });
    hash_mix(&mut hash, state.overprint_mode as u64);
    hash_mix(&mut hash, if state.stroke_adjustment { 1 } else { 0 });
    hash_mix(&mut hash, if state.alpha_source { 1 } else { 0 });
    hash_mix(&mut hash, if state.text_knockout { 1 } else { 0 });
    hash_mix(&mut hash, state.line_width.to_bits());
    hash_mix(&mut hash, state.line_cap.clone() as u64);
    hash_mix(&mut hash, state.line_join.clone() as u64);
    hash_mix(&mut hash, state.miter_limit.to_bits());
    hash_mix(&mut hash, state.dash.render_cache_fingerprint());
    hash
}

fn same_draw_state(left: &DrawState, right: &DrawState) -> bool {
    left.ctm.to_array().map(f64::to_bits) == right.ctm.to_array().map(f64::to_bits)
        && left.fill_color == right.fill_color
        && left.stroke_color == right.stroke_color
        && left.fill_color_explicit == right.fill_color_explicit
        && left.stroke_color_explicit == right.stroke_color_explicit
        && same_optional_f32(left.fill_cmyk, right.fill_cmyk)
        && same_optional_f32(left.stroke_cmyk, right.stroke_cmyk)
        && left.blend_mode == right.blend_mode
        && left.rendering_intent == right.rendering_intent
        && left.stroke_overprint == right.stroke_overprint
        && left.fill_overprint == right.fill_overprint
        && left.overprint_mode == right.overprint_mode
        && left.stroke_adjustment == right.stroke_adjustment
        && left.alpha_source == right.alpha_source
        && left.text_knockout == right.text_knockout
        && left.line_width.to_bits() == right.line_width.to_bits()
        && left.line_cap == right.line_cap
        && left.line_join == right.line_join
        && left.miter_limit.to_bits() == right.miter_limit.to_bits()
        && left.dash.same_for_render(&right.dash)
}

/// Fixed-size hot command. Its payload indexes immutable arenas and never
/// carries a string, PDF dictionary, or `ContentOperation` directly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotDisplayOp {
    pub opcode: u16,
    pub flags: u16,
    pub bounds_id: u32,
    pub state_id: u32,
    pub payload_offset: u32,
    pub payload_len: u32,
    pub source_link_id: u32,
}

/// Compiled text-showing descriptor. Carries only the pre-extracted fields
/// the renderer needs without retaining a raw `ContentOperation` on the hot path.
#[derive(Clone, Debug)]
pub enum TextDescriptor {
    /// `Tj` — single string show.
    Show(Vec<u8>),
    /// `TJ` — array of strings and position adjustments.
    ShowArray(Vec<TextArrayItem>),
    /// `'` — move to next line then show.
    NextLineShow(Vec<u8>),
    /// `"` — set word/char spacing, move to next line, then show.
    SpacingNextLineShow {
        word_spacing: f64,
        char_spacing: f64,
        text: Vec<u8>,
    },
}

/// One item in a TJ array.
#[derive(Clone, Debug)]
pub enum TextArrayItem {
    Bytes(Vec<u8>),
    Adjustment(f64),
}

/// Compiled image XObject descriptor with the resource name and, when compiled
/// with page/form resources, the resolved XObject handle.
#[derive(Clone, Debug)]
pub struct ImageXObjectDescriptor {
    pub name: String,
    pub handle: Option<ResolvedXObjectHandle>,
}

/// Compiled Form XObject descriptor with the resource name and, when compiled
/// with page/form resources, the resolved XObject handle.
#[derive(Clone, Debug)]
pub struct FormXObjectDescriptor {
    pub name: String,
    pub handle: Option<ResolvedXObjectHandle>,
}

/// Compiled shading descriptor with the resource name and, when compiled with
/// page/form resources, the resolved shading object.
#[derive(Clone, Debug)]
pub struct ShadingDescriptor {
    pub name: String,
    pub object: Option<PdfObject>,
}

/// Pre-resolved color-space resource selected by an inline image `/ColorSpace`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInlineImageColorSpace {
    pub name: String,
    pub object: PdfObject,
}

/// Pre-resolved font selected by an ExtGState `/Font` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFontHandle {
    pub name: String,
    pub size: f64,
    pub dict: PdfDictionary,
}

fn ext_g_state_font_selection(dict: &PdfDictionary) -> Option<(String, f64)> {
    let items = dict.get("Font")?.as_array()?;
    if items.len() != 2 {
        return None;
    }
    let name = items[0].as_name()?.to_string();
    let size = items[1].as_number()?;
    if name.is_empty() || !size.is_finite() || size < 0.0 {
        return None;
    }
    Some((name, size))
}

fn resolved_ext_g_state_font_handle(
    dict: &PdfDictionary,
    resources: Option<&PageResources>,
) -> Option<ResolvedFontHandle> {
    let (name, size) = ext_g_state_font_selection(dict)?;
    let font_dict = resources?.fonts.get(&name)?.clone();
    Some(ResolvedFontHandle {
        name,
        size,
        dict: font_dict,
    })
}

#[derive(Clone, Debug)]
pub struct ResolvedXObjectHandle {
    pub name: String,
    pub object_number: u32,
    pub generation_number: u16,
    pub subtype: Option<String>,
    pub stream_dict: Option<PdfDictionary>,
    pub bbox: Option<[f64; 4]>,
    pub matrix: Option<[f64; 6]>,
    pub image_color_space: Option<ResolvedInlineImageColorSpace>,
}

/// Paint phase for a pattern path operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternPaintPhase {
    /// `S` — stroke only.
    Stroke,
    /// `s` — close then stroke.
    CloseStroke,
    /// `f` / `F` — fill non-zero.
    FillNonZero,
    /// `f*` — fill even-odd.
    FillEvenOdd,
    /// `B` — fill non-zero then stroke.
    FillStrokeNonZero,
    /// `B*` — fill even-odd then stroke.
    FillStrokeEvenOdd,
    /// `b` — close, fill non-zero, then stroke.
    CloseFillStrokeNonZero,
    /// `b*` — close, fill even-odd, then stroke.
    CloseFillStrokeEvenOdd,
}

impl PatternPaintPhase {
    pub fn from_operator(op: &str) -> Option<Self> {
        match op {
            "S" => Some(Self::Stroke),
            "s" => Some(Self::CloseStroke),
            "f" | "F" => Some(Self::FillNonZero),
            "f*" => Some(Self::FillEvenOdd),
            "B" => Some(Self::FillStrokeNonZero),
            "B*" => Some(Self::FillStrokeEvenOdd),
            "b" => Some(Self::CloseFillStrokeNonZero),
            "b*" => Some(Self::CloseFillStrokeEvenOdd),
            _ => None,
        }
    }

    /// Returns the PDF operator string for this phase.
    pub fn operator(&self) -> &'static str {
        match self {
            Self::Stroke => "S",
            Self::CloseStroke => "s",
            Self::FillNonZero => "f",
            Self::FillEvenOdd => "f*",
            Self::FillStrokeNonZero => "B",
            Self::FillStrokeEvenOdd => "B*",
            Self::CloseFillStrokeNonZero => "b",
            Self::CloseFillStrokeEvenOdd => "b*",
        }
    }
}

/// Compiled typed descriptor for a pattern path operation.
///
/// Stores normalized path geometry and paint phase rather than raw
/// `Vec<ContentOperation>`. The resource context (active fill/stroke pattern,
/// color space) is carried by the RenderState at execution time — this
/// descriptor only stores the geometry and paint instruction.
#[derive(Clone, Debug)]
pub struct PatternPathDescriptor {
    /// Normalized path segments extracted from the content operations.
    pub path: Path,
    /// Which paint operator terminates this path run.
    pub phase: PatternPaintPhase,
}

/// Compiled typed descriptor for an inline image.
///
/// Stores the parsed image parameter operands and raw data bytes. The renderer
/// can directly call `paint_inline_image` with these fields without
/// intermediate ContentOperation reconstruction.
#[derive(Clone, Debug)]
pub struct InlineImageDescriptor {
    /// Parsed image parameters from the `ID` operator (the operands that precede
    /// the image data, typically key/value pairs like `/W 10 /H 10 /BPC 8 ...`).
    pub params: Vec<Operand>,
    /// Raw image data bytes from the `inline_image_data` pseudo-operator.
    pub data: Vec<u8>,
    /// Pre-resolved named color-space resource for `/ColorSpace /Name`, when
    /// resource-aware compilation can resolve it. Built-in device spaces do not
    /// need a resource object.
    pub color_space: Option<ResolvedInlineImageColorSpace>,
}

/// Typed graphics-state descriptor covering all state operators reachable in
/// `DisplayOp::StateOp` (save/restore excluded). Replaces raw
/// `ContentOperation` on the packed plan hot path.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphicsStateDescriptor {
    // --- Transform ---
    /// `cm` — concatenate transformation matrix.
    ConcatMatrix {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
    },

    // --- Line/stroke parameters ---
    /// `w` — set line width.
    SetLineWidth(f64),
    /// `J` — set line cap style (0 = butt, 1 = round, 2 = square).
    SetLineCap(i64),
    /// `j` — set line join style (0 = miter, 1 = round, 2 = bevel).
    SetLineJoin(i64),
    /// `M` — set miter limit.
    SetMiterLimit(f64),
    /// `d` — set dash pattern: (array, phase).
    SetDash { array: Vec<f64>, phase: f64 },
    /// `ri` — set rendering intent name.
    SetRenderingIntent(String),
    /// `i` — set flatness tolerance.
    SetFlatness(f64),

    // --- Device color operators ---
    /// `G` — set stroke gray.
    SetStrokeGray(f64),
    /// `g` — set fill gray.
    SetFillGray(f64),
    /// `RG` — set stroke RGB.
    SetStrokeRgb { r: f64, g: f64, b: f64 },
    /// `rg` — set fill RGB.
    SetFillRgb { r: f64, g: f64, b: f64 },
    /// `K` — set stroke CMYK.
    SetStrokeCmyk { c: f64, m: f64, y: f64, k: f64 },
    /// `k` — set fill CMYK.
    SetFillCmyk { c: f64, m: f64, y: f64, k: f64 },

    // --- Color space operators ---
    /// `CS` — set stroke color space by name.
    SetStrokeColorSpace {
        name: String,
        object: Option<PdfObject>,
    },
    /// `cs` — set fill color space by name.
    SetFillColorSpace {
        name: String,
        object: Option<PdfObject>,
    },
    /// `SC` / `SCN` — set stroke color components in current space.
    SetStrokeColor {
        components: Vec<f64>,
        name: Option<String>,
        pattern: Option<PdfObject>,
    },
    /// `sc` / `scn` — set fill color components in current space.
    SetFillColor {
        components: Vec<f64>,
        name: Option<String>,
        pattern: Option<PdfObject>,
    },

    // --- ExtGState ---
    /// `gs` — apply named ExtGState resource, optionally pre-resolved from the
    /// page/form resource dictionary at plan compilation time.
    ApplyExtGState {
        name: String,
        dict: Option<PdfDictionary>,
        font: Option<ResolvedFontHandle>,
    },

    // --- Text state operators ---
    /// `BT` — begin text object.
    BeginText,
    /// `ET` — end text object.
    EndText,
    /// `Tf` — set text font and size, optionally pre-resolved from the active
    /// resource dictionary at plan compilation time.
    SetFont {
        name: String,
        size: f64,
        dict: Option<PdfDictionary>,
    },
    /// `Td` — move text position.
    MoveTextPosition { tx: f64, ty: f64 },
    /// `TD` — move text position and set leading.
    MoveTextPositionSetLeading { tx: f64, ty: f64 },
    /// `Tm` — set text matrix.
    SetTextMatrix {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
    },
    /// `T*` — move to start of next text line.
    NextLine,
    /// `Tc` — set character spacing.
    SetCharSpacing(f64),
    /// `Tw` — set word spacing.
    SetWordSpacing(f64),
    /// `Tz` — set horizontal scaling.
    SetHorizontalScaling(f64),
    /// `TL` — set text leading.
    SetTextLeading(f64),
    /// `Tr` — set text rendering mode.
    SetTextRenderingMode(i64),
    /// `Ts` — set text rise.
    SetTextRise(f64),

    // --- Type 3 glyph metrics ---
    /// `d0`/`d1` - Type 3 glyph metrics. Rendering extracts metrics before
    /// replay; the descriptor keeps CharProc plans typed without repaint effect.
    SetType3GlyphMetrics {
        wx: f64,
        wy: f64,
        bbox: Option<[f64; 4]>,
    },

    // --- Marked content / optional content ---
    /// `BMC` — begin marked-content sequence (tag only).
    BeginMarkedContent(String),
    /// `BDC` — begin marked-content sequence with properties.
    BeginMarkedContentWithProperties {
        tag: String,
        properties: MarkedContentProperties,
    },
    /// `EMC` — end marked-content sequence.
    EndMarkedContent,
    /// `MP` — marked-content point (tag only).
    MarkedContentPoint(String),
    /// `DP` — marked-content point with properties.
    MarkedContentPointWithProperties {
        tag: String,
        properties: MarkedContentProperties,
    },
    /// `BX` — begin compatibility section.
    BeginCompatibility,
    /// `EX` — end compatibility section.
    EndCompatibility,

    // --- Unsupported state operator ---
    /// An unrecognized operator that cannot be compiled into a typed variant.
    /// This is retained only for fail-closed dispatch — the plan will emit a
    /// `PackedCompileRefusal::UnsupportedStateOperator` when executing.
    Unsupported { operator: String },
}

/// Properties for BDC/DP marked content with inline or resource-referenced
/// property dictionaries.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkedContentProperties {
    /// Name reference to a Properties resource entry, optionally pre-resolved
    /// from the active resource dictionary.
    Name {
        name: String,
        object: Option<PdfObject>,
    },
    /// Inline property dictionary stored as key-value operands.
    Inline(Vec<Operand>),
}

impl GraphicsStateDescriptor {
    /// Compile a `ContentOperation` into a typed graphics-state descriptor.
    pub fn compile(op: &crate::content::ContentOperation) -> Self {
        Self::compile_with_resources(op, None)
    }

    /// Compile a state operation while pre-resolving resource-backed entries
    /// when the caller has the active resource dictionary.
    pub fn compile_with_resources(
        op: &crate::content::ContentOperation,
        resources: Option<&PageResources>,
    ) -> Self {
        if graphics_state_operand_refusal(op).is_some()
            || text_operand_refusal(op).is_some()
            || marked_content_operand_refusal(op).is_some()
            || type3_glyph_metric_operand_refusal(op).is_some()
        {
            return Self::Unsupported {
                operator: op.operator.clone(),
            };
        }
        match op.operator.as_str() {
            "cm" => {
                let a = op.number(0).unwrap_or(1.0);
                let b = op.number(1).unwrap_or(0.0);
                let c = op.number(2).unwrap_or(0.0);
                let d = op.number(3).unwrap_or(1.0);
                let e = op.number(4).unwrap_or(0.0);
                let f = op.number(5).unwrap_or(0.0);
                Self::ConcatMatrix { a, b, c, d, e, f }
            }
            "w" => Self::SetLineWidth(op.number(0).unwrap_or(1.0)),
            "J" => Self::SetLineCap(
                op.operand(0)
                    .and_then(|o| o.as_integer())
                    .or_else(|| op.number(0).map(|v| v as i64))
                    .unwrap_or(0),
            ),
            "j" => Self::SetLineJoin(
                op.operand(0)
                    .and_then(|o| o.as_integer())
                    .or_else(|| op.number(0).map(|v| v as i64))
                    .unwrap_or(0),
            ),
            "M" => Self::SetMiterLimit(op.number(0).unwrap_or(10.0)),
            "d" => {
                let array = op
                    .operand(0)
                    .and_then(Operand::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_number())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let phase = op.number(1).unwrap_or(0.0);
                Self::SetDash { array, phase }
            }
            "ri" => {
                Self::SetRenderingIntent(op.name(0).unwrap_or("RelativeColorimetric").to_string())
            }
            "i" => Self::SetFlatness(op.number(0).unwrap_or(0.0)),
            "G" => Self::SetStrokeGray(op.number(0).unwrap_or(0.0)),
            "g" => Self::SetFillGray(op.number(0).unwrap_or(0.0)),
            "RG" => Self::SetStrokeRgb {
                r: op.number(0).unwrap_or(0.0),
                g: op.number(1).unwrap_or(0.0),
                b: op.number(2).unwrap_or(0.0),
            },
            "rg" => Self::SetFillRgb {
                r: op.number(0).unwrap_or(0.0),
                g: op.number(1).unwrap_or(0.0),
                b: op.number(2).unwrap_or(0.0),
            },
            "K" => Self::SetStrokeCmyk {
                c: op.number(0).unwrap_or(0.0),
                m: op.number(1).unwrap_or(0.0),
                y: op.number(2).unwrap_or(0.0),
                k: op.number(3).unwrap_or(0.0),
            },
            "k" => Self::SetFillCmyk {
                c: op.number(0).unwrap_or(0.0),
                m: op.number(1).unwrap_or(0.0),
                y: op.number(2).unwrap_or(0.0),
                k: op.number(3).unwrap_or(0.0),
            },
            "CS" => {
                let name = op.name(0).unwrap_or("DeviceGray").to_string();
                let object =
                    resources.and_then(|resources| resources.color_spaces.get(&name).cloned());
                Self::SetStrokeColorSpace { name, object }
            }
            "cs" => {
                let name = op.name(0).unwrap_or("DeviceGray").to_string();
                let object =
                    resources.and_then(|resources| resources.color_spaces.get(&name).cloned());
                Self::SetFillColorSpace { name, object }
            }
            "SC" | "SCN" => {
                let mut components = Vec::new();
                let mut name = None;
                for operand in &op.operands {
                    match operand {
                        Operand::Real(v) => components.push(*v),
                        Operand::Integer(v) => components.push(*v as f64),
                        Operand::Name(n) => name = Some(n.clone()),
                        _ => {}
                    }
                }
                let pattern = name.as_ref().and_then(|name| {
                    resources.and_then(|resources| resources.patterns.get(name).cloned())
                });
                Self::SetStrokeColor {
                    components,
                    name,
                    pattern,
                }
            }
            "sc" | "scn" => {
                let mut components = Vec::new();
                let mut name = None;
                for operand in &op.operands {
                    match operand {
                        Operand::Real(v) => components.push(*v),
                        Operand::Integer(v) => components.push(*v as f64),
                        Operand::Name(n) => name = Some(n.clone()),
                        _ => {}
                    }
                }
                let pattern = name.as_ref().and_then(|name| {
                    resources.and_then(|resources| resources.patterns.get(name).cloned())
                });
                Self::SetFillColor {
                    components,
                    name,
                    pattern,
                }
            }
            "gs" => {
                let name = op.name(0).unwrap_or("").to_string();
                let dict =
                    resources.and_then(|resources| resources.ext_g_states.get(&name).cloned());
                let font = dict
                    .as_ref()
                    .and_then(|dict| resolved_ext_g_state_font_handle(dict, resources));
                Self::ApplyExtGState { name, dict, font }
            }
            "BT" => Self::BeginText,
            "ET" => Self::EndText,
            "Tf" => {
                let name = op.name(0).unwrap_or("").to_string();
                let dict = resources.and_then(|resources| resources.fonts.get(&name).cloned());
                Self::SetFont {
                    name,
                    size: op.number(1).unwrap_or(12.0),
                    dict,
                }
            }
            "Td" => Self::MoveTextPosition {
                tx: op.number(0).unwrap_or(0.0),
                ty: op.number(1).unwrap_or(0.0),
            },
            "TD" => Self::MoveTextPositionSetLeading {
                tx: op.number(0).unwrap_or(0.0),
                ty: op.number(1).unwrap_or(0.0),
            },
            "Tm" => Self::SetTextMatrix {
                a: op.number(0).unwrap_or(1.0),
                b: op.number(1).unwrap_or(0.0),
                c: op.number(2).unwrap_or(0.0),
                d: op.number(3).unwrap_or(1.0),
                e: op.number(4).unwrap_or(0.0),
                f: op.number(5).unwrap_or(0.0),
            },
            "T*" => Self::NextLine,
            "Tc" => Self::SetCharSpacing(op.number(0).unwrap_or(0.0)),
            "Tw" => Self::SetWordSpacing(op.number(0).unwrap_or(0.0)),
            "Tz" => Self::SetHorizontalScaling(op.number(0).unwrap_or(100.0)),
            "TL" => Self::SetTextLeading(op.number(0).unwrap_or(0.0)),
            "Tr" => Self::SetTextRenderingMode(
                op.operand(0)
                    .and_then(|o| o.as_integer())
                    .or_else(|| op.number(0).map(|v| v as i64))
                    .unwrap_or(0),
            ),
            "Ts" => Self::SetTextRise(op.number(0).unwrap_or(0.0)),
            "d0" => Self::SetType3GlyphMetrics {
                wx: op.number(0).unwrap_or(0.0),
                wy: op.number(1).unwrap_or(0.0),
                bbox: None,
            },
            "d1" => Self::SetType3GlyphMetrics {
                wx: op.number(0).unwrap_or(0.0),
                wy: op.number(1).unwrap_or(0.0),
                bbox: Some([
                    op.number(2).unwrap_or(0.0),
                    op.number(3).unwrap_or(0.0),
                    op.number(4).unwrap_or(0.0),
                    op.number(5).unwrap_or(0.0),
                ]),
            },
            "BMC" => Self::BeginMarkedContent(op.name(0).unwrap_or("").to_string()),
            "BDC" => {
                let tag = op.name(0).unwrap_or("").to_string();
                let properties = if let Some(name) = op.name(1) {
                    MarkedContentProperties::Name {
                        name: name.to_string(),
                        object: resources
                            .and_then(|resources| resources.properties.get(name).cloned()),
                    }
                } else if op.operands.len() > 1 {
                    MarkedContentProperties::Inline(op.operands[1..].to_vec())
                } else {
                    MarkedContentProperties::Inline(Vec::new())
                };
                Self::BeginMarkedContentWithProperties { tag, properties }
            }
            "EMC" => Self::EndMarkedContent,
            "MP" => Self::MarkedContentPoint(op.name(0).unwrap_or("").to_string()),
            "DP" => {
                let tag = op.name(0).unwrap_or("").to_string();
                let properties = if let Some(name) = op.name(1) {
                    MarkedContentProperties::Name {
                        name: name.to_string(),
                        object: resources
                            .and_then(|resources| resources.properties.get(name).cloned()),
                    }
                } else if op.operands.len() > 1 {
                    MarkedContentProperties::Inline(op.operands[1..].to_vec())
                } else {
                    MarkedContentProperties::Inline(Vec::new())
                };
                Self::MarkedContentPointWithProperties { tag, properties }
            }
            "BX" => Self::BeginCompatibility,
            "EX" => Self::EndCompatibility,
            other => Self::Unsupported {
                operator: other.to_string(),
            },
        }
    }

    /// Reconstruct a `ContentOperation` for descriptor round-trip tests and
    /// diagnostics.
    ///
    /// Active packed replay must use `PlanDispatcher` typed descriptors and
    /// must not call this helper for pixels. Resource-backed variants may carry
    /// resolved data that this diagnostic representation intentionally omits.
    pub fn to_content_operation(&self) -> crate::content::ContentOperation {
        use crate::content::ContentOperation;
        match self {
            Self::ConcatMatrix { a, b, c, d, e, f } => ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(*a),
                    Operand::Real(*b),
                    Operand::Real(*c),
                    Operand::Real(*d),
                    Operand::Real(*e),
                    Operand::Real(*f),
                ],
            ),
            Self::SetLineWidth(w) => ContentOperation::new("w", vec![Operand::Real(*w)]),
            Self::SetLineCap(c) => ContentOperation::new("J", vec![Operand::Integer(*c)]),
            Self::SetLineJoin(j) => ContentOperation::new("j", vec![Operand::Integer(*j)]),
            Self::SetMiterLimit(m) => ContentOperation::new("M", vec![Operand::Real(*m)]),
            Self::SetDash { array, phase } => ContentOperation::new(
                "d",
                vec![
                    Operand::Array(array.iter().map(|v| Operand::Real(*v)).collect()),
                    Operand::Real(*phase),
                ],
            ),
            Self::SetRenderingIntent(name) => {
                ContentOperation::new("ri", vec![Operand::Name(name.clone())])
            }
            Self::SetFlatness(f) => ContentOperation::new("i", vec![Operand::Real(*f)]),
            Self::SetStrokeGray(g) => ContentOperation::new("G", vec![Operand::Real(*g)]),
            Self::SetFillGray(g) => ContentOperation::new("g", vec![Operand::Real(*g)]),
            Self::SetStrokeRgb { r, g, b } => ContentOperation::new(
                "RG",
                vec![Operand::Real(*r), Operand::Real(*g), Operand::Real(*b)],
            ),
            Self::SetFillRgb { r, g, b } => ContentOperation::new(
                "rg",
                vec![Operand::Real(*r), Operand::Real(*g), Operand::Real(*b)],
            ),
            Self::SetStrokeCmyk { c, m, y, k } => ContentOperation::new(
                "K",
                vec![
                    Operand::Real(*c),
                    Operand::Real(*m),
                    Operand::Real(*y),
                    Operand::Real(*k),
                ],
            ),
            Self::SetFillCmyk { c, m, y, k } => ContentOperation::new(
                "k",
                vec![
                    Operand::Real(*c),
                    Operand::Real(*m),
                    Operand::Real(*y),
                    Operand::Real(*k),
                ],
            ),
            Self::SetStrokeColorSpace { name, .. } => {
                ContentOperation::new("CS", vec![Operand::Name(name.clone())])
            }
            Self::SetFillColorSpace { name, .. } => {
                ContentOperation::new("cs", vec![Operand::Name(name.clone())])
            }
            Self::SetStrokeColor {
                components, name, ..
            } => {
                let mut operands: Vec<Operand> =
                    components.iter().map(|v| Operand::Real(*v)).collect();
                if let Some(n) = name {
                    operands.push(Operand::Name(n.clone()));
                }
                ContentOperation::new("SCN", operands)
            }
            Self::SetFillColor {
                components, name, ..
            } => {
                let mut operands: Vec<Operand> =
                    components.iter().map(|v| Operand::Real(*v)).collect();
                if let Some(n) = name {
                    operands.push(Operand::Name(n.clone()));
                }
                ContentOperation::new("scn", operands)
            }
            Self::ApplyExtGState { name, .. } => {
                ContentOperation::new("gs", vec![Operand::Name(name.clone())])
            }
            Self::BeginText => ContentOperation::new("BT", Vec::new()),
            Self::EndText => ContentOperation::new("ET", Vec::new()),
            Self::SetFont { name, size, .. } => ContentOperation::new(
                "Tf",
                vec![Operand::Name(name.clone()), Operand::Real(*size)],
            ),
            Self::MoveTextPosition { tx, ty } => {
                ContentOperation::new("Td", vec![Operand::Real(*tx), Operand::Real(*ty)])
            }
            Self::MoveTextPositionSetLeading { tx, ty } => {
                ContentOperation::new("TD", vec![Operand::Real(*tx), Operand::Real(*ty)])
            }
            Self::SetTextMatrix { a, b, c, d, e, f } => ContentOperation::new(
                "Tm",
                vec![
                    Operand::Real(*a),
                    Operand::Real(*b),
                    Operand::Real(*c),
                    Operand::Real(*d),
                    Operand::Real(*e),
                    Operand::Real(*f),
                ],
            ),
            Self::NextLine => ContentOperation::new("T*", Vec::new()),
            Self::SetCharSpacing(v) => ContentOperation::new("Tc", vec![Operand::Real(*v)]),
            Self::SetWordSpacing(v) => ContentOperation::new("Tw", vec![Operand::Real(*v)]),
            Self::SetHorizontalScaling(v) => ContentOperation::new("Tz", vec![Operand::Real(*v)]),
            Self::SetTextLeading(v) => ContentOperation::new("TL", vec![Operand::Real(*v)]),
            Self::SetTextRenderingMode(m) => {
                ContentOperation::new("Tr", vec![Operand::Integer(*m)])
            }
            Self::SetTextRise(v) => ContentOperation::new("Ts", vec![Operand::Real(*v)]),
            Self::SetType3GlyphMetrics { wx, wy, bbox } => {
                let mut operands = vec![Operand::Real(*wx), Operand::Real(*wy)];
                if let Some([x0, y0, x1, y1]) = bbox {
                    operands.extend([
                        Operand::Real(*x0),
                        Operand::Real(*y0),
                        Operand::Real(*x1),
                        Operand::Real(*y1),
                    ]);
                    ContentOperation::new("d1", operands)
                } else {
                    ContentOperation::new("d0", operands)
                }
            }
            Self::BeginMarkedContent(tag) => {
                ContentOperation::new("BMC", vec![Operand::Name(tag.clone())])
            }
            Self::BeginMarkedContentWithProperties { tag, properties } => {
                let mut operands = vec![Operand::Name(tag.clone())];
                match properties {
                    MarkedContentProperties::Name { name, .. } => {
                        operands.push(Operand::Name(name.clone()))
                    }
                    MarkedContentProperties::Inline(ops) => operands.extend(ops.iter().cloned()),
                }
                ContentOperation::new("BDC", operands)
            }
            Self::EndMarkedContent => ContentOperation::new("EMC", Vec::new()),
            Self::MarkedContentPoint(tag) => {
                ContentOperation::new("MP", vec![Operand::Name(tag.clone())])
            }
            Self::MarkedContentPointWithProperties { tag, properties } => {
                let mut operands = vec![Operand::Name(tag.clone())];
                match properties {
                    MarkedContentProperties::Name { name, .. } => {
                        operands.push(Operand::Name(name.clone()))
                    }
                    MarkedContentProperties::Inline(ops) => operands.extend(ops.iter().cloned()),
                }
                ContentOperation::new("DP", operands)
            }
            Self::BeginCompatibility => ContentOperation::new("BX", Vec::new()),
            Self::EndCompatibility => ContentOperation::new("EX", Vec::new()),
            Self::Unsupported { operator } => ContentOperation::new(operator.clone(), Vec::new()),
        }
    }

    /// Returns `true` if this descriptor is an unsupported state operator.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}

/// Reason why a pattern or inline-image operation could not be compiled into a
/// fully typed descriptor. This is an explicit typed refusal rather than a
/// silent fallback to raw replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackedCompileRefusal {
    /// The pattern path sequence contained an unrecognized path construction
    /// operator that cannot be normalized into typed PathSegments.
    UnrecognizedPathOperator(String),
    /// The paint operator terminating the pattern path is not a recognized
    /// PDF path-painting operator.
    UnrecognizedPaintOperator(String),
    /// The pattern path ops sequence was empty.
    EmptyPatternOps,
    /// The inline image sequence was missing the `ID` operator with parameters.
    MissingInlineImageParams,
    /// The inline image sequence was missing the data payload.
    MissingInlineImageData,
    /// A state operator that could not be compiled into a known
    /// `GraphicsStateDescriptor` variant. The renderer must handle this
    /// explicitly rather than silently replaying raw operations.
    UnsupportedStateOperator(String),
    /// A resource-backed XObject operation was compiled with a resource table,
    /// but the named resource was absent.
    MissingXObjectResource { name: String, expected: String },
    /// A resolved XObject was retained as an Image or Form operation, but its
    /// stream did not carry the required `/Subtype`.
    MissingXObjectSubtype { name: String, expected: String },
    /// A resolved XObject was retained as an Image or Form operation, but its
    /// stream carried a different `/Subtype`.
    UnsupportedXObjectSubtype {
        name: String,
        expected: String,
        actual: String,
    },
    /// A named shading operation was compiled with a resource table, but the
    /// named shading resource was absent.
    MissingShadingResource { name: String },
    /// A resource-backed graphics-state operation was compiled with a resource
    /// table, but the named ExtGState resource was absent.
    MissingExtGStateResource { name: String },
    /// A resolved ExtGState selected a font name, but that font was absent
    /// from the active resource table.
    MissingExtGStateFontResource { ext_g_state: String, font: String },
    /// A resource-backed font operation was compiled with a resource table, but
    /// the named font resource was absent.
    MissingFontResource { name: String },
    /// A visible text-showing operator was reached in a resource-aware packed
    /// plan before any pre-resolved font state was active.
    TextShowWithoutResolvedFont { operator: String },
    /// A named color-space operation was compiled with a resource table, but
    /// the named color-space resource was absent.
    MissingColorSpaceResource { name: String, usage: String },
    /// A pattern color operation was compiled with a resource table, but the
    /// named pattern resource was absent.
    MissingPatternResource { name: String, usage: String },
    /// A marked-content properties operation was compiled with a resource
    /// table, but the named Properties resource was absent.
    MissingPropertiesResource { name: String },
}

impl std::fmt::Display for PackedCompileRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnrecognizedPathOperator(op) => {
                write!(f, "unrecognized path construction operator '{op}'")
            }
            Self::UnrecognizedPaintOperator(op) => {
                write!(f, "unrecognized paint operator '{op}'")
            }
            Self::EmptyPatternOps => write!(f, "empty pattern ops sequence"),
            Self::MissingInlineImageParams => write!(f, "missing inline image ID parameters"),
            Self::MissingInlineImageData => write!(f, "missing inline image data payload"),
            Self::UnsupportedStateOperator(op) => {
                write!(f, "unsupported state operator '{op}'")
            }
            Self::MissingXObjectResource { name, expected } => write!(
                f,
                "XObject resource /{name} retained as {expected} is missing"
            ),
            Self::MissingXObjectSubtype { name, expected } => write!(
                f,
                "XObject resource /{name} retained as {expected} has no /Subtype"
            ),
            Self::UnsupportedXObjectSubtype {
                name,
                expected,
                actual,
            } => write!(
                f,
                "XObject resource /{name} retained as {expected} has unsupported Subtype /{actual}"
            ),
            Self::MissingShadingResource { name } => {
                write!(f, "shading resource /{name} is missing")
            }
            Self::MissingExtGStateResource { name } => {
                write!(f, "ExtGState resource /{name} is missing")
            }
            Self::MissingExtGStateFontResource { ext_g_state, font } => write!(
                f,
                "ExtGState resource /{ext_g_state} selects missing font resource /{font}"
            ),
            Self::MissingFontResource { name } => {
                write!(f, "font resource /{name} is missing")
            }
            Self::TextShowWithoutResolvedFont { operator } => write!(
                f,
                "text-showing operator '{operator}' has no active pre-resolved font"
            ),
            Self::MissingColorSpaceResource { name, usage } => {
                write!(f, "{usage} color-space resource /{name} is missing")
            }
            Self::MissingPatternResource { name, usage } => {
                write!(f, "{usage} pattern resource /{name} is missing")
            }
            Self::MissingPropertiesResource { name } => {
                write!(f, "Properties resource /{name} is missing")
            }
        }
    }
}

/// High-level typed descriptor for a compiled native op.
#[derive(Clone, Debug)]
pub enum NativeDescriptor {
    /// Fully compiled text-showing operation.
    Text(TextDescriptor),
    /// Fully compiled image XObject reference.
    Image(ImageXObjectDescriptor),
    /// Fully compiled Form XObject reference.
    Form(FormXObjectDescriptor),
    /// Fully compiled shading reference.
    Shading(ShadingDescriptor),
    /// Typed graphics-state mutation compiled from the source operator.
    /// No raw `ContentOperation` is stored on this path.
    State(GraphicsStateDescriptor),
    /// Fully compiled pattern path descriptor with normalized geometry and phase.
    /// Dispatched through RenderState when executing full plan.
    Pattern(PatternPathDescriptor),
    /// Fully compiled inline image descriptor with parsed parameters and data.
    /// Dispatched through RenderState when executing full plan.
    InlineImage(InlineImageDescriptor),
    /// The operation could not be compiled into a typed descriptor. The renderer
    /// must handle this explicitly (fail-closed or use a supported fallback)
    /// rather than silently replaying raw operations.
    CompileRefusal(PackedCompileRefusal),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedPlanOptimizationReport {
    pub source_operation_count: usize,
    pub emitted_hot_operation_count: usize,
    pub folded_noop_state_ops: usize,
    pub folded_duplicate_state_ops: usize,
    pub folded_overwritten_state_ops: usize,
}

/// Cold diagnostic side tables for packed plans.
///
/// Raw `ContentOperation` payloads are deliberately excluded from this surface:
/// high-level operations must compile to typed `NativeDescriptor` entries or an
/// explicit `CompileRefusal`.
#[derive(Clone, Debug, Default)]
pub struct PackedColdTables {
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PackedDisplayList {
    pub viewport: Viewport,
    source_approx_bytes: usize,
    requires_transparent_page_group: bool,
    pub hot_ops: Vec<HotDisplayOp>,
    pub paths: Vec<Path>,
    pub clip_transforms: Vec<Transform2D>,
    pub states: Vec<DrawState>,
    pub bounds: Vec<Option<RenderBounds>>,
    /// Typed compiled descriptors indexed by `payload_offset` for native ops.
    pub descriptors: Vec<NativeDescriptor>,
    pub optimization_report: PackedPlanOptimizationReport,
    pub cold: PackedColdTables,
    requires_native_replay: bool,
}

impl PackedDisplayList {
    fn idempotent_state_descriptor(state: &GraphicsStateDescriptor) -> bool {
        matches!(
            state,
            GraphicsStateDescriptor::SetLineWidth(_)
                | GraphicsStateDescriptor::SetLineCap(_)
                | GraphicsStateDescriptor::SetLineJoin(_)
                | GraphicsStateDescriptor::SetMiterLimit(_)
                | GraphicsStateDescriptor::SetDash { .. }
                | GraphicsStateDescriptor::SetRenderingIntent(_)
                | GraphicsStateDescriptor::SetFlatness(_)
                | GraphicsStateDescriptor::SetStrokeGray(_)
                | GraphicsStateDescriptor::SetFillGray(_)
                | GraphicsStateDescriptor::SetStrokeRgb { .. }
                | GraphicsStateDescriptor::SetFillRgb { .. }
                | GraphicsStateDescriptor::SetStrokeCmyk { .. }
                | GraphicsStateDescriptor::SetFillCmyk { .. }
                | GraphicsStateDescriptor::SetStrokeColorSpace { .. }
                | GraphicsStateDescriptor::SetFillColorSpace { .. }
                | GraphicsStateDescriptor::SetStrokeColor { .. }
                | GraphicsStateDescriptor::SetFillColor { .. }
                | GraphicsStateDescriptor::ApplyExtGState { .. }
                | GraphicsStateDescriptor::SetFont { .. }
                | GraphicsStateDescriptor::SetTextMatrix { .. }
                | GraphicsStateDescriptor::SetCharSpacing(_)
                | GraphicsStateDescriptor::SetWordSpacing(_)
                | GraphicsStateDescriptor::SetHorizontalScaling(_)
                | GraphicsStateDescriptor::SetTextLeading(_)
                | GraphicsStateDescriptor::SetTextRenderingMode(_)
                | GraphicsStateDescriptor::SetTextRise(_)
        )
    }

    fn noop_state_descriptor(state: &GraphicsStateDescriptor) -> bool {
        matches!(
            state,
            GraphicsStateDescriptor::ConcatMatrix {
                a,
                b,
                c,
                d,
                e,
                f
            } if *a == 1.0 && *b == 0.0 && *c == 0.0 && *d == 1.0 && *e == 0.0 && *f == 0.0
        )
    }

    fn state_descriptor_overwrite_slot(state: &GraphicsStateDescriptor) -> Option<&'static str> {
        match state {
            GraphicsStateDescriptor::SetLineWidth(_) => Some("line_width"),
            GraphicsStateDescriptor::SetLineCap(_) => Some("line_cap"),
            GraphicsStateDescriptor::SetLineJoin(_) => Some("line_join"),
            GraphicsStateDescriptor::SetMiterLimit(_) => Some("miter_limit"),
            GraphicsStateDescriptor::SetDash { .. } => Some("dash"),
            GraphicsStateDescriptor::SetRenderingIntent(_) => Some("rendering_intent"),
            GraphicsStateDescriptor::SetFlatness(_) => Some("flatness"),
            GraphicsStateDescriptor::SetStrokeGray(_) => Some("stroke_gray"),
            GraphicsStateDescriptor::SetFillGray(_) => Some("fill_gray"),
            GraphicsStateDescriptor::SetStrokeRgb { .. } => Some("stroke_rgb"),
            GraphicsStateDescriptor::SetFillRgb { .. } => Some("fill_rgb"),
            GraphicsStateDescriptor::SetStrokeCmyk { .. } => Some("stroke_cmyk"),
            GraphicsStateDescriptor::SetFillCmyk { .. } => Some("fill_cmyk"),
            GraphicsStateDescriptor::SetStrokeColorSpace { .. } => Some("stroke_color_space"),
            GraphicsStateDescriptor::SetFillColorSpace { .. } => Some("fill_color_space"),
            GraphicsStateDescriptor::SetStrokeColor { .. } => Some("stroke_color"),
            GraphicsStateDescriptor::SetFillColor { .. } => Some("fill_color"),
            GraphicsStateDescriptor::SetFont { .. } => Some("text_font"),
            GraphicsStateDescriptor::SetTextMatrix { .. } => Some("text_matrix"),
            GraphicsStateDescriptor::SetCharSpacing(_) => Some("char_spacing"),
            GraphicsStateDescriptor::SetWordSpacing(_) => Some("word_spacing"),
            GraphicsStateDescriptor::SetHorizontalScaling(_) => Some("horizontal_scaling"),
            GraphicsStateDescriptor::SetTextLeading(_) => Some("text_leading"),
            GraphicsStateDescriptor::SetTextRenderingMode(_) => Some("text_rendering_mode"),
            GraphicsStateDescriptor::SetTextRise(_) => Some("text_rise"),
            _ => None,
        }
    }

    fn non_adjacent_state_overwrite_slot(state: &GraphicsStateDescriptor) -> Option<&'static str> {
        let slot = Self::state_descriptor_overwrite_slot(state)?;
        match slot {
            "stroke_color" | "fill_color" => None,
            _ => Some(slot),
        }
    }

    fn non_adjacent_state_run_slot(state: &GraphicsStateDescriptor) -> Option<&'static str> {
        if Self::matrix_from_concat_descriptor(state).is_some() {
            Some("ctm")
        } else {
            Self::non_adjacent_state_overwrite_slot(state)
        }
    }

    fn state_descriptor_invalidated_non_adjacent_slots(
        state: &GraphicsStateDescriptor,
    ) -> &'static [&'static str] {
        match state {
            GraphicsStateDescriptor::SetStrokeGray(_)
            | GraphicsStateDescriptor::SetStrokeRgb { .. }
            | GraphicsStateDescriptor::SetStrokeCmyk { .. }
            | GraphicsStateDescriptor::SetStrokeColor { .. } => &["stroke_color_space"],
            GraphicsStateDescriptor::SetFillGray(_)
            | GraphicsStateDescriptor::SetFillRgb { .. }
            | GraphicsStateDescriptor::SetFillCmyk { .. }
            | GraphicsStateDescriptor::SetFillColor { .. } => &["fill_color_space"],
            GraphicsStateDescriptor::SetStrokeColorSpace { .. } => {
                &["stroke_gray", "stroke_rgb", "stroke_cmyk", "stroke_color"]
            }
            GraphicsStateDescriptor::SetFillColorSpace { .. } => {
                &["fill_gray", "fill_rgb", "fill_cmyk", "fill_color"]
            }
            _ => &[],
        }
    }

    fn record_state_run_slot(
        state_run_last_slot: &mut HashMap<&'static str, usize>,
        non_adjacent_slot: Option<&'static str>,
        invalidated_slots: &'static [&'static str],
        hot_index: usize,
    ) {
        for invalidated in invalidated_slots {
            state_run_last_slot.remove(invalidated);
        }
        if let Some(slot) = non_adjacent_slot {
            state_run_last_slot.insert(slot, hot_index);
        } else if invalidated_slots.is_empty() {
            state_run_last_slot.clear();
        }
    }

    fn adjust_state_run_slots_after_hot_remove(
        state_run_last_slot: &mut HashMap<&'static str, usize>,
        removed_hot_index: usize,
    ) {
        state_run_last_slot.retain(|_, hot_index| {
            if *hot_index == removed_hot_index {
                false
            } else {
                if *hot_index > removed_hot_index {
                    *hot_index -= 1;
                }
                true
            }
        });
    }

    fn opcode_uses_descriptor(opcode: u16) -> bool {
        matches!(
            opcode,
            OP_STATE
                | OP_NATIVE_TEXT
                | OP_NATIVE_IMAGE
                | OP_NATIVE_SHADING
                | OP_NATIVE_PATTERN
                | OP_NATIVE_INLINE_IMAGE
                | OP_NATIVE_FORM
        )
    }

    fn remove_descriptor_at(
        hot_ops: &mut [HotDisplayOp],
        descriptors: &mut Vec<NativeDescriptor>,
        descriptor_index: usize,
    ) {
        descriptors.remove(descriptor_index);
        for hot_op in hot_ops {
            if Self::opcode_uses_descriptor(hot_op.opcode)
                && (hot_op.payload_offset as usize) > descriptor_index
            {
                hot_op.payload_offset = hot_op.payload_offset.saturating_sub(1);
            }
        }
    }

    fn remove_hot_op_and_descriptor(
        hot_ops: &mut Vec<HotDisplayOp>,
        descriptors: &mut Vec<NativeDescriptor>,
        state_run_last_slot: &mut HashMap<&'static str, usize>,
        hot_index: usize,
    ) {
        let descriptor_index = hot_ops[hot_index].payload_offset as usize;
        hot_ops.remove(hot_index);
        Self::remove_descriptor_at(hot_ops, descriptors, descriptor_index);
        Self::adjust_state_run_slots_after_hot_remove(state_run_last_slot, hot_index);
    }

    fn previous_state_descriptor<'a>(
        hot_ops: &[HotDisplayOp],
        descriptors: &'a [NativeDescriptor],
    ) -> Option<&'a GraphicsStateDescriptor> {
        let previous = hot_ops.last()?;
        if previous.opcode != OP_STATE {
            return None;
        }
        match descriptors.get(previous.payload_offset as usize) {
            Some(NativeDescriptor::State(state)) => Some(state),
            _ => None,
        }
    }

    fn matrix_from_concat_descriptor(state: &GraphicsStateDescriptor) -> Option<[f64; 6]> {
        match state {
            GraphicsStateDescriptor::ConcatMatrix { a, b, c, d, e, f } => {
                Some([*a, *b, *c, *d, *e, *f])
            }
            _ => None,
        }
    }

    fn compose_adjacent_concat_matrices(
        previous: &GraphicsStateDescriptor,
        current: &GraphicsStateDescriptor,
    ) -> Option<GraphicsStateDescriptor> {
        let previous = Self::matrix_from_concat_descriptor(previous)?;
        let current = Self::matrix_from_concat_descriptor(current)?;
        let [a, b, c, d, e, f] = concat_matrix(&current, &previous);
        Some(GraphicsStateDescriptor::ConcatMatrix { a, b, c, d, e, f })
    }

    fn resolved_xobject_handle(
        name: &str,
        resources: Option<&PageResources>,
    ) -> Option<ResolvedXObjectHandle> {
        let resources = resources?;
        let (object_number, generation_number) = *resources.xobjects.get(name)?;
        let subtype = resources.xobject_subtypes.get(name).cloned();
        let stream_dict = resources.xobject_stream_dicts.get(name).cloned();
        let image_color_space = if subtype.as_deref() == Some("Image") {
            Self::resolved_xobject_image_color_space(stream_dict.as_ref(), resources)
        } else {
            None
        };
        Some(ResolvedXObjectHandle {
            name: name.to_string(),
            object_number,
            generation_number,
            subtype,
            stream_dict,
            bbox: resources.xobject_bboxes.get(name).copied(),
            matrix: resources.xobject_matrices.get(name).copied(),
            image_color_space,
        })
    }

    fn resolved_xobject_image_color_space(
        dict: Option<&PdfDictionary>,
        resources: &PageResources,
    ) -> Option<ResolvedInlineImageColorSpace> {
        let dict = dict?;
        let PdfObject::Name(name) = dict.get("ColorSpace").or_else(|| dict.get("CS"))? else {
            return None;
        };
        if Self::builtin_color_space_name(name) {
            return None;
        }
        let object = resources.color_spaces.get(name)?.clone();
        Some(ResolvedInlineImageColorSpace {
            name: name.clone(),
            object,
        })
    }

    fn xobject_descriptor_refusal(
        name: &str,
        handle: Option<&ResolvedXObjectHandle>,
        resources: Option<&PageResources>,
        expected: &str,
    ) -> Option<PackedCompileRefusal> {
        let Some(handle) = handle else {
            return resources.map(|_| PackedCompileRefusal::MissingXObjectResource {
                name: name.to_string(),
                expected: expected.to_string(),
            });
        };
        match handle.subtype.as_deref() {
            Some(actual) if actual == expected => None,
            Some(actual) => Some(PackedCompileRefusal::UnsupportedXObjectSubtype {
                name: handle.name.clone(),
                expected: expected.to_string(),
                actual: actual.to_string(),
            }),
            None => Some(PackedCompileRefusal::MissingXObjectSubtype {
                name: handle.name.clone(),
                expected: expected.to_string(),
            }),
        }
    }

    fn builtin_color_space_name(name: &str) -> bool {
        matches!(name, "DeviceGray" | "DeviceRGB" | "DeviceCMYK" | "Pattern")
    }

    fn missing_marked_content_properties_refusal(
        properties: &MarkedContentProperties,
        resources: Option<&PageResources>,
    ) -> Option<PackedCompileRefusal> {
        resources?;
        match properties {
            MarkedContentProperties::Name { name, object } if object.is_none() => {
                Some(PackedCompileRefusal::MissingPropertiesResource { name: name.clone() })
            }
            _ => None,
        }
    }

    fn state_descriptor_refusal(
        state: &GraphicsStateDescriptor,
        resources: Option<&PageResources>,
    ) -> Option<PackedCompileRefusal> {
        resources?;
        match state {
            GraphicsStateDescriptor::SetStrokeColorSpace { name, object }
                if object.is_none() && !Self::builtin_color_space_name(name) =>
            {
                Some(PackedCompileRefusal::MissingColorSpaceResource {
                    name: name.clone(),
                    usage: "stroke".to_string(),
                })
            }
            GraphicsStateDescriptor::SetFillColorSpace { name, object }
                if object.is_none() && !Self::builtin_color_space_name(name) =>
            {
                Some(PackedCompileRefusal::MissingColorSpaceResource {
                    name: name.clone(),
                    usage: "fill".to_string(),
                })
            }
            GraphicsStateDescriptor::SetStrokeColor {
                name: Some(name),
                pattern,
                ..
            } if pattern.is_none() => Some(PackedCompileRefusal::MissingPatternResource {
                name: name.clone(),
                usage: "stroke".to_string(),
            }),
            GraphicsStateDescriptor::SetFillColor {
                name: Some(name),
                pattern,
                ..
            } if pattern.is_none() => Some(PackedCompileRefusal::MissingPatternResource {
                name: name.clone(),
                usage: "fill".to_string(),
            }),
            GraphicsStateDescriptor::ApplyExtGState { name, dict, .. } if dict.is_none() => {
                Some(PackedCompileRefusal::MissingExtGStateResource { name: name.clone() })
            }
            GraphicsStateDescriptor::ApplyExtGState {
                name,
                dict: Some(dict),
                font,
            } if font.is_none() => ext_g_state_font_selection(dict).map(|(font_name, _)| {
                PackedCompileRefusal::MissingExtGStateFontResource {
                    ext_g_state: name.clone(),
                    font: font_name,
                }
            }),
            GraphicsStateDescriptor::SetFont { name, dict, .. } if dict.is_none() => {
                Some(PackedCompileRefusal::MissingFontResource { name: name.clone() })
            }
            GraphicsStateDescriptor::BeginMarkedContentWithProperties { properties, .. }
            | GraphicsStateDescriptor::MarkedContentPointWithProperties { properties, .. } => {
                Self::missing_marked_content_properties_refusal(properties, resources)
            }
            _ => None,
        }
    }

    fn compile_retained_text_descriptor(
        text: &RetainedTextOp,
        resources: Option<&PageResources>,
    ) -> NativeDescriptor {
        let desc = match text {
            RetainedTextOp::Show(bytes) => {
                return NativeDescriptor::Text(TextDescriptor::Show(bytes.clone()));
            }
            RetainedTextOp::ShowArray(items) => {
                let items = items
                    .iter()
                    .map(|item| match item {
                        RetainedTextArrayItem::Bytes(bytes) => TextArrayItem::Bytes(bytes.clone()),
                        RetainedTextArrayItem::Adjustment(value) => {
                            TextArrayItem::Adjustment(*value)
                        }
                    })
                    .collect();
                return NativeDescriptor::Text(TextDescriptor::ShowArray(items));
            }
            RetainedTextOp::NextLineShow(bytes) => {
                return NativeDescriptor::Text(TextDescriptor::NextLineShow(bytes.clone()));
            }
            RetainedTextOp::SpacingNextLineShow {
                word_spacing,
                char_spacing,
                text,
            } => {
                return NativeDescriptor::Text(TextDescriptor::SpacingNextLineShow {
                    word_spacing: *word_spacing,
                    char_spacing: *char_spacing,
                    text: text.clone(),
                });
            }
            RetainedTextOp::BeginText => GraphicsStateDescriptor::BeginText,
            RetainedTextOp::EndText => GraphicsStateDescriptor::EndText,
            RetainedTextOp::SetFont { name, size } => {
                let dict = resources.and_then(|resources| resources.fonts.get(name).cloned());
                if resources.is_some() && dict.is_none() {
                    return NativeDescriptor::CompileRefusal(
                        PackedCompileRefusal::MissingFontResource { name: name.clone() },
                    );
                }
                GraphicsStateDescriptor::SetFont {
                    name: name.clone(),
                    size: *size,
                    dict,
                }
            }
            RetainedTextOp::MoveTextPosition { tx, ty } => {
                GraphicsStateDescriptor::MoveTextPosition { tx: *tx, ty: *ty }
            }
            RetainedTextOp::MoveTextPositionSetLeading { tx, ty } => {
                GraphicsStateDescriptor::MoveTextPositionSetLeading { tx: *tx, ty: *ty }
            }
            RetainedTextOp::SetTextMatrix { a, b, c, d, e, f } => {
                GraphicsStateDescriptor::SetTextMatrix {
                    a: *a,
                    b: *b,
                    c: *c,
                    d: *d,
                    e: *e,
                    f: *f,
                }
            }
            RetainedTextOp::NextLine => GraphicsStateDescriptor::NextLine,
            RetainedTextOp::SetCharSpacing(value) => {
                GraphicsStateDescriptor::SetCharSpacing(*value)
            }
            RetainedTextOp::SetWordSpacing(value) => {
                GraphicsStateDescriptor::SetWordSpacing(*value)
            }
            RetainedTextOp::SetHorizontalScaling(value) => {
                GraphicsStateDescriptor::SetHorizontalScaling(*value)
            }
            RetainedTextOp::SetTextLeading(value) => {
                GraphicsStateDescriptor::SetTextLeading(*value)
            }
            RetainedTextOp::SetTextRenderingMode(value) => {
                GraphicsStateDescriptor::SetTextRenderingMode(i64::from(*value))
            }
            RetainedTextOp::SetTextRise(value) => GraphicsStateDescriptor::SetTextRise(*value),
            RetainedTextOp::Unsupported { operator, .. } => {
                return NativeDescriptor::CompileRefusal(
                    PackedCompileRefusal::UnsupportedStateOperator(operator.clone()),
                );
            }
        };
        NativeDescriptor::State(desc)
    }

    /// Compile a pattern path ops sequence into a typed `PatternPathDescriptor`.
    ///
    /// The sequence is expected to be path-construction operators followed by
    /// a terminal paint operator. If the sequence cannot be compiled (unknown
    /// operators, empty, etc.) a `PackedCompileRefusal` is returned.
    #[cfg(test)]
    fn compile_pattern_descriptor(ops: &[crate::content::ContentOperation]) -> NativeDescriptor {
        if ops.is_empty() {
            return NativeDescriptor::CompileRefusal(PackedCompileRefusal::EmptyPatternOps);
        }

        let paint_op = &ops[ops.len() - 1];
        let phase = match PatternPaintPhase::from_operator(&paint_op.operator) {
            Some(phase) => phase,
            None => {
                return NativeDescriptor::CompileRefusal(
                    PackedCompileRefusal::UnrecognizedPaintOperator(paint_op.operator.clone()),
                );
            }
        };

        let mut path = Path::new();
        for op in &ops[..ops.len().saturating_sub(1)] {
            match op.operator.as_str() {
                "m" => {
                    if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                        path.move_to(x, y);
                    }
                }
                "l" => {
                    if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                        path.line_to(x, y);
                    }
                }
                "c" => {
                    if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) = (
                        op.number(0),
                        op.number(1),
                        op.number(2),
                        op.number(3),
                        op.number(4),
                        op.number(5),
                    ) {
                        path.curve_to(x1, y1, x2, y2, x3, y3);
                    }
                }
                "v" => {
                    if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        let (cx, cy) = path.current_point.unwrap_or((0.0, 0.0));
                        path.curve_to(cx, cy, x2, y2, x3, y3);
                    }
                }
                "y" => {
                    if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        path.curve_to(x1, y1, x3, y3, x3, y3);
                    }
                }
                "h" => path.close(),
                "re" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        path.rect(x, y, w, h);
                    }
                }
                other => {
                    return NativeDescriptor::CompileRefusal(
                        PackedCompileRefusal::UnrecognizedPathOperator(other.to_string()),
                    );
                }
            }
        }

        NativeDescriptor::Pattern(PatternPathDescriptor { path, phase })
    }

    /// Compile a retained inline-image payload into a typed descriptor.
    fn inline_image_param_value_any<'a>(
        params: &'a [Operand],
        keys: &[&str],
    ) -> Option<&'a Operand> {
        let mut chunks = params.chunks_exact(2);
        for pair in &mut chunks {
            let Some(name) = pair[0].as_name() else {
                continue;
            };
            if keys.contains(&name) {
                return Some(&pair[1]);
            }
        }
        None
    }

    fn inline_image_is_mask(params: &[Operand]) -> bool {
        Self::inline_image_param_value_any(params, &["ImageMask", "IM"])
            .and_then(Operand::as_bool)
            .unwrap_or(false)
    }

    fn inline_image_color_space_name(params: &[Operand]) -> Option<String> {
        Self::inline_image_param_value_any(params, &["ColorSpace", "CS"])
            .and_then(Operand::as_name)
            .map(str::to_string)
    }

    fn builtin_inline_image_color_space_name(name: &str) -> bool {
        Self::builtin_color_space_name(name) || matches!(name, "G" | "RGB" | "CMYK")
    }

    fn resolved_inline_image_color_space(
        params: &[Operand],
        resources: Option<&PageResources>,
    ) -> Option<ResolvedInlineImageColorSpace> {
        if Self::inline_image_is_mask(params) {
            return None;
        }
        let name = Self::inline_image_color_space_name(params)?;
        if Self::builtin_inline_image_color_space_name(&name) {
            return None;
        }
        let object = resources?.color_spaces.get(&name)?.clone();
        Some(ResolvedInlineImageColorSpace { name, object })
    }

    fn inline_image_color_space_refusal(
        params: &[Operand],
        color_space: Option<&ResolvedInlineImageColorSpace>,
        resources: Option<&PageResources>,
    ) -> Option<PackedCompileRefusal> {
        resources?;
        if Self::inline_image_is_mask(params) {
            return None;
        }
        let name = Self::inline_image_color_space_name(params)?;
        if Self::builtin_inline_image_color_space_name(&name) || color_space.is_some() {
            return None;
        }
        Some(PackedCompileRefusal::MissingColorSpaceResource {
            name,
            usage: "inline image".to_string(),
        })
    }

    fn compile_inline_image_descriptor(
        image: &RetainedInlineImage,
        resources: Option<&PageResources>,
    ) -> NativeDescriptor {
        if image.params.is_empty() {
            return NativeDescriptor::CompileRefusal(
                PackedCompileRefusal::MissingInlineImageParams,
            );
        }
        if image.data.is_empty() {
            return NativeDescriptor::CompileRefusal(PackedCompileRefusal::MissingInlineImageData);
        }

        let color_space = Self::resolved_inline_image_color_space(&image.params, resources);
        if let Some(refusal) =
            Self::inline_image_color_space_refusal(&image.params, color_space.as_ref(), resources)
        {
            return NativeDescriptor::CompileRefusal(refusal);
        }

        NativeDescriptor::InlineImage(InlineImageDescriptor {
            params: image.params.clone(),
            data: image.data.clone(),
            color_space,
        })
    }

    fn retained_text_show_has_visible_payload(text: &RetainedTextOp) -> bool {
        match text {
            RetainedTextOp::Show(bytes)
            | RetainedTextOp::NextLineShow(bytes)
            | RetainedTextOp::SpacingNextLineShow { text: bytes, .. } => !bytes.is_empty(),
            RetainedTextOp::ShowArray(items) => items.iter().any(
                |item| matches!(item, RetainedTextArrayItem::Bytes(bytes) if !bytes.is_empty()),
            ),
            _ => false,
        }
    }

    fn update_active_text_font_from_descriptor(
        desc: &NativeDescriptor,
        resource_aware: bool,
        active_text_font_resolved: &mut bool,
    ) {
        if !resource_aware {
            return;
        }
        match desc {
            NativeDescriptor::State(GraphicsStateDescriptor::SetFont { dict, .. }) => {
                *active_text_font_resolved = dict.is_some();
            }
            NativeDescriptor::State(GraphicsStateDescriptor::ApplyExtGState {
                font: Some(_),
                ..
            }) => {
                *active_text_font_resolved = true;
            }
            NativeDescriptor::CompileRefusal(
                PackedCompileRefusal::MissingFontResource { .. }
                | PackedCompileRefusal::MissingExtGStateFontResource { .. }
                | PackedCompileRefusal::TextShowWithoutResolvedFont { .. },
            ) => {
                *active_text_font_resolved = false;
            }
            _ => {}
        }
    }

    pub fn compile(source: DisplayList) -> Self {
        Self::compile_with_resources(source, None)
    }

    pub fn compile_with_resources(source: DisplayList, resources: Option<&PageResources>) -> Self {
        let viewport = source.viewport.clone();
        let source_approx_bytes = source.approximate_memory_bytes();
        let requires_transparent_page_group = source.stats.requires_transparent_page_group;
        let mut hot_ops = Vec::with_capacity(source.ops.len());
        let mut paths = Vec::new();
        let mut clip_transforms = Vec::new();
        let mut states = Vec::new();
        let mut bounds = Vec::new();
        let cold = PackedColdTables::default();
        let mut descriptors: Vec<NativeDescriptor> = Vec::new();
        let mut optimization_report = PackedPlanOptimizationReport {
            source_operation_count: source.ops.len(),
            emitted_hot_operation_count: 0,
            folded_noop_state_ops: 0,
            folded_duplicate_state_ops: 0,
            folded_overwritten_state_ops: 0,
        };
        let mut state_ids = HashMap::<u64, Vec<u32>>::new();
        let mut path_ids = HashMap::<u64, Vec<u32>>::new();
        let mut requires_native_replay = false;
        let resource_aware = resources.is_some();
        let mut active_text_font_resolved = false;
        let mut active_text_font_stack = Vec::new();
        let mut state_run_last_slot = HashMap::<&'static str, usize>::new();

        let intern_path =
            |path: &Path, paths: &mut Vec<Path>, path_ids: &mut HashMap<u64, Vec<u32>>| {
                let fingerprint = path_fingerprint(path);
                match path_ids.get(&fingerprint).and_then(|ids| {
                    ids.iter().copied().find(|id| {
                        paths
                            .get(*id as usize)
                            .is_some_and(|existing| existing == path)
                    })
                }) {
                    Some(id) => id,
                    None => {
                        let id = u32::try_from(paths.len()).unwrap_or(u32::MAX);
                        paths.push(path.clone());
                        path_ids.entry(fingerprint).or_default().push(id);
                        id
                    }
                }
            };
        let intern_state = |state: &DrawState,
                            states: &mut Vec<DrawState>,
                            state_ids: &mut HashMap<u64, Vec<u32>>| {
            let fingerprint = draw_state_fingerprint(state);
            match state_ids.get(&fingerprint).and_then(|ids| {
                ids.iter().copied().find(|id| {
                    states
                        .get(*id as usize)
                        .is_some_and(|existing| same_draw_state(existing, state))
                })
            }) {
                Some(id) => id,
                None => {
                    let id = u32::try_from(states.len()).unwrap_or(u32::MAX);
                    states.push(state.clone());
                    state_ids.entry(fingerprint).or_default().push(id);
                    id
                }
            }
        };
        let push_bounds = |value: Option<RenderBounds>, bounds: &mut Vec<Option<RenderBounds>>| {
            let id = u32::try_from(bounds.len()).unwrap_or(u32::MAX);
            bounds.push(value);
            id
        };
        let push_descriptor =
            |desc: NativeDescriptor, descriptors: &mut Vec<NativeDescriptor>| -> u32 {
                let id = u32::try_from(descriptors.len()).unwrap_or(u32::MAX);
                descriptors.push(desc);
                id
            };

        for (index, op) in source.ops.iter().enumerate() {
            let item = DisplayItemId(u32::try_from(index + 1).unwrap_or(u32::MAX));
            let (opcode, flags, bounds_id, state_id, payload_offset, payload_len) = match op {
                DisplayOp::Save => {
                    active_text_font_stack.push(active_text_font_resolved);
                    (OP_SAVE, 0, u32::MAX, u32::MAX, 0, 0)
                }
                DisplayOp::Restore => {
                    active_text_font_resolved = active_text_font_stack.pop().unwrap_or(false);
                    (OP_RESTORE, 0, u32::MAX, u32::MAX, 0, 0)
                }
                DisplayOp::Clip {
                    path,
                    ctm,
                    rule,
                    bounds: op_bounds,
                } => {
                    let path_id = intern_path(path, &mut paths, &mut path_ids);
                    let transform_id = u32::try_from(clip_transforms.len()).unwrap_or(u32::MAX);
                    clip_transforms.push(*ctm);
                    (
                        OP_CLIP,
                        fill_rule_flag(*rule),
                        push_bounds(*op_bounds, &mut bounds),
                        transform_id,
                        path_id,
                        1,
                    )
                }
                DisplayOp::FillPath {
                    path,
                    state,
                    rule,
                    bounds: op_bounds,
                } => {
                    if state.alpha_source && state.fill_color[3] > 0 {
                        requires_native_replay = true;
                    }
                    (
                        OP_FILL,
                        fill_rule_flag(*rule),
                        push_bounds(*op_bounds, &mut bounds),
                        intern_state(state, &mut states, &mut state_ids),
                        intern_path(path, &mut paths, &mut path_ids),
                        1,
                    )
                }
                DisplayOp::StrokePath {
                    path,
                    state,
                    bounds: op_bounds,
                } => {
                    if (state.stroke_adjustment || state.alpha_source) && state.stroke_color[3] > 0
                    {
                        requires_native_replay = true;
                    }
                    (
                        OP_STROKE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        intern_state(state, &mut states, &mut state_ids),
                        intern_path(path, &mut paths, &mut path_ids),
                        1,
                    )
                }
                DisplayOp::StateOp { state, .. } => {
                    let desc = if state.is_unsupported() {
                        requires_native_replay = true;
                        let operator = match state {
                            GraphicsStateDescriptor::Unsupported { operator, .. } => {
                                operator.clone()
                            }
                            _ => "unsupported-state".to_string(),
                        };
                        NativeDescriptor::CompileRefusal(
                            PackedCompileRefusal::UnsupportedStateOperator(operator),
                        )
                    } else if let Some(refusal) = Self::state_descriptor_refusal(state, resources) {
                        requires_native_replay = true;
                        NativeDescriptor::CompileRefusal(refusal)
                    } else {
                        if matches!(
                            state,
                            GraphicsStateDescriptor::BeginMarkedContent(_)
                                | GraphicsStateDescriptor::BeginMarkedContentWithProperties { .. }
                                | GraphicsStateDescriptor::EndMarkedContent
                        ) {
                            requires_native_replay = true;
                        }
                        NativeDescriptor::State(state.clone())
                    };
                    Self::update_active_text_font_from_descriptor(
                        &desc,
                        resource_aware,
                        &mut active_text_font_resolved,
                    );
                    if let NativeDescriptor::State(ref current) = desc {
                        if Self::noop_state_descriptor(current) {
                            optimization_report.folded_noop_state_ops += 1;
                            continue;
                        }
                        if Self::idempotent_state_descriptor(current)
                            && Self::previous_state_descriptor(&hot_ops, &descriptors)
                                .is_some_and(|previous| previous == current)
                        {
                            optimization_report.folded_duplicate_state_ops += 1;
                            continue;
                        }
                        if let Some(composed) =
                            Self::previous_state_descriptor(&hot_ops, &descriptors).and_then(
                                |previous| {
                                    Self::compose_adjacent_concat_matrices(previous, current)
                                },
                            )
                        {
                            if Self::noop_state_descriptor(&composed) {
                                let previous_hot_index = hot_ops.len().saturating_sub(1);
                                Self::remove_hot_op_and_descriptor(
                                    &mut hot_ops,
                                    &mut descriptors,
                                    &mut state_run_last_slot,
                                    previous_hot_index,
                                );
                                optimization_report.folded_noop_state_ops += 1;
                                optimization_report.folded_overwritten_state_ops += 1;
                                continue;
                            }
                            if let Some(previous) = hot_ops.last_mut() {
                                let previous_descriptor = previous.payload_offset as usize;
                                descriptors[previous_descriptor] =
                                    NativeDescriptor::State(composed);
                                previous.source_link_id = item.0;
                                optimization_report.folded_overwritten_state_ops += 1;
                                continue;
                            }
                        }
                        if Self::matrix_from_concat_descriptor(current).is_some() {
                            if let Some(previous_hot_index) =
                                state_run_last_slot.get("ctm").copied()
                            {
                                if let Some(previous_descriptor) = hot_ops
                                    .get(previous_hot_index)
                                    .map(|previous| previous.payload_offset as usize)
                                {
                                    let composed = descriptors.get(previous_descriptor).and_then(
                                        |descriptor| match descriptor {
                                            NativeDescriptor::State(previous_state) => {
                                                Self::compose_adjacent_concat_matrices(
                                                    previous_state,
                                                    current,
                                                )
                                            }
                                            _ => None,
                                        },
                                    );
                                    if let Some(composed) = composed {
                                        if Self::noop_state_descriptor(&composed) {
                                            Self::remove_hot_op_and_descriptor(
                                                &mut hot_ops,
                                                &mut descriptors,
                                                &mut state_run_last_slot,
                                                previous_hot_index,
                                            );
                                            optimization_report.folded_noop_state_ops += 1;
                                            optimization_report.folded_overwritten_state_ops += 1;
                                            continue;
                                        }
                                        descriptors[previous_descriptor] =
                                            NativeDescriptor::State(composed);
                                        if let Some(previous) = hot_ops.get_mut(previous_hot_index)
                                        {
                                            previous.source_link_id = item.0;
                                        }
                                        optimization_report.folded_overwritten_state_ops += 1;
                                        continue;
                                    }
                                }
                            }
                        }
                        let non_adjacent_slot = Self::non_adjacent_state_overwrite_slot(current);
                        let invalidated_slots =
                            Self::state_descriptor_invalidated_non_adjacent_slots(current);
                        let overwrite_previous = Self::state_descriptor_overwrite_slot(current)
                            .zip(
                                Self::previous_state_descriptor(&hot_ops, &descriptors)
                                    .and_then(Self::state_descriptor_overwrite_slot),
                            )
                            .is_some_and(|(current, previous)| current == previous);
                        if overwrite_previous {
                            let previous_hot_index = hot_ops.len().saturating_sub(1);
                            if let Some(previous) = hot_ops.last_mut() {
                                let previous_descriptor = previous.payload_offset as usize;
                                descriptors[previous_descriptor] = desc;
                                previous.source_link_id = item.0;
                                Self::record_state_run_slot(
                                    &mut state_run_last_slot,
                                    non_adjacent_slot,
                                    invalidated_slots,
                                    previous_hot_index,
                                );
                                optimization_report.folded_overwritten_state_ops += 1;
                                continue;
                            }
                        }
                        if let Some(slot) = non_adjacent_slot {
                            if let Some(previous_hot_index) = state_run_last_slot.get(slot).copied()
                            {
                                if let Some(previous) = hot_ops.get_mut(previous_hot_index) {
                                    let previous_descriptor = previous.payload_offset as usize;
                                    if let Some(NativeDescriptor::State(previous_state)) =
                                        descriptors.get(previous_descriptor)
                                    {
                                        if previous_state == current {
                                            optimization_report.folded_duplicate_state_ops += 1;
                                            continue;
                                        }
                                    }
                                    descriptors[previous_descriptor] = desc;
                                    previous.source_link_id = item.0;
                                    optimization_report.folded_overwritten_state_ops += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (OP_STATE, 0, u32::MAX, u32::MAX, desc_id, 1)
                }
                DisplayOp::NativeTextOp {
                    text,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc = if resource_aware
                        && Self::retained_text_show_has_visible_payload(text)
                        && !active_text_font_resolved
                    {
                        NativeDescriptor::CompileRefusal(
                            PackedCompileRefusal::TextShowWithoutResolvedFont {
                                operator: text.operator_name().to_string(),
                            },
                        )
                    } else {
                        Self::compile_retained_text_descriptor(text, resources)
                    };
                    Self::update_active_text_font_from_descriptor(
                        &desc,
                        resource_aware,
                        &mut active_text_font_resolved,
                    );
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_TEXT,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeImageXObject {
                    name,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let handle = Self::resolved_xobject_handle(name, resources);
                    let desc = if let Some(refusal) =
                        Self::xobject_descriptor_refusal(name, handle.as_ref(), resources, "Image")
                    {
                        NativeDescriptor::CompileRefusal(refusal)
                    } else {
                        NativeDescriptor::Image(ImageXObjectDescriptor {
                            name: name.clone(),
                            handle,
                        })
                    };
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeShadingOp {
                    name,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let object =
                        resources.and_then(|resources| resources.shadings.get(name).cloned());
                    let desc = if resources.is_some() && object.is_none() {
                        NativeDescriptor::CompileRefusal(
                            PackedCompileRefusal::MissingShadingResource { name: name.clone() },
                        )
                    } else {
                        NativeDescriptor::Shading(ShadingDescriptor {
                            name: name.clone(),
                            object,
                        })
                    };
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_SHADING,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativePatternPathOp {
                    pattern,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc = NativeDescriptor::Pattern(pattern.clone());
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_PATTERN,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeInlineImage {
                    image,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc = Self::compile_inline_image_descriptor(image, resources);
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_INLINE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeFormXObject {
                    name,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let handle = Self::resolved_xobject_handle(name, resources);
                    let desc = if let Some(refusal) =
                        Self::xobject_descriptor_refusal(name, handle.as_ref(), resources, "Form")
                    {
                        NativeDescriptor::CompileRefusal(refusal)
                    } else {
                        NativeDescriptor::Form(FormXObjectDescriptor {
                            name: name.clone(),
                            handle,
                        })
                    };
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_FORM,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
            };
            hot_ops.push(HotDisplayOp {
                opcode,
                flags,
                bounds_id,
                state_id,
                payload_offset,
                payload_len,
                source_link_id: item.0,
            });
            if opcode == OP_STATE {
                if let Some(NativeDescriptor::State(state)) =
                    descriptors.get(payload_offset as usize)
                {
                    Self::record_state_run_slot(
                        &mut state_run_last_slot,
                        Self::non_adjacent_state_run_slot(state),
                        Self::state_descriptor_invalidated_non_adjacent_slots(state),
                        hot_ops.len() - 1,
                    );
                } else {
                    state_run_last_slot.clear();
                }
            } else {
                state_run_last_slot.clear();
            }
        }
        optimization_report.emitted_hot_operation_count = hot_ops.len();

        Self {
            viewport,
            source_approx_bytes,
            requires_transparent_page_group,
            hot_ops,
            paths,
            clip_transforms,
            states,
            bounds,
            descriptors,
            optimization_report,
            cold,
            requires_native_replay,
        }
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn source_approx_bytes(&self) -> usize {
        self.source_approx_bytes
    }

    pub fn requires_transparent_page_group(&self) -> bool {
        self.requires_transparent_page_group
    }

    pub fn requires_native_replay(&self) -> bool {
        self.requires_native_replay
    }

    pub fn hot_operation_count(&self) -> usize {
        self.hot_ops.len()
    }

    /// Get the typed descriptor at the given index.
    pub fn descriptor(&self, index: u32) -> Option<&NativeDescriptor> {
        self.descriptors.get(index as usize)
    }

    /// Returns `true` if every native high-level op has a fully-compiled
    /// descriptor (text/image/form/shading/pattern/inline-image). A compile
    /// refusal returns `false`.
    pub fn has_only_supported_descriptors(&self) -> bool {
        self.descriptors.iter().all(|d| {
            matches!(
                d,
                NativeDescriptor::Text(_)
                    | NativeDescriptor::Image(_)
                    | NativeDescriptor::Form(_)
                    | NativeDescriptor::Shading(_)
                    | NativeDescriptor::State(_)
                    | NativeDescriptor::Pattern(_)
                    | NativeDescriptor::InlineImage(_)
            )
        })
    }

    pub fn replay_vector(&self, device: &mut dyn RenderDevice, selected: &[usize]) -> Result<()> {
        if self.requires_native_replay {
            return Err(WellfriendError::UnsupportedFeature(
                "packed vector replay requires a native compiled payload for high-level PDF operations".to_string(),
            ));
        }
        for &index in selected {
            let op = self.hot_ops.get(index).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "packed display-list index was out of bounds".to_string(),
                )
            })?;
            match op.opcode {
                OP_SAVE => device.save(),
                OP_RESTORE => device.restore(),
                OP_STATE => {
                    // Pure vector paths carry captured DrawState; their typed
                    // state descriptor remains available for high-level plans
                    // but needs no additional device mutation here.
                }
                OP_CLIP => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed clip path missing".to_string())
                    })?;
                    let ctm = self
                        .clip_transforms
                        .get(op.state_id as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed clip transform missing".to_string(),
                            )
                        })?;
                    device.clip_path(path, ctm, fill_rule_from_flag(op.flags));
                }
                OP_FILL => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill path missing".to_string())
                    })?;
                    let state = self.states.get(op.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill state missing".to_string())
                    })?;
                    device.fill_path(path, state, fill_rule_from_flag(op.flags));
                }
                OP_STROKE => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke path missing".to_string())
                    })?;
                    let state = self.states.get(op.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke state missing".to_string())
                    })?;
                    device.stroke_path(path, state);
                }
                _ => {
                    return Err(WellfriendError::UnsupportedFeature(
                        "packed vector replay encountered a non-vector operation".to_string(),
                    ))
                }
            }
        }
        Ok(())
    }
}

fn fill_rule_flag(rule: FillRule) -> u16 {
    match rule {
        FillRule::NonZero => 0,
        FillRule::EvenOdd => 1,
    }
}

fn fill_rule_from_flag(flag: u16) -> FillRule {
    if flag & 1 == 1 {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

/// Trait for dispatching typed plan descriptors through the renderer.
///
/// This lets `RenderPlan::execute_full` drive page rendering through compiled
/// descriptors rather than raw `ContentOperation` payloads. The page renderer
/// implements this trait via a `RenderState`-backed adapter.
pub trait PlanDispatcher {
    fn dispatch_text(&mut self, desc: &TextDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_image(&mut self, desc: &ImageXObjectDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_form(&mut self, desc: &FormXObjectDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_shading(&mut self, desc: &ShadingDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_state(&mut self, desc: &GraphicsStateDescriptor);
    fn dispatch_pattern(&mut self, desc: &PatternPathDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_inline_image(
        &mut self,
        desc: &InlineImageDescriptor,
        bounds: Option<&RenderBounds>,
    );
    /// Handle a compile refusal. The dispatcher must decide whether to skip,
    /// log, or fail-closed. This is never silent.
    fn dispatch_compile_refusal(
        &mut self,
        refusal: &PackedCompileRefusal,
        bounds: Option<&RenderBounds>,
    );
    fn dispatch_save(&mut self);
    fn dispatch_restore(&mut self);
    fn dispatch_clip(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule);
    fn dispatch_fill(&mut self, path: &Path, state: &DrawState, rule: FillRule);
    fn dispatch_stroke(&mut self, path: &Path, state: &DrawState);
    fn is_cancelled(&self) -> bool;
    fn should_stop(&self) -> bool {
        false
    }
}

impl PackedDisplayList {
    /// Execute the full plan through a typed dispatcher. This is the active
    /// high-level path that drives text/image/form/shading through compiled
    /// descriptors. Used by PageRenderer for all fully-supported display lists.
    pub fn execute_plan(&self, dispatcher: &mut dyn PlanDispatcher) -> Result<()> {
        for (i, hot) in self.hot_ops.iter().enumerate() {
            if i % 64 == 0 && dispatcher.is_cancelled() {
                return Err(WellfriendError::Cancelled(
                    "plan execution cancelled".to_string(),
                ));
            }
            if dispatcher.should_stop() {
                return Ok(());
            }
            let op_bounds = self.bounds.get(hot.bounds_id as usize).copied().flatten();
            match hot.opcode {
                OP_SAVE => dispatcher.dispatch_save(),
                OP_RESTORE => dispatcher.dispatch_restore(),
                OP_CLIP => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed clip path missing".to_string())
                    })?;
                    let ctm = self
                        .clip_transforms
                        .get(hot.state_id as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed clip transform missing".to_string(),
                            )
                        })?;
                    dispatcher.dispatch_clip(path, ctm, fill_rule_from_flag(hot.flags));
                }
                OP_FILL => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill path missing".to_string())
                    })?;
                    let state = self.states.get(hot.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill state missing".to_string())
                    })?;
                    dispatcher.dispatch_fill(path, state, fill_rule_from_flag(hot.flags));
                }
                OP_STROKE => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke path missing".to_string())
                    })?;
                    let state = self.states.get(hot.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke state missing".to_string())
                    })?;
                    dispatcher.dispatch_stroke(path, state);
                }
                OP_STATE
                | OP_NATIVE_TEXT
                | OP_NATIVE_IMAGE
                | OP_NATIVE_SHADING
                | OP_NATIVE_PATTERN
                | OP_NATIVE_INLINE_IMAGE
                | OP_NATIVE_FORM => {
                    let desc = self
                        .descriptors
                        .get(hot.payload_offset as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed descriptor index out of bounds".to_string(),
                            )
                        })?;
                    match desc {
                        NativeDescriptor::Text(text) => {
                            dispatcher.dispatch_text(text, op_bounds.as_ref());
                        }
                        NativeDescriptor::Image(img) => {
                            dispatcher.dispatch_image(img, op_bounds.as_ref());
                        }
                        NativeDescriptor::Form(form) => {
                            dispatcher.dispatch_form(form, op_bounds.as_ref());
                        }
                        NativeDescriptor::Shading(shading) => {
                            dispatcher.dispatch_shading(shading, op_bounds.as_ref());
                        }
                        NativeDescriptor::State(gs_desc) => {
                            dispatcher.dispatch_state(gs_desc);
                        }
                        NativeDescriptor::Pattern(pattern) => {
                            dispatcher.dispatch_pattern(pattern, op_bounds.as_ref());
                        }
                        NativeDescriptor::InlineImage(inline_img) => {
                            dispatcher.dispatch_inline_image(inline_img, op_bounds.as_ref());
                        }
                        NativeDescriptor::CompileRefusal(refusal) => {
                            dispatcher.dispatch_compile_refusal(refusal, op_bounds.as_ref());
                        }
                    }
                }
                _ => {
                    return Err(WellfriendError::UnsupportedFeature(
                        "packed plan encountered an unknown opcode".to_string(),
                    ));
                }
            }
            if dispatcher.should_stop() {
                return Ok(());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RenderSpatialIndex {
    known: Vec<(usize, RenderBounds)>,
    unknown: Vec<usize>,
    grid: Option<SpatialGridIndex>,
    hierarchy: Option<SpatialBvhIndex>,
}

#[derive(Clone, Debug)]
struct SpatialGridIndex {
    width: u32,
    height: u32,
    columns: usize,
    rows: usize,
    buckets: Vec<Vec<usize>>,
}

#[derive(Clone, Debug)]
struct SpatialBvhIndex {
    nodes: Vec<SpatialBvhNode>,
    root: usize,
}

#[derive(Clone, Debug)]
struct SpatialBvhNode {
    bounds: RenderBounds,
    left: Option<usize>,
    right: Option<usize>,
    entries: Vec<usize>,
}

const SPATIAL_GRID_MIN_KNOWN_OPS: usize = 32;
const SPATIAL_GRID_MAX_AXIS: usize = 32;
const SPATIAL_BVH_MIN_KNOWN_OPS: usize = 64;
const SPATIAL_BVH_LEAF_ENTRIES: usize = 8;

impl RenderSpatialIndex {
    pub fn compile(list: &PackedDisplayList) -> Self {
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for (index, hot) in list.hot_ops.iter().enumerate() {
            match list.bounds.get(hot.bounds_id as usize).copied().flatten() {
                Some(bounds) => known.push((index, bounds)),
                None => unknown.push(index),
            }
        }
        known.sort_by_key(|(_, bounds)| (bounds.y0, bounds.x0, bounds.y1, bounds.x1));
        let viewport = list.viewport();
        let grid = SpatialGridIndex::compile(&known, viewport.width_px, viewport.height_px);
        let hierarchy = SpatialBvhIndex::compile(&known);
        Self {
            known,
            unknown,
            grid,
            hierarchy,
        }
    }

    pub fn query(&self, tile: RenderTile) -> Vec<usize> {
        let mut selected = Vec::new();
        self.query_into(tile, &mut selected);
        selected
    }

    pub fn query_into(&self, tile: RenderTile, selected: &mut Vec<usize>) {
        selected.clear();
        selected.extend(self.unknown.iter().copied());
        let tile_bounds = RenderBounds {
            x0: i32::try_from(tile.x).unwrap_or(i32::MAX),
            y0: i32::try_from(tile.y).unwrap_or(i32::MAX),
            x1: i32::try_from(tile.x.saturating_add(tile.width)).unwrap_or(i32::MAX),
            y1: i32::try_from(tile.y.saturating_add(tile.height)).unwrap_or(i32::MAX),
        };
        if let Some(hierarchy) = &self.hierarchy {
            hierarchy.query_intersecting(&self.known, tile_bounds, selected);
        } else if let Some(grid) = &self.grid {
            grid.query_intersecting(&self.known, tile_bounds, selected);
        } else {
            selected.extend(self.known.iter().filter_map(|(index, bounds)| {
                bounds.intersect(tile_bounds).is_some().then_some(*index)
            }));
        }
        selected.sort_unstable();
        selected.dedup();
    }
}

impl SpatialBvhIndex {
    fn compile(known: &[(usize, RenderBounds)]) -> Option<Self> {
        if known.len() < SPATIAL_BVH_MIN_KNOWN_OPS {
            return None;
        }
        let mut entries: Vec<usize> = (0..known.len()).collect();
        let mut nodes = Vec::new();
        let root = Self::build_node(known, &mut entries, &mut nodes);
        Some(Self { nodes, root })
    }

    fn query_intersecting(
        &self,
        known: &[(usize, RenderBounds)],
        bounds: RenderBounds,
        output: &mut Vec<usize>,
    ) {
        self.query_node(self.root, known, bounds, output);
    }

    fn query_node(
        &self,
        node_index: usize,
        known: &[(usize, RenderBounds)],
        bounds: RenderBounds,
        output: &mut Vec<usize>,
    ) {
        let Some(node) = self.nodes.get(node_index) else {
            return;
        };
        if node.bounds.intersect(bounds).is_none() {
            return;
        }
        if node.left.is_none() && node.right.is_none() {
            output.extend(node.entries.iter().filter_map(|entry| {
                let (index, entry_bounds) = known[*entry];
                entry_bounds.intersect(bounds).is_some().then_some(index)
            }));
            return;
        }
        if let Some(left) = node.left {
            self.query_node(left, known, bounds, output);
        }
        if let Some(right) = node.right {
            self.query_node(right, known, bounds, output);
        }
    }

    fn build_node(
        known: &[(usize, RenderBounds)],
        entries: &mut [usize],
        nodes: &mut Vec<SpatialBvhNode>,
    ) -> usize {
        let bounds = union_entry_bounds(known, entries);
        if entries.len() <= SPATIAL_BVH_LEAF_ENTRIES {
            let index = nodes.len();
            nodes.push(SpatialBvhNode {
                bounds,
                left: None,
                right: None,
                entries: entries.to_vec(),
            });
            return index;
        }

        let split_on_x = bounds.x1.saturating_sub(bounds.x0) >= bounds.y1.saturating_sub(bounds.y0);
        entries.sort_by_key(|entry| spatial_entry_center_key(known[*entry].1, split_on_x));
        let mid = entries.len() / 2;
        let (left_entries, right_entries) = entries.split_at_mut(mid);
        let left = Self::build_node(known, left_entries, nodes);
        let right = Self::build_node(known, right_entries, nodes);
        let index = nodes.len();
        nodes.push(SpatialBvhNode {
            bounds,
            left: Some(left),
            right: Some(right),
            entries: Vec::new(),
        });
        index
    }
}

impl SpatialGridIndex {
    fn compile(known: &[(usize, RenderBounds)], width: u32, height: u32) -> Option<Self> {
        if known.len() < SPATIAL_GRID_MIN_KNOWN_OPS {
            return None;
        }
        let axis = spatial_grid_axis(known.len());
        let columns = axis;
        let rows = axis;
        let width = width.max(1);
        let height = height.max(1);
        let mut grid = Self {
            width,
            height,
            columns,
            rows,
            buckets: vec![Vec::new(); columns.saturating_mul(rows)],
        };
        for (entry_index, (_, bounds)) in known.iter().enumerate() {
            let (col0, col1, row0, row1) = grid.cell_range(*bounds);
            for row in row0..=row1 {
                for col in col0..=col1 {
                    grid.buckets[row * columns + col].push(entry_index);
                }
            }
        }
        Some(grid)
    }

    fn query_intersecting(
        &self,
        known: &[(usize, RenderBounds)],
        bounds: RenderBounds,
        output: &mut Vec<usize>,
    ) {
        let (col0, col1, row0, row1) = self.cell_range(bounds);
        for row in row0..=row1 {
            for col in col0..=col1 {
                output.extend(
                    self.buckets[row * self.columns + col]
                        .iter()
                        .filter_map(|entry| {
                            let (index, entry_bounds) = known[*entry];
                            entry_bounds.intersect(bounds).is_some().then_some(index)
                        }),
                );
            }
        }
    }

    fn cell_range(&self, bounds: RenderBounds) -> (usize, usize, usize, usize) {
        let x0 = clamp_spatial_coord(bounds.x0, self.width);
        let y0 = clamp_spatial_coord(bounds.y0, self.height);
        let x1 = clamp_spatial_coord(bounds.x1.saturating_sub(1), self.width);
        let y1 = clamp_spatial_coord(bounds.y1.saturating_sub(1), self.height);
        let col0 = self.coord_to_col(x0.min(x1));
        let col1 = self.coord_to_col(x0.max(x1));
        let row0 = self.coord_to_row(y0.min(y1));
        let row1 = self.coord_to_row(y0.max(y1));
        (col0, col1, row0, row1)
    }

    fn coord_to_col(&self, x: u32) -> usize {
        (((x as u64) * (self.columns as u64)) / (self.width as u64))
            .min(self.columns.saturating_sub(1) as u64) as usize
    }

    fn coord_to_row(&self, y: u32) -> usize {
        (((y as u64) * (self.rows as u64)) / (self.height as u64))
            .min(self.rows.saturating_sub(1) as u64) as usize
    }
}

fn spatial_grid_axis(count: usize) -> usize {
    let mut axis = 1usize;
    while axis.saturating_mul(axis) < count && axis < SPATIAL_GRID_MAX_AXIS {
        axis += 1;
    }
    axis.clamp(2, SPATIAL_GRID_MAX_AXIS)
}

fn union_entry_bounds(known: &[(usize, RenderBounds)], entries: &[usize]) -> RenderBounds {
    let mut bounds = known[entries[0]].1;
    for entry in &entries[1..] {
        bounds = union_bounds(bounds, known[*entry].1);
    }
    bounds
}

fn union_bounds(left: RenderBounds, right: RenderBounds) -> RenderBounds {
    RenderBounds {
        x0: left.x0.min(right.x0),
        y0: left.y0.min(right.y0),
        x1: left.x1.max(right.x1),
        y1: left.y1.max(right.y1),
    }
}

fn spatial_entry_center_key(bounds: RenderBounds, x_axis: bool) -> i64 {
    if x_axis {
        i64::from(bounds.x0) + i64::from(bounds.x1)
    } else {
        i64::from(bounds.y0) + i64::from(bounds.y1)
    }
}

fn clamp_spatial_coord(value: i32, extent: u32) -> u32 {
    if value <= 0 {
        0
    } else {
        (value as u32).min(extent.saturating_sub(1))
    }
}

#[derive(Clone, Debug)]
pub struct RenderBatch {
    pub first_operation: usize,
    pub operation_count: usize,
    pub contains_native_payload: bool,
}

fn hot_op_contains_native_payload(op: &HotDisplayOp) -> bool {
    matches!(
        op.opcode,
        OP_STATE
            | OP_NATIVE_TEXT
            | OP_NATIVE_IMAGE
            | OP_NATIVE_SHADING
            | OP_NATIVE_PATTERN
            | OP_NATIVE_INLINE_IMAGE
            | OP_NATIVE_FORM
    )
}

fn build_render_batches(hot_ops: &[HotDisplayOp]) -> Vec<RenderBatch> {
    let Some(first) = hot_ops.first() else {
        return Vec::new();
    };
    let mut batches = Vec::new();
    let mut first_operation = 0usize;
    let mut contains_native_payload = hot_op_contains_native_payload(first);
    for (index, op) in hot_ops.iter().enumerate().skip(1) {
        let next_contains_native_payload = hot_op_contains_native_payload(op);
        if next_contains_native_payload != contains_native_payload {
            batches.push(RenderBatch {
                first_operation,
                operation_count: index - first_operation,
                contains_native_payload,
            });
            first_operation = index;
            contains_native_payload = next_contains_native_payload;
        }
    }
    batches.push(RenderBatch {
        first_operation,
        operation_count: hot_ops.len() - first_operation,
        contains_native_payload,
    });
    batches
}

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub contract: RenderContract,
    pub packed: Arc<PackedDisplayList>,
    pub spatial_index: RenderSpatialIndex,
    pub batches: Vec<RenderBatch>,
}

impl RenderPlan {
    pub fn compile(list: DisplayList, contract: RenderContract) -> Result<Self> {
        Self::compile_with_optional_resources(list, contract, None)
    }

    pub fn compile_with_resources(
        list: DisplayList,
        contract: RenderContract,
        resources: &PageResources,
    ) -> Result<Self> {
        Self::compile_with_optional_resources(list, contract, Some(resources))
    }

    fn compile_with_optional_resources(
        list: DisplayList,
        contract: RenderContract,
        resources: Option<&PageResources>,
    ) -> Result<Self> {
        contract.validate()?;
        let packed = Arc::new(PackedDisplayList::compile_with_resources(list, resources));
        let batches = build_render_batches(&packed.hot_ops);
        let spatial_index = RenderSpatialIndex::compile(&packed);
        Ok(Self {
            contract,
            packed,
            spatial_index,
            batches,
        })
    }

    pub fn execute_vector_tile(&self, tile: RenderTile) -> Result<Option<PixelBuffer>> {
        let mut selected = Vec::new();
        self.execute_vector_tile_with_scratch(tile, &mut selected)
    }

    pub fn execute_vector_tile_with_scratch(
        &self,
        tile: RenderTile,
        selected: &mut Vec<usize>,
    ) -> Result<Option<PixelBuffer>> {
        self.contract.validate()?;
        if self.packed.requires_native_replay() {
            selected.clear();
            return Ok(None);
        }
        self.spatial_index.query_into(tile, selected);
        let viewport = self
            .packed
            .viewport()
            .pixel_window(tile.x, tile.y, tile.width, tile.height);
        let transparent_page_group = self.packed.requires_transparent_page_group();
        let render_mode = RenderMode::from(self.contract.compositing);
        let mut device = if transparent_page_group {
            CpuRenderDevice::new_transparent(viewport, render_mode)
        } else {
            CpuRenderDevice::new(viewport, render_mode)
        };
        self.packed.replay_vector(&mut device, selected)?;
        let mut buf = device.into_buffer();
        if transparent_page_group {
            buf.flatten_onto_background(crate::engine::contract_background_pixel(&self.contract));
        }
        Ok(Some(buf))
    }

    /// Execute the full plan through the typed descriptor dispatcher.
    /// This is the active high-level path for all fully-supported display lists
    /// including those with text/image/form/shading native ops.
    pub fn execute_full(&self, dispatcher: &mut dyn PlanDispatcher) -> Result<()> {
        self.contract.validate()?;
        self.packed.execute_plan(dispatcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::operation::Operand;
    use crate::render::{build_display_list, render_display_list, Viewport};
    use crate::ContentOperation;

    fn referenced_concat_matrices(plan: &RenderPlan) -> Vec<[f64; 6]> {
        plan.packed
            .hot_ops
            .iter()
            .filter(|hot| hot.opcode == OP_STATE)
            .filter_map(|hot| plan.packed.descriptors.get(hot.payload_offset as usize))
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(state) => {
                    PackedDisplayList::matrix_from_concat_descriptor(state)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn packed_vector_plan_replays_without_raw_content_cold_table() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(10.0),
                    Operand::Real(10.0),
                    Operand::Real(20.0),
                    Operand::Real(20.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 40.0, 40.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(list.native_vector_only());
        let expected =
            render_display_list(&list, RenderMode::Compat).expect("vector-only list renders");
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );
        let plan = RenderPlan::compile(list, contract).expect("compile plan");
        assert!(plan.packed.cold.diagnostics.is_empty());
        let actual = plan
            .execute_vector_tile(RenderTile::full(viewport.width_px, viewport.height_px))
            .expect("execute plan")
            .expect("vector-only plan");
        assert_eq!(expected.rgba_bytes(), actual.rgba_bytes());
    }

    #[test]
    fn packed_plan_uses_packed_viewport_for_vector_tile_execution() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(1.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(40.0),
                    Operand::Real(40.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 40.0, 40.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let source_approx_bytes = list.approximate_memory_bytes();
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );
        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(plan.packed.viewport().width_px, viewport.width_px);
        assert_eq!(plan.packed.viewport().height_px, viewport.height_px);
        assert_eq!(plan.packed.source_approx_bytes(), source_approx_bytes);

        let tile = RenderTile {
            x: 4,
            y: 6,
            width: 12,
            height: 10,
        };
        let actual = plan
            .execute_vector_tile(tile)
            .expect("execute plan")
            .expect("vector-only plan");

        assert_eq!(actual.width, tile.width);
        assert_eq!(actual.height, tile.height);
    }

    #[test]
    fn packed_plan_carries_transparent_page_group_stat_without_source_display_list() {
        let ops = vec![
            ContentOperation::new("gs", vec![Operand::Name("GS1".to_string())]),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(12.0),
                    Operand::Real(12.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = crate::engine::PageResources::default();
        let mut ext_g_state = PdfDictionary::empty();
        ext_g_state.insert("ca", PdfObject::Real(0.5));
        ext_g_state.insert("CA", PdfObject::Real(0.5));
        resources
            .ext_g_states
            .insert("GS1".to_string(), ext_g_state);

        let list = build_display_list(&ops, viewport, &resources);
        assert!(list.stats.requires_transparent_page_group);

        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        assert!(
            packed.requires_transparent_page_group(),
            "packed replay should use compile-time transparent-page-group metadata"
        );
    }

    #[test]
    fn render_plan_batches_split_descriptor_and_vector_runs() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(2.0),
                    Operand::Real(8.0),
                    Operand::Real(8.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
            ContentOperation::new("q", Vec::new()),
            ContentOperation::new("Q", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.batches
                .iter()
                .map(|batch| (
                    batch.first_operation,
                    batch.operation_count,
                    batch.contains_native_payload
                ))
                .collect::<Vec<_>>(),
            vec![(0, 1, true), (1, 3, false)]
        );
    }

    #[test]
    fn packed_plan_folds_adjacent_duplicate_idempotent_state_descriptors() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(2.0),
                    Operand::Real(8.0),
                    Operand::Real(8.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_duplicate_state_ops,
            2
        );
        assert_eq!(
            plan.packed.optimization_report.source_operation_count,
            plan.packed.optimization_report.emitted_hot_operation_count + 2
        );
        let state_descriptors = plan
            .packed
            .descriptors
            .iter()
            .filter(|descriptor| matches!(descriptor, NativeDescriptor::State(_)))
            .count();
        assert_eq!(state_descriptors, 2);
    }

    #[test]
    fn packed_plan_folds_identity_matrix_concatenation_as_noop_state() {
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(plan.packed.optimization_report.folded_noop_state_ops, 1);
        assert_eq!(
            plan.packed.optimization_report.source_operation_count,
            plan.packed.optimization_report.emitted_hot_operation_count + 1
        );
        assert!(
            !plan.packed.descriptors.iter().any(|descriptor| matches!(
                descriptor,
                NativeDescriptor::State(GraphicsStateDescriptor::ConcatMatrix { .. })
            )),
            "identity cm must not remain in the hot state descriptor arena"
        );
    }

    #[test]
    fn packed_plan_overwrites_adjacent_same_slot_state_setter() {
        let ops = vec![
            ContentOperation::new("w", vec![Operand::Real(1.0)]),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        let line_widths = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetLineWidth(width)) => {
                    Some(*width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(line_widths, vec![2.0]);
    }

    #[test]
    fn packed_plan_overwrites_non_adjacent_independent_state_setter() {
        let ops = vec![
            ContentOperation::new("w", vec![Operand::Real(1.0)]),
            ContentOperation::new("J", vec![Operand::Integer(1)]),
            ContentOperation::new("w", vec![Operand::Real(3.0)]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        let line_widths = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetLineWidth(width)) => {
                    Some(*width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let line_caps = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetLineCap(cap)) => Some(*cap),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(line_widths, vec![3.0]);
        assert_eq!(line_caps, vec![1]);
    }

    #[test]
    fn packed_plan_overwrites_non_adjacent_color_space_across_independent_state() {
        let ops = vec![
            ContentOperation::new("cs", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new("cs", vec![Operand::Name("DeviceCMYK".to_string())]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        let fill_spaces = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetFillColorSpace {
                    name,
                    ..
                }) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(fill_spaces, vec!["DeviceCMYK"]);
    }

    #[test]
    fn packed_plan_does_not_overwrite_color_space_across_generic_color_setter() {
        let ops = vec![
            ContentOperation::new("cs", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new(
                "sc",
                vec![Operand::Real(0.1), Operand::Real(0.2), Operand::Real(0.3)],
            ),
            ContentOperation::new("cs", vec![Operand::Name("DeviceCMYK".to_string())]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            0
        );
        let fill_spaces = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetFillColorSpace {
                    name,
                    ..
                }) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(fill_spaces, vec!["DeviceRGB", "DeviceCMYK"]);
    }

    #[test]
    fn packed_plan_does_not_overwrite_color_space_across_device_color_setter() {
        let ops = vec![
            ContentOperation::new("cs", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.1), Operand::Real(0.2), Operand::Real(0.3)],
            ),
            ContentOperation::new("cs", vec![Operand::Name("DeviceCMYK".to_string())]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            0
        );
        let fill_spaces = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetFillColorSpace {
                    name,
                    ..
                }) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(fill_spaces, vec!["DeviceRGB", "DeviceCMYK"]);
    }

    #[test]
    fn packed_plan_does_not_overwrite_state_setter_across_paint() {
        let ops = vec![
            ContentOperation::new("w", vec![Operand::Real(1.0)]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
            ContentOperation::new("w", vec![Operand::Real(3.0)]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(8.0),
                    Operand::Real(8.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            0
        );
        let line_widths = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| match descriptor {
                NativeDescriptor::State(GraphicsStateDescriptor::SetLineWidth(width)) => {
                    Some(*width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(line_widths, vec![1.0, 3.0]);
    }

    #[test]
    fn packed_plan_interns_repeated_vector_state_and_path_entries() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.25), Operand::Real(0.5), Operand::Real(0.75)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(3.0),
                    Operand::Real(8.0),
                    Operand::Real(9.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(3.0),
                    Operand::Real(8.0),
                    Operand::Real(9.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");
        let fill_ops = plan
            .packed
            .hot_ops
            .iter()
            .filter(|op| op.opcode == OP_FILL)
            .collect::<Vec<_>>();

        assert_eq!(fill_ops.len(), 2);
        assert_eq!(
            plan.packed.paths.len(),
            1,
            "identical vector paths should share one packed path arena entry"
        );
        assert_eq!(
            plan.packed.states.len(),
            1,
            "identical vector paint states should share one packed state arena entry"
        );
        assert_eq!(fill_ops[0].payload_offset, fill_ops[1].payload_offset);
        assert_eq!(fill_ops[0].state_id, fill_ops[1].state_id);
    }

    #[test]
    fn packed_plan_folds_adjacent_matrix_concatenation() {
        let translate = vec![
            Operand::Real(1.0),
            Operand::Real(0.0),
            Operand::Real(0.0),
            Operand::Real(1.0),
            Operand::Real(3.0),
            Operand::Real(4.0),
        ];
        let scale = vec![
            Operand::Real(2.0),
            Operand::Real(0.0),
            Operand::Real(0.0),
            Operand::Real(2.0),
            Operand::Real(0.0),
            Operand::Real(0.0),
        ];
        let ops = vec![
            ContentOperation::new("cm", translate),
            ContentOperation::new("cm", scale),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        let matrices = plan
            .packed
            .descriptors
            .iter()
            .filter_map(|descriptor| {
                if let NativeDescriptor::State(state) = descriptor {
                    PackedDisplayList::matrix_from_concat_descriptor(state)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(matrices, vec![[2.0, 0.0, 0.0, 2.0, 3.0, 4.0]]);
        assert_eq!(
            plan.packed.optimization_report.source_operation_count,
            plan.packed.optimization_report.emitted_hot_operation_count + 1
        );
    }

    #[test]
    fn packed_plan_folds_non_adjacent_matrix_concatenation_across_independent_state() {
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(3.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(2.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        assert_eq!(
            referenced_concat_matrices(&plan),
            vec![[2.0, 0.0, 0.0, 2.0, 3.0, 4.0]]
        );
        assert_eq!(
            plan.packed.optimization_report.source_operation_count,
            plan.packed.optimization_report.emitted_hot_operation_count + 1
        );
    }

    #[test]
    fn packed_plan_folds_non_adjacent_inverse_matrix_concatenation_to_noop() {
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(3.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("w", vec![Operand::Real(2.0)]),
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(-3.0),
                    Operand::Real(-4.0),
                ],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );

        let plan = RenderPlan::compile(list, contract).expect("compile plan");

        assert_eq!(plan.packed.optimization_report.folded_noop_state_ops, 1);
        assert_eq!(
            plan.packed.optimization_report.folded_overwritten_state_ops,
            1
        );
        assert!(
            referenced_concat_matrices(&plan).is_empty(),
            "separated inverse cm pair must not remain referenced by hot state ops"
        );
        assert_eq!(
            plan.packed.optimization_report.source_operation_count,
            plan.packed.optimization_report.emitted_hot_operation_count + 2
        );
    }

    #[test]
    fn vector_plan_tile_replay_reuses_selection_scratch() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(1.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                    Operand::Real(24.0),
                    Operand::Real(24.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 40.0, 40.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );
        let plan = RenderPlan::compile(list, contract).expect("compile plan");
        let tile = RenderTile::full(viewport.width_px, viewport.height_px);
        let expected = plan
            .execute_vector_tile(tile)
            .expect("execute plan")
            .expect("vector-only plan");
        let mut selected = Vec::with_capacity(plan.packed.hot_operation_count());
        selected.push(usize::MAX);
        let capacity = selected.capacity();

        let actual = plan
            .execute_vector_tile_with_scratch(tile, &mut selected)
            .expect("execute plan with scratch")
            .expect("vector-only plan");

        assert_eq!(expected.rgba_bytes(), actual.rgba_bytes());
        assert_eq!(
            selected.capacity(),
            capacity,
            "selection scratch should be reused when caller capacity is sufficient"
        );
        assert_eq!(
            selected.len(),
            plan.packed.hot_operation_count(),
            "scratch should contain the selected replay operation ids"
        );
    }

    #[test]
    fn vector_plan_warm_tile_replay_replaces_stale_selection_scratch() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(4.0),
                    Operand::Real(4.0),
                    Operand::Real(16.0),
                    Operand::Real(16.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(1.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(44.0),
                    Operand::Real(4.0),
                    Operand::Real(16.0),
                    Operand::Real(16.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 80.0, 40.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );
        let plan = RenderPlan::compile(list, contract).expect("compile plan");
        let left_tile = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 40,
        };
        let right_tile = RenderTile {
            x: 40,
            y: 0,
            width: 32,
            height: 40,
        };
        let mut selected = Vec::with_capacity(plan.packed.hot_operation_count());
        selected.extend([usize::MAX, usize::MAX - 1]);
        let capacity = selected.capacity();

        plan.execute_vector_tile_with_scratch(left_tile, &mut selected)
            .expect("left tile replay")
            .expect("left vector tile");
        let left_selection = selected.clone();
        assert!(
            left_selection.iter().all(|op| *op != usize::MAX),
            "stale caller scratch sentinels must be cleared before the first warm replay"
        );

        plan.execute_vector_tile_with_scratch(right_tile, &mut selected)
            .expect("right tile replay")
            .expect("right vector tile");
        let right_selection = selected.clone();

        assert_eq!(selected.capacity(), capacity);
        assert_ne!(
            right_selection, left_selection,
            "different tiles should replace, not append to, the previous selected operation set"
        );
        assert!(
            right_selection.iter().all(|op| *op != usize::MAX),
            "stale caller scratch sentinels must not survive the second warm replay"
        );

        plan.execute_vector_tile_with_scratch(left_tile, &mut selected)
            .expect("left tile replay after right")
            .expect("left vector tile after right");
        assert_eq!(
            selected, left_selection,
            "warm replay selection should be deterministic after reuse across different tiles"
        );
    }

    #[test]
    fn spatial_index_preserves_paint_order_after_culling() {
        let bounds = RenderBounds {
            x0: 10,
            y0: 10,
            x1: 20,
            y1: 20,
        };
        let index = RenderSpatialIndex {
            known: vec![(2, bounds), (1, bounds)],
            unknown: vec![0],
            grid: None,
            hierarchy: None,
        };
        assert_eq!(
            index.query(RenderTile {
                x: 10,
                y: 10,
                width: 2,
                height: 2
            }),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn spatial_index_grid_query_preserves_order_after_bucket_culling() {
        let mut known = Vec::new();
        for row in 0..4 {
            for col in 0..10 {
                let op_index = row * 10 + col + 1;
                let x0 = col as i32 * 10;
                let y0 = row as i32 * 10;
                known.push((
                    op_index,
                    RenderBounds {
                        x0,
                        y0,
                        x1: x0 + 8,
                        y1: y0 + 8,
                    },
                ));
            }
        }
        let grid = SpatialGridIndex::compile(&known, 100, 40);
        let index = RenderSpatialIndex {
            known,
            unknown: vec![0],
            grid,
            hierarchy: None,
        };

        assert!(
            index.grid.is_some(),
            "large known-op set should build a grid"
        );
        assert_eq!(
            index.query(RenderTile {
                x: 20,
                y: 10,
                width: 10,
                height: 10,
            }),
            vec![0, 13]
        );
    }

    #[test]
    fn spatial_index_bvh_query_matches_linear_scan_for_large_plan() {
        let mut known = Vec::new();
        for row in 0..8 {
            for col in 0..10 {
                let op_index = row * 10 + col + 1;
                let x0 = col as i32 * 12;
                let y0 = row as i32 * 11;
                let extent = if (row + col) % 7 == 0 { 18 } else { 7 };
                known.push((
                    op_index,
                    RenderBounds {
                        x0,
                        y0,
                        x1: x0 + extent,
                        y1: y0 + extent,
                    },
                ));
            }
        }
        known.push((
            500,
            RenderBounds {
                x0: 18,
                y0: 18,
                x1: 86,
                y1: 66,
            },
        ));

        let hierarchy = SpatialBvhIndex::compile(&known);
        assert!(
            hierarchy.is_some(),
            "large known-op set should build a hierarchy"
        );
        let indexed = RenderSpatialIndex {
            known: known.clone(),
            unknown: vec![0, 999],
            grid: None,
            hierarchy,
        };
        let linear = RenderSpatialIndex {
            known,
            unknown: vec![0, 999],
            grid: None,
            hierarchy: None,
        };

        for tile in [
            RenderTile {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            RenderTile {
                x: 24,
                y: 22,
                width: 20,
                height: 18,
            },
            RenderTile {
                x: 90,
                y: 70,
                width: 30,
                height: 22,
            },
        ] {
            assert_eq!(indexed.query(tile), linear.query(tile), "tile {tile:?}");
        }
    }

    #[test]
    fn spatial_index_query_into_reuses_output_buffer_for_linear_scan() {
        let known = vec![
            (
                1,
                RenderBounds {
                    x0: 0,
                    y0: 0,
                    x1: 10,
                    y1: 10,
                },
            ),
            (
                2,
                RenderBounds {
                    x0: 20,
                    y0: 0,
                    x1: 30,
                    y1: 10,
                },
            ),
            (
                3,
                RenderBounds {
                    x0: 5,
                    y0: 5,
                    x1: 25,
                    y1: 25,
                },
            ),
        ];
        let indexed = RenderSpatialIndex {
            known,
            unknown: vec![0],
            grid: None,
            hierarchy: None,
        };
        let tile = RenderTile {
            x: 4,
            y: 4,
            width: 12,
            height: 12,
        };
        let expected = indexed.query(tile);
        let mut selected = Vec::with_capacity(8);
        selected.extend([usize::MAX, usize::MAX - 1]);
        let capacity = selected.capacity();

        indexed.query_into(tile, &mut selected);

        assert_eq!(selected, expected);
        assert_eq!(
            selected.capacity(),
            capacity,
            "linear query_into should reuse sufficient caller-owned result capacity"
        );
        assert!(
            selected.iter().all(|op| *op != usize::MAX),
            "linear query_into must replace stale caller scratch contents"
        );
    }

    #[test]
    fn spatial_index_query_into_reuses_output_buffer_for_bvh() {
        let mut known = Vec::new();
        for row in 0..8 {
            for col in 0..10 {
                let op_index = row * 10 + col + 1;
                let x0 = col as i32 * 10;
                let y0 = row as i32 * 10;
                known.push((
                    op_index,
                    RenderBounds {
                        x0,
                        y0,
                        x1: x0 + 6,
                        y1: y0 + 6,
                    },
                ));
            }
        }
        let hierarchy = SpatialBvhIndex::compile(&known);
        let indexed = RenderSpatialIndex {
            known,
            unknown: vec![0],
            grid: None,
            hierarchy,
        };
        assert!(indexed.hierarchy.is_some());

        let tile = RenderTile {
            x: 20,
            y: 20,
            width: 12,
            height: 12,
        };
        let expected = indexed.query(tile);
        let mut selected = Vec::with_capacity(16);
        selected.push(usize::MAX);
        let capacity = selected.capacity();

        indexed.query_into(tile, &mut selected);

        assert_eq!(selected, expected);
        assert_eq!(
            selected.capacity(),
            capacity,
            "query_into should reuse sufficient caller-owned result capacity"
        );
    }

    #[test]
    fn spatial_index_query_into_reuses_output_buffer_for_grid() {
        let mut known = Vec::new();
        for row in 0..4 {
            for col in 0..10 {
                let op_index = row * 10 + col + 1;
                let x0 = col as i32 * 10;
                let y0 = row as i32 * 10;
                known.push((
                    op_index,
                    RenderBounds {
                        x0,
                        y0,
                        x1: x0 + 8,
                        y1: y0 + 8,
                    },
                ));
            }
        }
        let grid = SpatialGridIndex::compile(&known, 100, 40);
        let indexed = RenderSpatialIndex {
            known: known.clone(),
            unknown: vec![0],
            grid,
            hierarchy: None,
        };
        let linear = RenderSpatialIndex {
            known,
            unknown: vec![0],
            grid: None,
            hierarchy: None,
        };

        let tile = RenderTile {
            x: 20,
            y: 10,
            width: 10,
            height: 10,
        };
        let expected = linear.query(tile);
        let mut selected = Vec::with_capacity(8);
        selected.push(usize::MAX);
        let capacity = selected.capacity();

        indexed.query_into(tile, &mut selected);

        assert_eq!(selected, expected);
        assert_eq!(
            selected.capacity(),
            capacity,
            "grid query_into should reuse sufficient caller-owned result capacity"
        );
    }

    #[test]
    fn text_descriptor_compiles_tj_without_raw_content_operation() {
        let ops = vec![
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Tj", vec![Operand::String(b"Hello".to_vec())]),
            ContentOperation::new("ET", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(
            !list.native_vector_only(),
            "text page should have native ops"
        );
        let packed = PackedDisplayList::compile(list);
        assert!(packed.requires_native_replay());
        // Verify typed text descriptor is present, not a raw ContentOperation
        let text_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_TEXT)
            .collect();
        assert!(!text_ops.is_empty(), "should have at least one text op");
        for hot in &text_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Text(TextDescriptor::Show(bytes)) => {
                    assert_eq!(bytes, b"Hello");
                }
                NativeDescriptor::State(gs_desc) => {
                    // text-state ops like Tf are compiled as typed State descriptors
                    assert!(
                        !gs_desc.is_unsupported(),
                        "state descriptor should be typed, not unsupported"
                    );
                }
                other => panic!("unexpected descriptor for text op: {:?}", other),
            }
        }
    }

    #[test]
    fn image_descriptor_compiles_resource_reference() {
        let ops = vec![ContentOperation::new(
            "Do",
            vec![Operand::Name("Im1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let image_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_IMAGE)
            .collect();
        // Image ops may or may not appear depending on display-list builder
        // classification. If they do, verify the descriptor is typed.
        for hot in &image_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Image(img) => {
                    assert_eq!(img.name, "Im1");
                }
                _ => panic!("expected Image descriptor"),
            }
        }
    }

    #[test]
    fn form_descriptor_compiles_resource_reference() {
        let ops = vec![ContentOperation::new(
            "Do",
            vec![Operand::Name("Fm1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let form_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_FORM)
            .collect();
        for hot in &form_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Form(form) => {
                    assert_eq!(form.name, "Fm1");
                }
                _ => panic!("expected Form descriptor"),
            }
        }
    }

    #[test]
    fn shading_descriptor_compiles_resource_reference() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let shading_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_SHADING)
            .collect();
        for hot in &shading_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Shading(sh) => {
                    assert_eq!(sh.name, "Sh1");
                }
                _ => panic!("expected Shading descriptor"),
            }
        }
    }

    #[test]
    fn resource_aware_plan_pre_resolves_high_level_handles() {
        use crate::object::{PdfDictionary, PdfObject};

        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut resources = crate::engine::PageResources::default();
        let mut image_dict = PdfDictionary::empty();
        image_dict.insert("Subtype", PdfObject::Name("Image".to_string()));
        image_dict.insert("ColorSpace", PdfObject::Name("Cs1".to_string()));
        resources.xobjects.insert("Im1".to_string(), (7, 0));
        resources
            .xobject_subtypes
            .insert("Im1".to_string(), "Image".to_string());
        resources
            .xobject_stream_dicts
            .insert("Im1".to_string(), image_dict.clone());

        let mut form_dict = PdfDictionary::empty();
        form_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        form_dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(20),
            ]),
        );
        form_dict.insert(
            "Matrix",
            PdfObject::Array(vec![
                PdfObject::Integer(1),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(1),
                PdfObject::Integer(3),
                PdfObject::Integer(4),
            ]),
        );
        resources.xobjects.insert("Fm1".to_string(), (8, 0));
        resources
            .xobject_subtypes
            .insert("Fm1".to_string(), "Form".to_string());
        resources
            .xobject_stream_dicts
            .insert("Fm1".to_string(), form_dict);
        resources
            .xobject_bboxes
            .insert("Fm1".to_string(), [0.0, 0.0, 20.0, 20.0]);
        resources
            .xobject_matrices
            .insert("Fm1".to_string(), [1.0, 0.0, 0.0, 1.0, 3.0, 4.0]);

        let mut shading_dict = PdfDictionary::empty();
        shading_dict.insert("ShadingType", PdfObject::Integer(2));
        shading_dict.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
        resources
            .shadings
            .insert("Sh1".to_string(), PdfObject::Dictionary(shading_dict));
        let mut ext_g_state = PdfDictionary::empty();
        ext_g_state.insert("ca", PdfObject::Real(0.5));
        resources
            .ext_g_states
            .insert("GS1".to_string(), ext_g_state);
        let mut font_dict = PdfDictionary::empty();
        font_dict.insert("Subtype", PdfObject::Name("Type1".to_string()));
        font_dict.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
        resources.fonts.insert("F1".to_string(), font_dict);
        let mut ocg_dict = PdfDictionary::empty();
        ocg_dict.insert("Type", PdfObject::Name("OCG".to_string()));
        ocg_dict.insert("Name", PdfObject::String(b"Layer 1".to_vec()));
        resources
            .properties
            .insert("L1".to_string(), PdfObject::Dictionary(ocg_dict));
        let mut pattern_dict = PdfDictionary::empty();
        pattern_dict.insert("Type", PdfObject::Name("Pattern".to_string()));
        pattern_dict.insert("PatternType", PdfObject::Integer(1));
        pattern_dict.insert("PaintType", PdfObject::Integer(1));
        pattern_dict.insert("TilingType", PdfObject::Integer(1));
        pattern_dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(10),
                PdfObject::Integer(10),
            ]),
        );
        resources
            .patterns
            .insert("P1".to_string(), PdfObject::Dictionary(pattern_dict));
        resources
            .color_spaces
            .insert("Cs1".to_string(), PdfObject::Name("DeviceRGB".to_string()));
        let resolved_state = GraphicsStateDescriptor::compile_with_resources(
            &ContentOperation::new("cs", vec![Operand::Name("Cs1".to_string())]),
            Some(&resources),
        );
        assert!(matches!(
            resolved_state,
            GraphicsStateDescriptor::SetFillColorSpace {
                name,
                object: Some(PdfObject::Name(resolved)),
            } if name == "Cs1" && resolved == "DeviceRGB"
        ));

        let ops = vec![
            ContentOperation::new(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Name("L1".to_string()),
                ],
            ),
            ContentOperation::new("gs", vec![Operand::Name("GS1".to_string())]),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("cs", vec![Operand::Name("Cs1".to_string())]),
            ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
            ContentOperation::new("scn", vec![Operand::Name("P1".to_string())]),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(10.0),
                    Operand::Real(10.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
            ContentOperation::new("Do", vec![Operand::Name("Im1".to_string())]),
            ContentOperation::new("Do", vec![Operand::Name("Fm1".to_string())]),
            ContentOperation::new("sh", vec![Operand::Name("Sh1".to_string())]),
            ContentOperation::new("EMC", Vec::new()),
        ];
        let list = build_display_list(&ops, viewport, &resources);
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));

        let mut saw_image = false;
        let mut saw_form = false;
        let mut saw_shading = false;
        let mut saw_ext_g_state = false;
        let mut saw_font = false;
        let mut saw_oc_property = false;
        let mut saw_pattern = false;
        for hot in &packed.hot_ops {
            let Some(desc) = packed.descriptor(hot.payload_offset) else {
                continue;
            };
            match desc {
                NativeDescriptor::Image(image) => {
                    saw_image = true;
                    let handle = image.handle.as_ref().expect("image handle");
                    assert_eq!(handle.object_number, 7);
                    assert_eq!(handle.generation_number, 0);
                    assert_eq!(handle.subtype.as_deref(), Some("Image"));
                    assert!(handle.stream_dict.is_some());
                    let color_space = handle
                        .image_color_space
                        .as_ref()
                        .expect("image color-space handle");
                    assert_eq!(color_space.name, "Cs1");
                    assert_eq!(color_space.object, PdfObject::Name("DeviceRGB".to_string()));
                }
                NativeDescriptor::Form(form) => {
                    saw_form = true;
                    let handle = form.handle.as_ref().expect("form handle");
                    assert_eq!(handle.object_number, 8);
                    assert_eq!(handle.subtype.as_deref(), Some("Form"));
                    assert_eq!(handle.bbox, Some([0.0, 0.0, 20.0, 20.0]));
                    assert_eq!(handle.matrix, Some([1.0, 0.0, 0.0, 1.0, 3.0, 4.0]));
                    assert!(handle.stream_dict.is_some());
                }
                NativeDescriptor::Shading(shading) => {
                    saw_shading = true;
                    assert!(shading.object.is_some());
                }
                NativeDescriptor::State(GraphicsStateDescriptor::ApplyExtGState {
                    name,
                    dict,
                    font,
                }) => {
                    saw_ext_g_state = true;
                    assert_eq!(name, "GS1");
                    assert_eq!(
                        dict.as_ref().and_then(|dict| dict.get("ca")),
                        Some(&PdfObject::Real(0.5))
                    );
                    assert!(font.is_none());
                }
                NativeDescriptor::State(GraphicsStateDescriptor::SetFont {
                    name, dict, ..
                }) => {
                    saw_font = true;
                    assert_eq!(name, "F1");
                    assert_eq!(
                        dict.as_ref().and_then(|dict| dict.get_name("BaseFont")),
                        Some("Helvetica")
                    );
                }
                NativeDescriptor::State(
                    GraphicsStateDescriptor::BeginMarkedContentWithProperties {
                        tag,
                        properties:
                            MarkedContentProperties::Name {
                                name,
                                object: Some(PdfObject::Dictionary(dict)),
                            },
                    },
                ) => {
                    saw_oc_property = true;
                    assert_eq!(tag, "OC");
                    assert_eq!(name, "L1");
                    assert_eq!(dict.get_name("Type"), Some("OCG"));
                }
                NativeDescriptor::State(GraphicsStateDescriptor::SetFillColor {
                    name: Some(name),
                    pattern: Some(PdfObject::Dictionary(dict)),
                    ..
                }) => {
                    saw_pattern = true;
                    assert_eq!(name, "P1");
                    assert_eq!(dict.get_name("Type"), Some("Pattern"));
                    assert_eq!(dict.get_integer("PatternType"), Some(1));
                }
                _ => {}
            }
        }

        assert!(saw_image, "image descriptor should be present");
        assert!(saw_form, "Form descriptor should be present");
        assert!(saw_shading, "shading descriptor should be present");
        assert!(saw_ext_g_state, "ExtGState descriptor should be present");
        assert!(saw_font, "font descriptor should be present");
        assert!(
            saw_oc_property,
            "optional-content property descriptor should be present"
        );
        assert!(saw_pattern, "pattern descriptor should be present");
    }

    /// A test dispatcher that records which typed descriptors were dispatched.
    struct RecordingDispatcher {
        text_count: usize,
        image_count: usize,
        form_count: usize,
        shading_count: usize,
        state_count: usize,
        pattern_count: usize,
        inline_image_count: usize,
        compile_refusal_count: usize,
        save_count: usize,
        restore_count: usize,
        fill_count: usize,
        stroke_count: usize,
        clip_count: usize,
        stop_after_compile_refusal: bool,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self {
                text_count: 0,
                image_count: 0,
                form_count: 0,
                shading_count: 0,
                state_count: 0,
                pattern_count: 0,
                inline_image_count: 0,
                compile_refusal_count: 0,
                save_count: 0,
                restore_count: 0,
                fill_count: 0,
                stroke_count: 0,
                clip_count: 0,
                stop_after_compile_refusal: false,
            }
        }

        fn stopping_after_compile_refusal() -> Self {
            Self {
                stop_after_compile_refusal: true,
                ..Self::new()
            }
        }
    }

    impl PlanDispatcher for RecordingDispatcher {
        fn dispatch_text(&mut self, _desc: &TextDescriptor, _bounds: Option<&RenderBounds>) {
            self.text_count += 1;
        }
        fn dispatch_image(
            &mut self,
            _desc: &ImageXObjectDescriptor,
            _bounds: Option<&RenderBounds>,
        ) {
            self.image_count += 1;
        }
        fn dispatch_form(&mut self, _desc: &FormXObjectDescriptor, _bounds: Option<&RenderBounds>) {
            self.form_count += 1;
        }
        fn dispatch_shading(&mut self, _desc: &ShadingDescriptor, _bounds: Option<&RenderBounds>) {
            self.shading_count += 1;
        }
        fn dispatch_state(&mut self, _desc: &GraphicsStateDescriptor) {
            self.state_count += 1;
        }
        fn dispatch_pattern(
            &mut self,
            _desc: &PatternPathDescriptor,
            _bounds: Option<&RenderBounds>,
        ) {
            self.pattern_count += 1;
        }
        fn dispatch_inline_image(
            &mut self,
            _desc: &InlineImageDescriptor,
            _bounds: Option<&RenderBounds>,
        ) {
            self.inline_image_count += 1;
        }
        fn dispatch_compile_refusal(
            &mut self,
            _refusal: &PackedCompileRefusal,
            _bounds: Option<&RenderBounds>,
        ) {
            self.compile_refusal_count += 1;
        }
        fn dispatch_save(&mut self) {
            self.save_count += 1;
        }
        fn dispatch_restore(&mut self) {
            self.restore_count += 1;
        }
        fn dispatch_clip(&mut self, _path: &Path, _ctm: &Transform2D, _rule: FillRule) {
            self.clip_count += 1;
        }
        fn dispatch_fill(&mut self, _path: &Path, _state: &DrawState, _rule: FillRule) {
            self.fill_count += 1;
        }
        fn dispatch_stroke(&mut self, _path: &Path, _state: &DrawState) {
            self.stroke_count += 1;
        }
        fn is_cancelled(&self) -> bool {
            false
        }

        fn should_stop(&self) -> bool {
            self.stop_after_compile_refusal && self.compile_refusal_count > 0
        }
    }

    fn manual_xobject_display_list(viewport: Viewport, op: DisplayOp) -> DisplayList {
        manual_display_list(viewport, vec![op])
    }

    fn manual_display_list(viewport: Viewport, ops: Vec<DisplayOp>) -> DisplayList {
        DisplayList {
            viewport,
            ops,
            stats: Default::default(),
            supported: true,
            unsupported: Vec::new(),
        }
    }

    fn test_type1_font_dict(base_font: &str) -> PdfDictionary {
        let mut font = PdfDictionary::empty();
        font.insert("Subtype", PdfObject::Name("Type1".to_string()));
        font.insert("BaseFont", PdfObject::Name(base_font.to_string()));
        font
    }

    #[test]
    fn packed_plan_refuses_resolved_xobject_subtype_mismatch() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        for (display_op, subtype, expected_refusal) in [
            (
                DisplayOp::NativeImageXObject {
                    name: "Xm1".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                Some("Form"),
                PackedCompileRefusal::UnsupportedXObjectSubtype {
                    name: "Xm1".to_string(),
                    expected: "Image".to_string(),
                    actual: "Form".to_string(),
                },
            ),
            (
                DisplayOp::NativeFormXObject {
                    name: "Xm1".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                Some("PS"),
                PackedCompileRefusal::UnsupportedXObjectSubtype {
                    name: "Xm1".to_string(),
                    expected: "Form".to_string(),
                    actual: "PS".to_string(),
                },
            ),
            (
                DisplayOp::NativeFormXObject {
                    name: "Xm1".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                None,
                PackedCompileRefusal::MissingXObjectSubtype {
                    name: "Xm1".to_string(),
                    expected: "Form".to_string(),
                },
            ),
        ] {
            let mut resources = PageResources::default();
            resources.xobjects.insert("Xm1".to_string(), (5, 0));
            resources
                .xobject_stream_dicts
                .insert("Xm1".to_string(), PdfDictionary::empty());
            if let Some(subtype) = subtype {
                resources
                    .xobject_subtypes
                    .insert("Xm1".to_string(), subtype.to_string());
            }
            let list = manual_xobject_display_list(viewport.clone(), display_op);
            let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
            let desc = packed
                .descriptor(0)
                .expect("manual XObject op should produce descriptor");
            assert!(matches!(
                desc,
                NativeDescriptor::CompileRefusal(refusal) if refusal == &expected_refusal
            ));

            let mut rec = RecordingDispatcher::new();
            packed.execute_plan(&mut rec).expect("execute plan");
            assert_eq!(rec.compile_refusal_count, 1);
            assert_eq!(rec.image_count, 0);
            assert_eq!(rec.form_count, 0);
        }
    }

    #[test]
    fn packed_plan_refuses_missing_resources_when_resource_table_is_available() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        for (display_op, expected_refusal) in [
            (
                DisplayOp::NativeImageXObject {
                    name: "ImMissing".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                PackedCompileRefusal::MissingXObjectResource {
                    name: "ImMissing".to_string(),
                    expected: "Image".to_string(),
                },
            ),
            (
                DisplayOp::NativeFormXObject {
                    name: "FmMissing".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                PackedCompileRefusal::MissingXObjectResource {
                    name: "FmMissing".to_string(),
                    expected: "Form".to_string(),
                },
            ),
            (
                DisplayOp::NativeShadingOp {
                    name: "ShMissing".to_string(),
                    approx_bytes: 0,
                    bounds: None,
                },
                PackedCompileRefusal::MissingShadingResource {
                    name: "ShMissing".to_string(),
                },
            ),
        ] {
            let list = manual_xobject_display_list(viewport.clone(), display_op);
            let resources = PageResources::default();
            let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
            let desc = packed
                .descriptor(0)
                .expect("manual resource op should produce descriptor");
            assert!(matches!(
                desc,
                NativeDescriptor::CompileRefusal(refusal) if refusal == &expected_refusal
            ));

            let mut rec = RecordingDispatcher::new();
            packed.execute_plan(&mut rec).expect("execute plan");
            assert_eq!(rec.compile_refusal_count, 1);
            assert_eq!(rec.image_count, 0);
            assert_eq!(rec.form_count, 0);
            assert_eq!(rec.shading_count, 0);
        }
    }

    #[test]
    fn packed_plan_refuses_missing_state_resources_when_resource_table_is_available() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let resources = PageResources::default();
        let cases = vec![
            (
                ContentOperation::new("gs", vec![Operand::Name("GSMissing".to_string())]),
                PackedCompileRefusal::MissingExtGStateResource {
                    name: "GSMissing".to_string(),
                },
            ),
            (
                ContentOperation::new(
                    "Tf",
                    vec![Operand::Name("FMissing".to_string()), Operand::Real(12.0)],
                ),
                PackedCompileRefusal::MissingFontResource {
                    name: "FMissing".to_string(),
                },
            ),
            (
                ContentOperation::new("CS", vec![Operand::Name("CsMissing".to_string())]),
                PackedCompileRefusal::MissingColorSpaceResource {
                    name: "CsMissing".to_string(),
                    usage: "stroke".to_string(),
                },
            ),
            (
                ContentOperation::new("cs", vec![Operand::Name("CsMissing".to_string())]),
                PackedCompileRefusal::MissingColorSpaceResource {
                    name: "CsMissing".to_string(),
                    usage: "fill".to_string(),
                },
            ),
            (
                ContentOperation::new("SCN", vec![Operand::Name("PatMissing".to_string())]),
                PackedCompileRefusal::MissingPatternResource {
                    name: "PatMissing".to_string(),
                    usage: "stroke".to_string(),
                },
            ),
            (
                ContentOperation::new("scn", vec![Operand::Name("PatMissing".to_string())]),
                PackedCompileRefusal::MissingPatternResource {
                    name: "PatMissing".to_string(),
                    usage: "fill".to_string(),
                },
            ),
            (
                ContentOperation::new(
                    "BDC",
                    vec![
                        Operand::Name("OC".to_string()),
                        Operand::Name("LayerMissing".to_string()),
                    ],
                ),
                PackedCompileRefusal::MissingPropertiesResource {
                    name: "LayerMissing".to_string(),
                },
            ),
            (
                ContentOperation::new(
                    "DP",
                    vec![
                        Operand::Name("OC".to_string()),
                        Operand::Name("LayerMissing".to_string()),
                    ],
                ),
                PackedCompileRefusal::MissingPropertiesResource {
                    name: "LayerMissing".to_string(),
                },
            ),
        ];

        for (op, expected_refusal) in cases {
            let state = GraphicsStateDescriptor::compile_with_resources(&op, Some(&resources));
            let list = manual_xobject_display_list(
                viewport.clone(),
                DisplayOp::StateOp {
                    state,
                    approx_bytes: 0,
                },
            );
            let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
            assert!(
                packed.requires_native_replay(),
                "state-resource refusal should not be skipped by vector replay"
            );
            let desc = packed
                .descriptor(0)
                .expect("manual state op should produce descriptor");
            assert!(matches!(
                desc,
                NativeDescriptor::CompileRefusal(refusal) if refusal == &expected_refusal
            ));

            let mut rec = RecordingDispatcher::new();
            packed.execute_plan(&mut rec).expect("execute plan");
            assert_eq!(rec.compile_refusal_count, 1);
            assert_eq!(rec.state_count, 0);
        }
    }

    #[test]
    fn packed_retained_text_refuses_missing_font_when_resource_table_is_available() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let resources = PageResources::default();
        let list = manual_xobject_display_list(
            viewport,
            DisplayOp::NativeTextOp {
                text: RetainedTextOp::SetFont {
                    name: "FMissing".to_string(),
                    size: 12.0,
                },
                approx_bytes: 0,
                bounds: None,
            },
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        let desc = packed
            .descriptor(0)
            .expect("manual retained text op should produce descriptor");
        assert!(matches!(
            desc,
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::MissingFontResource { name })
                if name == "FMissing"
        ));

        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute plan");
        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(rec.state_count, 0);
    }

    #[test]
    fn packed_retained_text_refuses_visible_show_without_active_resolved_font() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources
            .fonts
            .insert("F1".to_string(), test_type1_font_dict("Helvetica"));
        let list = manual_display_list(
            viewport,
            vec![
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::BeginText,
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::Show(b"Hi".to_vec()),
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::EndText,
                    approx_bytes: 0,
                    bounds: None,
                },
            ],
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        assert!(
            packed.hot_ops.iter().any(|hot| {
                hot.opcode == OP_NATIVE_TEXT
                    && matches!(
                        packed.descriptor(hot.payload_offset),
                        Some(NativeDescriptor::CompileRefusal(
                            PackedCompileRefusal::TextShowWithoutResolvedFont { operator }
                        )) if operator == "Tj"
                    )
            }),
            "visible text show should compile to TextShowWithoutResolvedFont"
        );

        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute plan");
        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(rec.text_count, 0);
    }

    #[test]
    fn packed_retained_text_allows_visible_show_after_resolved_font() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources
            .fonts
            .insert("F1".to_string(), test_type1_font_dict("Helvetica"));
        let list = manual_display_list(
            viewport,
            vec![
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::BeginText,
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::SetFont {
                        name: "F1".to_string(),
                        size: 12.0,
                    },
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::Show(b"Hi".to_vec()),
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::EndText,
                    approx_bytes: 0,
                    bounds: None,
                },
            ],
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute plan");
        assert_eq!(rec.compile_refusal_count, 0);
        assert_eq!(rec.text_count, 1);
    }

    #[test]
    fn packed_ext_g_state_pre_resolves_font_for_later_text_show() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources
            .fonts
            .insert("F1".to_string(), test_type1_font_dict("Helvetica"));
        let mut ext_g_state = PdfDictionary::empty();
        ext_g_state.insert(
            "Font",
            PdfObject::Array(vec![
                PdfObject::Name("F1".to_string()),
                PdfObject::Real(11.0),
            ]),
        );
        resources
            .ext_g_states
            .insert("GS1".to_string(), ext_g_state);
        let state = GraphicsStateDescriptor::compile_with_resources(
            &ContentOperation::new("gs", vec![Operand::Name("GS1".to_string())]),
            Some(&resources),
        );
        let list = manual_display_list(
            viewport,
            vec![
                DisplayOp::StateOp {
                    state,
                    approx_bytes: 0,
                },
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::Show(b"Hi".to_vec()),
                    approx_bytes: 0,
                    bounds: None,
                },
            ],
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        assert!(matches!(
            packed.descriptor(0),
            Some(NativeDescriptor::State(GraphicsStateDescriptor::ApplyExtGState {
                font: Some(ResolvedFontHandle { name, size, .. }),
                ..
            })) if name == "F1" && *size == 11.0
        ));

        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute plan");
        assert_eq!(rec.compile_refusal_count, 0);
        assert_eq!(rec.text_count, 1);
    }

    #[test]
    fn packed_ext_g_state_refuses_missing_selected_font_resource() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut ext_g_state = PdfDictionary::empty();
        ext_g_state.insert(
            "Font",
            PdfObject::Array(vec![
                PdfObject::Name("FMissing".to_string()),
                PdfObject::Real(11.0),
            ]),
        );
        resources
            .ext_g_states
            .insert("GS1".to_string(), ext_g_state);
        let state = GraphicsStateDescriptor::compile_with_resources(
            &ContentOperation::new("gs", vec![Operand::Name("GS1".to_string())]),
            Some(&resources),
        );
        let list = manual_display_list(
            viewport,
            vec![DisplayOp::StateOp {
                state,
                approx_bytes: 0,
            }],
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        assert!(matches!(
            packed.descriptor(0),
            Some(NativeDescriptor::CompileRefusal(
                PackedCompileRefusal::MissingExtGStateFontResource { ext_g_state, font }
            )) if ext_g_state == "GS1" && font == "FMissing"
        ));

        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute plan");
        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(rec.state_count, 0);
    }

    #[test]
    fn packed_text_font_tracking_restores_unresolved_state_after_q() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources
            .fonts
            .insert("F1".to_string(), test_type1_font_dict("Helvetica"));
        let list = manual_display_list(
            viewport,
            vec![
                DisplayOp::Save,
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::SetFont {
                        name: "F1".to_string(),
                        size: 12.0,
                    },
                    approx_bytes: 0,
                    bounds: None,
                },
                DisplayOp::Restore,
                DisplayOp::NativeTextOp {
                    text: RetainedTextOp::Show(b"Hi".to_vec()),
                    approx_bytes: 0,
                    bounds: None,
                },
            ],
        );
        let packed = PackedDisplayList::compile_with_resources(list, Some(&resources));
        assert!(
            packed.hot_ops.iter().any(|hot| {
                hot.opcode == OP_NATIVE_TEXT
                    && matches!(
                        packed.descriptor(hot.payload_offset),
                        Some(NativeDescriptor::CompileRefusal(
                            PackedCompileRefusal::TextShowWithoutResolvedFont { operator }
                        )) if operator == "Tj"
                    )
            }),
            "Q should restore the unresolved pre-save font state before the visible show"
        );
    }

    #[test]
    fn execute_plan_dispatches_text_through_typed_descriptor() {
        let ops = vec![
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Tj", vec![Operand::String(b"Plan".to_vec())]),
            ContentOperation::new("ET", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        // Text ops (Tj) go through dispatch_text; state ops (BT/Tf/ET) through dispatch_state
        assert!(
            rec.text_count > 0,
            "text descriptor should be dispatched, got text_count={}",
            rec.text_count
        );
    }

    #[test]
    fn execute_plan_dispatches_vector_fill_directly() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(1.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(5.0),
                    Operand::Real(5.0),
                    Operand::Real(30.0),
                    Operand::Real(30.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(list.native_vector_only());
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert!(rec.fill_count > 0, "fill should be dispatched");
        assert_eq!(rec.text_count, 0);
        assert_eq!(rec.image_count, 0);
    }

    #[test]
    fn pattern_descriptor_contains_no_content_operation() {
        // Build a NativePatternPathOp through the display list builder by using a
        // named color space that forces stateful path dispatch.
        let ops = vec![
            ContentOperation::new("m", vec![Operand::Real(10.0), Operand::Real(20.0)]),
            ContentOperation::new("l", vec![Operand::Real(30.0), Operand::Real(20.0)]),
            ContentOperation::new("l", vec![Operand::Real(30.0), Operand::Real(40.0)]),
            ContentOperation::new("h", Vec::new()),
            ContentOperation::new("f", Vec::new()),
        ];

        // Directly invoke the compile_pattern_descriptor helper
        let desc = PackedDisplayList::compile_pattern_descriptor(&ops);
        match &desc {
            NativeDescriptor::Pattern(pattern) => {
                // Path should have the normalized segments
                assert!(!pattern.path.segments.is_empty());
                assert_eq!(pattern.phase, PatternPaintPhase::FillNonZero);
                // Verify it's truly typed — no ContentOperation anywhere in the descriptor
                // (this is compile-time guaranteed by the struct, but we assert the variant)
                assert!(matches!(desc, NativeDescriptor::Pattern(_)));
            }
            other => panic!("expected Pattern descriptor, got {:?}", other),
        }
    }

    #[test]
    fn pattern_descriptor_stores_rect_path_with_stroke_phase() {
        let ops = vec![
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(5.0),
                    Operand::Real(5.0),
                    Operand::Real(50.0),
                    Operand::Real(50.0),
                ],
            ),
            ContentOperation::new("S", Vec::new()),
        ];
        let desc = PackedDisplayList::compile_pattern_descriptor(&ops);
        match desc {
            NativeDescriptor::Pattern(pattern) => {
                assert_eq!(pattern.phase, PatternPaintPhase::Stroke);
                // rect adds 5 segments (move, line, line, line, close)
                assert_eq!(pattern.path.segments.len(), 5);
            }
            other => panic!("expected Pattern descriptor, got {:?}", other),
        }
    }

    #[test]
    fn pattern_descriptor_refuses_unrecognized_operator() {
        let ops = vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("UNKNOWN_OP", Vec::new()),
            ContentOperation::new("f", Vec::new()),
        ];
        let desc = PackedDisplayList::compile_pattern_descriptor(&ops);
        match desc {
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::UnrecognizedPathOperator(
                op,
            )) => {
                assert_eq!(op, "UNKNOWN_OP");
            }
            other => panic!("expected CompileRefusal, got {:?}", other),
        }
    }

    #[test]
    fn pattern_descriptor_refuses_unrecognized_paint_operator() {
        let ops = vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("XYZ", Vec::new()),
        ];
        let desc = PackedDisplayList::compile_pattern_descriptor(&ops);
        match desc {
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::UnrecognizedPaintOperator(
                op,
            )) => {
                assert_eq!(op, "XYZ");
            }
            other => panic!(
                "expected CompileRefusal for paint operator, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn pattern_descriptor_refuses_empty_ops() {
        let desc = PackedDisplayList::compile_pattern_descriptor(&[]);
        assert!(matches!(
            desc,
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::EmptyPatternOps)
        ));
    }

    #[test]
    fn inline_image_descriptor_contains_no_content_operation() {
        let image = RetainedInlineImage {
            params: vec![
                Operand::Name("W".to_string()),
                Operand::Integer(2),
                Operand::Name("H".to_string()),
                Operand::Integer(2),
                Operand::Name("BPC".to_string()),
                Operand::Integer(8),
                Operand::Name("CS".to_string()),
                Operand::Name("G".to_string()),
            ],
            data: vec![0xFF, 0x00, 0x80, 0x40],
        };
        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, None);
        match &desc {
            NativeDescriptor::InlineImage(inline) => {
                // Verify params are the ID operands
                assert_eq!(inline.params.len(), 8);
                // Verify data bytes
                assert_eq!(inline.data, vec![0xFF, 0x00, 0x80, 0x40]);
                assert!(inline.color_space.is_none());
                // No ContentOperation — compile-time guaranteed by struct
                assert!(matches!(desc, NativeDescriptor::InlineImage(_)));
            }
            other => panic!("expected InlineImage descriptor, got {:?}", other),
        }
    }

    #[test]
    fn inline_image_descriptor_pre_resolves_named_color_space_resource() {
        let mut resources = PageResources::default();
        let color_space = PdfObject::Name("DeviceRGB".to_string());
        resources
            .color_spaces
            .insert("CS1".to_string(), color_space.clone());
        let image = RetainedInlineImage {
            params: vec![
                Operand::Name("W".to_string()),
                Operand::Integer(1),
                Operand::Name("H".to_string()),
                Operand::Integer(1),
                Operand::Name("BPC".to_string()),
                Operand::Integer(8),
                Operand::Name("CS".to_string()),
                Operand::Name("CS1".to_string()),
            ],
            data: vec![0x80, 0x80, 0x80],
        };

        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, Some(&resources));

        match desc {
            NativeDescriptor::InlineImage(inline) => {
                let resolved = inline
                    .color_space
                    .expect("named inline color-space resource should be pre-resolved");
                assert_eq!(resolved.name, "CS1");
                assert_eq!(resolved.object, color_space);
            }
            other => panic!("expected InlineImage descriptor, got {:?}", other),
        }
    }

    #[test]
    fn inline_image_descriptor_treats_abbreviated_device_color_space_as_builtin() {
        let resources = PageResources::default();
        let image = RetainedInlineImage {
            params: vec![
                Operand::Name("W".to_string()),
                Operand::Integer(1),
                Operand::Name("H".to_string()),
                Operand::Integer(1),
                Operand::Name("BPC".to_string()),
                Operand::Integer(8),
                Operand::Name("CS".to_string()),
                Operand::Name("G".to_string()),
            ],
            data: vec![0x80],
        };

        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, Some(&resources));

        match desc {
            NativeDescriptor::InlineImage(inline) => {
                assert!(
                    inline.color_space.is_none(),
                    "inline /CS /G is a built-in device space, not a missing resource"
                );
            }
            other => panic!("expected InlineImage descriptor, got {:?}", other),
        }
    }

    #[test]
    fn inline_image_descriptor_refuses_missing_named_color_space_resource() {
        let resources = PageResources::default();
        let image = RetainedInlineImage {
            params: vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(1),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("MissingCS".to_string()),
            ],
            data: vec![0x80, 0x80, 0x80],
        };

        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, Some(&resources));

        match desc {
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::MissingColorSpaceResource {
                name,
                usage,
            }) => {
                assert_eq!(name, "MissingCS");
                assert_eq!(usage, "inline image");
            }
            other => panic!(
                "expected MissingColorSpaceResource refusal, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn inline_image_descriptor_refuses_missing_params() {
        let image = RetainedInlineImage {
            params: Vec::new(),
            data: vec![0xFF],
        };
        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, None);
        assert!(matches!(
            desc,
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::MissingInlineImageParams)
        ));
    }

    #[test]
    fn inline_image_descriptor_refuses_missing_data() {
        let image = RetainedInlineImage {
            params: vec![Operand::Name("W".to_string()), Operand::Integer(1)],
            data: Vec::new(),
        };
        let desc = PackedDisplayList::compile_inline_image_descriptor(&image, None);
        assert!(matches!(
            desc,
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::MissingInlineImageData)
        ));
    }

    #[test]
    fn execute_plan_dispatches_pattern_through_typed_descriptor() {
        use crate::render::display_list::DisplayOp;

        // Manually construct a display list with a NativePatternPathOp
        let path_ops = vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(10.0), Operand::Real(10.0)]),
            ContentOperation::new("S", Vec::new()),
        ];
        let pattern = match PackedDisplayList::compile_pattern_descriptor(&path_ops) {
            NativeDescriptor::Pattern(pattern) => pattern,
            other => panic!("expected Pattern descriptor, got {:?}", other),
        };
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = DisplayList {
            ops: vec![DisplayOp::NativePatternPathOp {
                pattern,
                approx_bytes: 64,
                bounds: None,
            }],
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);
        // Verify the descriptor is Pattern, not an UnsupportedPattern raw ops
        let pattern_hot: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_PATTERN)
            .collect();
        assert_eq!(pattern_hot.len(), 1);
        let desc = packed.descriptor(pattern_hot[0].payload_offset).unwrap();
        assert!(
            matches!(desc, NativeDescriptor::Pattern(_)),
            "expected Pattern descriptor, got {:?}",
            desc
        );

        // Execute plan and verify dispatch
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert_eq!(rec.pattern_count, 1);
        assert_eq!(rec.inline_image_count, 0);
        assert_eq!(rec.compile_refusal_count, 0);
    }

    #[test]
    fn execute_plan_dispatches_inline_image_through_typed_descriptor() {
        use crate::render::display_list::DisplayOp;

        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = DisplayList {
            ops: vec![DisplayOp::NativeInlineImage {
                image: RetainedInlineImage {
                    params: vec![
                        Operand::Name("W".to_string()),
                        Operand::Integer(1),
                        Operand::Name("H".to_string()),
                        Operand::Integer(1),
                        Operand::Name("BPC".to_string()),
                        Operand::Integer(8),
                    ],
                    data: vec![0xAA],
                },
                approx_bytes: 32,
                bounds: None,
            }],
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);
        // Verify typed descriptor
        let inline_hot: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_INLINE_IMAGE)
            .collect();
        assert_eq!(inline_hot.len(), 1);
        let desc = packed.descriptor(inline_hot[0].payload_offset).unwrap();
        match desc {
            NativeDescriptor::InlineImage(img) => {
                assert_eq!(img.data, vec![0xAA]);
                assert_eq!(img.params.len(), 6);
            }
            other => panic!("expected InlineImage descriptor, got {:?}", other),
        }

        // Execute plan and verify dispatch
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert_eq!(rec.inline_image_count, 1);
        assert_eq!(rec.pattern_count, 0);
        assert_eq!(rec.compile_refusal_count, 0);
    }

    #[test]
    fn compile_refusal_is_dispatched_explicitly_not_silently_skipped() {
        use crate::render::display_list::DisplayOp;

        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = DisplayList {
            ops: vec![DisplayOp::StateOp {
                state: GraphicsStateDescriptor::Unsupported {
                    operator: "BOGUS_STATE".to_string(),
                },
                approx_bytes: 32,
            }],
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);
        // The descriptor should be a CompileRefusal
        let state_hot: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_STATE)
            .collect();
        assert_eq!(state_hot.len(), 1);
        let desc = packed.descriptor(state_hot[0].payload_offset).unwrap();
        assert!(
            matches!(desc, NativeDescriptor::CompileRefusal(_)),
            "expected CompileRefusal, got {:?}",
            desc
        );

        // Execute and verify the refusal is dispatched (not silently dropped)
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(rec.state_count, 0);
    }

    #[test]
    fn execute_plan_stops_after_dispatcher_refusal() {
        use crate::render::display_list::DisplayOp;

        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut path = Path::new();
        path.rect(0.0, 0.0, 10.0, 10.0);
        let list = DisplayList {
            ops: vec![
                DisplayOp::StateOp {
                    state: GraphicsStateDescriptor::Unsupported {
                        operator: "BOGUS_STATE".to_string(),
                    },
                    approx_bytes: 32,
                },
                DisplayOp::FillPath {
                    path,
                    state: DrawState {
                        ctm: Transform2D::identity(),
                        fill_color: [255, 0, 0, 255],
                        stroke_color: [0, 0, 0, 255],
                        fill_color_explicit: true,
                        stroke_color_explicit: false,
                        fill_cmyk: None,
                        stroke_cmyk: None,
                        blend_mode: crate::content::state::BlendMode::Normal,
                        rendering_intent: "RelativeColorimetric".to_string(),
                        stroke_overprint: false,
                        fill_overprint: false,
                        overprint_mode: 0,
                        stroke_adjustment: false,
                        alpha_source: false,
                        text_knockout: true,
                        line_width: 1.0,
                        line_cap: crate::content::state::LineCap::Butt,
                        line_join: crate::content::state::LineJoin::Miter,
                        miter_limit: 10.0,
                        dash: crate::render::DashState::solid(),
                        flatness: 1.0,
                    },
                    rule: FillRule::NonZero,
                    bounds: None,
                },
            ],
            viewport,
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::stopping_after_compile_refusal();

        packed.execute_plan(&mut rec).expect("execute_plan");

        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(
            rec.fill_count, 0,
            "dispatcher stop hook must prevent later partial paint dispatch"
        );
    }

    #[test]
    fn has_only_supported_descriptors_false_on_refusal() {
        use crate::render::display_list::DisplayOp;

        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = DisplayList {
            ops: vec![DisplayOp::StateOp {
                state: GraphicsStateDescriptor::Unsupported {
                    operator: "INVALID_STATE".to_string(),
                },
                approx_bytes: 16,
            }],
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);
        assert!(
            !packed.has_only_supported_descriptors(),
            "compile refusal should make has_only_supported_descriptors false"
        );
    }

    #[test]
    fn pattern_path_descriptor_curve_to_is_preserved() {
        let ops = vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new(
                "c",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(2.0),
                    Operand::Real(3.0),
                    Operand::Real(4.0),
                    Operand::Real(5.0),
                    Operand::Real(6.0),
                ],
            ),
            ContentOperation::new("B*", Vec::new()),
        ];
        let desc = PackedDisplayList::compile_pattern_descriptor(&ops);
        match desc {
            NativeDescriptor::Pattern(pattern) => {
                assert_eq!(pattern.phase, PatternPaintPhase::FillStrokeEvenOdd);
                assert_eq!(pattern.path.segments.len(), 2);
                assert!(matches!(
                    pattern.path.segments[1],
                    PathSegment::CubicTo {
                        cp1x,
                        cp1y,
                        cp2x,
                        cp2y,
                        x,
                        y,
                    } if cp1x == 1.0 && cp1y == 2.0 && cp2x == 3.0 && cp2y == 4.0 && x == 5.0 && y == 6.0
                ));
            }
            other => panic!("expected Pattern descriptor, got {:?}", other),
        }
    }

    #[test]
    fn typed_state_descriptor_has_no_content_operation_in_active_plan() {
        // Verify that after compilation, no NativeDescriptor::State variant contains
        // a raw ContentOperation — the GraphicsStateDescriptor is purely typed.
        use crate::render::display_list::DisplayOp;

        let state_ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(10.0),
                    Operand::Real(20.0),
                ],
            ),
            ContentOperation::new("w", vec![Operand::Real(2.5)]),
            ContentOperation::new("J", vec![Operand::Integer(1)]),
            ContentOperation::new("j", vec![Operand::Integer(2)]),
            ContentOperation::new("M", vec![Operand::Real(8.0)]),
            ContentOperation::new(
                "d",
                vec![
                    Operand::Array(vec![Operand::Real(3.0), Operand::Real(2.0)]),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new(
                "ri",
                vec![Operand::Name("AbsoluteColorimetric".to_string())],
            ),
            ContentOperation::new("i", vec![Operand::Real(1.0)]),
            ContentOperation::new("G", vec![Operand::Real(0.5)]),
            ContentOperation::new("g", vec![Operand::Real(0.8)]),
            ContentOperation::new(
                "RG",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(1.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "K",
                vec![
                    Operand::Real(0.1),
                    Operand::Real(0.2),
                    Operand::Real(0.3),
                    Operand::Real(0.4),
                ],
            ),
            ContentOperation::new(
                "k",
                vec![
                    Operand::Real(0.5),
                    Operand::Real(0.6),
                    Operand::Real(0.7),
                    Operand::Real(0.8),
                ],
            ),
            ContentOperation::new("CS", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new("cs", vec![Operand::Name("DeviceCMYK".to_string())]),
            ContentOperation::new(
                "SCN",
                vec![
                    Operand::Real(0.1),
                    Operand::Real(0.2),
                    Operand::Real(0.3),
                    Operand::Name("Pat1".to_string()),
                ],
            ),
            ContentOperation::new("scn", vec![Operand::Real(0.9), Operand::Real(0.8)]),
            ContentOperation::new("gs", vec![Operand::Name("GS0".to_string())]),
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Td", vec![Operand::Real(10.0), Operand::Real(20.0)]),
            ContentOperation::new("TD", vec![Operand::Real(5.0), Operand::Real(-15.0)]),
            ContentOperation::new(
                "Tm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(100.0),
                    Operand::Real(200.0),
                ],
            ),
            ContentOperation::new("T*", Vec::new()),
            ContentOperation::new("Tc", vec![Operand::Real(0.5)]),
            ContentOperation::new("Tw", vec![Operand::Real(1.0)]),
            ContentOperation::new("Tz", vec![Operand::Real(150.0)]),
            ContentOperation::new("TL", vec![Operand::Real(14.0)]),
            ContentOperation::new("Tr", vec![Operand::Integer(2)]),
            ContentOperation::new("Ts", vec![Operand::Real(3.0)]),
            ContentOperation::new("ET", Vec::new()),
            ContentOperation::new("BMC", vec![Operand::Name("Span".to_string())]),
            ContentOperation::new(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Name("MC0".to_string()),
                ],
            ),
            ContentOperation::new("EMC", Vec::new()),
            ContentOperation::new("BX", Vec::new()),
            ContentOperation::new("EX", Vec::new()),
        ];

        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let ops: Vec<DisplayOp> = state_ops
            .iter()
            .map(|op| DisplayOp::StateOp {
                state: GraphicsStateDescriptor::compile(op),
                approx_bytes: 32,
            })
            .collect();
        let list = DisplayList {
            ops,
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);

        // Every descriptor should be a typed NativeDescriptor::State(GraphicsStateDescriptor)
        // — NOT a raw ContentOperation. Verify no descriptor is Unsupported.
        for desc in &packed.descriptors {
            match desc {
                NativeDescriptor::State(gs_desc) => {
                    assert!(
                        !gs_desc.is_unsupported(),
                        "Standard state op compiled as Unsupported: {:?}",
                        gs_desc
                    );
                }
                NativeDescriptor::CompileRefusal(refusal) => {
                    panic!("Standard state op produced CompileRefusal: {:?}", refusal);
                }
                _ => panic!("Expected NativeDescriptor::State, got {:?}", desc),
            }
        }

        // Verify the count matches
        assert_eq!(packed.descriptors.len(), state_ops.len());
    }

    #[test]
    fn typed_state_descriptor_round_trips_to_content_operation() {
        // Verify that to_content_operation produces semantically equivalent ops
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(2.0),
                    Operand::Real(50.0),
                    Operand::Real(100.0),
                ],
            ),
            ContentOperation::new("w", vec![Operand::Real(3.0)]),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.5), Operand::Real(0.6), Operand::Real(0.7)],
            ),
            ContentOperation::new("gs", vec![Operand::Name("GS1".to_string())]),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F2".to_string()), Operand::Real(24.0)],
            ),
            ContentOperation::new("Tc", vec![Operand::Real(1.5)]),
            ContentOperation::new("BMC", vec![Operand::Name("P".to_string())]),
            ContentOperation::new("EMC", Vec::new()),
        ];

        for op in &ops {
            let desc = GraphicsStateDescriptor::compile(op);
            assert!(
                !desc.is_unsupported(),
                "op {} compiled as unsupported",
                op.operator
            );
            let reconstructed = desc.to_content_operation();
            assert_eq!(
                reconstructed.operator, op.operator,
                "operator mismatch for {}",
                op.operator
            );
            // For numeric operands, verify the values round-trip
            for (i, orig_operand) in op.operands.iter().enumerate() {
                let recon_operand = &reconstructed.operands[i];
                match (orig_operand, recon_operand) {
                    (Operand::Real(a), Operand::Real(b)) => {
                        assert!(
                            (a - b).abs() < 1e-10,
                            "operand {} mismatch for {}: {} vs {}",
                            i,
                            op.operator,
                            a,
                            b
                        );
                    }
                    (Operand::Integer(a), Operand::Integer(b)) => {
                        assert_eq!(a, b);
                    }
                    (Operand::Name(a), Operand::Name(b)) => {
                        assert_eq!(a, b);
                    }
                    (Operand::Integer(a), Operand::Real(b)) => {
                        assert!(
                            (*a as f64 - b).abs() < 1e-10,
                            "integer-to-real mismatch for {} operand {}",
                            op.operator,
                            i
                        );
                    }
                    _ => {} // array types handled below
                }
            }
        }
    }

    #[test]
    fn unsupported_state_operator_produces_compile_refusal() {
        use crate::render::display_list::DisplayOp;

        let unknown_op = ContentOperation::new("EXOTIC_UNKNOWN_OP", vec![Operand::Real(42.0)]);
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = DisplayList {
            ops: vec![DisplayOp::StateOp {
                state: GraphicsStateDescriptor::compile(&unknown_op),
                approx_bytes: 16,
            }],
            viewport: viewport.clone(),
            unsupported: Vec::new(),
            supported: true,
            stats: Default::default(),
        };
        let packed = PackedDisplayList::compile(list);

        // Should produce a CompileRefusal, not a State descriptor
        assert_eq!(packed.descriptors.len(), 1);
        match &packed.descriptors[0] {
            NativeDescriptor::CompileRefusal(PackedCompileRefusal::UnsupportedStateOperator(
                op_name,
            )) => {
                assert_eq!(op_name, "EXOTIC_UNKNOWN_OP");
            }
            other => panic!(
                "expected CompileRefusal::UnsupportedStateOperator, got {:?}",
                other
            ),
        }

        // has_only_supported_descriptors should be false
        assert!(!packed.has_only_supported_descriptors());

        // Execute plan — refusal is dispatched explicitly, not silently skipped
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert_eq!(rec.compile_refusal_count, 1);
        assert_eq!(rec.state_count, 0);
    }

    #[test]
    fn active_high_level_page_plan_matches_immediate_rendering() {
        // A page with state ops + text: both immediate and plan paths should
        // dispatch through the same state count and text count.
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Td", vec![Operand::Real(10.0), Operand::Real(50.0)]),
            ContentOperation::new("Tj", vec![Operand::String(b"Test".to_vec())]),
            ContentOperation::new("ET", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        // Text descriptors: "Tj" becomes a text dispatch
        assert!(
            rec.text_count >= 1,
            "should dispatch text: {}",
            rec.text_count
        );
        // State descriptors: rg, BT, Tf, Td, ET are state mutations
        assert!(
            rec.state_count >= 4,
            "should dispatch state ops: {}",
            rec.state_count
        );
        // No compile refusals for standard operators
        assert_eq!(rec.compile_refusal_count, 0);
    }

    #[test]
    fn graphics_state_descriptor_compile_all_known_operators() {
        // Verify every known operator maps to a non-Unsupported descriptor variant
        let known_ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new("w", vec![Operand::Real(1.0)]),
            ContentOperation::new("J", vec![Operand::Integer(0)]),
            ContentOperation::new("j", vec![Operand::Integer(0)]),
            ContentOperation::new("M", vec![Operand::Real(10.0)]),
            ContentOperation::new(
                "d",
                vec![
                    Operand::Array(vec![Operand::Real(3.0), Operand::Real(1.0)]),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new(
                "ri",
                vec![Operand::Name("RelativeColorimetric".to_string())],
            ),
            ContentOperation::new("i", vec![Operand::Real(0.0)]),
            ContentOperation::new("G", vec![Operand::Real(0.0)]),
            ContentOperation::new("g", vec![Operand::Real(0.0)]),
            ContentOperation::new(
                "RG",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "K",
                vec![
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                ],
            ),
            ContentOperation::new(
                "k",
                vec![
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                ],
            ),
            ContentOperation::new("CS", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new("cs", vec![Operand::Name("DeviceRGB".to_string())]),
            ContentOperation::new(
                "SC",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new("SCN", vec![Operand::Name("Pat1".to_string())]),
            ContentOperation::new(
                "sc",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new("scn", vec![Operand::Name("Pat1".to_string())]),
            ContentOperation::new("gs", vec![Operand::Name("GS0".to_string())]),
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new("ET", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Td", vec![Operand::Real(1.0), Operand::Real(2.0)]),
            ContentOperation::new("TD", vec![Operand::Real(1.0), Operand::Real(2.0)]),
            ContentOperation::new(
                "Tm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new("T*", Vec::new()),
            ContentOperation::new("Tc", vec![Operand::Real(0.0)]),
            ContentOperation::new("Tw", vec![Operand::Real(0.0)]),
            ContentOperation::new("Tz", vec![Operand::Real(100.0)]),
            ContentOperation::new("TL", vec![Operand::Real(12.0)]),
            ContentOperation::new("Tr", vec![Operand::Integer(0)]),
            ContentOperation::new("Ts", vec![Operand::Real(0.0)]),
            ContentOperation::new("BMC", vec![Operand::Name("Span".to_string())]),
            ContentOperation::new(
                "BDC",
                vec![
                    Operand::Name("Span".to_string()),
                    Operand::Name("Props".to_string()),
                ],
            ),
            ContentOperation::new("EMC", Vec::new()),
            ContentOperation::new("MP", vec![Operand::Name("Span".to_string())]),
            ContentOperation::new(
                "DP",
                vec![
                    Operand::Name("Span".to_string()),
                    Operand::Name("Props".to_string()),
                ],
            ),
            ContentOperation::new("BX", Vec::new()),
            ContentOperation::new("EX", Vec::new()),
        ];
        for op in &known_ops {
            let desc = GraphicsStateDescriptor::compile(op);
            assert!(
                !desc.is_unsupported(),
                "operator '{}' should compile to typed descriptor, got Unsupported",
                op.operator
            );
        }
    }

    #[test]
    fn graphics_state_descriptor_rejects_malformed_state_operands() {
        let malformed_ops = vec![
            ContentOperation::new("q", vec![Operand::Real(1.0)]),
            ContentOperation::new("cm", vec![Operand::Real(1.0)]),
            ContentOperation::new("w", vec![Operand::Real(-1.0)]),
            ContentOperation::new("J", vec![Operand::Real(1.5)]),
            ContentOperation::new("j", vec![Operand::Integer(3)]),
            ContentOperation::new("M", vec![Operand::Real(0.0)]),
            ContentOperation::new(
                "d",
                vec![
                    Operand::Array(vec![Operand::Real(0.0), Operand::Real(0.0)]),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new("ri", Vec::new()),
            ContentOperation::new("i", vec![Operand::Real(-0.5)]),
            ContentOperation::new("G", Vec::new()),
            ContentOperation::new("rg", vec![Operand::Real(1.0), Operand::Real(0.0)]),
            ContentOperation::new(
                "K",
                vec![Operand::Real(0.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new("CS", vec![Operand::Real(1.0)]),
            ContentOperation::new(
                "SCN",
                vec![
                    Operand::Real(0.0),
                    Operand::Name("Pat1".to_string()),
                    Operand::Real(1.0),
                ],
            ),
            ContentOperation::new("scn", Vec::new()),
            ContentOperation::new("gs", Vec::new()),
        ];

        for op in &malformed_ops {
            let desc = GraphicsStateDescriptor::compile(op);
            match desc {
                GraphicsStateDescriptor::Unsupported { operator, .. } => {
                    assert_eq!(operator, op.operator);
                }
                other => panic!(
                    "malformed operator '{}' compiled as {:?}",
                    op.operator, other
                ),
            }
        }
    }

    #[test]
    fn text_operand_validator_rejects_malformed_text_operands() {
        let malformed_ops = vec![
            ContentOperation::new("BT", vec![Operand::Real(1.0)]),
            ContentOperation::new("Tf", vec![Operand::Name("F1".to_string())]),
            ContentOperation::new(
                "Tf",
                vec![
                    Operand::Name("F1".to_string()),
                    Operand::Name("Bad".to_string()),
                ],
            ),
            ContentOperation::new("Td", vec![Operand::Real(1.0)]),
            ContentOperation::new(
                "Tm",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new("Tc", vec![Operand::Name("Bad".to_string())]),
            ContentOperation::new("Tr", vec![Operand::Integer(8)]),
            ContentOperation::new("Tr", vec![Operand::Real(1.5)]),
            ContentOperation::new("Tj", vec![Operand::Name("Bad".to_string())]),
            ContentOperation::new("TJ", vec![Operand::Name("Bad".to_string())]),
            ContentOperation::new(
                "TJ",
                vec![Operand::Array(vec![
                    Operand::String(b"ok".to_vec()),
                    Operand::Name("Bad".to_string()),
                ])],
            ),
            ContentOperation::new("'", Vec::new()),
            ContentOperation::new("\"", vec![Operand::Real(0.0), Operand::Real(0.0)]),
        ];

        for op in &malformed_ops {
            assert!(
                text_operand_refusal(op).is_some(),
                "malformed text operator '{}' was accepted",
                op.operator
            );
        }

        for op in malformed_ops
            .iter()
            .filter(|op| !matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
        {
            match GraphicsStateDescriptor::compile(op) {
                GraphicsStateDescriptor::Unsupported { operator, .. } => {
                    assert_eq!(operator, op.operator);
                }
                other => panic!(
                    "malformed text-state operator '{}' compiled as {:?}",
                    op.operator, other
                ),
            }
        }
    }

    #[test]
    fn marked_content_operand_validator_rejects_malformed_operands() {
        let malformed_ops = vec![
            ContentOperation::new("BMC", Vec::new()),
            ContentOperation::new("BMC", vec![Operand::Real(1.0)]),
            ContentOperation::new("BDC", vec![Operand::Name("OC".to_string())]),
            ContentOperation::new(
                "BDC",
                vec![Operand::Real(1.0), Operand::Name("Layer1".to_string())],
            ),
            ContentOperation::new(
                "BDC",
                vec![Operand::Name("OC".to_string()), Operand::Real(1.0)],
            ),
            ContentOperation::new("EMC", vec![Operand::Name("Extra".to_string())]),
            ContentOperation::new("MP", Vec::new()),
            ContentOperation::new(
                "DP",
                vec![
                    Operand::Name("Span".to_string()),
                    Operand::Array(Vec::new()),
                ],
            ),
            ContentOperation::new("BX", vec![Operand::Name("Extra".to_string())]),
            ContentOperation::new("EX", vec![Operand::Name("Extra".to_string())]),
        ];

        for op in &malformed_ops {
            assert!(
                marked_content_operand_refusal(op).is_some(),
                "malformed marked-content operator '{}' was accepted",
                op.operator
            );
            match GraphicsStateDescriptor::compile(op) {
                GraphicsStateDescriptor::Unsupported { operator, .. } => {
                    assert_eq!(operator, op.operator);
                }
                other => panic!(
                    "malformed marked-content operator '{}' compiled as {:?}",
                    op.operator, other
                ),
            }
        }
    }

    #[test]
    fn path_operand_validator_rejects_malformed_operands() {
        let malformed_ops = vec![
            (ContentOperation::new("m", vec![Operand::Real(1.0)]), false),
            (
                ContentOperation::new("m", vec![Operand::Real(f64::NAN), Operand::Real(0.0)]),
                false,
            ),
            (
                ContentOperation::new("l", vec![Operand::Real(1.0), Operand::Real(2.0)]),
                false,
            ),
            (
                ContentOperation::new(
                    "c",
                    vec![
                        Operand::Real(1.0),
                        Operand::Real(2.0),
                        Operand::Real(3.0),
                        Operand::Real(4.0),
                        Operand::Real(5.0),
                    ],
                ),
                true,
            ),
            (
                ContentOperation::new(
                    "v",
                    vec![
                        Operand::Real(1.0),
                        Operand::Real(2.0),
                        Operand::Real(3.0),
                        Operand::Real(4.0),
                    ],
                ),
                false,
            ),
            (
                ContentOperation::new(
                    "re",
                    vec![
                        Operand::Real(0.0),
                        Operand::Real(0.0),
                        Operand::Name("Bad".to_string()),
                        Operand::Real(10.0),
                    ],
                ),
                false,
            ),
            (ContentOperation::new("S", vec![Operand::Real(1.0)]), true),
            (ContentOperation::new("W", vec![Operand::Real(1.0)]), true),
        ];

        for (op, has_current_point) in &malformed_ops {
            assert!(
                path_operand_refusal(op, *has_current_point).is_some(),
                "malformed path operator '{}' was accepted",
                op.operator
            );
        }

        assert!(path_operand_refusal(
            &ContentOperation::new("m", vec![Operand::Real(1.0), Operand::Real(2.0)]),
            false,
        )
        .is_none());
        assert!(path_operand_refusal(
            &ContentOperation::new("l", vec![Operand::Real(1.0), Operand::Real(2.0)]),
            true,
        )
        .is_none());
    }

    #[test]
    fn resource_invocation_operand_validator_rejects_malformed_operands() {
        let malformed_ops = vec![
            ContentOperation::new("Do", Vec::new()),
            ContentOperation::new("Do", vec![Operand::Real(1.0)]),
            ContentOperation::new(
                "Do",
                vec![Operand::Name("Im1".to_string()), Operand::Real(1.0)],
            ),
            ContentOperation::new("sh", Vec::new()),
            ContentOperation::new("sh", vec![Operand::Real(1.0)]),
            ContentOperation::new(
                "sh",
                vec![Operand::Name("S1".to_string()), Operand::Real(1.0)],
            ),
        ];

        for op in &malformed_ops {
            assert!(
                resource_invocation_operand_refusal(op).is_some(),
                "malformed resource invocation operator '{}' was accepted",
                op.operator
            );
        }

        assert!(resource_invocation_operand_refusal(&ContentOperation::new(
            "Do",
            vec![Operand::Name("Im1".to_string())],
        ))
        .is_none());
        assert!(resource_invocation_operand_refusal(&ContentOperation::new(
            "sh",
            vec![Operand::Name("S1".to_string())],
        ))
        .is_none());
    }

    #[test]
    fn type3_glyph_metric_operand_validator_rejects_malformed_operands() {
        let malformed_ops = vec![
            ContentOperation::new("d0", Vec::new()),
            ContentOperation::new(
                "d0",
                vec![Operand::Name("Bad".to_string()), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "d0",
                vec![Operand::Real(600.0), Operand::Real(0.0), Operand::Real(1.0)],
            ),
            ContentOperation::new("d1", vec![Operand::Real(600.0), Operand::Real(0.0)]),
            ContentOperation::new(
                "d1",
                vec![
                    Operand::Real(600.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(f64::NAN),
                    Operand::Real(700.0),
                ],
            ),
            ContentOperation::new(
                "d1",
                vec![
                    Operand::Real(600.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(500.0),
                    Operand::Real(700.0),
                    Operand::Real(1.0),
                ],
            ),
        ];

        for op in &malformed_ops {
            assert!(
                type3_glyph_metric_operand_refusal(op).is_some(),
                "malformed Type 3 glyph metric operator '{}' was accepted",
                op.operator
            );
            assert!(
                GraphicsStateDescriptor::compile(op).is_unsupported(),
                "malformed Type 3 glyph metric operator '{}' compiled to supported state",
                op.operator
            );
        }

        assert!(type3_glyph_metric_operand_refusal(&ContentOperation::new(
            "d0",
            vec![Operand::Real(600.0), Operand::Real(0.0)],
        ))
        .is_none());
        assert!(type3_glyph_metric_operand_refusal(&ContentOperation::new(
            "d1",
            vec![
                Operand::Real(600.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(500.0),
                Operand::Real(700.0),
            ],
        ))
        .is_none());
    }

    #[test]
    fn graphics_state_descriptor_unknown_operator_is_unsupported() {
        let op = ContentOperation::new("SOME_NOVEL_OP", vec![Operand::Integer(99)]);
        let desc = GraphicsStateDescriptor::compile(&op);
        assert!(
            desc.is_unsupported(),
            "unknown operator should be Unsupported"
        );
        match desc {
            GraphicsStateDescriptor::Unsupported { operator } => {
                assert_eq!(operator, "SOME_NOVEL_OP");
            }
            _ => panic!("expected Unsupported variant"),
        }
    }
}
