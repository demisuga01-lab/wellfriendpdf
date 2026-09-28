//! Bind colour spaces to their selecting resource scope (ISO 32000-1 8.6.5.6).
//! Defaults replace device spaces without transforming the component vector.
//! This is an ephemeral rendering graph, never a mutation of source objects.
use crate::engine::PageResources;
use crate::{PdfObject, PdfReader};

const MAX_DEPTH: usize = 32;

pub(crate) fn has_defaults(resources: &PageResources) -> bool {
    ["DefaultGray", "DefaultRGB", "DefaultCMYK"]
        .iter()
        .any(|name| {
            resources
                .color_spaces
                .get(*name)
                .is_some_and(|value| !matches!(value, PdfObject::Null))
        })
}

fn device(name: &str) -> Option<(&'static str, &'static str, usize)> {
    match name {
        "DeviceGray" => Some(("DeviceGray", "DefaultGray", 1)),
        "DeviceRGB" => Some(("DeviceRGB", "DefaultRGB", 3)),
        "DeviceCMYK" => Some(("DeviceCMYK", "DefaultCMYK", 4)),
        _ => None,
    }
}

/// Name aliases and references share the same bounded traversal. In particular,
/// a resource alias may not shadow the intrinsic DeviceRGB/DeviceGray names.
pub(crate) fn bind(
    object: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
) -> Result<PdfObject, String> {
    walk(object, resources, reader, true, 0)
}

/// Resolve original sample/paint graphs in the selecting scope before device
/// remapping. Indexed lookup domains belong to this original base graph.
pub(crate) fn bind_source(
    object: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
) -> Result<PdfObject, String> {
    walk(object, resources, reader, false, 0)
}

/// Abbreviations belong to inline-image syntax only. Canonicalise colour-space
/// positions, not arbitrary names in tint functions, colourants or dictionaries.
/// Named resource definitions themselves are subsequently bound without this
/// inline syntax treatment.
pub(crate) fn canonical_inline(object: &PdfObject) -> Result<PdfObject, String> {
    fn normalize(object: &PdfObject, depth: usize) -> Result<PdfObject, String> {
        if depth >= MAX_DEPTH {
            return Err("inline colour-space nesting exceeds 32".into());
        }
        let canonical = |name: &str| match name {
            "G" => "DeviceGray".to_string(),
            "RGB" => "DeviceRGB".to_string(),
            "CMYK" => "DeviceCMYK".to_string(),
            "I" => "Indexed".to_string(),
            other => other.to_string(),
        };
        match object {
            PdfObject::Name(name) => Ok(PdfObject::Name(canonical(name))),
            PdfObject::Array(items) => {
                let mut result = items.clone();
                let name = items
                    .first()
                    .and_then(PdfObject::as_name)
                    .ok_or("inline colour space has no family")?;
                let family = canonical(name);
                result[0] = PdfObject::Name(family.clone());
                let child = match family.as_str() {
                    "Pattern" if items.len() == 2 => Some(1),
                    "Indexed" if items.len() == 4 => Some(1),
                    "Separation" | "DeviceN" if items.len() >= 4 => Some(2),
                    _ => None,
                };
                if let Some(index) = child {
                    result[index] = normalize(&items[index], depth + 1)?;
                }
                Ok(PdfObject::Array(result))
            }
            _ => Err("inline colour space is not a name or array".into()),
        }
    }
    normalize(object, 0)
}

/// CS/cs initialise components in the selected source space; default remapping
/// then carries those values unchanged into the compatible replacement.
pub(crate) fn initial_components(
    object: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
) -> Result<Vec<f64>, String> {
    let raw = walk(object, resources, reader, false, 0)?;
    initial(&raw, reader, 0)
}
fn initial(object: &PdfObject, reader: &PdfReader, depth: usize) -> Result<Vec<f64>, String> {
    if depth >= MAX_DEPTH {
        return Err("initial colour-space nesting exceeds 32".into());
    }
    if family(object) == Some("Pattern") {
        return match object {
            PdfObject::Array(items) if items.len() == 2 => initial(&items[1], reader, depth + 1),
            _ => Ok(Vec::new()),
        };
    }
    let mut values: Vec<f64> = vec![0.0; component_count(object, reader, depth + 1)?];
    match family(object) {
        Some("DeviceCMYK") => values[3] = 1.0,
        Some("Separation" | "DeviceN") => values.fill(1.0),
        Some("Lab") => {
            let params = object
                .as_array()
                .and_then(|a| a.get(1))
                .ok_or("Lab parameters missing")?;
            let params = reader.resolve(params.clone()).map_err(|e| e.to_string())?;
            let dict = params
                .as_dict()
                .ok_or("Lab parameters are not a dictionary")?;
            if let Some(range) = dict.get("Range") {
                let range = reader.resolve(range.clone()).map_err(|e| e.to_string())?;
                if !matches!(range, PdfObject::Null) {
                    let ranges = range_pairs(&range, 2, reader)?;
                    for (value, (lo, hi)) in values[1..].iter_mut().zip(ranges) {
                        *value = value.clamp(lo, hi);
                    }
                }
            }
        }
        Some("ICCBased") => {
            let profile = object
                .as_array()
                .and_then(|a| a.get(1))
                .ok_or("ICCBased profile missing")?;
            let profile = reader.resolve(profile.clone()).map_err(|e| e.to_string())?;
            let PdfObject::Stream { dict, .. } = profile else {
                return Err("ICCBased profile is not a stream".into());
            };
            for (value, (lo, hi)) in values.iter_mut().zip(icc_component_ranges(&dict, reader)?) {
                *value = value.clamp(lo, hi);
            }
        }
        _ => {}
    }
    Ok(values)
}

/// ICC /Range clips component values; it is not a scale-to-unit transform.
/// Shared by initial graphics state and CMM paint/image input preparation.
pub(crate) fn icc_component_ranges(
    dict: &crate::PdfDictionary,
    reader: &PdfReader,
) -> Result<Vec<(f64, f64)>, String> {
    let count = match dict.get("N").and_then(PdfObject::as_integer) {
        Some(n @ (1 | 3 | 4)) => n as usize,
        _ => return Err("invalid ICCBased component count".into()),
    };
    match dict.get("Range") {
        None | Some(PdfObject::Null) => Ok(vec![(0.0, 1.0); count]),
        Some(range) => {
            let range = reader.resolve(range.clone()).map_err(|e| e.to_string())?;
            if matches!(range, PdfObject::Null) {
                Ok(vec![(0.0, 1.0); count])
            } else {
                range_pairs(&range, count, reader)
            }
        }
    }
}

fn range_pairs(
    object: &PdfObject,
    count: usize,
    reader: &PdfReader,
) -> Result<Vec<(f64, f64)>, String> {
    let object = reader.resolve(object.clone()).map_err(|e| e.to_string())?;
    let values = object
        .as_array()
        .ok_or("colour-space Range is not an array")?;
    if values.len() != count * 2 {
        return Err("colour-space Range has the wrong component count".into());
    }
    values
        .chunks_exact(2)
        .map(|pair| {
            let lo = pair[0]
                .as_number()
                .ok_or("colour-space Range lower bound is not numeric")?;
            let hi = pair[1]
                .as_number()
                .ok_or("colour-space Range upper bound is not numeric")?;
            if !lo.is_finite() || !hi.is_finite() || lo > hi {
                return Err("colour-space Range bounds are not ordered finite numbers".into());
            }
            Ok((lo, hi))
        })
        .collect()
}

fn walk(
    object: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
    remap: bool,
    depth: usize,
) -> Result<PdfObject, String> {
    if depth >= MAX_DEPTH {
        return Err("colour-space reference/default nesting exceeds 32".into());
    }
    crate::cancel::check_current_cancel("default colour-space binding")
        .map_err(|e| e.to_string())?;
    match object {
        PdfObject::Reference { .. } => {
            let resolved = reader.resolve(object.clone()).map_err(|e| e.to_string())?;
            walk(&resolved, resources, reader, remap, depth + 1)
        }
        PdfObject::Name(name) => {
            if let Some((canonical, key, components)) = device(name) {
                if remap {
                    if let Some(value) = resources.color_spaces.get(key) {
                        // Do not recursively reapply defaults inside their own
                        // replacement graph (including a same-device identity).
                        let replacement = walk(value, resources, reader, false, depth + 1)?;
                        if !matches!(replacement, PdfObject::Null) {
                            let family = family(&replacement);
                            if matches!(family, Some("Lab" | "Indexed" | "I" | "Pattern"))
                                || component_count(&replacement, reader, depth + 1)? != components
                            {
                                return Err(format!("/{key} is not a compatible {components}-component default colour space"));
                            }
                            return Ok(replacement);
                        }
                    }
                }
                Ok(PdfObject::Name(canonical.into()))
            } else if matches!(name.as_str(), "Pattern") {
                Ok(object.clone())
            } else if let Some(value) = resources.color_spaces.get(name) {
                walk(value, resources, reader, remap, depth + 1)
            } else {
                Err(format!("colour-space resource /{name} is missing"))
            }
        }
        PdfObject::Array(items) => {
            let family = items
                .first()
                .and_then(PdfObject::as_name)
                .ok_or("colour space has no family name")?;
            if device(family).is_some() && items.len() == 1 {
                return walk(&items[0], resources, reader, remap, depth + 1);
            }
            let mut result = items.clone();
            if family == "ICCBased" {
                if items.len() != 2 {
                    return Err("ICCBased space must contain exactly one profile".into());
                }
                let profile = reader
                    .resolve(items[1].clone())
                    .map_err(|e| e.to_string())?;
                let PdfObject::Stream { mut dict, raw } = profile else {
                    return Err("ICCBased profile is not a stream".into());
                };
                let count = icc_component_ranges(&dict, reader)?.len();
                let alternate = dict.get("Alternate").cloned().unwrap_or(PdfObject::Null);
                let alternate = reader.resolve(alternate).map_err(|e| e.to_string())?;
                let alternate = if matches!(alternate, PdfObject::Null) {
                    PdfObject::Name(
                        match count {
                            1 => "DeviceGray",
                            3 => "DeviceRGB",
                            4 => "DeviceCMYK",
                            _ => unreachable!(),
                        }
                        .into(),
                    )
                } else {
                    alternate
                };
                let alternate = walk(&alternate, resources, reader, remap, depth + 1)?;
                if self::family(&alternate) == Some("Pattern")
                    || component_count(&alternate, reader, depth + 1)? != count
                {
                    return Err(
                        "ICCBased Alternate must be a non-Pattern space with matching components"
                            .into(),
                    );
                }
                dict.insert("Alternate", alternate);
                result[1] = PdfObject::Stream { dict, raw };
                return Ok(PdfObject::Array(result));
            }
            // The screen conversion uses the alternate of spot/DeviceN spaces;
            // real-plate handling remains separate from that fallback graph.
            let child = match family {
                "Pattern" if items.len() == 2 => Some(1),
                "Indexed" | "I" if items.len() == 4 => Some(1),
                "Separation" | "DeviceN" if items.len() >= 4 => Some(2),
                _ => None,
            };
            if let Some(index) = child {
                result[index] = walk(&items[index], resources, reader, remap, depth + 1)?;
            }
            if family == "I" {
                result[0] = PdfObject::Name("Indexed".into());
            }
            Ok(PdfObject::Array(result))
        }
        PdfObject::Null => Ok(PdfObject::Null),
        _ => Err("colour space is not a name or array".into()),
    }
}

pub(crate) fn family(object: &PdfObject) -> Option<&str> {
    match object {
        PdfObject::Name(name) => Some(name),
        PdfObject::Array(items) => items.first().and_then(PdfObject::as_name),
        _ => None,
    }
}

/// Valid component domains for an already bound graph. Used when passing ICC
/// source values unchanged to an alternate: clipping is not normalization.
pub(crate) fn component_ranges(
    object: &PdfObject,
    reader: &PdfReader,
) -> Result<Vec<(f64, f64)>, String> {
    let object = reader.resolve(object.clone()).map_err(|e| e.to_string())?;
    let count = component_count(&object, reader, 0)?;
    match family(&object) {
        Some("Pattern") => Err("Pattern cannot be an ICC alternate".into()),
        Some("Lab") => {
            let mut dict = crate::PdfDictionary::empty();
            dict.insert("ColorSpace", object);
            let params = crate::render::cmm::try_lab_params_from_image_dict(&dict, Some(reader))?;
            Ok(vec![
                (0.0, 100.0),
                (f64::from(params.range[0]), f64::from(params.range[1])),
                (f64::from(params.range[2]), f64::from(params.range[3])),
            ])
        }
        Some("ICCBased") => {
            let profile = object
                .as_array()
                .and_then(|a| a.get(1))
                .ok_or("ICCBased profile missing")?;
            let profile = reader.resolve(profile.clone()).map_err(|e| e.to_string())?;
            let PdfObject::Stream { dict, .. } = profile else {
                return Err("ICCBased profile is not a stream".into());
            };
            icc_component_ranges(&dict, reader)
        }
        Some("Indexed") => {
            let hival = object
                .as_array()
                .and_then(|a| a.get(2))
                .and_then(PdfObject::as_integer)
                .filter(|n| (0..=255).contains(n))
                .ok_or("invalid Indexed hival")?;
            Ok(vec![(0.0, hival as f64)])
        }
        _ => Ok(vec![(0.0, 1.0); count]),
    }
}

fn component_count(object: &PdfObject, reader: &PdfReader, depth: usize) -> Result<usize, String> {
    if depth >= MAX_DEPTH {
        return Err("colour-space component nesting exceeds 32".into());
    }
    if matches!(object, PdfObject::Reference { .. }) {
        let resolved = reader.resolve(object.clone()).map_err(|e| e.to_string())?;
        return component_count(&resolved, reader, depth + 1);
    }
    let items = match object {
        PdfObject::Array(items) => items.as_slice(),
        _ => &[],
    };
    match family(object) {
        Some("DeviceGray" | "CalGray" | "Separation" | "Indexed") => Ok(1),
        Some("DeviceRGB" | "CalRGB" | "Lab") => Ok(3),
        Some("DeviceCMYK") => Ok(4),
        Some("DeviceN") => items
            .get(1)
            .and_then(PdfObject::as_array)
            .map(|names| names.len())
            .filter(|n| *n > 0 && *n <= 16)
            .ok_or_else(|| "invalid DeviceN component names".into()),
        Some("ICCBased") => {
            let profile = items.get(1).ok_or("ICCBased space has no profile")?;
            let profile = reader.resolve(profile.clone()).map_err(|e| e.to_string())?;
            let dict = match &profile {
                PdfObject::Stream { dict, .. } => dict,
                _ => return Err("ICCBased profile is not a stream".into()),
            };
            match dict.get("N").and_then(PdfObject::as_integer) {
                Some(n @ (1 | 3 | 4)) => Ok(n as usize),
                _ => Err("invalid ICCBased component count".into()),
            }
        }
        Some("Pattern") => items
            .get(1)
            .map(|base| component_count(base, reader, depth + 1))
            .unwrap_or(Ok(0)),
        _ => Err("unsupported default colour-space family".into()),
    }
}

#[cfg(test)]
#[path = "default_colorspace_tests.rs"]
mod tests;
