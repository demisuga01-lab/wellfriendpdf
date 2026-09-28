//! Multi-font vertical shaping shared by story measurement and final emission.
use super::{
    fallback,
    shaper::{LineBidi, OpenTypeSettings},
    vertical::{self, Orientation, VerticalGlyph},
    ShapeOptions, ShapedRun, TextDirection, WritingMode,
};
use crate::{editing_transactions::ApprovedFontAsset, Result, WellfriendError};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
#[cfg(test)]
#[path = "vertical_fonts_tests.rs"]
mod tests;

pub struct FontRun {
    pub range: Range<usize>,
    pub font_index: usize,
    /// Clusters are local to range, like horizontal fallback FontRun.
    pub glyphs: Vec<VerticalGlyph>,
}
pub struct StyledFontRun {
    pub range: Range<usize>,
    pub font_index: usize,
    pub style_index: usize,
    pub glyphs: Vec<VerticalGlyph>,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
fn check_coverage(font: &[u8], text: &str, glyphs: &[VerticalGlyph]) -> Result<bool> {
    super::shaper::has_missing_glyphs(
        font,
        text,
        &ShapedRun {
            glyphs: glyphs.iter().map(|g| g.glyph.clone()).collect(),
            direction: TextDirection::LeftToRight,
            used_complex_shaping: true,
        },
    )
    .map(|missing| !missing)
}

pub fn covers(
    font: &[u8],
    text: &str,
    options: ShapeOptions,
    settings: &OpenTypeSettings,
) -> Result<bool> {
    let prepared = super::shaper::ParagraphBidi::new(text, options)?;
    prepared.all_hard_lines(|range, bidi| {
        crate::cancel::check_current_cancel("vertical font coverage")?;
        let visible = &text[range];
        let glyphs = vertical::shape_resolved(font, visible, &bidi, settings)?;
        check_coverage(font, visible, &glyphs)
    })
}

/// Upright orientation units retain logical top-to-bottom order. Sideways
/// orientation units use the horizontal multi-font bidi implementation as one
/// unit, so changing fonts cannot reverse each word independently.
pub fn shape_line(
    text: &str,
    bidi: &LineBidi,
    spans: &[fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &OpenTypeSettings,
) -> Result<Vec<FontRun>> {
    shape_line_impl(text, bidi, spans, fonts, settings, None)
}

pub(crate) fn shape_line_prepared(
    text: &str,
    bidi: &LineBidi,
    spans: &[fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &OpenTypeSettings,
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
) -> Result<Vec<FontRun>> {
    shape_line_impl(text, bidi, spans, fonts, settings, Some(metrics))
}

fn slice_styled_spans(
    spans: &[fallback::StyledFontSpan],
    range: Range<usize>,
) -> Vec<fallback::StyledFontSpan> {
    let from = spans.partition_point(|span| span.range[1] <= range.start);
    spans[from..]
        .iter()
        .take_while(|span| span.range[0] < range.end)
        .filter_map(|span| {
            let start = span.range[0].max(range.start);
            let end = span.range[1].min(range.end);
            (start < end).then_some(fallback::StyledFontSpan {
                range: [start - range.start, end - range.start],
                font_index: span.font_index,
                style_index: span.style_index,
            })
        })
        .collect()
}

/// Vertical itemization with one exact font/style partition. Sideways units
/// still receive one UAX #9 visual ordering pass, while upright units retain
/// logical top-to-bottom order. Every shaping call keeps paragraph neighbours.
pub(crate) fn shape_styled_line_prepared(
    text: &str,
    bidi: &LineBidi,
    spans: &[fallback::StyledFontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &[OpenTypeSettings],
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
) -> Result<Vec<StyledFontRun>> {
    bidi.context.validate()?;
    if text.chars().any(super::hard_break::is_hard_break)
        || bidi.levels.len() != text.len()
        || text.len() > 4_000_000
        || settings.len() > 100_000
    {
        return Err(fail("invalid styled vertical line context or budget"));
    }
    let boundaries = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut end = 0usize;
    for span in spans {
        if span.range[0] != end
            || span.range[0] >= span.range[1]
            || !boundaries.contains(&span.range[0])
            || !boundaries.contains(&span.range[1])
            || span.font_index >= fonts.len()
            || span.style_index >= settings.len()
        {
            return Err(fail(
                "styled vertical spans divide a grapheme or do not partition the line",
            ));
        }
        end = span.range[1];
    }
    if end != text.len() {
        return Err(fail("styled vertical spans do not cover the line"));
    }
    let mut orientations: Vec<(Range<usize>, bool)> = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        let sideways =
            vertical::orientation(grapheme.chars().next().unwrap()) == Orientation::Rotated;
        if let Some((range, previous)) = orientations.last_mut() {
            if *previous == sideways {
                range.end = start + grapheme.len();
                continue;
            }
        }
        orientations.push((start..start + grapheme.len(), sideways));
    }
    let mut result = Vec::new();
    for (range, sideways) in orientations {
        crate::cancel::check_current_cancel("styled vertical font run shaping")?;
        let sliced = slice_styled_spans(spans, range.clone());
        let local = bidi.slice(text, range.clone(), bidi.rtl)?;
        if sideways {
            for run in
                fallback::shape_styled_line(&text[range.clone()], &local, &sliced, fonts, settings)?
            {
                let prepared = metrics
                    .get(run.font_index)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| fail("styled vertical font metrics are not prepared"))?;
                let center = (f64::from(prepared.ascender()) + f64::from(prepared.descender()))
                    / f64::from(prepared.units_per_em()).max(1.0)
                    * 500.0;
                let glyphs = run
                    .shaped
                    .glyphs
                    .into_iter()
                    .map(|mut glyph| {
                        glyph.offset_y -= center;
                        VerticalGlyph {
                            glyph,
                            rotate_clockwise: true,
                            vertical_alternate: false,
                            cross_advance: 0.0,
                        }
                    })
                    .collect();
                result.push(StyledFontRun {
                    range: range.start + run.range.start..range.start + run.range.end,
                    font_index: run.font_index,
                    style_index: run.style_index,
                    glyphs,
                });
            }
        } else {
            for span in sliced {
                let start = range.start + span.range[0];
                let end = range.start + span.range[1];
                let resolved = bidi.slice(text, start..end, bidi.rtl)?;
                let glyphs = vertical::shape_resolved(
                    &fonts[span.font_index].bytes,
                    &text[start..end],
                    &resolved,
                    &settings[span.style_index],
                )?;
                if !check_coverage(&fonts[span.font_index].bytes, &text[start..end], &glyphs)? {
                    return Err(fail("styled vertical line lost glyph coverage"));
                }
                result.push(StyledFontRun {
                    range: start..end,
                    font_index: span.font_index,
                    style_index: span.style_index,
                    glyphs,
                });
            }
        }
    }
    Ok(result)
}

fn shape_line_impl(
    text: &str,
    bidi: &LineBidi,
    spans: &[fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &OpenTypeSettings,
    metrics: Option<&[Option<super::line_layout::PreparedFontMetrics<'_>>]>,
) -> Result<Vec<FontRun>> {
    bidi.context.validate()?;
    if text.chars().any(super::hard_break::is_hard_break) {
        return Err(fail("vertical font runs expect one visible logical line"));
    }
    if bidi.levels.len() != text.len() || text.len() > 4_000_000 {
        return Err(fail("vertical fallback text/bidi budget"));
    }
    let boundaries = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut end = 0;
    for span in spans {
        if span.range[0] != end
            || span.range[0] >= span.range[1]
            || !boundaries.contains(&span.range[0])
            || !boundaries.contains(&span.range[1])
            || span.font_index >= fonts.len()
        {
            return Err(fail(
                "vertical fallback font spans divide a grapheme or do not partition the line",
            ));
        }
        end = span.range[1];
    }
    if end != text.len() {
        return Err(fail("vertical fallback font spans do not cover the line"));
    }
    let mut segments: Vec<(Range<usize>, bool)> = Vec::new();
    for (start, g) in text.grapheme_indices(true) {
        let sideways = vertical::orientation(g.chars().next().unwrap()) == Orientation::Rotated;
        if let Some((range, previous)) = segments.last_mut() {
            if *previous == sideways {
                range.end = start + g.len();
                continue;
            }
        }
        segments.push((start..start + g.len(), sideways));
    }
    let mut result = Vec::new();
    for (range, sideways) in segments {
        crate::cancel::check_current_cancel("vertical font run shaping")?;
        let sliced = fallback::slice_spans(spans, range.clone());
        let local = bidi.slice(text, range.clone(), bidi.rtl)?;
        if sideways {
            for run in fallback::shape_line(&text[range.clone()], &local, &sliced, fonts, settings)?
            {
                let center = if let Some(prepared) = metrics
                    .and_then(|metrics| metrics.get(run.font_index))
                    .and_then(Option::as_ref)
                {
                    (f64::from(prepared.ascender()) + f64::from(prepared.descender()))
                        / f64::from(prepared.units_per_em()).max(1.0)
                        * 500.0
                } else {
                    let face = ttf_parser::Face::parse(&fonts[run.font_index].bytes, 0)
                        .map_err(|_| fail("invalid vertical fallback font"))?;
                    (f64::from(face.ascender()) + f64::from(face.descender()))
                        / f64::from(face.units_per_em()).max(1.0)
                        * 500.0
                };
                let glyphs = run
                    .shaped
                    .glyphs
                    .into_iter()
                    .map(|mut glyph| {
                        glyph.offset_y -= center;
                        VerticalGlyph {
                            glyph,
                            rotate_clockwise: true,
                            vertical_alternate: false,
                            cross_advance: 0.0,
                        }
                    })
                    .collect();
                result.push(FontRun {
                    range: range.start + run.range.start..range.start + run.range.end,
                    font_index: run.font_index,
                    glyphs,
                });
            }
        } else {
            for span in sliced {
                let start = range.start + span.range[0];
                let end = range.start + span.range[1];
                let resolved = bidi.slice(text, start..end, bidi.rtl)?;
                let glyphs = vertical::shape_resolved(
                    &fonts[span.font_index].bytes,
                    &text[start..end],
                    &resolved,
                    settings,
                )?;
                if !check_coverage(&fonts[span.font_index].bytes, &text[start..end], &glyphs)? {
                    return Err(fail("vertical fallback line lost glyph coverage"));
                }
                result.push(FontRun {
                    range: start..end,
                    font_index: span.font_index,
                    glyphs,
                });
            }
        }
    }
    Ok(result)
}

/// Return metrics in the paginator's inline/block axes. Shape offsets are
/// physical x/y; neither vertical-lr nor vertical-rl mirrors the glyph outlines.
pub fn measure_line(
    runs: &[FontRun],
    fonts: &[ApprovedFontAsset],
    size: f64,
    mode: WritingMode,
) -> Result<super::line_layout::LineMetrics> {
    measure_line_impl(runs, fonts, None, size, mode)
}

pub(crate) fn measure_line_prepared(
    runs: &[FontRun],
    fonts: &[ApprovedFontAsset],
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
    size: f64,
    mode: WritingMode,
) -> Result<super::line_layout::LineMetrics> {
    measure_line_impl(runs, fonts, Some(metrics), size, mode)
}

pub(crate) fn measure_styled_line_prepared(
    runs: &[StyledFontRun],
    fonts: &[ApprovedFontAsset],
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
    sizes: &[f64],
    mode: WritingMode,
) -> Result<super::line_layout::LineMetrics> {
    if !mode.is_vertical()
        || sizes.is_empty()
        || sizes.iter().any(|size| !size.is_finite() || *size <= 0.0)
    {
        return Err(fail("invalid styled vertical story metrics"));
    }
    let maximum_size = sizes.iter().copied().fold(0.0, f64::max);
    let mut down = 0.0f64;
    let mut cross = 0.0f64;
    let mut top = 0.0f64;
    let mut bottom = 0.0f64;
    let mut left = -maximum_size / 2.0;
    let mut right = maximum_size / 2.0;
    for run in runs {
        crate::cancel::check_current_cancel("styled vertical story ink measurement")?;
        if fonts.get(run.font_index).is_none() {
            return Err(fail("styled vertical font index"));
        }
        let prepared = metrics
            .get(run.font_index)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("styled vertical font metrics are not prepared"))?;
        let size = sizes
            .get(run.style_index)
            .copied()
            .ok_or_else(|| fail("styled vertical size index"))?;
        let unit = size / f64::from(prepared.units_per_em()).max(1.0);
        for glyph in &run.glyphs {
            let bounds = prepared.bounds(glyph.glyph.glyph_id)?;
            if let Some(bounds) = bounds {
                for x in [bounds.x_min, bounds.x_max] {
                    for y in [bounds.y_min, bounds.y_max] {
                        let x = f64::from(x) * unit + glyph.glyph.offset_x * size / 1000.0;
                        let y = f64::from(y) * unit + glyph.glyph.offset_y * size / 1000.0;
                        let (x, progression) = if glyph.rotate_clockwise {
                            (cross + y, down + x)
                        } else {
                            (cross + x, down - y)
                        };
                        left = left.min(x);
                        right = right.max(x);
                        top = top.min(progression);
                        bottom = bottom.max(progression);
                    }
                }
            }
            down += glyph.glyph.advance * size / 1000.0;
            cross += glyph.cross_advance * size / 1000.0;
            top = top.min(down);
            bottom = bottom.max(down);
        }
    }
    if ![down, left, right, top, bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(fail("non-finite styled vertical story metrics"));
    }
    Ok(super::line_layout::LineMetrics {
        advance: down,
        left_pad: -top,
        right_pad: (bottom - down).max(0.0),
        ascent: if mode == WritingMode::VerticalRl {
            right.max(0.0)
        } else {
            (-left).max(0.0)
        },
        descent: if mode == WritingMode::VerticalRl {
            (-left).max(0.0)
        } else {
            right.max(0.0)
        },
    })
}

fn measure_line_impl(
    runs: &[FontRun],
    fonts: &[ApprovedFontAsset],
    metrics: Option<&[Option<super::line_layout::PreparedFontMetrics<'_>>]>,
    size: f64,
    mode: WritingMode,
) -> Result<super::line_layout::LineMetrics> {
    if !mode.is_vertical() || !size.is_finite() || size <= 0.0 {
        return Err(fail("invalid vertical story metrics"));
    }
    let mut down = 0.0f64;
    let mut cross = 0.0f64;
    let mut top = 0.0f64;
    let mut bottom = 0.0f64;
    let mut left = -size / 2.0;
    let mut right = size / 2.0;
    for run in runs {
        let font = fonts
            .get(run.font_index)
            .ok_or_else(|| fail("vertical font index"))?;
        let prepared = metrics
            .and_then(|metrics| metrics.get(run.font_index))
            .and_then(Option::as_ref);
        let fallback_face = if prepared.is_none() {
            Some(
                ttf_parser::Face::parse(&font.bytes, 0)
                    .map_err(|_| fail("invalid vertical font"))?,
            )
        } else {
            None
        };
        let fallback_outliner = fallback_face
            .as_ref()
            .map(super::sfnt_outline::Outliner::new)
            .transpose()?;
        let unit = size
            / f64::from(
                prepared
                    .map(|prepared| prepared.units_per_em())
                    .or_else(|| fallback_face.as_ref().map(|face| face.units_per_em()))
                    .ok_or_else(|| fail("vertical font metrics missing"))?,
            )
            .max(1.0);
        for g in &run.glyphs {
            crate::cancel::check_current_cancel("vertical story ink measurement")?;
            let bounds = if let Some(prepared) = prepared {
                prepared.bounds(g.glyph.glyph_id)?
            } else {
                fallback_outliner
                    .as_ref()
                    .ok_or_else(|| fail("vertical outline metrics missing"))?
                    .bounds(ttf_parser::GlyphId(g.glyph.glyph_id))?
            };
            if let Some(bounds) = bounds {
                for x in [bounds.x_min, bounds.x_max] {
                    for y in [bounds.y_min, bounds.y_max] {
                        let x = f64::from(x) * unit + g.glyph.offset_x * size / 1000.0;
                        let y = f64::from(y) * unit + g.glyph.offset_y * size / 1000.0;
                        let (x, d) = if g.rotate_clockwise {
                            (cross + y, down + x)
                        } else {
                            (cross + x, down - y)
                        };
                        left = left.min(x);
                        right = right.max(x);
                        top = top.min(d);
                        bottom = bottom.max(d);
                    }
                }
            }
            down += g.glyph.advance * size / 1000.0;
            cross += g.cross_advance * size / 1000.0;
            top = top.min(down);
            bottom = bottom.max(down);
        }
    }
    if ![down, left, right, top, bottom]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(fail("non-finite vertical story metrics"));
    }
    Ok(super::line_layout::LineMetrics {
        advance: down,
        left_pad: -top,
        right_pad: (bottom - down).max(0.0),
        ascent: if mode == WritingMode::VerticalRl {
            right.max(0.0)
        } else {
            (-left).max(0.0)
        },
        descent: if mode == WritingMode::VerticalRl {
            (-left).max(0.0)
        } else {
            right.max(0.0)
        },
    })
}
