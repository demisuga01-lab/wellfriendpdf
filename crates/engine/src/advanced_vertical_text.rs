//! Shared vertical glyph/layout/emission path for generated PDF source text.
use super::*;
use crate::fonts::shaper::{LineBidi, OpenTypeSettings};
#[cfg(test)]
#[path = "advanced_vertical_text_tests.rs"]
mod tests;

pub(super) fn glyph_plan(
    text: &str,
    font: &[u8],
    bidi: Option<&LineBidi>,
) -> Result<Vec<GeneratedGlyph>> {
    let shaped = match bidi {
        Some(bidi) => {
            crate::fonts::vertical::shape_resolved(font, text, bidi, &OpenTypeSettings::default())?
        }
        None => crate::fonts::vertical::shape(font, text, &OpenTypeSettings::default())?,
    };
    let run = crate::fonts::ShapedRun {
        glyphs: shaped.iter().map(|g| g.glyph.clone()).collect(),
        direction: TextDirection::LeftToRight,
        used_complex_shaping: !text.is_empty(),
    };
    let mut result = generated_glyphs_from_shaped(text, font, run)?;
    for (glyph, source) in result.iter_mut().zip(shaped) {
        glyph.orientation = if source.rotate_clockwise {
            VerticalGlyphOrientation::RotateClockwise
        } else if source.vertical_alternate {
            VerticalGlyphOrientation::FontVerticalAlternate
        } else {
            VerticalGlyphOrientation::Upright
        };
        glyph.cross_advance = source.cross_advance;
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Style {
    size: f64,
    hscale: f64,
    rise: f64,
    char_space: f64,
    word_space: f64,
    vertical_source: bool,
}
fn style(glyph: &GeneratedGlyph, size: f64, spans: Option<&[PreservedStyleSpan]>) -> Result<Style> {
    let result = match spans {
        Some(spans) => {
            let s = generated_style_for_offset(spans, glyph.logical_byte_start)?;
            Style {
                size: s.font_size,
                hscale: s.horizontal_scaling / 100.0,
                rise: s.text_rise,
                char_space: s.character_spacing,
                word_space: s.word_spacing,
                vertical_source: s.vertical,
            }
        }
        None => Style {
            size,
            hscale: 1.0,
            rise: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            vertical_source: true,
        },
    };
    if ![
        result.size,
        result.hscale,
        result.rise,
        result.char_space,
        result.word_space,
    ]
    .iter()
    .all(|v| v.is_finite())
        || result.size <= 0.0
    {
        return Err(WellfriendError::invalid_input(
            "invalid vertical text style",
        ));
    }
    Ok(result)
}
fn rotated(glyph: &GeneratedGlyph) -> bool {
    glyph.orientation == VerticalGlyphOrientation::RotateClockwise
}
fn advance(glyph: &GeneratedGlyph, s: Style, cluster_end: bool) -> Result<f64> {
    let spacing = if cluster_end {
        s.char_space
            + if glyph.visual_unicode == " " {
                s.word_space
            } else {
                0.0
            }
    } else {
        0.0
    };
    let value = glyph.advance * s.size / 1000.0 * if rotated(glyph) { s.hscale } else { 1.0 }
        + if s.vertical_source { -spacing } else { spacing };
    if !value.is_finite() {
        return Err(WellfriendError::UnsupportedFeature(
            "non-finite generated vertical advance".into(),
        ));
    }
    Ok(value)
}
#[derive(Default)]
struct Metrics {
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
}
impl Metrics {
    fn height(&self) -> f64 {
        self.bottom - self.top
    }
    fn width(&self) -> f64 {
        self.right - self.left
    }
}
fn metrics(
    glyphs: &[GeneratedGlyph],
    size: f64,
    spans: Option<&[PreservedStyleSpan]>,
) -> Result<Metrics> {
    let mut result = Metrics {
        left: -size / 2.0,
        right: size / 2.0,
        ..Default::default()
    };
    let mut down = 0.0;
    let mut cross = 0.0;
    for (index, glyph) in glyphs.iter().enumerate() {
        crate::cancel::check_current_cancel("vertical ink bounds")?;
        let s = style(glyph, size, spans)?;
        let scale = s.size / 1000.0;
        result.left = result.left.min(-s.size / 2.0);
        result.right = result.right.max(s.size / 2.0);
        if let Some(bounds) = glyph.bounds {
            for x in [bounds[0], bounds[2]] {
                for y in [bounds[1], bounds[3]] {
                    let local_x = (glyph.offset_x + x) * scale * s.hscale;
                    let local_y = (glyph.offset_y + y) * scale;
                    let (x, y) = if rotated(glyph) {
                        (cross + local_y, down + local_x - s.rise)
                    } else {
                        (cross + local_x, down - local_y - s.rise)
                    };
                    result.left = result.left.min(x);
                    result.right = result.right.max(x);
                    result.top = result.top.min(y);
                    result.bottom = result.bottom.max(y);
                }
            }
        }
        let cluster_end = glyphs
            .get(index + 1)
            .is_none_or(|next| next.logical_byte_start != glyph.logical_byte_start);
        down += advance(glyph, s, cluster_end)?;
        cross += glyph.cross_advance * scale * s.hscale;
        result.top = result.top.min(down);
        result.bottom = result.bottom.max(down);
    }
    Ok(result)
}
pub(super) fn extent(glyphs: &[GeneratedGlyph], size: f64) -> Result<f64> {
    Ok(metrics(glyphs, size, None)?.height())
}

pub(super) fn layout(
    text: &str,
    font: &[u8],
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    let prepared = crate::fonts::line_layout::PreparedParagraph::new(
        text,
        ShapeOptions {
            direction: Some(TextDirection::LeftToRight),
        },
    )?;
    let lines = prepared.break_lines_measured(
        0,
        options.region[3] - options.region[1],
        options.max_lines_or_columns + 1,
        |range| {
            let visible =
                text[range.clone()].trim_end_matches(crate::fonts::hard_break::is_hard_break);
            let bidi = prepared
                .bidi
                .line(range.start..range.start + visible.len())?;
            extent(&glyph_plan(visible, font, Some(&bidi))?, options.font_size)
        },
    )?;
    if lines.len() > options.max_lines_or_columns
        || lines.last().is_some_and(|l| l.bytes.end != text.len())
    {
        return Err(WellfriendError::UnsupportedFeature(
            "vertical text exceeds the declared column budget".into(),
        ));
    }
    let explicit = lines
        .iter()
        .map(|line| {
            let logical_text = text[line.bytes.clone()].to_owned();
            let visual_text = logical_text
                .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                .to_owned();
            Ok(ExplicitLayoutLine {
                bidi: Some(
                    prepared
                        .bidi
                        .line(line.bytes.start..line.bytes.start + visual_text.len())?,
                ),
                logical_text,
                visual_text,
                inserted_visual_hyphen: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut layout = layout_generated_explicit_lines(
        &explicit,
        AdvancedTextMode::ParagraphReflowVertical,
        font,
        options,
        None,
    )?;
    let mut line_byte_base = 0usize;
    for (line, glyphs) in explicit.iter().zip(layout.iter_mut()) {
        for glyph in glyphs {
            glyph.logical_byte_start = glyph.logical_byte_start.saturating_add(line_byte_base);
        }
        line_byte_base = line_byte_base
            .checked_add(line.logical_text.len())
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("vertical partition byte offset".into())
            })?;
    }
    if line_byte_base != text.len() {
        return Err(WellfriendError::MalformedPdf(
            "vertical partition layout byte coverage drifted".into(),
        ));
    }
    Ok(layout)
}

/// Break one paint partition into vertical columns while retaining the bidi
/// levels and non-emitting shaping neighbours of the complete replacement.
/// The local paragraph owns only break opportunities; every width probe and
/// final column is shaped against `full_text` through its global byte range.
pub(super) fn layout_with_context(
    full_text: &str,
    segment: std::ops::Range<usize>,
    font: &[u8],
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    let text = full_text.get(segment.clone()).ok_or_else(|| {
        WellfriendError::invalid_input("vertical paint partition is outside replacement text")
    })?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let shape_options = ShapeOptions {
        direction: Some(TextDirection::LeftToRight),
    };
    let local = crate::fonts::line_layout::PreparedParagraph::new(text, shape_options)?;
    let paragraph_bidi = crate::fonts::shaper::ParagraphBidi::new(full_text, shape_options)?;
    let lines = local.break_lines_measured(
        0,
        options.region[3] - options.region[1],
        options.max_lines_or_columns + 1,
        |range| {
            let visible =
                text[range.clone()].trim_end_matches(crate::fonts::hard_break::is_hard_break);
            let global_start = segment.start + range.start;
            let bidi = paragraph_bidi.line(global_start..global_start + visible.len())?;
            extent(&glyph_plan(visible, font, Some(&bidi))?, options.font_size)
        },
    )?;
    if lines.len() > options.max_lines_or_columns
        || lines
            .last()
            .is_some_and(|line| line.bytes.end != text.len())
    {
        return Err(WellfriendError::UnsupportedFeature(
            "vertical paint partition exceeds the declared column budget".into(),
        ));
    }
    let explicit = lines
        .iter()
        .map(|line| {
            let logical_text = text[line.bytes.clone()].to_owned();
            let visual_text = logical_text
                .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                .to_owned();
            let global_start = segment.start + line.bytes.start;
            Ok(ExplicitLayoutLine {
                bidi: Some(paragraph_bidi.line(global_start..global_start + visual_text.len())?),
                logical_text,
                visual_text,
                inserted_visual_hyphen: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut layout = layout_generated_explicit_lines(
        &explicit,
        AdvancedTextMode::ParagraphReflowVertical,
        font,
        options,
        None,
    )?;
    let mut line_byte_base = 0usize;
    for (line, glyphs) in explicit.iter().zip(layout.iter_mut()) {
        for glyph in glyphs {
            glyph.logical_byte_start = glyph.logical_byte_start.saturating_add(line_byte_base);
        }
        line_byte_base = line_byte_base
            .checked_add(line.logical_text.len())
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("vertical contextual partition byte offset".into())
            })?;
    }
    if line_byte_base != text.len() {
        return Err(WellfriendError::MalformedPdf(
            "vertical contextual partition layout byte coverage drifted".into(),
        ));
    }
    Ok(layout)
}

/// Paint within a caller-owned BT/ET. W2's origins are zero: the glyph Tm
/// supplies the actual horizontal-outline origin from shaping exactly once.
/// All gaps are applied to explicit positions; they never split a cluster.
pub(super) fn serialize(
    content: &mut String,
    layout: &[Vec<GeneratedGlyph>],
    font: &str,
    options: &AdvancedTextEditOptions,
    regions: Option<&[[f64; 4]]>,
    spans: Option<&[PreservedStyleSpan]>,
) -> Result<Vec<GeneratedLineAdjustment>> {
    if regions.is_some_and(|r| r.len() != layout.len()) {
        return Err(WellfriendError::invalid_input(
            "vertical region count differs from columns",
        ));
    }
    if ![options.max_word_spacing, options.max_character_spacing]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0)
    {
        return Err(WellfriendError::invalid_input(
            "invalid vertical spacing limits",
        ));
    }
    let mut right = options.region[2];
    let mut reports = Vec::new();
    for (column, glyphs) in layout.iter().enumerate() {
        crate::cancel::check_current_cancel("vertical column emission")?;
        let region = regions.map(|r| r[column]).unwrap_or(options.region);
        let bounds = metrics(glyphs, options.font_size, spans)?;
        if regions.is_some() {
            right = region[2];
        }
        let width = bounds.width();
        let available = region[3] - region[1];
        let natural = bounds.height();
        if !natural.is_finite()
            || natural > available + EPSILON
            || right - width < region[0] - EPSILON
        {
            return Err(WellfriendError::UnsupportedFeature(
                "vertical glyph outline or column placement exceeds the source region".into(),
            ));
        }
        let mut extra = vec![0.0; glyphs.len()];
        let mut word_extra = 0.0;
        let mut char_extra = 0.0;
        let gaps = (0..glyphs.len().saturating_sub(1))
            .filter(|&i| glyphs[i].logical_byte_start != glyphs[i + 1].logical_byte_start)
            .collect::<Vec<_>>();
        let words = gaps
            .iter()
            .copied()
            .filter(|&i| glyphs[i].visual_unicode == " ")
            .collect::<Vec<_>>();
        let last = column + 1 == layout.len();
        let mut residual = (available - natural).max(0.0);
        if options.alignment == GeneratedTextAlignment::Justify
            && (!last || options.justify_last_line)
            && !glyphs.is_empty()
        {
            if !words.is_empty() {
                word_extra = (residual / words.len() as f64)
                    .min(options.max_word_spacing * options.font_size);
                for &i in &words {
                    extra[i] += word_extra;
                }
                residual -= word_extra * words.len() as f64;
            }
            if !gaps.is_empty() {
                char_extra = (residual / gaps.len() as f64)
                    .min(options.max_character_spacing * options.font_size);
                for &i in &gaps {
                    extra[i] += char_extra;
                }
                residual -= char_extra * gaps.len() as f64;
            }
            if residual > EPSILON {
                return Err(WellfriendError::UnsupportedFeature(
                    "vertical full justification exceeds configured spacing limits".into(),
                ));
            }
        }
        let painted = natural + extra.iter().sum::<f64>();
        let padding = match options.alignment {
            GeneratedTextAlignment::Right | GeneratedTextAlignment::End => {
                (available - painted).max(0.0)
            }
            GeneratedTextAlignment::Center => (available - painted).max(0.0) / 2.0,
            _ => 0.0,
        };
        let x = right - bounds.right;
        let y = region[3] + bounds.top - padding;
        let mut down = 0.0;
        let mut cross = 0.0;
        for (index, glyph) in glyphs.iter().enumerate() {
            let s = style(glyph, options.font_size, spans)?;
            let scale = s.size / 1000.0;
            if let Some(spans) = spans {
                append_generated_preserved_style(
                    content,
                    font,
                    generated_style_for_offset(spans, glyph.logical_byte_start)?,
                );
            }
            let ox = glyph.offset_x * scale * s.hscale;
            let oy = glyph.offset_y * scale;
            let (a, b, c, d, gx, gy) = if rotated(glyph) {
                (0.0, -1.0, 1.0, 0.0, x + cross + oy, y - down - ox + s.rise)
            } else {
                (1.0, 0.0, 0.0, 1.0, x + cross + ox, y - down + oy + s.rise)
            };
            content.push_str("0 Ts\n");
            let mut marked = Vec::new();
            inline_text::append_glyph_marker(
                &mut marked,
                [1.0, 0.0, 0.0, 1.0, x + cross, y - down + s.rise],
                [a, b, c, d, gx, gy],
                glyph.cid,
            )?;
            content.push_str(std::str::from_utf8(&marked).map_err(|_| {
                WellfriendError::MalformedPdf("generated glyph marker is not ASCII".into())
            })?);
            let cluster_end = glyphs
                .get(index + 1)
                .is_none_or(|next| next.logical_byte_start != glyph.logical_byte_start);
            down += advance(glyph, s, cluster_end)? + extra[index];
            cross += glyph.cross_advance * scale * s.hscale;
        }
        reports.push(GeneratedLineAdjustment {
            line_index: column,
            natural_width: natural,
            target_width: available,
            residual: residual.max(0.0),
            word_spacing: word_extra / options.font_size,
            character_spacing: char_extra / options.font_size,
            alignment: options.alignment,
            last_line: last,
            applied: true,
            refusal_reason: None,
        });
        right -= width * options.line_spacing;
    }
    Ok(reports)
}
