//! Logical line breaking with final-line OpenType measurement. A visual glyph
//! array is never sliced into lines: final lines use paragraph-derived bidi and
//! bounded non-emitting context for joining across soft wraps.
use super::line_break_policy::{LineBreakSettings, LineComposition};
use super::shaper::{OpenTypeSettings, ParagraphBidi};
use super::{ShapeOptions, TextShaper};
use crate::error::{Result, WellfriendError};
use std::collections::BTreeSet;
use std::ops::Range;
use unicode_linebreak::{linebreaks, BreakOpportunity};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone)]
pub struct MeasuredLine {
    pub bytes: Range<usize>,
    pub width: f64,
}

/// A geometric inability to lay out the next line, not a shaping/decoder error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineBlockReason {
    GraphemeTooWide,
    ProtectedSequenceTooWide,
    ChosenLineTooWide,
    NoFittingContinuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineBlock {
    pub byte_start: usize,
    pub reason: LineBlockReason,
}
impl LineBlock {
    pub fn into_error(self) -> WellfriendError {
        let message = match self.reason {
            LineBlockReason::GraphemeTooWide => "one shaped grapheme exceeds frame width",
            LineBlockReason::ProtectedSequenceTooWide => {
                "unbreakable text exceeds frame width under paragraph wrap policy"
            }
            LineBlockReason::ChosenLineTooWide => "final shaped line exceeds frame width",
            LineBlockReason::NoFittingContinuation => {
                "no permitted shaped continuation fits this frame width"
            }
        };
        WellfriendError::UnsupportedFeature(format!(
            "{message} at paragraph byte {}",
            self.byte_start
        ))
    }
}

/// A bounded, measured prefix. `blocked` only describes the next attempted line;
/// `None` can mean either paragraph completion or exhaustion of `max_lines`.
#[derive(Debug, Clone)]
pub struct LineBreakBatch {
    pub lines: Vec<MeasuredLine>,
    pub blocked: Option<LineBlock>,
}

#[derive(Debug, Clone, Copy)]
pub struct LineMetrics {
    pub advance: f64,
    pub left_pad: f64,
    pub right_pad: f64,
    pub ascent: f64,
    pub descent: f64,
}

/// Request-local parsed face and outline program. The bytes stay owned by the
/// caller's approved font asset; CFF2 program decoding and sfnt table parsing
/// happen once, while glyph bounds remain exact for every measured run.
pub(crate) struct PreparedFontMetrics<'a> {
    face: ttf_parser::Face<'a>,
    outlines: super::sfnt_outline::PreparedOutlines,
}
impl<'a> PreparedFontMetrics<'a> {
    pub(crate) fn new(font: &'a [u8]) -> Result<Self> {
        let face = ttf_parser::Face::parse(font, 0)
            .map_err(|_| WellfriendError::invalid_input("invalid line font"))?;
        let outlines = super::sfnt_outline::PreparedOutlines::new(&face)?;
        Ok(Self { face, outlines })
    }

    pub(crate) fn measure(&self, run: &super::ShapedRun, font_size: f64) -> Result<LineMetrics> {
        self.measure_progression(run, font_size, false)
    }

    pub(crate) fn units_per_em(&self) -> u16 {
        self.face.units_per_em()
    }

    pub(crate) fn ascender(&self) -> i16 {
        self.face.ascender()
    }

    pub(crate) fn descender(&self) -> i16 {
        self.face.descender()
    }

    pub(crate) fn bounds(&self, glyph: u16) -> Result<Option<ttf_parser::Rect>> {
        self.outlines.bounds(&self.face, ttf_parser::GlyphId(glyph))
    }

    fn measure_progression(
        &self,
        run: &super::ShapedRun,
        font_size: f64,
        signed: bool,
    ) -> Result<LineMetrics> {
        measure_prepared_progression(&self.face, &self.outlines, run, font_size, signed)
    }
}
impl LineMetrics {
    pub fn width(self) -> f64 {
        self.advance + self.left_pad + self.right_pad
    }
}

/// Include outline extents and GPOS offsets: advance width alone does not bound
/// italic overhangs, combining marks, ascenders or descenders.
pub fn measure_run(font: &[u8], run: &super::ShapedRun, font_size: f64) -> Result<LineMetrics> {
    measure_run_progression(font, run, font_size, false)
}

/// Authoring emits signed per-occurrence advances. Preserve those pen movements
/// when measuring, instead of using the older editor's absolute-advance policy.
pub(crate) fn measure_signed_run(
    font: &[u8],
    run: &super::ShapedRun,
    font_size: f64,
) -> Result<LineMetrics> {
    measure_run_progression(font, run, font_size, true)
}

fn measure_run_progression(
    font: &[u8],
    run: &super::ShapedRun,
    font_size: f64,
    signed: bool,
) -> Result<LineMetrics> {
    PreparedFontMetrics::new(font)?.measure_progression(run, font_size, signed)
}

fn measure_prepared_progression(
    face: &ttf_parser::Face<'_>,
    outlines: &super::sfnt_outline::PreparedOutlines,
    run: &super::ShapedRun,
    font_size: f64,
    signed: bool,
) -> Result<LineMetrics> {
    if !font_size.is_finite() || font_size <= 0.0 {
        return Err(WellfriendError::invalid_input("invalid measured font size"));
    }
    let scale = font_size / f64::from(face.units_per_em()).max(1.0);
    let mut pen = 0.0f64;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut ascent = (f64::from(face.ascender()) * scale).max(0.0);
    let mut descent = (-f64::from(face.descender()) * scale).max(0.0);
    for (index, glyph) in run.glyphs.iter().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("line glyph measurement")?;
        }
        if ![glyph.advance, glyph.offset_x, glyph.offset_y]
            .iter()
            .all(|n| n.is_finite())
        {
            return Err(WellfriendError::invalid_input(
                "invalid measured glyph position",
            ));
        }
        if let Some(bounds) = outlines.bounds(face, ttf_parser::GlyphId(glyph.glyph_id))? {
            let x = pen + glyph.offset_x * font_size / 1000.0;
            let y = glyph.offset_y * font_size / 1000.0;
            left = left.min(x + f64::from(bounds.x_min) * scale);
            right = right.max(x + f64::from(bounds.x_max) * scale);
            ascent = ascent.max(y + f64::from(bounds.y_max) * scale);
            descent = descent.max(-y - f64::from(bounds.y_min) * scale);
        }
        pen += (if signed {
            glyph.advance
        } else {
            glyph.advance.abs()
        }) * font_size
            / 1000.0;
        left = left.min(pen);
        right = right.max(pen);
    }
    if ![pen, left, right, ascent, descent]
        .iter()
        .all(|n| n.is_finite())
    {
        return Err(WellfriendError::invalid_input(
            "measured line geometry overflow",
        ));
    }
    Ok(LineMetrics {
        advance: pen,
        left_pad: (-left).max(0.0),
        right_pad: (right - pen).max(0.0),
        ascent,
        descent,
    })
}

pub fn break_lines(
    font: &[u8],
    text: &str,
    font_size: f64,
    width: f64,
    options: ShapeOptions,
) -> Result<Vec<MeasuredLine>> {
    break_lines_from(
        font,
        text,
        0,
        font_size,
        width,
        options,
        &OpenTypeSettings::default(),
    )
}

/// Break the remaining text without discarding the preceding paragraph's bidi
/// context. Returned ranges remain relative to the full logical paragraph.
pub fn break_lines_from(
    font: &[u8],
    text: &str,
    from: usize,
    font_size: f64,
    width: f64,
    options: ShapeOptions,
    settings: &OpenTypeSettings,
) -> Result<Vec<MeasuredLine>> {
    break_lines_from_limited(
        font, text, from, font_size, width, options, settings, 100_000,
    )
}

/// Bounded lookahead for pagination: a frame needs its capacity plus the widow
/// margin, not every remaining line of a thousand-page story on every page.
pub fn break_lines_from_limited(
    font: &[u8],
    text: &str,
    from: usize,
    font_size: f64,
    width: f64,
    options: ShapeOptions,
    settings: &OpenTypeSettings,
    max_lines: usize,
) -> Result<Vec<MeasuredLine>> {
    PreparedParagraph::new(text, options)?
        .break_lines(font, from, font_size, width, settings, max_lines)
}

/// Reusable logical indexes prepared once per paragraph, not once per frame.
/// Ordered opportunity insertion and boundary checks add logarithmic factors;
/// line ranges always refer to the original text.
pub struct PreparedParagraph<'a> {
    pub text: &'a str,
    pub bidi: ParagraphBidi<'a>,
    boundaries: Vec<usize>,
    allowed: BTreeSet<usize>,
    emergency: Vec<usize>,
    mandatory: Vec<usize>,
    composition: LineComposition,
}
impl<'a> PreparedParagraph<'a> {
    pub fn new(text: &'a str, options: ShapeOptions) -> Result<Self> {
        Self::with_break_settings(text, options, &LineBreakSettings::default())
    }

    pub fn with_break_settings(
        text: &'a str,
        options: ShapeOptions,
        settings: &LineBreakSettings,
    ) -> Result<Self> {
        settings.validate()?;
        if text.len() > 16 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "line layout paragraph exceeds 16 MiB".into(),
            ));
        }
        crate::cancel::check_current_cancel("paragraph preparation")?;
        let bidi = ParagraphBidi::new(text, options)?;
        let boundaries = text
            .grapheme_indices(true)
            .map(|(n, _)| n)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let opportunities = linebreaks(text).collect::<Vec<_>>();
        let mut allowed = opportunities
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| boundaries.binary_search(n).is_ok())
            .collect();
        let mandatory = opportunities
            .iter()
            .filter_map(|(n, k)| (*k == BreakOpportunity::Mandatory).then_some(*n))
            .collect::<Vec<_>>();
        let mut emergency = settings.apply(text, &boundaries, &mut allowed, &mandatory)?;
        // A tabbed hard line is one positioned row. Allowing the Unicode BA
        // opportunity after U+0009 (or a later word break inside its field)
        // would move the continuation to inline origin zero and silently lose
        // the approved stop geometry. Keep the whole row together; callers can
        // widen it, change its stops or insert an explicit hard separator.
        for line in super::hard_break::logical_lines(text) {
            let line = line?;
            if !text[line.visible.clone()].contains('\t') {
                continue;
            }
            allowed.retain(|offset| *offset <= line.logical.start || *offset >= line.logical.end);
            emergency.retain(|offset| *offset <= line.logical.start || *offset >= line.logical.end);
        }
        Ok(Self {
            text,
            bidi,
            boundaries,
            allowed,
            emergency,
            mandatory,
            composition: settings.composition,
        })
    }
    pub fn break_lines(
        &self,
        font: &[u8],
        from: usize,
        font_size: f64,
        width: f64,
        settings: &OpenTypeSettings,
        max_lines: usize,
    ) -> Result<Vec<MeasuredLine>> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return Err(WellfriendError::invalid_input("invalid line font size"));
        }
        self.break_lines_measured(from, width, max_lines, |range| {
            let visible =
                self.text[range.clone()].trim_end_matches(crate::fonts::hard_break::is_hard_break);
            let bidi = self.bidi.line(range.start..range.start + visible.len())?;
            let run = TextShaper::shape_resolved(font, visible, &bidi, settings)?;
            Ok(measure_run(font, &run, font_size)?.width())
        })
    }

    /// One index/line-break algorithm for single and mixed-font measurement.
    pub fn break_lines_measured(
        &self,
        from: usize,
        width: f64,
        max_lines: usize,
        measure_range: impl Fn(Range<usize>) -> Result<f64>,
    ) -> Result<Vec<MeasuredLine>> {
        let batch = self.break_lines_measured_prefix(from, width, max_lines, measure_range)?;
        match batch.blocked {
            Some(blocked) => Err(blocked.into_error()),
            None => Ok(batch.lines),
        }
    }

    /// Retain a fitting prefix so a paginator can continue in a differently
    /// sized approved frame. No font, cancellation, validation or resource error
    /// is converted into a geometric block or a successful partial result.
    pub fn break_lines_measured_prefix(
        &self,
        from: usize,
        width: f64,
        max_lines: usize,
        measure_range: impl Fn(Range<usize>) -> Result<f64>,
    ) -> Result<LineBreakBatch> {
        if max_lines == 0 || max_lines > 100_000 || !width.is_finite() || width <= 0.0 {
            return Err(WellfriendError::invalid_input(
                "invalid line layout dimensions/budget",
            ));
        }
        if self.composition == LineComposition::Balanced {
            return balanced::compose(self, from, width, max_lines, measure_range);
        }
        let text = self.text;
        let boundaries = &self.boundaries;
        let allowed = &self.allowed;
        let mandatory = &self.mandatory;
        let mut lines = Vec::new();
        let mut start = boundaries
            .binary_search(&from)
            .map_err(|_| WellfriendError::invalid_input("line start divides a grapheme"))?;
        while start + 1 < boundaries.len() {
            crate::cancel::check_current_cancel("logical line breaking")?;
            if lines.len() >= 100_000 {
                return Err(WellfriendError::ResourceLimit(
                    "line count exceeds 100000".into(),
                ));
            }
            if lines.len() >= max_lines {
                break;
            }
            let stop = mandatory
                .get(mandatory.partition_point(|n| *n <= boundaries[start]))
                .copied()
                .unwrap_or(text.len());
            let limit = boundaries.partition_point(|n| *n <= stop).saturating_sub(1);
            let measure = |end: usize| {
                let measured = measure_range(boundaries[start]..boundaries[end])?;
                if !measured.is_finite() || measured < 0.0 {
                    return Err(WellfriendError::invalid_input(
                        "non-finite or negative shaped line width",
                    ));
                }
                Ok(measured)
            };
            // Exponential probe followed by binary refinement bounds shape calls
            // even for a million-character paragraph. Every chosen line is measured
            // again; kerning cannot cause an unverified over-width result.
            let mut fit = start;
            let mut probe = (start + 1).min(limit);
            loop {
                if measure(probe)? > width + 1e-7 {
                    break;
                }
                fit = probe;
                if fit == limit {
                    break;
                }
                probe = (start + (probe - start).saturating_mul(2)).min(limit);
            }
            let mut high = probe;
            while fit + 1 < high {
                let middle = fit + (high - fit) / 2;
                if measure(middle)? <= width + 1e-7 {
                    fit = middle;
                } else {
                    high = middle;
                }
            }
            if fit == start {
                return Ok(LineBreakBatch {
                    lines,
                    blocked: Some(LineBlock {
                        byte_start: boundaries[start],
                        reason: LineBlockReason::GraphemeTooWide,
                    }),
                });
            }
            let end = if fit == limit {
                fit
            } else {
                let range = (
                    std::ops::Bound::Excluded(boundaries[start]),
                    std::ops::Bound::Included(boundaries[fit]),
                );
                let offset = allowed.range(range).next_back().or_else(|| {
                    self.emergency
                        .get(
                            self.emergency
                                .partition_point(|n| *n <= boundaries[fit])
                                .checked_sub(1)?,
                        )
                        .filter(|n| **n > boundaries[start])
                });
                let Some(offset) = offset else {
                    return Ok(LineBreakBatch {
                        lines,
                        blocked: Some(LineBlock {
                            byte_start: boundaries[start],
                            reason: LineBlockReason::ProtectedSequenceTooWide,
                        }),
                    });
                };
                boundaries.binary_search(offset).map_err(|_| {
                    WellfriendError::invalid_input("line-break opportunity divides a grapheme")
                })?
            };
            let advance = measure(end)?;
            if advance > width + 1e-7 {
                return Ok(LineBreakBatch {
                    lines,
                    blocked: Some(LineBlock {
                        byte_start: boundaries[start],
                        reason: LineBlockReason::ChosenLineTooWide,
                    }),
                });
            }
            lines.push(MeasuredLine {
                bytes: boundaries[start]..boundaries[end],
                width: advance,
            });
            start = end;
        }
        Ok(LineBreakBatch {
            lines,
            blocked: None,
        })
    }
}

#[path = "line_composition.rs"]
mod balanced;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabbed_hard_line_never_wraps_a_field_back_to_inline_origin_zero() {
        let text = "A\tB C";
        let paragraph = PreparedParagraph::new(text, ShapeOptions::default()).unwrap();
        let blocked = paragraph
            .break_lines_measured_prefix(0, 3.0, 10, |range| Ok(range.len() as f64))
            .unwrap();
        assert!(blocked.lines.is_empty());
        assert_eq!(
            blocked.blocked,
            Some(LineBlock {
                byte_start: 0,
                reason: LineBlockReason::ProtectedSequenceTooWide,
            })
        );
        let fitting = paragraph
            .break_lines_measured(0, text.len() as f64, 10, |range| Ok(range.len() as f64))
            .unwrap();
        assert_eq!(fitting.len(), 1);
        assert_eq!(fitting[0].bytes, 0..text.len());
    }

    #[test]
    fn line_slices_preserve_logical_text_and_fit() {
        let font = crate::render::get_fallback_font("Symbol").unwrap();
        let text = "office AV e\u{301} שלום 123\nnext paragraph";
        let lines = break_lines(font, text, 12.0, 75.0, ShapeOptions::default()).unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|l| &text[l.bytes.clone()])
                .collect::<String>(),
            text
        );
        assert!(lines.iter().all(|l| l.width <= 75.0 + 1e-7));
    }

    #[test]
    fn request_local_prepared_metrics_match_standalone_measurement() {
        let font = crate::render::get_fallback_font("Symbol").unwrap();
        let text = "office AV e\u{301}";
        let shaped = TextShaper::shape(font, text, ShapeOptions::default()).unwrap();
        let standalone = measure_run(font, &shaped, 13.0).unwrap();
        let prepared = PreparedFontMetrics::new(font).unwrap();
        let reused = prepared.measure(&shaped, 13.0).unwrap();
        assert_eq!(standalone.advance, reused.advance);
        assert_eq!(standalone.left_pad, reused.left_pad);
        assert_eq!(standalone.right_pad, reused.right_pad);
        assert_eq!(standalone.ascent, reused.ascent);
        assert_eq!(standalone.descent, reused.descent);
        assert_eq!(
            prepared.bounds(shaped.glyphs[0].glyph_id).unwrap(),
            ttf_parser::Face::parse(font, 0)
                .unwrap()
                .glyph_bounding_box(ttf_parser::GlyphId(shaped.glyphs[0].glyph_id))
        );
    }
}
