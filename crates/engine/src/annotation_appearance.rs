//! Shared source-appearance selection and placement. No synthetic appearance
//! is invented here: synthesized pixels have no source MCID or stream identity.
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::transform::Transform2D;
use std::collections::{HashMap, HashSet};

type ObjectId = (u32, u16);

pub(crate) struct SelectedAppearance {
    pub stream: Option<(u32, u16)>,
    pub state: Option<String>,
    pub dict: PdfDictionary,
    pub raw: Vec<u8>,
}

fn malformed(message: impl Into<String>) -> WellfriendError {
    WellfriendError::MalformedPdf(format!("annotation appearance: {}", message.into()))
}

pub(crate) fn select_normal(
    annotation: &PdfDictionary,
    reader: &PdfReader,
) -> Result<Option<SelectedAppearance>> {
    let Some(value) = annotation.get("AP").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = reader.resolve(value.clone())?;
    let ap = object
        .as_dict()
        .ok_or_else(|| malformed("AP is not a dictionary"))?;
    let normal = ap.get("N").ok_or_else(|| malformed("AP has no N"))?;
    let value = reader.resolve(normal.clone())?;
    let selected = match value {
        PdfObject::Stream { dict, raw } => {
            return Ok(Some(SelectedAppearance {
                stream: normal.as_reference(),
                state: None,
                dict,
                raw,
            }));
        }
        PdfObject::Dictionary(states) => {
            let state = match annotation.get("AS") {
                None => None,
                Some(PdfObject::Name(name)) => Some(name.as_str()),
                Some(value) => {
                    return Err(malformed(format!(
                        "/AS resolved to {}, expected Name",
                        value.variant_name()
                    )))
                }
            };
            if let Some(state) = state {
                // An explicit unavailable state must not select another state's text.
                states
                    .get(state)
                    .map(|value| (value.clone(), state.to_string()))
            } else {
                states
                    .get("Off")
                    .map(|value| (value.clone(), "Off".to_string()))
                    .or_else(|| {
                        states
                            .entries()
                            .find(|(name, _)| name.as_str() != "Off")
                            .map(|(name, value)| (value.clone(), name.clone()))
                    })
            }
        }
        _ => return Err(malformed("AP N is not a stream or state dictionary")),
    };
    let Some((selected, state)) = selected else {
        return Ok(None);
    };
    match reader.resolve(selected.clone())? {
        PdfObject::Stream { dict, raw } => Ok(Some(SelectedAppearance {
            stream: selected.as_reference(),
            state: Some(state),
            dict,
            raw,
        })),
        _ => Err(malformed("selected state is not a stream")),
    }
}

pub(crate) fn rectangle(dict: &PdfDictionary, key: &str, reader: &PdfReader) -> Result<[f64; 4]> {
    let value = dict
        .get(key)
        .ok_or_else(|| malformed(format!("missing {key}")))?;
    let resolved = reader.resolve(value.clone())?;
    let array = resolved
        .as_array()
        .ok_or_else(|| malformed(format!("{key} is not an array")))?;
    if array.len() != 4 {
        return Err(malformed(format!("{key} must contain four numbers")));
    }
    let mut result = [0.0; 4];
    for (target, value) in result.iter_mut().zip(array) {
        *target = reader
            .resolve(value.clone())?
            .as_number()
            .filter(|n| n.is_finite())
            .ok_or_else(|| malformed(format!("invalid {key} number")))?;
    }
    Ok(result)
}

/// One extraction/recovery transaction's ownership index. Each page annotation
/// array and owner's appearance-state graph is scanned once, not once per MCID.
#[derive(Default)]
pub(crate) struct AppearanceOwnerIndex {
    pages: HashMap<ObjectId, HashSet<ObjectId>>,
    streams: HashMap<ObjectId, HashSet<ObjectId>>,
    entries: usize,
}

impl AppearanceOwnerIndex {
    fn charge(&mut self) -> Result<()> {
        crate::cancel::check_current_cancel("annotation owner index")?;
        self.entries += 1;
        if self.entries > 1_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "annotation owner index limit".into(),
            ));
        }
        Ok(())
    }

    /// Inactive N/R/D states are valid owners but are not extracted as active text.
    pub(crate) fn validate(
        &mut self,
        reader: &PdfReader,
        page: (u32, u16),
        owner: (u32, u16),
        stream: (u32, u16),
    ) -> Result<()> {
        if !self.pages.contains_key(&page) {
            self.charge()?;
            let object = reader.get_object(page.0, page.1)?;
            let dict = object
                .as_dict()
                .ok_or_else(|| malformed("owner page is not a dictionary"))?;
            let annots = dict
                .get("Annots")
                .ok_or_else(|| malformed("StmOwn is not on the MCR page"))?;
            let annots = reader.resolve(annots.clone())?;
            let annots = annots
                .as_array()
                .ok_or_else(|| malformed("Annots is not an array"))?;
            if annots.len() > 250_000 {
                return Err(WellfriendError::ResourceLimit(
                    "annotation owner lookup limit".into(),
                ));
            }
            let mut owners = HashSet::new();
            for value in annots {
                self.charge()?;
                if let Some(id) = value.as_reference() {
                    owners.insert(id);
                }
            }
            self.pages.insert(page, owners);
        }
        if !self
            .pages
            .get(&page)
            .is_some_and(|owners| owners.contains(&owner))
        {
            return Err(malformed("StmOwn is not on the MCR page"));
        }
        if let Some(streams) = self.streams.get(&owner) {
            return if streams.contains(&stream) {
                Ok(())
            } else {
                Err(malformed(
                    "StmOwn does not reference the MCR Stm appearance",
                ))
            };
        }
        self.charge()?;
        let object = reader.get_object(owner.0, owner.1)?;
        let dict = object
            .as_dict()
            .ok_or_else(|| malformed("StmOwn is not an annotation dictionary"))?;
        let ap = dict
            .get("AP")
            .ok_or_else(|| malformed("StmOwn has no AP"))?;
        let ap = reader.resolve(ap.clone())?;
        let ap = ap
            .as_dict()
            .ok_or_else(|| malformed("StmOwn AP is not a dictionary"))?;
        let mut owned_streams = HashSet::new();
        for key in ["N", "R", "D"] {
            let Some(value) = ap.get(key) else {
                continue;
            };
            match reader.resolve(value.clone())? {
                PdfObject::Stream { .. } => {
                    if let Some(id) = value.as_reference() {
                        owned_streams.insert(id);
                    }
                }
                PdfObject::Dictionary(states) => {
                    let mut count = 0;
                    for (_, value) in states.entries() {
                        self.charge()?;
                        count += 1;
                        if count > 4096 {
                            return Err(WellfriendError::ResourceLimit(
                                "annotation state owner lookup limit".into(),
                            ));
                        }
                        if let Some(id) = value.as_reference() {
                            owned_streams.insert(id);
                        }
                    }
                }
                _ => {
                    return Err(malformed(
                        "appearance entry is not a stream or state dictionary",
                    ))
                }
            }
        }
        let present = owned_streams.contains(&stream);
        self.streams.insert(owner, owned_streams);
        if present {
            Ok(())
        } else {
            Err(malformed(
                "StmOwn does not reference the MCR Stm appearance",
            ))
        }
    }
}

/// ISO 32000 appearance placement maps the *Matrix-transformed* BBox to Rect.
/// The caller applies Matrix exactly once before this returned placement.
pub(crate) fn placement(rect: [f64; 4], bbox: [f64; 4], matrix: [f64; 6]) -> Option<Transform2D> {
    if rect
        .iter()
        .chain(bbox.iter())
        .chain(matrix.iter())
        .any(|v| !v.is_finite())
    {
        return None;
    }
    let transform = Transform2D::from(matrix);
    let corners = [
        transform.transform_point(bbox[0], bbox[1]),
        transform.transform_point(bbox[2], bbox[1]),
        transform.transform_point(bbox[0], bbox[3]),
        transform.transform_point(bbox[2], bbox[3]),
    ];
    if corners
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return None;
    }
    let x0 = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let y0 = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let x1 = corners
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let y1 = corners
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let width = x1 - x0;
    let height = y1 - y0;
    let rect_width = (rect[2] - rect[0]).abs();
    let rect_height = (rect[3] - rect[1]).abs();
    if width <= 0.0 || height <= 0.0 || rect_width <= 0.0 || rect_height <= 0.0 {
        return None;
    }
    let result = Transform2D::translation(-x0, -y0)
        .concat(&Transform2D::scale(
            rect_width / width,
            rect_height / height,
        ))
        .concat(&Transform2D::translation(
            rect[0].min(rect[2]),
            rect[1].min(rect[3]),
        ));
    result
        .to_array()
        .iter()
        .all(|v| v.is_finite())
        .then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_transformed_corners_fill_rect_for_translation_rotation_and_shear() {
        let bbox = [-10.0, 5.0, 90.0, 25.0];
        let rect = [60.0, 140.0, 20.0, 100.0]; // reversed Rect is normalized
        for matrix in [
            [1.0, 0.0, 0.0, 1.0, 100.0, -70.0],
            [0.0, 1.0, -1.0, 0.0, 30.0, 40.0],
            [1.0, 0.2, -0.3, 2.0, 30.0, 40.0],
            [-2.0, 0.0, 0.0, -3.0, 30.0, 40.0],
        ] {
            let ctm = Transform2D::from(matrix).concat(&placement(rect, bbox, matrix).unwrap());
            let points = [
                (bbox[0], bbox[1]),
                (bbox[2], bbox[1]),
                (bbox[0], bbox[3]),
                (bbox[2], bbox[3]),
            ]
            .map(|(x, y)| ctm.transform_point(x, y));
            for (actual, expected) in [
                (
                    points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
                    20.0,
                ),
                (
                    points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
                    60.0,
                ),
                (
                    points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
                    100.0,
                ),
                (
                    points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
                    140.0,
                ),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-9,
                    "{matrix:?}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn degenerate_or_nonfinite_appearance_geometry_is_not_placed() {
        let unit = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert!(placement([0.0; 4], [0.0, 0.0, 10.0, 20.0], unit).is_none());
        assert!(placement([0.0, 0.0, 10.0, 20.0], [0.0; 4], unit).is_none());
        assert!(placement(
            [0.0, 0.0, 10.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [f64::NAN; 6]
        )
        .is_none());
        assert!(placement([0.0, 0.0, 10.0, 20.0], [0.0, 0.0, 10.0, 20.0], [0.0; 6]).is_none());
    }
}
