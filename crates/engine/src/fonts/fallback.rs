//! Contextual multi-font fallback shared by line measurement and PDF emission.
//! Switch only at script/whitespace boundaries, never within a joining word,
//! Indic syllable or extended grapheme. Uncovered contextual units fail closed.
use super::shaper::{has_missing_glyphs, script_ranges, LineBidi, OpenTypeSettings};
use super::{ShapeOptions, ShapedRun, TextShaper};
use crate::editing_transactions::ApprovedFontAsset;
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FontSpan {
    pub range: [usize; 2],
    pub font_index: usize,
}
pub struct FontRun {
    pub range: Range<usize>,
    pub font_index: usize,
    pub shaped: ShapedRun,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StyledFontSpan {
    pub range: [usize; 2],
    pub font_index: usize,
    pub style_index: usize,
}
pub struct StyledFontRun {
    pub range: Range<usize>,
    pub font_index: usize,
    pub style_index: usize,
    pub shaped: ShapedRun,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}

pub fn editable_font(font: &[u8]) -> bool {
    let Ok(face) = ttf_parser::Face::parse(font, 0) else {
        return false;
    };
    !font.starts_with(b"ttcf")
        && (face.tables().glyf.is_some()
            || (font.starts_with(b"OTTO") && face.tables().cff.is_some()))
        && super::pdf_embedding::editable_outline_embedding_allowed(&face)
}

/// Bounded Viterbi assignment minimizes font switches plus ranked font cost.
/// Costs are explicit engineering preferences, not a visual-equivalence proof.
pub fn resolve_contextual_fonts(
    text: &str,
    fonts: &[ApprovedFontAsset],
    ranked: &[(usize, f64)],
    options: ShapeOptions,
    settings: &OpenTypeSettings,
) -> Result<Vec<FontSpan>> {
    resolve_contextual_fonts_for_mode(
        text,
        fonts,
        ranked,
        options,
        settings,
        super::WritingMode::HorizontalTb,
    )
}

pub fn resolve_contextual_fonts_for_mode(
    text: &str,
    fonts: &[ApprovedFontAsset],
    ranked: &[(usize, f64)],
    options: ShapeOptions,
    settings: &OpenTypeSettings,
    mode: super::WritingMode,
) -> Result<Vec<FontSpan>> {
    let programs = fonts
        .iter()
        .map(|font| font.bytes.as_slice())
        .collect::<Vec<_>>();
    resolve_contextual_programs(text, &programs, ranked, options, settings, mode)
}

/// Borrow exact font programs without cloning them into transaction assets.
/// Both the original public API and authoring use this same resolver.
pub(crate) fn resolve_contextual_programs(
    text: &str,
    fonts: &[&[u8]],
    ranked: &[(usize, f64)],
    options: ShapeOptions,
    settings: &OpenTypeSettings,
    mode: super::WritingMode,
) -> Result<Vec<FontSpan>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if ranked.is_empty() || ranked.len() > 256 || text.len() > 4_000_000 {
        return Err(fail("fallback font/text budget exceeded"));
    }
    let mut units = Vec::new();
    for (range, _) in script_ranges(text) {
        let mut start = range.start;
        // Consume whole whitespace graphemes with their preceding contextual
        // word. No split_word_bounds call that could divide emoji/ZWJ clusters.
        for (offset, grapheme) in text[range.clone()].grapheme_indices(true) {
            if grapheme.chars().all(char::is_whitespace) {
                let end = range.start + offset + grapheme.len();
                units.push(start..end);
                if units.len().saturating_mul(ranked.len()) > 1_000_000 {
                    return Err(fail("fallback assignment exceeds one million states"));
                }
                start = end;
            }
        }
        if start < range.end {
            units.push(start..range.end);
        }
    }
    if units.len().saturating_mul(ranked.len()) > 1_000_000 {
        return Err(fail("fallback assignment exceeds one million states"));
    }
    let mut costs = vec![0.0f64; ranked.len()];
    let mut parents = Vec::new();
    for unit in &units {
        crate::cancel::check_current_cancel("contextual font fallback")?;
        let mut next = vec![f64::INFINITY; ranked.len()];
        let mut prev = vec![0usize; ranked.len()];
        let best = costs
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        for (candidate, &(font_index, cost)) in ranked.iter().enumerate() {
            if !cost.is_finite() || cost < 0.0 {
                return Err(fail("invalid fallback font cost"));
            }
            let font = fonts
                .get(font_index)
                .ok_or_else(|| fail("fallback font index out of range"))?;
            if !editable_font(font) {
                continue;
            }
            let slice = &text[unit.clone()];
            let covered = if mode.is_vertical() {
                super::vertical_fonts::covers(font, slice, options, settings)?
            } else {
                let shaped = TextShaper::shape_with_settings(font, slice, options, settings)?;
                !has_missing_glyphs(font, slice, &shaped)?
            };
            if !covered {
                continue;
            }
            let predecessor = if costs[candidate] <= costs[best] + 0.25 {
                candidate
            } else {
                best
            };
            next[candidate] =
                costs[predecessor] + cost + if candidate != predecessor { 0.25 } else { 0.0 };
            prev[candidate] = predecessor;
        }
        if next.iter().all(|c| !c.is_finite()) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "no approved font covers complete contextual unit {}..{}",
                unit.start, unit.end
            )));
        }
        parents.push(prev);
        costs = next;
    }
    let mut current = costs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    let mut assigned = vec![0; units.len()];
    for i in (0..units.len()).rev() {
        assigned[i] = ranked[current].0;
        current = parents[i][current];
    }
    let mut result: Vec<FontSpan> = Vec::new();
    for (unit, index) in units.into_iter().zip(assigned) {
        if let Some(previous) = result.last_mut() {
            if previous.font_index == index && previous.range[1] == unit.start {
                previous.range[1] = unit.end;
                continue;
            }
        }
        result.push(FontSpan {
            range: [unit.start, unit.end],
            font_index: index,
        });
    }
    Ok(result)
}

pub fn slice_spans(spans: &[FontSpan], range: Range<usize>) -> Vec<FontSpan> {
    let from = spans.partition_point(|s| s.range[1] <= range.start);
    spans[from..]
        .iter()
        .take_while(|s| s.range[0] < range.end)
        .filter_map(|s| {
            let start = s.range[0].max(range.start);
            let end = s.range[1].min(range.end);
            (start < end).then_some(FontSpan {
                range: [start - range.start, end - range.start],
                font_index: s.font_index,
            })
        })
        .collect()
}

/// Intersect two complete logical partitions without changing their source
/// order. The style values live in the caller's model; this function binds only
/// their stable ordinal to the resolved font run used by shaping and emission.
pub fn intersect_styled_spans(
    fonts: &[FontSpan],
    styles: &[[usize; 2]],
    text_len: usize,
) -> Result<Vec<StyledFontSpan>> {
    if text_len == 0 {
        return Ok(Vec::new());
    }
    let validate = |ranges: &mut dyn Iterator<Item = [usize; 2]>, label: &str| {
        let mut cursor = 0usize;
        for range in ranges {
            if range[0] != cursor || range[0] >= range[1] || range[1] > text_len {
                return Err(fail(label));
            }
            cursor = range[1];
        }
        if cursor != text_len {
            return Err(fail(label));
        }
        Ok(())
    };
    validate(
        &mut fonts.iter().map(|span| span.range),
        "font spans do not partition styled text",
    )?;
    validate(
        &mut styles.iter().copied(),
        "style spans do not partition styled text",
    )?;
    let (mut font_index, mut style_index, mut cursor) = (0usize, 0usize, 0usize);
    let mut result = Vec::new();
    while cursor < text_len {
        let font = &fonts[font_index];
        let style = styles[style_index];
        let end = font.range[1].min(style[1]);
        if end <= cursor {
            return Err(fail("styled font intersection made no progress"));
        }
        result.push(StyledFontSpan {
            range: [cursor, end],
            font_index: font.font_index,
            style_index,
        });
        cursor = end;
        if cursor == font.range[1] {
            font_index += 1;
        }
        if cursor == style[1] {
            style_index += 1;
        }
    }
    Ok(result)
}

/// Split by resolved direction and font, then UAX #9 L2 visual order. Glyph
/// clusters are local to each returned run. The same function paints/measures.
pub fn shape_line(
    text: &str,
    bidi: &LineBidi,
    spans: &[FontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &OpenTypeSettings,
) -> Result<Vec<FontRun>> {
    let programs = fonts
        .iter()
        .map(|font| font.bytes.as_slice())
        .collect::<Vec<_>>();
    shape_line_programs(text, bidi, spans, &programs, settings)
}

pub(crate) fn shape_line_programs(
    text: &str,
    bidi: &LineBidi,
    spans: &[FontSpan],
    fonts: &[&[u8]],
    settings: &OpenTypeSettings,
) -> Result<Vec<FontRun>> {
    crate::cancel::check_current_cancel("fallback line itemization")?;
    if text.len() > 4_000_000 || fonts.len() > 256 {
        return Err(fail("fallback line font/text budget exceeded"));
    }
    bidi.context.validate()?;
    if bidi.levels.len() != text.len() {
        return Err(fail("fallback bidi length mismatch"));
    }
    let mut end = 0;
    for span in spans {
        if span.range[0] != end
            || span.range[0] >= span.range[1]
            || !text.is_char_boundary(span.range[0])
            || !text.is_char_boundary(span.range[1])
            || span.font_index >= fonts.len()
        {
            return Err(fail("fallback spans are not a contiguous source partition"));
        }
        end = span.range[1];
    }
    if end != text.len() {
        return Err(fail("fallback spans do not cover line"));
    }
    let mut logical: Vec<(Range<usize>, usize, Level)> = Vec::new();
    let mut index = 0;
    for (offset, ch) in text.char_indices() {
        while spans[index].range[1] <= offset {
            index += 1;
        }
        let level =
            Level::new(bidi.levels[offset]).map_err(|_| fail("invalid fallback bidi level"))?;
        let next = offset + ch.len_utf8();
        if bidi.levels[offset..next]
            .iter()
            .any(|v| *v != level.number())
        {
            return Err(fail("bidi level divides scalar"));
        }
        let font = spans[index].font_index;
        if let Some((range, previous_font, previous_level)) = logical.last_mut() {
            if *previous_font == font && *previous_level == level {
                range.end = next;
                continue;
            }
        }
        logical.push((offset..next, font, level));
    }
    let order = BidiInfo::reorder_visual(&logical.iter().map(|r| r.2).collect::<Vec<_>>());
    order
        .into_iter()
        .map(|i| {
            let (range, font_index, level) = &logical[i];
            let slice = &text[range.clone()];
            let resolved = bidi.slice(text, range.clone(), level.is_rtl())?;
            let shaped =
                TextShaper::shape_resolved(fonts[*font_index], slice, &resolved, settings)?;
            if has_missing_glyphs(fonts[*font_index], slice, &shaped)? {
                return Err(fail("fallback line lost glyph coverage"));
            }
            Ok(FontRun {
                range: range.clone(),
                font_index: *font_index,
                shaped,
            })
        })
        .collect()
}

/// Itemize one already-resolved line by font, inline style and bidi level, then
/// apply UAX #9 L2 once to the combined run list. Shaping each style segment in
/// a caller loop would lose visual ordering across segment boundaries.
pub fn shape_styled_line(
    text: &str,
    bidi: &LineBidi,
    spans: &[StyledFontSpan],
    fonts: &[ApprovedFontAsset],
    settings: &[OpenTypeSettings],
) -> Result<Vec<StyledFontRun>> {
    crate::cancel::check_current_cancel("styled fallback line itemization")?;
    if text.len() > 4_000_000 || fonts.len() > 256 || settings.len() > 100_000 {
        return Err(fail("styled fallback line budget exceeded"));
    }
    bidi.context.validate()?;
    if bidi.levels.len() != text.len() {
        return Err(fail("styled fallback bidi length mismatch"));
    }
    let mut end = 0usize;
    for span in spans {
        if span.range[0] != end
            || span.range[0] >= span.range[1]
            || !text.is_char_boundary(span.range[0])
            || !text.is_char_boundary(span.range[1])
            || span.font_index >= fonts.len()
            || span.style_index >= settings.len()
        {
            return Err(fail(
                "styled fallback spans are not a contiguous source partition",
            ));
        }
        end = span.range[1];
    }
    if end != text.len() {
        return Err(fail("styled fallback spans do not cover line"));
    }
    let mut logical: Vec<(Range<usize>, usize, usize, Level)> = Vec::new();
    let mut index = 0usize;
    for (offset, character) in text.char_indices() {
        while spans[index].range[1] <= offset {
            index += 1;
        }
        let level =
            Level::new(bidi.levels[offset]).map_err(|_| fail("invalid styled bidi level"))?;
        let next = offset + character.len_utf8();
        if bidi.levels[offset..next]
            .iter()
            .any(|value| *value != level.number())
        {
            return Err(fail("styled bidi level divides scalar"));
        }
        let span = &spans[index];
        if let Some((range, font, style, previous_level)) = logical.last_mut() {
            if *font == span.font_index && *style == span.style_index && *previous_level == level {
                range.end = next;
                continue;
            }
        }
        logical.push((offset..next, span.font_index, span.style_index, level));
    }
    let order = BidiInfo::reorder_visual(
        &logical
            .iter()
            .map(|(_, _, _, level)| *level)
            .collect::<Vec<_>>(),
    );
    order
        .into_iter()
        .map(|item| {
            let (range, font_index, style_index, level) = &logical[item];
            let slice = &text[range.clone()];
            let resolved = bidi.slice(text, range.clone(), level.is_rtl())?;
            let shaped = TextShaper::shape_resolved(
                &fonts[*font_index].bytes,
                slice,
                &resolved,
                &settings[*style_index],
            )?;
            if has_missing_glyphs(&fonts[*font_index].bytes, slice, &shaped)? {
                return Err(fail("styled fallback line lost glyph coverage"));
            }
            Ok(StyledFontRun {
                range: range.clone(),
                font_index: *font_index,
                style_index: *style_index,
                shaped,
            })
        })
        .collect()
}

pub(crate) fn measure_styled_line_prepared(
    runs: &[StyledFontRun],
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
    sizes: &[f64],
) -> Result<super::line_layout::LineMetrics> {
    let mut pen = 0.0f64;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut ascent = 0.0f64;
    let mut descent = 0.0f64;
    for run in runs {
        crate::cancel::check_current_cancel("prepared styled line measurement")?;
        let size = sizes
            .get(run.style_index)
            .copied()
            .filter(|size| size.is_finite() && *size > 0.0)
            .ok_or_else(|| fail("styled line font size is absent or invalid"))?;
        let prepared = metrics
            .get(run.font_index)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("styled line measurement font is not prepared"))?;
        let measured = prepared.measure(&run.shaped, size)?;
        left = left.min(pen - measured.left_pad);
        right = right.max(pen + measured.advance + measured.right_pad);
        ascent = ascent.max(measured.ascent);
        descent = descent.max(measured.descent);
        pen += measured.advance;
    }
    Ok(super::line_layout::LineMetrics {
        advance: pen,
        left_pad: -left,
        right_pad: (right - pen).max(0.0),
        ascent,
        descent,
    })
}

pub fn measure_line(
    runs: &[FontRun],
    fonts: &[ApprovedFontAsset],
    size: f64,
) -> Result<super::line_layout::LineMetrics> {
    let programs = fonts
        .iter()
        .map(|font| font.bytes.as_slice())
        .collect::<Vec<_>>();
    measure_line_programs(runs, &programs, size)
}

pub(crate) fn measure_line_prepared(
    runs: &[FontRun],
    metrics: &[Option<super::line_layout::PreparedFontMetrics<'_>>],
    size: f64,
) -> Result<super::line_layout::LineMetrics> {
    if !size.is_finite() || size <= 0.0 {
        return Err(fail("invalid fallback font size"));
    }
    let mut pen = 0.0f64;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut ascent = 0.0f64;
    let mut descent = 0.0f64;
    for run in runs {
        crate::cancel::check_current_cancel("prepared fallback line measurement")?;
        let prepared = metrics
            .get(run.font_index)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("fallback measurement font is not prepared"))?;
        let m = prepared.measure(&run.shaped, size)?;
        left = left.min(pen - m.left_pad);
        right = right.max(pen + m.advance + m.right_pad);
        ascent = ascent.max(m.ascent);
        descent = descent.max(m.descent);
        pen += m.advance;
    }
    Ok(super::line_layout::LineMetrics {
        advance: pen,
        left_pad: -left,
        right_pad: (right - pen).max(0.0),
        ascent,
        descent,
    })
}

pub(crate) fn measure_line_programs(
    runs: &[FontRun],
    fonts: &[&[u8]],
    size: f64,
) -> Result<super::line_layout::LineMetrics> {
    measure_line_progression(runs, fonts, size, false)
}

pub(crate) fn measure_signed_line_programs(
    runs: &[FontRun],
    fonts: &[&[u8]],
    size: f64,
) -> Result<super::line_layout::LineMetrics> {
    measure_line_progression(runs, fonts, size, true)
}

fn measure_line_progression(
    runs: &[FontRun],
    fonts: &[&[u8]],
    size: f64,
    signed: bool,
) -> Result<super::line_layout::LineMetrics> {
    if !size.is_finite() || size <= 0.0 {
        return Err(fail("invalid fallback font size"));
    }
    let mut pen = 0.0f64;
    let mut left = 0.0f64;
    let mut right = 0.0f64;
    let mut ascent = 0.0f64;
    let mut descent = 0.0f64;
    for run in runs {
        crate::cancel::check_current_cancel("fallback line measurement")?;
        let font = fonts
            .get(run.font_index)
            .ok_or_else(|| fail("fallback measurement font index"))?;
        let m = if signed {
            super::line_layout::measure_signed_run(font, &run.shaped, size)?
        } else {
            super::line_layout::measure_run(font, &run.shaped, size)?
        };
        left = left.min(pen - m.left_pad);
        right = right.max(pen + m.advance + m.right_pad);
        ascent = ascent.max(m.ascent);
        descent = descent.max(m.descent);
        pen += m.advance;
    }
    Ok(super::line_layout::LineMetrics {
        advance: pen,
        left_pad: -left,
        right_pad: (right - pen).max(0.0),
        ascent,
        descent,
    })
}
