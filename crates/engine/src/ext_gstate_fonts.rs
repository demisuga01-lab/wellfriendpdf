//! Resolve the ExtGState /Font array to the same font registry used by Tf.
//! The internal name is not a claim that it already occurs in PDF /Resources.
use crate::{PdfDictionary, PdfObject, PdfReader};
use std::collections::{BTreeSet, HashMap};

pub(crate) fn normalize(
    dictionary: &mut PdfDictionary,
    fonts: &mut HashMap<String, PdfDictionary>,
    references: &mut HashMap<String, (u32, u16)>,
    reserved: &mut BTreeSet<String>,
    reader: &PdfReader,
) {
    let Some(value) = dictionary.get("Font") else {
        return;
    };
    let Ok(PdfObject::Array(mut items)) = reader.resolve(value.clone()) else {
        return;
    };
    if items.len() != 2 {
        return;
    }
    let Ok(size) = reader.resolve(items[1].clone()) else {
        return;
    };
    if !size.as_number().is_some_and(f64::is_finite) {
        return;
    }
    if let Some(reference) = items[0].as_reference() {
        let Ok(PdfObject::Dictionary(font)) = reader.resolve(items[0].clone()) else {
            return;
        };
        // Sorting is essential: HashMap iteration must not change editing
        // identities, saved resource names or renderer caches between calls.
        let name = references
            .iter()
            .filter(|(name, candidate)| **candidate == reference && fonts.contains_key(*name))
            .map(|(name, _)| name.clone())
            .min()
            .unwrap_or_else(|| {
                let stem = format!("WFExtFont{}_{}", reference.0, reference.1);
                let mut name = stem.clone();
                let mut suffix = 0usize;
                while reserved.contains(&name) {
                    suffix += 1;
                    name = format!("{stem}_{suffix}");
                }
                reserved.insert(name.clone());
                references.insert(name.clone(), reference);
                fonts.insert(name.clone(), font);
                name
            });
        items[0] = PdfObject::Name(name);
    }
    // Preserve the existing named-font compatibility route. Invalid first
    // operands stay invalid for the graphics-state validator, not guessed.
    items[1] = size;
    dictionary.insert("Font", PdfObject::Array(items));
}

/// Add only missing aliases referenced by normalized ExtGState font selections.
/// The original font object and shared ExtGState dictionaries remain unchanged.
pub(crate) fn materialize(
    parsed: &crate::PageResources,
    resources: &mut PdfDictionary,
    reader: &PdfReader,
) -> crate::Result<bool> {
    let names = parsed
        .ext_g_states
        .values()
        .filter_map(|d| d.get("Font")?.as_array()?.first()?.as_name())
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        return Ok(false);
    }
    let mut fonts = match resources.get("Font") {
        Some(value) => reader
            .resolve(value.clone())?
            .as_dict()
            .cloned()
            .ok_or_else(|| {
                crate::WellfriendError::MalformedPdf(
                    "source Font resource is not a dictionary".into(),
                )
            })?,
        None => PdfDictionary::empty(),
    };
    let mut changed = false;
    for name in names {
        crate::cancel::check_current_cancel("ExtGState font resource materialization")?;
        if let Some(&(number, generation)) = parsed.font_references.get(name) {
            if let Some(existing) = fonts.get(name) {
                if existing.as_reference() != Some((number, generation))
                    && reader.resolve(existing.clone())?.as_dict() != parsed.fonts.get(name)
                {
                    return Err(crate::WellfriendError::MalformedPdf(
                        "ExtGState font alias collides with a different output font".into(),
                    ));
                }
                continue;
            }
            fonts.insert(name, PdfObject::Reference { number, generation });
            changed = true;
        }
    }
    if changed {
        resources.insert("Font", PdfObject::Dictionary(fonts));
    }
    Ok(changed)
}
