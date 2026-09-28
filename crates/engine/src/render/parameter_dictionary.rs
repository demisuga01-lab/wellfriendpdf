//! Bounded, copy-on-write resolution of known numeric/boolean PDF parameters.
//! Unknown entries and function/colour/resource graphs are not flattened.
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::borrow::Cow;

const MAX_FUNCTION_PARAMETERS: usize = 8192;
pub(super) const MAX_STITCHING_FUNCTIONS: usize = 4096;

#[derive(Clone, Copy)]
enum Field {
    Scalar,
    Array(usize),
    // Resolve presence (including indirect null), retaining nonnull references
    // so stream ownership, decoding and recursive function limits stay intact.
    Presence,
}

fn resolve<'a>(
    value: &'a PdfObject,
    reader: Option<&PdfReader>,
) -> Result<Cow<'a, PdfObject>, String> {
    crate::cancel::check_current_cancel("render parameter resolution")
        .map_err(|e| e.to_string())?;
    match value {
        PdfObject::Reference { .. } => reader
            .ok_or("indirect render parameter requires a PDF reader")?
            .resolve(value.clone())
            .map(Cow::Owned)
            .map_err(|e| e.to_string()),
        _ => Ok(Cow::Borrowed(value)),
    }
}

fn fields<'a>(
    dict: &'a PdfDictionary,
    reader: Option<&PdfReader>,
    entries: &[(&str, Field)],
) -> Result<Cow<'a, PdfDictionary>, String> {
    let mut output = Cow::Borrowed(dict);
    for &(key, kind) in entries {
        let Some(original) = dict.get(key) else {
            continue;
        };
        let result = resolve(original, reader).map_err(|e| format!("/{key}: {e}"))?;
        if matches!(result.as_ref(), PdfObject::Null) {
            // A dictionary null has the meaning of an absent entry. Required
            // entries are still rejected by the type-specific validator.
            output.to_mut().remove(key);
            continue;
        }
        if matches!(kind, Field::Presence) {
            continue;
        }
        if let (Field::Array(max), PdfObject::Array(items)) = (kind, result.as_ref()) {
            if items.len() > max {
                return Err(format!(
                    "malformed /{key}: exceeds the {max}-element render parameter limit"
                ));
            }
            let mut resolved_items: Option<Vec<PdfObject>> = None;
            for (index, item) in items.iter().enumerate() {
                let value = resolve(item, reader).map_err(|e| format!("/{key}[{index}]: {e}"))?;
                if let Cow::Owned(value) = value {
                    resolved_items.get_or_insert_with(|| items.clone())[index] = value;
                }
            }
            if let Some(items) = resolved_items {
                output.to_mut().insert(key, PdfObject::Array(items));
                continue;
            }
        }
        if let Cow::Owned(value) = result {
            output.to_mut().insert(key, value);
        }
    }
    Ok(output)
}

pub(super) fn shading<'a>(
    dict: &'a PdfDictionary,
    reader: Option<&PdfReader>,
) -> Result<Cow<'a, PdfDictionary>, String> {
    use Field::*;
    fields(
        dict,
        reader,
        &[
            ("ShadingType", Scalar),
            ("Coords", Array(6)),
            ("Domain", Array(4)),
            ("Matrix", Array(6)),
            ("Extend", Array(2)),
            ("BBox", Array(4)),
            (
                "Background",
                Array(crate::render::colorspace::MAX_DEVICEN_COMPONENTS),
            ),
            ("AntiAlias", Scalar),
            ("BitsPerCoordinate", Scalar),
            ("BitsPerComponent", Scalar),
            ("BitsPerFlag", Scalar),
            ("VerticesPerRow", Scalar),
            (
                "Decode",
                Array(4 + 2 * crate::render::colorspace::MAX_DEVICEN_COMPONENTS),
            ),
            ("Function", Presence),
        ],
    )
}

pub(super) fn function<'a>(
    dict: &'a PdfDictionary,
    reader: Option<&PdfReader>,
) -> Result<Cow<'a, PdfDictionary>, String> {
    use Field::*;
    // Functions is deliberately not an Array field: its entries remain object
    // references, to be visited by the bounded recursive function evaluator.
    let resolved = fields(
        dict,
        reader,
        &[
            ("FunctionType", Scalar),
            ("Domain", Array(MAX_FUNCTION_PARAMETERS)),
            ("Range", Array(MAX_FUNCTION_PARAMETERS)),
            ("C0", Array(MAX_FUNCTION_PARAMETERS)),
            ("C1", Array(MAX_FUNCTION_PARAMETERS)),
            ("N", Scalar),
            (
                "Size",
                Array(super::function::MAX_TYPE0_INTERPOLATION_DIMENSIONS),
            ),
            ("BitsPerSample", Scalar),
            ("Order", Scalar),
            ("Encode", Array(MAX_FUNCTION_PARAMETERS)),
            ("Decode", Array(MAX_FUNCTION_PARAMETERS)),
            ("Bounds", Array(MAX_STITCHING_FUNCTIONS)),
            ("Functions", Scalar),
        ],
    )?;
    if let Some(PdfObject::Array(items)) = resolved.get("Functions") {
        if items.len() > MAX_STITCHING_FUNCTIONS {
            return Err("/Functions exceeds stitching child limit".into());
        }
    }
    Ok(resolved)
}

pub(super) fn pattern<'a>(
    dict: &'a PdfDictionary,
    reader: Option<&PdfReader>,
) -> Result<Cow<'a, PdfDictionary>, String> {
    use Field::*;
    fields(
        dict,
        reader,
        &[
            ("PatternType", Scalar),
            ("Matrix", Array(6)),
            ("PaintType", Scalar),
            ("TilingType", Scalar),
            ("XStep", Scalar),
            ("YStep", Scalar),
            ("BBox", Array(4)),
            ("Shading", Presence),
        ],
    )
}

#[cfg(test)]
#[path = "parameter_dictionary_tests.rs"]
pub(crate) mod tests;
