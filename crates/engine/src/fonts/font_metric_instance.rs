//! Source-bound metric preparation for the complete-font instancing transaction.
//! No partially-instanced sfnt bytes are exposed from this component.
use super::{
    font_container::Container,
    glyph_metric_instance::{self, GlyphGeometry, GlyphMetricStage, OutlineKind},
    sfnt_outline::Outliner,
    variations::{apply_request_checked, VariationRequest},
};
use crate::{Result, WellfriendError};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct PreparedMetricInstance {
    pub source_sha256: String,
    pub face_index: u32,
    pub outline_kind: OutlineKind,
    pub coordinates: Vec<ttf_parser::NormalizedCoordinate>,
    pub geometry: Vec<GlyphGeometry>,
    pub metrics: GlyphMetricStage,
    /// Same source face and coordinates as metrics; not yet a whole-font writer.
    pub layout: Option<super::font_layout_instance::LayoutStage>,
    /// CVT values only: instruction and outline freezing are separate stages.
    pub cvt: Option<super::cvar_instance::FrozenCvt>,
    /// Source-bound point-preserving TrueType tables and their exact geometry.
    /// Retained instructions are not yet a complete hint-freezing transaction.
    pub outlines: Option<super::glyf_instance::OutlineStage>,
    /// Static-instance GETVARIATION fallback plus unresolved semantic receipts.
    /// Review conditions must be handled by the eventual public transaction.
    pub hints: Option<super::tt_hint_instance::HintStage>,
    /// Retained source tuple ownership for diagnostics and subsequent stages.
    #[allow(dead_code)] // Retained provenance is inspected by instance tests.
    pub glyph_variations: Option<super::gvar_instance::PreparedGvar>,
    /// Non-variable faces ignore variation-only tables as required by fvar.
    #[allow(dead_code)] // Retained provenance is inspected by instance tests.
    pub ignored_variation_tables: Vec<[u8; 4]>,
    // Retain only source metric owners, sharing the existing captured buffers.
    // Contour normalization must rebase against the source, not apply MVAR twice.
    metric_source: BTreeMap<[u8; 4], Arc<[u8]>>,
}
impl PreparedMetricInstance {
    pub(crate) fn rebase_cff_geometry(
        &mut self,
        bounds: Vec<Option<ttf_parser::Rect>>,
    ) -> Result<()> {
        if self.outline_kind != OutlineKind::Cff || bounds.len() != self.geometry.len() {
            return Err(fail(
                "normalized geometry does not match the CFF glyph domain",
            ));
        }
        let mut geometry = self.geometry.clone();
        for (glyph, bounds) in geometry.iter_mut().zip(bounds) {
            glyph.instance = bounds;
        }
        let metrics = glyph_metric_instance::freeze(
            &self.metric_source,
            &self.coordinates,
            self.outline_kind,
            &geometry,
        )?;
        if metrics.horizontal.iter().map(|m| m.advance).ne(self
            .metrics
            .horizontal
            .iter()
            .map(|m| m.advance))
        {
            return Err(fail(
                "contour normalization changed selected horizontal advances",
            ));
        }
        self.geometry = geometry;
        self.metrics = metrics;
        Ok(())
    }
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font metric preparation: {message}"))
}
pub(crate) fn prepare(
    source: &[u8],
    face_index: u32,
    request: &VariationRequest,
) -> Result<PreparedMetricInstance> {
    crate::cancel::check_current_cancel("font metric source")?;
    let container = Container::parse(source)?;
    let directory = container
        .faces
        .get(face_index as usize)
        .ok_or_else(|| fail("face index outside source"))?;
    let default =
        ttf_parser::Face::parse(source, face_index).map_err(|_| fail("invalid sfnt face"))?;
    super::instance_coordinates::validate(&default)?;
    let axes = default
        .variation_axes()
        .into_iter()
        .map(|axis| axis.tag)
        .collect::<Vec<_>>();
    if request.axes().iter().any(|axis| !axes.contains(&axis.tag)) {
        return Err(fail("explicit instance request contains an unknown axis"));
    }
    let mut selected = default.clone();
    apply_request_checked(&mut selected, request)?;
    let coordinates = selected.variation_coordinates().to_vec();
    let raw = |tag| default.raw_face().table(ttf_parser::Tag::from_bytes(tag));
    let kind = match (
        raw(b"glyf").is_some(),
        raw(b"CFF ").is_some(),
        raw(b"CFF2").is_some(),
    ) {
        (true, false, false) => OutlineKind::TrueType,
        (false, true, false) | (false, false, true) => OutlineKind::Cff,
        _ => return Err(fail("missing or competing outline programs")),
    };
    let variable = default.is_variable();
    let has_gvar = variable && raw(b"gvar").is_some();
    if has_gvar && kind != OutlineKind::TrueType {
        return Err(fail("invalid or inapplicable gvar table"));
    }
    // Alias identical table extents. Expansion limits apply before retaining a
    // second copy; collection offsets are resolved by the container directory.
    let mut tables = BTreeMap::new();
    let mut copies = BTreeMap::<(usize, usize), Arc<[u8]>>::new();
    let mut retained = 0usize;
    let mut ignored_variation_tables = Vec::new();
    for (tag, range) in &directory.tables {
        if !variable && matches!(tag, b"HVAR" | b"VVAR" | b"MVAR" | b"gvar" | b"cvar") {
            ignored_variation_tables.push(*tag);
            continue;
        }
        let key = (range.start, range.end);
        let bytes = if let Some(copy) = copies.get(&key) {
            Arc::clone(copy)
        } else {
            retained = retained
                .checked_add(range.len())
                .ok_or_else(|| fail("table capture size overflow"))?;
            if retained > super::font_container::MAX_BYTES {
                return Err(WellfriendError::ResourceLimit(
                    "font metric capture exceeds 256 MiB".into(),
                ));
            }
            let mut copy = Vec::with_capacity(range.len());
            for chunk in source[range.clone()].chunks(65536) {
                crate::cancel::check_current_cancel("font metric table capture")?;
                copy.extend_from_slice(chunk);
            }
            let copy: Arc<[u8]> = copy.into();
            copies.insert(key, Arc::clone(&copy));
            copy
        };
        tables.insert(*tag, bytes);
    }
    let glyph_variations = if has_gvar {
        Some(super::gvar_instance::PreparedGvar::prepare(
            Arc::clone(
                tables
                    .get(b"gvar")
                    .ok_or_else(|| fail("missing captured gvar"))?,
            ),
            &coordinates,
            default.number_of_glyphs(),
        )?)
    } else {
        None
    };
    let mut outlines = if kind == OutlineKind::TrueType {
        Some(super::glyf_instance::freeze(
            &tables,
            glyph_variations.as_ref(),
        )?)
    } else {
        None
    };
    let geometry = if let Some(outlines) = &outlines {
        outlines.geometry.clone()
    } else {
        let default_outlines = Outliner::new(&default)?;
        let selected_outlines = Outliner::new(&selected)?;
        let mut geometry = Vec::with_capacity(usize::from(default.number_of_glyphs()));
        for id in 0..default.number_of_glyphs() {
            crate::cancel::check_current_cancel("font metric glyph geometry")?;
            let gid = ttf_parser::GlyphId(id);
            geometry.push(GlyphGeometry {
                default: capture_cff_bounds(&default, &default_outlines, gid)?,
                instance: capture_cff_bounds(&selected, &selected_outlines, gid)?,
                phantom: None,
            });
        }
        geometry
    };
    let metrics = glyph_metric_instance::freeze(&tables, &coordinates, kind, &geometry)?;
    if let Some(outlines) = &mut outlines {
        outlines.synchronize_sidebearing_flag(&metrics.horizontal)?;
    }
    let hints = if variable {
        outlines
            .as_ref()
            .map(|outlines| super::tt_hint_instance::freeze(&tables, outlines, &coordinates))
            .transpose()?
    } else {
        None
    };
    // Both stages expose maxp for later composition. Keep their bytes identical
    // so table-merge ordering cannot restore an obsolete hint capacity.
    if let (Some(outlines), Some(hints)) = (&mut outlines, &hints) {
        let profile = hints
            .tables
            .get(b"maxp")
            .ok_or_else(|| fail("hint stage omitted maxp"))?;
        outlines.tables.insert(*b"maxp", profile.clone());
    }
    // Non-variable sources must not start applying stray VariationIndex data.
    let layout = if variable {
        Some(super::font_layout_instance::freeze_with_points(
            &tables,
            &coordinates,
            outlines
                .as_ref()
                .map(|outline| Arc::from(outline.point_counts.clone())),
        )?)
    } else {
        None
    };
    let cvt = if variable {
        if let Some(cvar) = tables.get(b"cvar") {
            if kind != OutlineKind::TrueType {
                return Err(fail("cvar requires TrueType outlines"));
            }
            let cvt = tables
                .get(b"cvt ")
                .ok_or_else(|| fail("cvar has no CVT owner"))?;
            Some(super::cvar_instance::freeze(cvt, cvar, &coordinates)?)
        } else {
            None
        }
    } else {
        None
    };
    let mut hash = Sha256::new();
    for chunk in source.chunks(65536) {
        crate::cancel::check_current_cancel("font metric source digest")?;
        hash.update(chunk);
    }
    crate::cancel::check_current_cancel("prepared font metric publication")?;
    Ok(PreparedMetricInstance {
        source_sha256: format!("{:x}", hash.finalize()),
        face_index,
        outline_kind: kind,
        coordinates,
        geometry,
        metrics,
        layout,
        cvt,
        outlines,
        hints,
        glyph_variations,
        ignored_variation_tables,
        metric_source: if kind == OutlineKind::Cff {
            tables
                .into_iter()
                .filter(|(tag, _)| {
                    matches!(
                        tag,
                        b"maxp"
                            | b"hhea"
                            | b"hmtx"
                            | b"vhea"
                            | b"vmtx"
                            | b"VORG"
                            | b"HVAR"
                            | b"VVAR"
                            | b"MVAR"
                            | b"OS/2"
                            | b"post"
                            | b"gasp"
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        },
    })
}

fn capture_cff_bounds(
    face: &ttf_parser::Face<'_>,
    outliner: &Outliner<'_, '_>,
    gid: ttf_parser::GlyphId,
) -> Result<Option<ttf_parser::Rect>> {
    if face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"CFF2"))
        .is_some()
    {
        return outliner.bounds(gid);
    }
    let cff = face
        .tables()
        .cff
        .as_ref()
        .ok_or_else(|| fail("CFF1 outline table did not parse"))?;
    match cff.outline(gid, &mut Sink) {
        Ok(bounds) => Ok(Some(bounds)),
        Err(ttf_parser::CFFError::ZeroBBox) => Ok(None),
        Err(error) => Err(fail(&format!("CFF1 metric geometry failed: {error:?}"))),
    }
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
#[path = "font_metric_instance_tests.rs"]
mod tests;
