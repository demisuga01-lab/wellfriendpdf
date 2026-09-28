//! Explicit, source-hash-bound selection of standalone/collection font faces.
//! Preparation changes the sfnt container, not glyphs, shaping or font rights.
use super::font_container::Container;
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "font_asset_tests.rs"]
pub(crate) mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlineFormat {
    TrueType,
    Cff1,
    Cff2,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAxis {
    pub tag: String,
    pub min: f32,
    pub default: f32,
    pub max: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAssetFace {
    pub face_index: u32,
    pub family: Option<String>,
    pub subfamily: Option<String>,
    pub postscript_name: Option<String>,
    pub outline_format: OutlineFormat,
    pub glyph_count: u16,
    pub units_per_em: u16,
    pub axes: Vec<FontAxis>,
    /// Parsed OS/2 bits only; not a legal determination about use of the font.
    pub permission_bits_allow_editing: bool,
    pub subsetting_allowed: bool,
    pub signature_present: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAssetCatalog {
    pub schema_version: u32,
    pub source_sha256: String,
    pub collection: bool,
    pub faces: Vec<FontAssetFace>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFaceSelection {
    pub source_sha256: String,
    /// Explicit even for a one-face collection. Standalone files use zero.
    pub face_index: u32,
    /// Extraction invalidates a collection signature. Never remove it silently.
    #[serde(default)]
    pub allow_signature_removal: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontPreparationReport {
    pub schema_version: u32,
    pub source_sha256: String,
    pub prepared_sha256: String,
    pub face_index: u32,
    pub source_face_count: usize,
    pub extracted_collection: bool,
    pub removed_signature: bool,
    pub signature_verified: bool,
    pub outline_format: OutlineFormat,
    pub subsetting_allowed: bool,
    /// Axes are retained at their defaults, not frozen to a new static instance.
    pub retained_variation_axes: Vec<FontAxis>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedFontAsset {
    pub bytes: Vec<u8>,
    pub report: FontPreparationReport,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font asset: {message}"))
}
fn hash(bytes: &[u8]) -> Result<String> {
    let mut digest = Sha256::new();
    for chunk in bytes.chunks(64 * 1024) {
        crate::cancel::check_current_cancel("font asset hash")?;
        digest.update(chunk);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn summary(bytes: &[u8], container: &Container, index: usize) -> Result<FontAssetFace> {
    let face = ttf_parser::Face::parse(bytes, index as u32)
        .map_err(|_| fail("selected sfnt face is malformed"))?;
    let tables = &container.faces[index].tables;
    let outline_format = if tables.contains_key(b"CFF2") {
        OutlineFormat::Cff2
    } else if face.tables().glyf.is_some() && face.tables().cff.is_none() {
        OutlineFormat::TrueType
    } else if face.tables().cff.is_some() && face.tables().glyf.is_none() {
        OutlineFormat::Cff1
    } else {
        OutlineFormat::Other
    };
    let mut names = std::collections::BTreeMap::<u16, (bool, String)>::new();
    // A malicious name table must not turn discovery into unbounded allocation.
    let records = face.names();
    if records.len() > 4096 {
        return Err(WellfriendError::ResourceLimit(
            "font name records exceed 4096".into(),
        ));
    }
    for (index, name) in records.into_iter().enumerate() {
        if index % 32 == 0 {
            crate::cancel::check_current_cancel("font face names")?;
        }
        if !matches!(name.name_id, 1 | 2 | 6 | 16 | 17) || name.name.len() > 2048 {
            continue;
        }
        let english = name.language_id == 0x0409;
        if names
            .get(&name.name_id)
            .is_some_and(|(preferred, _)| *preferred || !english)
        {
            continue;
        }
        if let Some(value) = name.to_string() {
            names.insert(name.name_id, (english, value));
        }
    }
    let name = |primary, fallback| {
        names
            .get(&primary)
            .or_else(|| names.get(&fallback))
            .map(|(_, value)| value.clone())
    };
    let variation_axes = face.variation_axes();
    if variation_axes.len() > 64 {
        return Err(fail("invalid or excessive variation axes"));
    }
    if tables.contains_key(b"OS/2") && face.tables().os2.is_none() {
        return Err(fail("malformed OS/2 permissions table"));
    }
    let axes = variation_axes
        .into_iter()
        .map(|axis| FontAxis {
            tag: String::from_utf8_lossy(&axis.tag.to_bytes()).into_owned(),
            min: axis.min_value,
            default: axis.def_value,
            max: axis.max_value,
        })
        .collect::<Vec<_>>();
    let mut axis_tags = std::collections::BTreeSet::new();
    if axes.iter().any(|a| {
        !a.tag.bytes().all(|b| (0x20..=0x7e).contains(&b)) || !axis_tags.insert(a.tag.clone())
    }) {
        return Err(fail("invalid or duplicate variation-axis tag"));
    }
    if axes.len() > 64
        || axes.iter().any(|a| {
            !a.min.is_finite()
                || !a.default.is_finite()
                || !a.max.is_finite()
                || a.min > a.default
                || a.default > a.max
        })
    {
        return Err(fail("invalid or excessive variation axes"));
    }
    let permission_bits_allow_editing =
        super::pdf_embedding::editable_outline_embedding_allowed(&face);
    Ok(FontAssetFace {
        face_index: index as u32,
        family: name(16, 1),
        subfamily: name(17, 2),
        postscript_name: name(6, 6),
        outline_format,
        glyph_count: face.number_of_glyphs(),
        units_per_em: face.units_per_em(),
        axes,
        permission_bits_allow_editing,
        subsetting_allowed: super::pdf_embedding::serialized_subsetting_allowed(&face),
        signature_present: container.collection_signature || tables.contains_key(b"DSIG"),
    })
}

/// Enumerate without inventing a face selection, installing fonts, or modifying
/// the PDF. CFF2 can be discovered even though editable preparation is separate.
pub fn inspect_font_asset(bytes: &[u8]) -> Result<FontAssetCatalog> {
    let container = Container::parse(bytes)?;
    let mut faces = Vec::with_capacity(container.faces.len());
    for index in 0..container.faces.len() {
        crate::cancel::check_current_cancel("font asset discovery")?;
        faces.push(summary(bytes, &container, index)?);
    }
    Ok(FontAssetCatalog {
        schema_version: 1,
        source_sha256: hash(bytes)?,
        collection: container.collection,
        faces,
    })
}

/// Prepare a caller-selected font for existing generated-text/authoring paths.
/// Unknown variation coordinates cannot be supplied: this does not approximate
/// CFF2 instantiation by discarding variation-dependent layout/metric tables.
pub fn prepare_font_asset(
    bytes: &[u8],
    selection: &FontFaceSelection,
) -> Result<PreparedFontAsset> {
    prepare_font_asset_bounded(bytes, selection, super::font_container::MAX_BYTES)
}

pub(crate) fn prepare_font_asset_bounded(
    bytes: &[u8],
    selection: &FontFaceSelection,
    output_limit: usize,
) -> Result<PreparedFontAsset> {
    let container = Container::parse(bytes)?;
    let source_sha256 = hash(bytes)?;
    if source_sha256 != selection.source_sha256 {
        return Err(fail("selection belongs to different source bytes"));
    }
    let index = usize::try_from(selection.face_index).map_err(|_| fail("face index overflow"))?;
    if index >= container.faces.len() {
        return Err(fail("face index outside collection"));
    }
    let face = summary(bytes, &container, index)?;
    if !face.permission_bits_allow_editing {
        return Err(WellfriendError::UnsupportedFeature(
            "font permissions do not allow editable outline embedding".into(),
        ));
    }
    if !matches!(
        face.outline_format,
        OutlineFormat::TrueType | OutlineFormat::Cff1
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "editable preparation requires glyf or CFF1; CFF2 needs static instantiation".into(),
        ));
    }
    let removed_signature = container.collection && face.signature_present;
    if removed_signature && !selection.allow_signature_removal {
        return Err(fail(
            "selected collection face requires explicit signature-removal approval",
        ));
    }
    let output = if container.collection {
        container.extract(bytes, index, output_limit)?
    } else {
        if bytes.len() > output_limit {
            return Err(WellfriendError::ResourceLimit(
                "font exceeds preparation output budget".into(),
            ));
        }
        bytes.to_vec()
    };
    let embedding = super::pdf_embedding::EmbeddingInfo::parse(&output)?;
    let prepared_sha256 = hash(&output)?;
    crate::cancel::check_current_cancel("prepared font publication")?;
    Ok(PreparedFontAsset {
        bytes: output,
        report: FontPreparationReport {
            schema_version: 1,
            source_sha256,
            prepared_sha256,
            face_index: selection.face_index,
            source_face_count: container.faces.len(),
            extracted_collection: container.collection,
            removed_signature,
            signature_verified: false,
            outline_format: face.outline_format,
            subsetting_allowed: embedding.may_subset,
            retained_variation_axes: face.axes,
        },
    })
}
