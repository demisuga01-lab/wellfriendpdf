//! Source-bound static TrueType/CFF2 publication. No partially instanced
//! bytes escape if an owner, postcondition, permission or budget fails.
pub use super::cff2_program::instance::CffReport as Cff2InstanceReport;
pub use super::cff2_program::instance::{
    ContourNormalization as Cff2ContourNormalization, ContourReport as Cff2ContourReport,
};
use super::{
    font_asset::{self, FontFaceSelection, OutlineFormat},
    font_container::Container,
    variation_store::{bytes as slice, u16_at},
};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontStyleLink {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}
impl FontStyleLink {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Regular => "Regular",
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::BoldItalic => "Bold Italic",
        }
    }
}
/// Explicit replacement identity, not a guessed translation of source names.
/// Existing copyright/licence/vendor/custom strings are preserved. Identity
/// translations are replaced by canonical Unicode/Windows English records.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontInstanceNaming {
    pub family: String,
    pub subfamily: String,
    pub legacy_family: String,
    pub postscript_name: String,
    pub style_link: FontStyleLink,
}
impl FontInstanceNaming {
    pub(super) fn validate(&self) -> Result<()> {
        for value in [&self.family, &self.subfamily, &self.legacy_family] {
            if value.trim().is_empty()
                || value.encode_utf16().count() > 256
                || value.chars().any(char::is_control)
            {
                return Err(fail(
                    "instance name is empty, too long or contains controls",
                ));
            }
        }
        if self.postscript_name.is_empty()
            || self.postscript_name.len() > 63
            || self
                .postscript_name
                .bytes()
                .any(|b| !(33..=126).contains(&b) || b"[](){}<>/%".contains(&b))
        {
            return Err(fail(
                "instance PostScript name is not a valid 1..63-byte name",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontInstanceRequest {
    pub selection: FontFaceSelection,
    /// User-space coordinates; omitted axes use their declared defaults.
    /// Unknown/non-finite/out-of-range values are rejected, not silently clamped.
    pub coordinates: BTreeMap<String, f32>,
    pub naming: FontInstanceNaming,
    /// HVAR/VVAR can disagree with rounded outline-derived redundant metrics.
    /// Opt-in accepts the metric stage's declared-metric precedence and report.
    #[serde(default)]
    pub accept_redundant_metric_differences: bool,
    /// Explicit curve normalization, with a separate decision for hint loss.
    /// None checks compatibility and preserves contour/hint bytes, rejecting
    /// outlines that need reconstruction. It never publishes unchecked overlaps.
    #[serde(default)]
    pub cff2_contours: Option<Cff2ContourNormalization>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceMetricDifference {
    pub glyph: u16,
    pub vertical: bool,
    pub field: String,
    pub declared: i32,
    pub derived: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontInstanceReport {
    pub schema_version: u32,
    pub source_sha256: String,
    pub prepared_sha256: String,
    pub face_index: u32,
    pub source_face_count: usize,
    pub coordinates: BTreeMap<String, f32>,
    pub normalized_coordinates: Vec<i16>,
    pub naming: FontInstanceNaming,
    pub glyph_count: u16,
    pub output_outline_format: OutlineFormat,
    pub cff2: Option<Cff2InstanceReport>,
    pub removed_tables: Vec<String>,
    pub changed_tables: Vec<String>,
    pub preserved_tables: Vec<String>,
    pub removed_signature: bool,
    pub signature_verified: bool,
    pub subsetting_allowed: bool,
    pub metric_differences: Vec<InstanceMetricDifference>,
    pub normalized_component_offsets: Vec<[u16; 2]>,
    pub repaired_source_bounds: Vec<u16>,
    pub checked_layout_point_references: usize,
    pub replaced_name_records: usize,
    pub relocated_stat_name_ids: BTreeMap<u16, u16>,
    pub retained_stat_values: usize,
    pub removed_stat_values: usize,
    pub hint_instruction_capacity: u16,
    pub hint_stack_capacity: u16,
    /// Structural reopen postconditions are not rasterizer/corpus qualification.
    pub structural_postconditions_checked: bool,
    pub independently_render_verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstancedFontAsset {
    pub bytes: Vec<u8>,
    pub report: FontInstanceReport,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font instance: {message}"))
}
fn tag(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}
fn retired(tag: &[u8; 4]) -> bool {
    matches!(
        tag,
        b"fvar" | b"avar" | b"gvar" | b"cvar" | b"HVAR" | b"VVAR" | b"MVAR" | b"DSIG" | b"CFF2"
    )
}
fn supported(tag: &[u8; 4]) -> bool {
    retired(tag)
        || matches!(
            tag,
            b"head"
                | b"hhea"
                | b"maxp"
                | b"OS/2"
                | b"hmtx"
                | b"cmap"
                | b"loca"
                | b"glyf"
                | b"name"
                | b"post"
                | b"vhea"
                | b"vmtx"
                | b"fpgm"
                | b"prep"
                | b"cvt "
                | b"gasp"
                | b"GDEF"
                | b"GPOS"
                | b"GSUB"
                | b"BASE"
                | b"JSTF"
                | b"STAT"
                | b"ltag"
                | b"meta"
                | b"kern"
                | b"VORG"
        )
}
fn static_kern(data: &[u8], glyphs: u16) -> Result<()> {
    if u16_at(data, 0)? != 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "extended/variation kern tables need a dedicated instancing owner".into(),
        ));
    }
    let count = usize::from(u16_at(data, 2)?);
    if count > 4096 {
        return Err(fail("kern subtable count exceeds 4096"));
    }
    let mut at = 4;
    for _ in 0..count {
        crate::cancel::check_current_cancel("static kerning preservation")?;
        let length = usize::from(u16_at(data, at + 2)?);
        let coverage = u16_at(data, at + 4)?;
        if length < 6
            || u16_at(data, at)? != 0
            || coverage & 0xf0 != 0
            || !matches!(coverage >> 8, 0 | 2)
        {
            return Err(WellfriendError::UnsupportedFeature(
                "kern subtable is not a supported non-variable owner".into(),
            ));
        }
        let table = slice(data, at, length)?;
        if coverage >> 8 == 0 {
            let pairs = usize::from(u16_at(table, 6)?);
            let mut previous = None;
            for (i, pair) in slice(table, 14, pairs * 6)?.chunks_exact(6).enumerate() {
                if i % 256 == 0 {
                    crate::cancel::check_current_cancel("static kern glyph identities")?;
                }
                let key = (u16_at(pair, 0)?, u16_at(pair, 2)?);
                if key.0 >= glyphs || key.1 >= glyphs || previous.is_some_and(|p| p >= key) {
                    return Err(fail("kern pair identity/order is invalid"));
                }
                previous = Some(key);
            }
        } else {
            slice(table, 0, 14)?;
            let width = usize::from(u16_at(table, 6)?);
            if width == 0 || width % 2 != 0 {
                return Err(fail("kern class row width is invalid"));
            }
            let array = usize::from(u16_at(table, 12)?);
            if array < 14 {
                return Err(fail("kern array overlaps header"));
            }
            let mut row_ends = array + width;
            let mut classes = Vec::new();
            for field in [8, 10] {
                let owner = usize::from(u16_at(table, field)?);
                if owner < 14 {
                    return Err(fail("kern class owner overlaps header"));
                }
                let first = usize::from(u16_at(table, owner)?);
                let count = usize::from(u16_at(table, owner + 2)?);
                if first + count > usize::from(glyphs) {
                    return Err(fail("kern class glyph range outside font"));
                }
                for (i, entry) in slice(table, owner + 4, count * 2)?
                    .chunks_exact(2)
                    .enumerate()
                {
                    if i % 256 == 0 {
                        crate::cancel::check_current_cancel("kern class references")?;
                    }
                    let value = usize::from(u16_at(entry, 0)?);
                    // Left entries include the array's subtable-relative offset;
                    // right entries are byte offsets within a row. Do not
                    // interpret either mapping as a raw class number.
                    if field == 8 {
                        if value < array || (value - array) % width != 0 {
                            return Err(fail("kern left class is not an array row"));
                        }
                        slice(table, value, width)?;
                        row_ends = row_ends.max(value + width);
                    } else if value % 2 != 0 || value >= width {
                        return Err(fail("kern right class is outside its row"));
                    }
                }
                classes.push(owner..owner + 4 + count * 2);
            }
            slice(table, array, row_ends - array)?;
            if classes
                .iter()
                .any(|range| range.start < row_ends && array < range.end)
            {
                return Err(fail("kern class records overlap the value array"));
            }
            // The omitted-glyph class is row/column zero, with no adjustment.
            for row in table[array..row_ends].chunks_exact(width) {
                if row[..2] != [0, 0] {
                    return Err(fail("kern default column is nonzero"));
                }
            }
            if table[array..array + width].iter().any(|n| *n != 0) {
                return Err(fail("kern default row is nonzero"));
            }
        }
        at += length;
    }
    Ok(())
}
fn output_size(tables: &BTreeMap<[u8; 4], Vec<u8>>, limit: usize) -> Result<()> {
    let mut size = 12 + tables.len() * 16;
    for value in tables.values() {
        size = size
            .checked_add((value.len() + 3) & !3)
            .ok_or_else(|| fail("output extent overflow"))?;
        if size > limit.min(super::font_container::MAX_BYTES) {
            return Err(WellfriendError::ResourceLimit(
                "static font exceeds output budget".into(),
            ));
        }
    }
    Ok(())
}
fn digest(bytes: &[u8]) -> Result<String> {
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(65536) {
        crate::cancel::check_current_cancel("static font digest")?;
        hash.update(chunk);
    }
    Ok(format!("{:x}", hash.finalize()))
}
type TableReceipts = BTreeMap<[u8; 4], (usize, [u8; 32])>;
fn table_digest(owner: &[u8; 4], data: &[u8]) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    // The canonical writer owns this field; all other head bytes must match.
    let tail = if owner == b"head" {
        hash.update(slice(data, 0, 8)?);
        slice(data, 8, 4)?;
        hash.update([0u8; 4]);
        &data[12..]
    } else {
        data
    };
    for chunk in tail.chunks(65536) {
        crate::cancel::check_current_cancel("static font table receipt")?;
        hash.update(chunk);
    }
    Ok(hash.finalize().into())
}
fn verify_tables(bytes: &[u8], expected: &TableReceipts) -> Result<()> {
    let saved = Container::parse(bytes)?;
    if saved.collection || saved.faces.len() != 1 {
        return Err(fail("saved instance is not one standalone face"));
    }
    let actual = &saved.faces[0].tables;
    if actual.len() != expected.len() || actual.keys().any(retired) {
        return Err(fail(
            "saved instance table owners differ from staged owners",
        ));
    }
    for (owner, (len, hash)) in expected {
        crate::cancel::check_current_cancel("static font table postconditions")?;
        let range = actual
            .get(owner)
            .ok_or_else(|| fail("saved instance lost a staged table"))?;
        if range.len() != *len || table_digest(owner, &bytes[range.clone()])? != *hash {
            return Err(fail("saved instance table differs from staged bytes"));
        }
    }
    Ok(())
}
pub fn prepare_font_instance(
    source: &[u8],
    request: &FontInstanceRequest,
) -> Result<InstancedFontAsset> {
    prepare_bounded(source, request, super::font_container::MAX_BYTES)
}
pub(crate) fn prepare_bounded(
    source: &[u8],
    request: &FontInstanceRequest,
    output_limit: usize,
) -> Result<InstancedFontAsset> {
    crate::cancel::check_current_cancel("static font request")?;
    request.naming.validate()?;
    if request.coordinates.len() > 64 {
        return Err(fail("coordinate count exceeds 64"));
    }
    let catalog = font_asset::inspect_font_asset(source)?;
    if catalog.source_sha256 != request.selection.source_sha256 {
        return Err(fail(
            "selection is stale or belongs to different source bytes",
        ));
    }
    let face = catalog
        .faces
        .get(request.selection.face_index as usize)
        .ok_or_else(|| fail("face index outside source"))?;
    if !face.permission_bits_allow_editing {
        return Err(WellfriendError::UnsupportedFeature(
            "font permissions do not allow editable outline embedding".into(),
        ));
    }
    let cff2 = face.outline_format == OutlineFormat::Cff2;
    if let Some(options) = &request.cff2_contours {
        options.validate()?;
        if !cff2 {
            return Err(fail("CFF2 contour normalization requires a CFF2 source"));
        }
    }
    if !cff2 && (face.outline_format != OutlineFormat::TrueType || face.axes.is_empty()) {
        return Err(WellfriendError::UnsupportedFeature("static publication requires variable TrueType or CFF2 outlines; use ordinary preparation for other static faces".into()));
    }
    if face.signature_present && !request.selection.allow_signature_removal {
        return Err(fail(
            "static instantiation requires explicit font-signature-removal approval",
        ));
    }
    let mut design = BTreeMap::new();
    let mut selected = BTreeMap::new();
    let mut variation = super::VariationRequest::none();
    for axis in &face.axes {
        let value = request
            .coordinates
            .get(&axis.tag)
            .copied()
            .unwrap_or(axis.default);
        if !value.is_finite() || value < axis.min || value > axis.max {
            return Err(fail("coordinate outside declared axis range"));
        }
        let bytes: [u8; 4] = axis
            .tag
            .as_bytes()
            .try_into()
            .map_err(|_| fail("axis tag is not four ASCII bytes"))?;
        design.insert(bytes, value);
        selected.insert(axis.tag.clone(), value);
        variation = variation.with_axis(ttf_parser::Tag::from_bytes(&bytes), value);
    }
    if request
        .coordinates
        .keys()
        .any(|tag| !selected.contains_key(tag))
    {
        return Err(fail("coordinate request includes an unknown axis"));
    }
    let container = Container::parse(source)?;
    let directory = &container.faces[request.selection.face_index as usize];
    for owner in directory.tables.keys() {
        if !supported(owner) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "static font owner {} needs an instance-aware preservation stage",
                tag(owner)
            )));
        }
        if (cff2
            && matches!(
                owner,
                b"glyf"
                    | b"loca"
                    | b"gvar"
                    | b"cvar"
                    | b"cvt "
                    | b"prep"
                    | b"fpgm"
                    | b"gasp"
                    | b"kern"
            ))
            || (!cff2 && owner == b"VORG")
        {
            return Err(fail(
                "table owner is incompatible with selected outline format",
            ));
        }
    }
    if let Some(range) = directory.tables.get(b"cvt ") {
        if range.len() % 2 != 0 {
            return Err(fail("CVT has a partial value"));
        }
    }
    if let Some(range) = directory.tables.get(b"kern") {
        static_kern(&source[range.clone()], face.glyph_count)?;
    }
    let mut stage =
        super::font_metric_instance::prepare(source, request.selection.face_index, &variation)?;
    if stage.source_sha256 != catalog.source_sha256
        || stage.face_index != request.selection.face_index
    {
        return Err(fail("prepared stages lost source identity"));
    }
    if !stage.metrics.ignored_global_tags.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "private MVAR metrics have no publication owner".into(),
        ));
    }
    if request.cff2_contours.is_none()
        && !request.accept_redundant_metric_differences
        && !stage.metrics.differences.is_empty()
    {
        return Err(fail(
            "redundant metric differences require explicit acceptance",
        ));
    }
    let outlines = stage.outlines.take();
    let hints = stage.hints.take();
    if !cff2 && (outlines.is_none() || hints.is_none()) {
        return Err(fail("missing TrueType publication stage"));
    }
    if let Some(hints) = &hints {
        if !hints.review.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "static hint semantics require resolution ({} conditions; first eight: {:?})",
                hints.review.len(),
                &hints.review[..hints.review.len().min(8)]
            )));
        }
    }
    let layout = stage.layout.take();
    if !face.axes.is_empty() && layout.is_none() {
        return Err(fail("missing variable layout stage"));
    }
    if layout
        .as_ref()
        .is_some_and(|l| l.unchecked_point_references != 0 || l.retained_gdef_store)
    {
        return Err(fail("layout stage retains unresolved ownership"));
    }
    let mut output = BTreeMap::new();
    let normalized_component_offsets = outlines.as_ref().map_or_else(Vec::new, |o| {
        o.default_offset_components
            .iter()
            .map(|(a, b)| [*a, *b])
            .collect()
    });
    let repaired_source_bounds = outlines
        .as_ref()
        .map_or_else(Vec::new, |o| o.repaired_default_bounds.clone());
    if let Some(outlines) = outlines {
        output.extend(outlines.tables);
    }
    output.extend(std::mem::take(&mut stage.metrics.tables));
    let checked_layout_point_references = layout.as_ref().map_or(0, |l| l.checked_point_references);
    if let Some(layout) = layout {
        output.extend(layout.tables);
    }
    let hint_instruction_capacity = hints.as_ref().map_or(0, |h| h.instruction_capacity);
    let hint_stack_capacity = hints.as_ref().map_or(0, |h| h.stack_capacity);
    if let Some(hints) = hints {
        output.extend(hints.tables);
    }
    let cff_report = if cff2 {
        let source_table = &source[directory.tables[b"CFF2"].clone()];
        let program = super::cff2_program::load(source_table)?;
        let mut bbox: Option<[i16; 4]> = None;
        for glyph in &stage.geometry {
            if let Some(rect) = glyph.instance {
                let value = [rect.x_min, rect.y_min, rect.x_max, rect.y_max];
                bbox = Some(bbox.map_or(value, |b| {
                    [
                        b[0].min(value[0]),
                        b[1].min(value[1]),
                        b[2].max(value[2]),
                        b[3].max(value[3]),
                    ]
                }));
            }
        }
        let mut bbox = bbox.unwrap_or([0; 4]);
        let frozen = super::cff2_program::instance::freeze_with_contours(
            &program,
            &stage.coordinates,
            &stage
                .metrics
                .horizontal
                .iter()
                .map(|m| m.advance)
                .collect::<Vec<_>>(),
            bbox,
            &request.naming.postscript_name,
            output_limit,
            request.cff2_contours.as_ref(),
        )?;
        if let Some(bounds) = frozen.normalized_bounds {
            let mut joined: Option<[i16; 4]> = None;
            for r in bounds.iter().flatten() {
                let next = [r.x_min, r.y_min, r.x_max, r.y_max];
                joined = Some(joined.map_or(next, |b| {
                    [
                        b[0].min(next[0]),
                        b[1].min(next[1]),
                        b[2].max(next[2]),
                        b[3].max(next[3]),
                    ]
                }));
            }
            bbox = joined.unwrap_or([0; 4]);
            stage.rebase_cff_geometry(bounds)?;
            if !request.accept_redundant_metric_differences && !stage.metrics.differences.is_empty()
            {
                return Err(fail(
                    "normalized-outline metric differences require explicit acceptance",
                ));
            }
            output.extend(std::mem::take(&mut stage.metrics.tables));
        }
        output.insert(*b"CFF ", frozen.bytes);
        // CFF has no TrueType profile/instruction fields. post format 3 avoids
        // stale source name-index arrays now that charset uses explicit CIDs.
        let mut maxp = 0x00005000u32.to_be_bytes().to_vec();
        maxp.extend(face.glyph_count.to_be_bytes());
        output.insert(*b"maxp", maxp);
        let mut head = output
            .remove(b"head")
            .unwrap_or_else(|| source[directory.tables[b"head"].clone()].to_vec());
        slice(&head, 0, 54)?;
        for (i, value) in bbox.iter().enumerate() {
            head[36 + i * 2..38 + i * 2].copy_from_slice(&value.to_be_bytes());
        }
        head[50..54].fill(0);
        if stage
            .metrics
            .horizontal
            .iter()
            .zip(&stage.geometry)
            .any(|(m, g)| m.bearing != g.instance.map_or(0, |r| r.x_min))
        {
            let flags = u16::from_be_bytes(head[16..18].try_into().unwrap()) & !2;
            head[16..18].copy_from_slice(&flags.to_be_bytes());
        }
        output.insert(*b"head", head);
        if let Some(range) = directory.tables.get(b"post") {
            let post = output
                .remove(b"post")
                .unwrap_or_else(|| source[range.clone()].to_vec());
            let mut post = slice(&post, 0, 32)?.to_vec();
            post[..4].copy_from_slice(&0x00030000u32.to_be_bytes());
            output.insert(*b"post", post);
        }
        Some(frozen.report)
    } else {
        None
    };
    if let Some(cvt) = stage.cvt.take() {
        output.insert(*b"cvt ", cvt.bytes);
    }
    // Bound materialization before each original table is copied. Every retired
    // owner has completed its stage above; unknown owners never reach this loop.
    for (owner, range) in &directory.tables {
        if retired(owner) || output.contains_key(owner) {
            continue;
        }
        let retained_size = output.values().map(Vec::len).sum::<usize>();
        if retained_size
            .checked_add(range.len())
            .is_none_or(|n| n > output_limit.min(super::font_container::MAX_BYTES))
        {
            return Err(WellfriendError::ResourceLimit(
                "static font materialization budget".into(),
            ));
        }
        let mut data = Vec::with_capacity(range.len());
        for chunk in source[range.clone()].chunks(65536) {
            crate::cancel::check_current_cancel("static font table preservation")?;
            data.extend_from_slice(chunk);
        }
        output.insert(*owner, data);
    }
    output_size(&output, output_limit)?;
    let mut unique_hash = Sha256::new();
    unique_hash.update(catalog.source_sha256.as_bytes());
    unique_hash.update(request.selection.face_index.to_be_bytes());
    for (axis, value) in &design {
        unique_hash.update(axis);
        unique_hash.update(value.to_bits().to_be_bytes());
    }
    unique_hash.update(
        serde_json::to_vec(&request.naming).map_err(|_| fail("name request encoding failed"))?,
    );
    unique_hash.update(
        serde_json::to_vec(&request.cff2_contours)
            .map_err(|_| fail("contour request encoding failed"))?,
    );
    let unique = format!("WFInstance-{:x}", unique_hash.finalize());
    let metadata = super::font_instance_metadata::freeze(
        &output,
        &design,
        &request.naming,
        &unique,
        &stage
            .metrics
            .horizontal
            .iter()
            .map(|m| m.advance)
            .collect::<Vec<_>>(),
    )?;
    output.extend(metadata.tables);
    output_size(&output, output_limit)?;
    let mut changed = Vec::new();
    let mut preserved = Vec::new();
    for (owner, data) in &output {
        if directory
            .tables
            .get(owner)
            .is_some_and(|range| &source[range.clone()] == data.as_slice())
        {
            preserved.push(tag(owner));
        } else {
            changed.push(tag(owner));
        }
    }
    // head's checksum adjustment is rebuilt even if the staged bytes match.
    preserved.retain(|tag| tag != "head");
    if !changed.iter().any(|tag| tag == "head") {
        changed.push("head".into());
    }
    changed.sort();
    let removed = directory
        .tables
        .keys()
        .filter(|tag| retired(tag))
        .map(tag)
        .collect::<Vec<_>>();
    let table_receipts = output
        .iter()
        .map(|(owner, data)| Ok((*owner, (data.len(), table_digest(owner, data)?))))
        .collect::<Result<TableReceipts>>()?;
    let bytes = super::sfnt_subset::build_sfnt(if cff2 { *b"OTTO" } else { [0, 1, 0, 0] }, output)
        .map_err(|error| fail(&error.to_string()))?;
    crate::cancel::check_current_cancel("static font structural reopen")?;
    verify_tables(&bytes, &table_receipts)?;
    let reopened = ttf_parser::Face::parse(&bytes, 0)
        .map_err(|_| fail("saved static font does not reopen"))?;
    if reopened.is_variable()
        || reopened.number_of_glyphs() != face.glyph_count
        || reopened.units_per_em() != face.units_per_em
    {
        return Err(fail("saved font identity differs from selected face"));
    }
    for (id, row) in stage.metrics.horizontal.iter().enumerate() {
        if id % 256 == 0 {
            crate::cancel::check_current_cancel("static font metric postconditions")?;
        }
        let glyph = ttf_parser::GlyphId(id as u16);
        if reopened.glyph_hor_advance(glyph) != Some(row.advance)
            || reopened.glyph_hor_side_bearing(glyph) != Some(row.bearing)
        {
            return Err(fail("saved horizontal metric differs from staged value"));
        }
        if let Some(rows) = &stage.metrics.vertical {
            if reopened.glyph_ver_advance(glyph) != Some(rows[id].advance)
                || reopened.glyph_ver_side_bearing(glyph) != Some(rows[id].bearing)
            {
                return Err(fail("saved vertical metric differs from staged value"));
            }
        }
        if let Some(origins) = &stage.metrics.origins {
            if reopened.glyph_y_origin(glyph) != Some(origins[id]) {
                return Err(fail("saved vertical origin differs from staged value"));
            }
        }
        if cff2 {
            let cff = reopened
                .tables()
                .cff
                .as_ref()
                .ok_or_else(|| fail("saved CFF1 outline owner"))?;
            let bounds = match cff.outline(glyph, &mut Sink) {
                Ok(bounds) => Some(bounds),
                Err(ttf_parser::CFFError::ZeroBBox) => None,
                Err(_) => return Err(fail("saved CFF1 glyph does not parse")),
            };
            if bounds != stage.geometry[id].instance {
                return Err(fail(
                    "saved CFF1 bounds differ from selected source instance",
                ));
            }
        }
    }
    let embedding = super::pdf_embedding::EmbeddingInfo::parse(&bytes)?;
    let prepared_sha256 = digest(&bytes)?;
    let report = FontInstanceReport {
        schema_version: 1,
        source_sha256: catalog.source_sha256.clone(),
        prepared_sha256,
        face_index: request.selection.face_index,
        source_face_count: catalog.faces.len(),
        coordinates: selected,
        normalized_coordinates: stage.coordinates.iter().map(|n| n.get()).collect(),
        naming: request.naming.clone(),
        glyph_count: face.glyph_count,
        output_outline_format: if cff2 {
            OutlineFormat::Cff1
        } else {
            OutlineFormat::TrueType
        },
        cff2: cff_report,
        removed_tables: removed,
        changed_tables: changed,
        preserved_tables: preserved,
        removed_signature: face.signature_present,
        signature_verified: false,
        subsetting_allowed: embedding.may_subset,
        metric_differences: stage
            .metrics
            .differences
            .iter()
            .map(|d| InstanceMetricDifference {
                glyph: d.glyph,
                vertical: d.vertical,
                field: d.field.into(),
                declared: d.declared,
                derived: d.derived,
            })
            .collect(),
        normalized_component_offsets,
        repaired_source_bounds,
        checked_layout_point_references,
        replaced_name_records: metadata.replaced_name_records,
        relocated_stat_name_ids: metadata.relocated_name_ids,
        retained_stat_values: metadata.kept_stat_values,
        removed_stat_values: metadata.removed_stat_values,
        hint_instruction_capacity,
        hint_stack_capacity,
        structural_postconditions_checked: true,
        independently_render_verified: false,
    };
    crate::cancel::check_current_cancel("static font asset publication")?;
    Ok(InstancedFontAsset { bytes, report })
}

struct Sink;
impl ttf_parser::OutlineBuilder for Sink {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

#[cfg(test)]
#[path = "font_instance_tests.rs"]
pub(crate) mod tests;
