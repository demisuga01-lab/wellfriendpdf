//! Vertical story output shares the font runs/metrics used by pagination.
use super::*;
use crate::editing_transactions::ApprovedFontAsset;
use crate::fonts::{fallback::FontSpan, shaper::LineBidi, WritingMode};
#[cfg(test)]
#[path = "advanced_story_vertical_tests.rs"]
mod tests;

pub(super) fn shape_line(
    line: &StoryPaintLine,
    visual: &str,
    bidi: &LineBidi,
    spans: &[FontSpan],
    fonts: &[ApprovedFontAsset],
    metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    region: [f64; 4],
) -> Result<Vec<(usize, Vec<GeneratedGlyph>)>> {
    let runs = crate::fonts::vertical_fonts::shape_line_prepared(
        visual,
        bidi,
        spans,
        fonts,
        &line.shaping,
        metrics,
    )?;
    let metric = crate::fonts::vertical_fonts::measure_line_prepared(
        &runs,
        fonts,
        metrics,
        line.font_size,
        line.writing_mode,
    )?;
    if !line.width.is_finite() || metric.advance > line.width + 1e-7 {
        return Err(WellfriendError::MalformedPdf(
            "vertical story measurement/emission inline extent mismatch".into(),
        ));
    }
    let (left, right) = if line.writing_mode == WritingMode::VerticalRl {
        (metric.descent, metric.ascent)
    } else {
        (metric.ascent, metric.descent)
    };
    let bounds = [
        line.x - left,
        line.baseline - metric.advance - metric.right_pad,
        line.x + right,
        line.baseline + metric.left_pad,
    ];
    if !bounds.iter().all(|v| v.is_finite())
        || bounds[0] < region[0] - 1e-7
        || bounds[1] < region[1] - 1e-7
        || bounds[2] > region[2] + 1e-7
        || bounds[3] > region[3] + 1e-7
    {
        return Err(WellfriendError::MalformedPdf(
            "vertical story glyph bounds exceed the approved physical frame".into(),
        ));
    }
    let mut result = Vec::new();
    for run in runs {
        let shaped = crate::fonts::ShapedRun {
            glyphs: run.glyphs.iter().map(|g| g.glyph.clone()).collect(),
            direction: TextDirection::LeftToRight,
            used_complex_shaping: true,
        };
        let mut glyphs =
            generated_glyphs_from_shaped(&visual[run.range], &fonts[run.font_index].bytes, shaped)?;
        for (glyph, vertical) in glyphs.iter_mut().zip(run.glyphs) {
            glyph.orientation = if vertical.rotate_clockwise {
                VerticalGlyphOrientation::RotateClockwise
            } else if vertical.vertical_alternate {
                VerticalGlyphOrientation::FontVerticalAlternate
            } else {
                VerticalGlyphOrientation::Upright
            };
            glyph.cross_advance = vertical.cross_advance;
        }
        result.push((run.font_index, glyphs));
    }
    Ok(result)
}

pub(super) fn shape_styled_line(
    line: &StoryPaintLine,
    visual: &str,
    bidi: &LineBidi,
    spans: &[crate::fonts::fallback::StyledFontSpan],
    styles: &[StoryPaintStyleSpan],
    fonts: &[ApprovedFontAsset],
    metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    region: [f64; 4],
) -> Result<Vec<(usize, usize, Vec<GeneratedGlyph>)>> {
    let settings = styles
        .iter()
        .map(|style| style.shaping.clone())
        .collect::<Vec<_>>();
    let sizes = styles
        .iter()
        .map(|style| style.font_size)
        .collect::<Vec<_>>();
    let runs = crate::fonts::vertical_fonts::shape_styled_line_prepared(
        visual, bidi, spans, fonts, &settings, metrics,
    )?;
    let metric = crate::fonts::vertical_fonts::measure_styled_line_prepared(
        &runs,
        fonts,
        metrics,
        &sizes,
        line.writing_mode,
    )?;
    if !line.width.is_finite() || metric.advance > line.width + 1e-7 {
        return Err(WellfriendError::MalformedPdf(
            "styled vertical story measurement/emission inline extent mismatch".into(),
        ));
    }
    let (left, right) = if line.writing_mode == WritingMode::VerticalRl {
        (metric.descent, metric.ascent)
    } else {
        (metric.ascent, metric.descent)
    };
    let bounds = [
        line.x - left,
        line.baseline - metric.advance - metric.right_pad,
        line.x + right,
        line.baseline + metric.left_pad,
    ];
    if !bounds.iter().all(|value| value.is_finite())
        || bounds[0] < region[0] - 1e-7
        || bounds[1] < region[1] - 1e-7
        || bounds[2] > region[2] + 1e-7
        || bounds[3] > region[3] + 1e-7
    {
        return Err(WellfriendError::MalformedPdf(
            "styled vertical story glyph bounds exceed the approved physical frame".into(),
        ));
    }
    let mut result = Vec::new();
    for run in runs {
        let shaped = crate::fonts::ShapedRun {
            glyphs: run.glyphs.iter().map(|glyph| glyph.glyph.clone()).collect(),
            direction: TextDirection::LeftToRight,
            used_complex_shaping: true,
        };
        let mut glyphs =
            generated_glyphs_from_shaped(&visual[run.range], &fonts[run.font_index].bytes, shaped)?;
        for (glyph, vertical) in glyphs.iter_mut().zip(run.glyphs) {
            glyph.orientation = if vertical.rotate_clockwise {
                VerticalGlyphOrientation::RotateClockwise
            } else if vertical.vertical_alternate {
                VerticalGlyphOrientation::FontVerticalAlternate
            } else {
                VerticalGlyphOrientation::Upright
            };
            glyph.cross_advance = vertical.cross_advance;
        }
        result.push((run.font_index, run.style_index, glyphs));
    }
    Ok(result)
}

pub(super) fn append_run(
    output: &mut String,
    line: &StoryPaintLine,
    glyphs: &[GeneratedGlyph],
    font: &str,
    font_size: f64,
    pen: &mut [f64; 2],
) -> Result<()> {
    output.push_str(&format!(
        "BT\n0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n/{} {} Tf\n",
        serialized_name_body(font),
        fmt_num(font_size)
    ));
    let scale = font_size / 1000.0;
    for glyph in glyphs {
        crate::cancel::check_current_cancel("vertical story glyph serialization")?;
        let x = line.x + pen[0];
        let y = line.baseline - pen[1];
        let ox = glyph.offset_x * scale;
        let oy = glyph.offset_y * scale;
        let matrix = if glyph.orientation == VerticalGlyphOrientation::RotateClockwise {
            [0.0, -1.0, 1.0, 0.0, x + oy, y - ox]
        } else {
            [1.0, 0.0, 0.0, 1.0, x + ox, y + oy]
        };
        let mut marked = Vec::new();
        inline_text::append_glyph_marker(
            &mut marked,
            [1.0, 0.0, 0.0, 1.0, x, y],
            matrix,
            glyph.cid,
        )?;
        output.push_str(std::str::from_utf8(&marked).map_err(|_| {
            WellfriendError::MalformedPdf("invalid generated vertical marker".into())
        })?);
        pen[0] += glyph.cross_advance * scale;
        pen[1] += glyph.advance * scale;
    }
    output.push_str("ET\n");
    Ok(())
}
