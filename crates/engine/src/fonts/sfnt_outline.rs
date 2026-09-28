//! One source-aware sfnt outline gateway for paint, coverage and measurement.
use crate::{Result, WellfriendError};
use std::sync::Arc;
pub(crate) struct Outliner<'f, 'd> {
    face: &'f ttf_parser::Face<'d>,
    prepared: PreparedOutlines,
}

/// Parsed outline-program state that can be reused with one immutable face.
/// Unlike `Outliner`, this does not borrow the face and can therefore live next
/// to it in a request-local prepared-font collection without self references.
pub(crate) struct PreparedOutlines {
    cff2: Option<Arc<super::cff2_program::Program>>,
}
impl PreparedOutlines {
    pub fn new(face: &ttf_parser::Face<'_>) -> Result<Self> {
        let cff2 = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"CFF2"))
            .map(super::cff2_program::load)
            .transpose()?;
        if let Some(program) = &cff2 {
            if [b"glyf", b"CFF "].iter().any(|tag| {
                face.raw_face()
                    .table(ttf_parser::Tag::from_bytes(tag))
                    .is_some()
            }) {
                return Err(WellfriendError::invalid_input(
                    "CFF2 font also declares a competing outline program",
                ));
            }
            if program.glyph_count() != usize::from(face.number_of_glyphs()) {
                return Err(WellfriendError::invalid_input(
                    "CFF2 and maxp glyph counts differ",
                ));
            }
            if (program.matrix_scale * f64::from(face.units_per_em()) - 1.0).abs() > 0.000001 {
                return Err(WellfriendError::invalid_input(
                    "CFF2 FontMatrix and unitsPerEm disagree",
                ));
            }
        }
        Ok(Self { cff2 })
    }
    pub fn outline(
        &self,
        face: &ttf_parser::Face<'_>,
        gid: ttf_parser::GlyphId,
        pen: &mut dyn ttf_parser::OutlineBuilder,
    ) -> Result<Option<ttf_parser::Rect>> {
        crate::cancel::check_current_cancel("sfnt glyph outline")?;
        if gid.0 >= face.number_of_glyphs() {
            return Err(WellfriendError::invalid_input("glyph outside sfnt program"));
        }
        match &self.cff2 {
            Some(program) => program.outline(gid.0, face.variation_coordinates(), pen),
            None => Ok(face.outline_glyph(gid, pen)),
        }
    }
    pub fn bounds(
        &self,
        face: &ttf_parser::Face<'_>,
        gid: ttf_parser::GlyphId,
    ) -> Result<Option<ttf_parser::Rect>> {
        if self.cff2.is_none() {
            return Ok(face.glyph_bounding_box(gid));
        }
        self.outline(face, gid, &mut Sink)
    }
}
impl<'f, 'd> Outliner<'f, 'd> {
    pub fn new(face: &'f ttf_parser::Face<'d>) -> Result<Self> {
        Ok(Self {
            face,
            prepared: PreparedOutlines::new(face)?,
        })
    }
    pub fn outline(
        &self,
        gid: ttf_parser::GlyphId,
        pen: &mut dyn ttf_parser::OutlineBuilder,
    ) -> Result<Option<ttf_parser::Rect>> {
        self.prepared.outline(self.face, gid, pen)
    }
    pub fn bounds(&self, gid: ttf_parser::GlyphId) -> Result<Option<ttf_parser::Rect>> {
        self.prepared.bounds(self.face, gid)
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
pub(crate) fn outline(
    face: &ttf_parser::Face<'_>,
    gid: ttf_parser::GlyphId,
    pen: &mut dyn ttf_parser::OutlineBuilder,
) -> Result<Option<ttf_parser::Rect>> {
    Outliner::new(face)?.outline(gid, pen)
}
