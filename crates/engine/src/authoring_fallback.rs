//! Explicit authoring fallback using the editor's contextual assignment,
//! paragraph-derived bidi, shared measurement and canonical Type0 writer.
use super::*;
use crate::fonts::{
    fallback as engine, line_layout::PreparedParagraph, shaper::LineBidi, ShapedRun, TextDirection,
    WritingMode,
};
use std::{collections::BTreeSet, ops::Range};

#[cfg(test)]
#[path = "authoring_fallback_tests.rs"]
mod tests;

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

pub(super) fn register(builder: &mut PdfBuilder, requested: &[FontFace]) -> Result<FontFace> {
    crate::cancel::check_current_cancel("authoring font stack registration")?;
    if requested.is_empty() || requested.len() > 256 {
        return Err(fail("authoring stack needs 1..=256 fonts"));
    }
    let mut flat = Vec::new();
    for &font in requested {
        let members = if let FontFace::Fallback(id) = font {
            builder
                .font_stacks
                .get(id.0 as usize)
                .ok_or_else(|| fail("unregistered nested font stack"))?
                .as_slice()
        } else {
            std::slice::from_ref(&font)
        };
        for &member in members {
            if !flat.contains(&member) {
                flat.push(member);
            }
        }
        if flat.len() > 256 {
            return Err(fail("flattened authoring stack exceeds 256 fonts"));
        }
    }
    // Validate the complete declaration before registration changes any state.
    for &font in &flat {
        crate::cancel::check_current_cancel("authoring stack font validation")?;
        let bytes = match font {
            FontFace::Standard(standard) => get_fallback_font(standard.fallback_font_name())
                .ok_or_else(|| fail("bundled Standard-14 equivalent unavailable"))?,
            _ => font_bytes_for_face(builder, font)?,
        };
        crate::fonts::pdf_embedding::EmbeddingInfo::parse(bytes)?;
    }
    let previous_fonts = Arc::clone(&builder.custom_fonts);
    let previous_stacks = Arc::clone(&builder.font_stacks);
    let result: Result<FontFace> = (|| {
        let mut physical = Vec::new();
        for font in flat {
            crate::cancel::check_current_cancel("authoring stack embedding resolution")?;
            let font = if let FontFace::Standard(standard) = font {
                let bytes = get_fallback_font(standard.fallback_font_name())
                    .ok_or_else(|| fail("bundled equivalent unavailable"))?;
                if let Some(existing) = builder
                    .custom_fonts
                    .iter()
                    .find(|font| font.bytes.as_ref() == bytes)
                {
                    FontFace::Custom(existing.id)
                } else {
                    builder.register_font_bytes(
                        format!("WellfriendFallback{}", standard.base_font_name()),
                        bytes,
                    )?
                }
            } else {
                font
            };
            if !physical.contains(&font) {
                physical.push(font);
            }
        }
        let index = if let Some(index) = builder
            .font_stacks
            .iter()
            .position(|members| members == &physical)
        {
            index
        } else {
            let index = builder.font_stacks.len();
            u32::try_from(index).map_err(|_| fail("authoring stack count"))?;
            Arc::make_mut(&mut builder.font_stacks).push(physical);
            index
        };
        for page in &mut builder.pages {
            page.custom_fonts = Arc::clone(&builder.custom_fonts);
            page.font_stacks = Arc::clone(&builder.font_stacks);
        }
        Ok(FontFace::Fallback(FontStackId(index as u32)))
    })();
    if result.is_err() {
        builder.custom_fonts = previous_fonts;
        builder.font_stacks = previous_stacks;
        for page in &mut builder.pages {
            page.custom_fonts = Arc::clone(&builder.custom_fonts);
            page.font_stacks = Arc::clone(&builder.font_stacks);
        }
    }
    result
}

#[derive(Debug, Clone)]
struct Run {
    text: String,
    range: Range<usize>,
    font: FontFace,
    asset: Option<Arc<[u8]>>,
    bidi: LineBidi,
    shaped: Arc<ShapedRun>,
    advance: f64,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedLine {
    logical: String,
    range: Range<usize>,
    source_font: FontFace,
    size: f64,
    runs: Vec<Run>,
}

#[derive(Debug, Clone)]
pub struct FontStackRunPreview {
    pub logical_utf8_range: [usize; 2],
    pub font: FontFace,
    pub font_sha256: String,
    pub fallback: bool,
    pub right_to_left: bool,
    pub glyph_count: usize,
    pub advance: f64,
}

#[derive(Debug, Clone)]
pub struct FontStackLinePreview {
    pub logical_utf8_range: [usize; 2],
    pub logical_text: String,
    pub width: f64,
    pub advance: f64,
    pub ascent: f64,
    pub descent: f64,
    pub logical_carrier: bool,
    /// Paint order; logical byte ranges retain source order independently.
    pub runs: Vec<FontStackRunPreview>,
}

impl ResolvedLine {
    pub(super) fn command(&self, x: f64, y: f64, style: &TextStyle) -> Result<PageCommand> {
        if style.font != self.source_font || style.size != self.size {
            return Err(fail(
                "fallback command differs from its measured font or size",
            ));
        }
        if crate::fonts::logical_carrier::is_text(&self.logical) {
            return Ok(PageCommand::LogicalBreak {
                text: self.logical.clone(),
                x,
                y,
                size: style.size,
            });
        }
        let mut pen = x;
        let mut commands = Vec::new();
        for run in &self.runs {
            crate::cancel::check_current_cancel("authoring fallback command")?;
            if !pen.is_finite() {
                return Err(fail("non-finite fallback position"));
            }
            commands.push(PageCommand::Text {
                text: run.text.clone(),
                x: pen,
                y,
                style: TextStyle {
                    font: run.font,
                    ..style.clone()
                },
                bidi: Some(run.bidi.clone()),
                logical_text: None,
                suppress_actual_text: false,
                font_asset: run.asset.clone(),
                shaped: Some(Arc::clone(&run.shaped)),
            });
            pen += run.advance;
        }
        if !pen.is_finite() {
            return Err(fail("non-finite fallback endpoint"));
        }
        Ok(PageCommand::TextGroup {
            logical_text: self.logical.clone(),
            runs: commands,
        })
    }

    pub(super) fn owned_commands(
        &self,
        x: f64,
        y: f64,
        style: &TextStyle,
        spans: &[layout::OwnedTextSpan],
    ) -> Result<Vec<PageCommand>> {
        if style.font != self.source_font || style.size != self.size {
            return Err(fail(
                "fallback semantic command differs from its measured font or size",
            ));
        }
        let mut pen = x;
        let mut paints = Vec::new();
        for run in &self.runs {
            crate::cancel::check_current_cancel("authoring fallback semantic command")?;
            let run_start = run
                .range
                .start
                .checked_sub(self.range.start)
                .ok_or_else(|| fail("fallback run precedes its line"))?;
            if run.range.end > self.range.end || run.text.len() != run.range.len() {
                return Err(fail("fallback run escaped its logical line"));
            }
            let mut boundaries = run
                .shaped
                .glyphs
                .iter()
                .map(|glyph| {
                    usize::try_from(glyph.cluster)
                        .map_err(|_| fail("fallback cluster offset overflow"))
                })
                .collect::<Result<Vec<_>>>()?;
            boundaries.push(0);
            boundaries.push(run.text.len());
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut cursor = 0usize;
            let mut seen = BTreeSet::new();
            while cursor < run.shaped.glyphs.len() {
                let cluster = run.shaped.glyphs[cursor].cluster;
                if !seen.insert(cluster) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "noncontiguous fallback cluster cannot be semantically partitioned".into(),
                    ));
                }
                let first_pen = pen;
                let mut end_glyph = cursor;
                let mut glyphs = Vec::new();
                while end_glyph < run.shaped.glyphs.len()
                    && run.shaped.glyphs[end_glyph].cluster == cluster
                {
                    let mut glyph = run.shaped.glyphs[end_glyph].clone();
                    glyph.cluster = 0;
                    pen += glyph.advance * style.size / 1000.0;
                    glyphs.push(glyph);
                    end_glyph += 1;
                }
                let start = usize::try_from(cluster)
                    .map_err(|_| fail("fallback cluster offset overflow"))?;
                let end = boundaries
                    .iter()
                    .copied()
                    .find(|candidate| *candidate > start)
                    .unwrap_or(run.text.len());
                if start >= end
                    || end > run.text.len()
                    || !run.text.is_char_boundary(start)
                    || !run.text.is_char_boundary(end)
                {
                    return Err(fail("invalid fallback cluster boundary"));
                }
                let local = run_start + start..run_start + end;
                let owner = layout::owner_for_cluster(spans, local)?;
                let resolved = run.bidi.slice(
                    &run.text,
                    start..end,
                    run.shaped.direction == TextDirection::RightToLeft,
                )?;
                paints.push(layout::OwnedPaint {
                    element: owner,
                    command: PageCommand::Text {
                        text: run.text[start..end].to_owned(),
                        x: first_pen,
                        y,
                        style: TextStyle {
                            font: run.font,
                            ..style.clone()
                        },
                        bidi: Some(resolved),
                        logical_text: None,
                        suppress_actual_text: true,
                        font_asset: run.asset.clone(),
                        shaped: Some(Arc::new(ShapedRun {
                            glyphs,
                            direction: run.shaped.direction,
                            used_complex_shaping: true,
                        })),
                    },
                });
                cursor = end_glyph;
            }
        }
        if !pen.is_finite() {
            return Err(fail("non-finite fallback semantic endpoint"));
        }
        layout::wrap_owned_paints(&self.logical, spans, paints, x, y, style.size)
    }
}

struct Prepared<'a> {
    page: &'a PdfPageBuilder,
    text: &'a str,
    members: &'a [FontFace],
    programs: Vec<&'a [u8]>,
    spans: Vec<engine::FontSpan>,
    paragraph: PreparedParagraph<'a>,
}

impl<'a> Prepared<'a> {
    fn new(page: &'a PdfPageBuilder, text: &'a str, style: &TextStyle) -> Result<Self> {
        let FontFace::Fallback(id) = style.font else {
            return Err(fail("expected an authoring font stack"));
        };
        let members = page
            .font_stacks
            .get(id.0 as usize)
            .ok_or_else(|| fail("authoring stack is not registered on this page"))?;
        let programs = members
            .iter()
            .map(|font| {
                page.font_program(*font)?
                    .ok_or_else(|| fail("unresolved Standard-14 font in fallback stack"))
            })
            .collect::<Result<Vec<_>>>()?;
        let ranked = (0..members.len())
            .map(|index| (index, index as f64))
            .collect::<Vec<_>>();
        // U+0009 is layout state, not a glyph. Preserve byte-for-byte offsets
        // while asking the coverage resolver about a neutral one-byte boundary;
        // tabbed emission later shapes only the exact non-tab fields.
        let coverage_text = text.replace('\t', " ");
        let spans = engine::resolve_contextual_programs(
            &coverage_text,
            &programs,
            &ranked,
            ShapeOptions::default(),
            &Default::default(),
            WritingMode::HorizontalTb,
        )?;
        Ok(Self {
            page,
            text,
            members,
            programs,
            spans,
            paragraph: PreparedParagraph::new(text, ShapeOptions::default())?,
        })
    }

    fn shape(
        &self,
        range: Range<usize>,
        supplied: Option<&LineBidi>,
    ) -> Result<(String, LineBidi, Vec<engine::FontRun>)> {
        let logical = &self.text[range.clone()];
        let visible = logical.trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let bidi = if let Some(bidi) = supplied {
            bidi.clone()
        } else {
            self.paragraph
                .bidi
                .line(range.start..range.start + visible.len())?
        };
        let spans = engine::slice_spans(&self.spans, range.start..range.start + visible.len());
        let runs = engine::shape_line_programs(
            visible,
            &bidi,
            &spans,
            &self.programs,
            &Default::default(),
        )?;
        Ok((visible.to_owned(), bidi, runs))
    }

    fn line(
        &self,
        range: Range<usize>,
        style: &TextStyle,
        supplied: Option<&LineBidi>,
    ) -> Result<layout::Line> {
        let (visual, bidi, shaped) = self.shape(range.clone(), supplied)?;
        let metrics = layout::checked_metrics(engine::measure_signed_line_programs(
            &shaped,
            &self.programs,
            style.size,
        )?)?;
        let mut runs = Vec::new();
        // A logical-only line is emitted using the private zero-width carrier,
        // not any of the stack's visible font runs. Preview must agree.
        let logical_only = crate::fonts::logical_carrier::is_text(&self.text[range.clone()]);
        for run in shaped.into_iter().filter(|_| !logical_only) {
            let font = self.members[run.font_index];
            let rtl = run.shaped.direction == TextDirection::RightToLeft;
            let resolved = bidi.slice(&visual, run.range.clone(), rtl)?;
            let advance = run
                .shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.advance)
                .sum::<f64>()
                * style.size
                / 1000.0;
            runs.push(Run {
                text: visual[run.range.clone()].to_owned(),
                range: range.start + run.range.start..range.start + run.range.end,
                font,
                asset: self.page.custom_font_asset(font),
                bidi: resolved,
                shaped: Arc::new(run.shaped),
                advance,
            });
        }
        let logical = self.text[range.clone()].to_owned();
        let plan = ResolvedLine {
            logical: logical.clone(),
            range,
            source_font: style.font,
            size: style.size,
            runs,
        };
        Ok(layout::Line {
            logical,
            visual,
            metrics,
            bidi: None,
            font_asset: None,
            fallback: Some(Arc::new(plan)),
            tab_segments: Vec::new(),
            tab_decorations: Vec::new(),
        })
    }
}

pub(super) fn single_command(
    page: &PdfPageBuilder,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    bidi: Option<&LineBidi>,
) -> Result<PageCommand> {
    let prepared = Prepared::new(page, text, style)?;
    prepared
        .line(0..text.len(), style, bidi)?
        .command(x, y, style)
}

pub(super) fn width(page: &PdfPageBuilder, text: &str, style: &TextStyle) -> Result<f64> {
    let prepared = Prepared::new(page, text, style)?;
    Ok(prepared.line(0..text.len(), style, None)?.metrics.width())
}

pub(super) fn prepare(
    page: &PdfPageBuilder,
    text: &str,
    width: f64,
    style: &TextStyle,
) -> Result<Vec<layout::Line>> {
    let prepared = Prepared::new(page, text, style)?;
    let measured = prepared
        .paragraph
        .break_lines_measured(0, width, 100_000, |range| {
            let (_, _, runs) = prepared.shape(range, None)?;
            Ok(
                layout::checked_metrics(engine::measure_signed_line_programs(
                    &runs,
                    &prepared.programs,
                    style.size,
                )?)?
                .width(),
            )
        })?;
    if measured.last().map_or(0, |line| line.bytes.end) != text.len() {
        return Err(WellfriendError::ResourceLimit(
            "authoring fallback paragraph line budget".into(),
        ));
    }
    measured
        .into_iter()
        .map(|line| prepared.line(line.bytes, style, None))
        .collect()
}

pub(super) fn prepare_with_tabs(
    page: &PdfPageBuilder,
    text: &str,
    width: f64,
    style: &TextStyle,
    tabs: &crate::fonts::tab_stops::TabStops,
) -> Result<Vec<layout::Line>> {
    let prepared = Prepared::new(page, text, style)?;
    let measure_line = |range: Range<usize>| -> Result<f64> {
        let visible_end = range.end
            - text[range.clone()]
                .chars()
                .rev()
                .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
                .map(char::len_utf8)
                .sum::<usize>();
        let plan =
            crate::fonts::tab_stops::plan_line(text, range.start..visible_end, tabs, |segment| {
                Ok(prepared.line(segment, style, None)?.metrics.width())
            })?;
        Ok(plan.advance)
    };
    let measured = prepared
        .paragraph
        .break_lines_measured(0, width, 100_000, measure_line)?;
    if measured.last().map_or(0, |line| line.bytes.end) != text.len() {
        return Err(WellfriendError::ResourceLimit(
            "authoring fallback tabbed paragraph line budget".into(),
        ));
    }
    let mut output = Vec::with_capacity(measured.len());
    for measured_line in measured {
        crate::cancel::check_current_cancel("authoring fallback tabbed final line")?;
        let logical_range = measured_line.bytes.clone();
        let visible_end = logical_range.end
            - text[logical_range.clone()]
                .chars()
                .rev()
                .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
                .map(char::len_utf8)
                .sum::<usize>();
        let plan = crate::fonts::tab_stops::plan_line(
            text,
            logical_range.start..visible_end,
            tabs,
            |segment| Ok(prepared.line(segment, style, None)?.metrics.width()),
        )?;
        if plan.advance > width + 1e-7 {
            return Err(WellfriendError::UnsupportedFeature(
                "resolved fallback tab fields exceed the authoring paragraph width".into(),
            ));
        }
        let mut segments = Vec::with_capacity(plan.segments.len());
        for segment in &plan.segments {
            if segment.range.is_empty() {
                continue;
            }
            segments.push(layout::PositionedLineSegment {
                range: segment.range.start - logical_range.start
                    ..segment.range.end - logical_range.start,
                origin: segment.origin,
                line: Box::new(prepared.line(segment.range.clone(), style, None)?),
            });
        }
        let metrics = layout::tabbed_metrics(&plan, &segments)?;
        output.push(layout::Line {
            logical: text[logical_range.clone()].to_owned(),
            visual: text[logical_range.start..visible_end].to_owned(),
            metrics,
            bidi: None,
            font_asset: None,
            fallback: None,
            tab_segments: segments,
            tab_decorations: plan.decorations,
        });
    }
    Ok(output)
}

pub(super) fn prepare_ranges(
    page: &PdfPageBuilder,
    text: &str,
    ranges: &[Range<usize>],
    style: &TextStyle,
) -> Result<Vec<layout::Line>> {
    let prepared = Prepared::new(page, text, style)?;
    ranges
        .iter()
        .cloned()
        .map(|range| prepared.line(range, style, None))
        .collect()
}

pub(super) fn prepare_ranges_with_tabs(
    page: &PdfPageBuilder,
    text: &str,
    ranges: &[Range<usize>],
    style: &TextStyle,
    tabs: &crate::fonts::tab_stops::TabStops,
) -> Result<Vec<layout::Line>> {
    let prepared = Prepared::new(page, text, style)?;
    let mut output = Vec::with_capacity(ranges.len());
    for logical_range in ranges {
        let visible_end = logical_range.end
            - text[logical_range.clone()]
                .chars()
                .rev()
                .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
                .map(char::len_utf8)
                .sum::<usize>();
        let plan = crate::fonts::tab_stops::plan_line(
            text,
            logical_range.start..visible_end,
            tabs,
            |segment| Ok(prepared.line(segment, style, None)?.metrics.width()),
        )?;
        let mut segments = Vec::with_capacity(plan.segments.len());
        for segment in &plan.segments {
            if segment.range.is_empty() {
                continue;
            }
            segments.push(layout::PositionedLineSegment {
                range: segment.range.start - logical_range.start
                    ..segment.range.end - logical_range.start,
                origin: segment.origin,
                line: Box::new(prepared.line(segment.range.clone(), style, None)?),
            });
        }
        let metrics = layout::tabbed_metrics(&plan, &segments)?;
        output.push(layout::Line {
            logical: text[logical_range.clone()].to_owned(),
            visual: text[logical_range.start..visible_end].to_owned(),
            metrics,
            bidi: None,
            font_asset: None,
            fallback: None,
            tab_segments: segments,
            tab_decorations: plan.decorations,
        });
    }
    Ok(output)
}

pub(super) fn range_metrics(
    page: &PdfPageBuilder,
    text: &str,
    range: Range<usize>,
    style: &TextStyle,
) -> Result<crate::fonts::line_layout::LineMetrics> {
    Ok(Prepared::new(page, text, style)?
        .line(range, style, None)?
        .metrics)
}

pub(super) fn range_x_bounds(
    page: &PdfPageBuilder,
    text: &str,
    line: Range<usize>,
    field: Range<usize>,
    style: &TextStyle,
) -> Result<(f64, f64)> {
    let prepared = Prepared::new(page, text, style)?;
    let (visual, _, runs) = prepared.shape(line.clone(), None)?;
    let relative = field.start - line.start..field.end - line.start;
    let mut pen = 0.0f64;
    let mut left = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    for run in runs {
        let run_text = &visual[run.range.clone()];
        let mut clusters = run
            .shaped
            .glyphs
            .iter()
            .filter_map(|glyph| usize::try_from(glyph.cluster).ok())
            .filter(|offset| *offset <= run_text.len() && run_text.is_char_boundary(*offset))
            .collect::<Vec<_>>();
        clusters.push(0);
        clusters.push(run_text.len());
        clusters.sort_unstable();
        clusters.dedup();
        for glyph in &run.shaped.glyphs {
            let start = usize::try_from(glyph.cluster)
                .map_err(|_| fail("fallback field glyph cluster overflow"))?;
            let end = clusters
                .iter()
                .copied()
                .find(|offset| *offset > start)
                .unwrap_or(run_text.len());
            let global = run.range.start + start..run.range.start + end;
            let next = pen + glyph.advance * style.size / 1000.0;
            let glyph_x = pen + glyph.offset_x * style.size / 1000.0;
            if global.start < relative.end && global.end > relative.start {
                left = left.min(glyph_x.min(next));
                right = right.max(glyph_x.max(next));
            }
            pen = next;
        }
    }
    if !left.is_finite() || !right.is_finite() || right < left {
        return Err(fail("fallback body field has no measurable glyph interval"));
    }
    Ok((left, right))
}

pub(super) fn preview(
    page: &PdfPageBuilder,
    text: &str,
    width: f64,
    style: &TextStyle,
) -> Result<Vec<FontStackLinePreview>> {
    let FontFace::Fallback(id) = style.font else {
        return Err(fail("font preview requires an explicit fallback stack"));
    };
    let preferred = *page
        .font_stacks
        .get(id.0 as usize)
        .and_then(|members| members.first())
        .ok_or_else(|| fail("font stack is not registered on this page"))?;
    let lines = layout::prepare(page, text, width, style)?;
    let mut hashes = BTreeMap::new();
    let mut output = Vec::new();
    for line in lines {
        crate::cancel::check_current_cancel("authoring font preview")?;
        let plan = line
            .fallback
            .as_ref()
            .ok_or_else(|| fail("font preview lost fallback plan"))?;
        let mut runs = Vec::new();
        for run in &plan.runs {
            if let std::collections::btree_map::Entry::Vacant(e) = hashes.entry(run.font) {
                let bytes = page
                    .font_program(run.font)?
                    .ok_or_else(|| fail("preview font program missing"))?;
                e.insert(format!("{:x}", Sha256::digest(bytes)));
            }
            runs.push(FontStackRunPreview {
                logical_utf8_range: [run.range.start, run.range.end],
                font: run.font,
                font_sha256: hashes[&run.font].clone(),
                fallback: run.font != preferred,
                right_to_left: run.bidi.rtl,
                glyph_count: run.shaped.glyphs.len(),
                advance: run.advance,
            });
        }
        output.push(FontStackLinePreview {
            logical_utf8_range: [plan.range.start, plan.range.end],
            logical_text: line.logical,
            width: line.metrics.width(),
            advance: line.metrics.advance,
            ascent: line.metrics.ascent,
            descent: line.metrics.descent,
            logical_carrier: crate::fonts::logical_carrier::is_text(&plan.logical),
            runs,
        });
    }
    Ok(output)
}
