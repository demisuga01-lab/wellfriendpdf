//! One paragraph plan for measurement and authoring, retaining logical source,
//! paragraph-derived bidi/context and the registered font asset. No PDF work is
//! performed during planning and no page commands are appended until it succeeds.
use super::*;
use crate::fonts::line_layout::{
    measure_signed_run as measure_run, LineMetrics, PreparedParagraph,
};
use crate::fonts::shaper::LineBidi;
use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnedTextSpan {
    pub(super) range: Range<usize>,
    pub(super) element: u64,
}

pub(super) struct OwnedPaint {
    pub(super) element: u64,
    pub(super) command: PageCommand,
}

#[derive(Debug, Clone)]
pub(super) struct Line {
    pub logical: String,
    pub visual: String,
    pub metrics: LineMetrics,
    pub(super) bidi: Option<LineBidi>,
    pub(super) font_asset: Option<Arc<[u8]>>,
    pub(super) fallback: Option<Arc<super::fallback::ResolvedLine>>,
    /// Non-empty only when the logical line contains real tab layout. Each
    /// child is shaped without its separating U+0009 and painted at the exact
    /// resolved inline origin; the parent owns the complete logical text.
    pub(super) tab_segments: Vec<PositionedLineSegment>,
    /// Decorative leader/bar geometry, kept out of logical text and structure.
    pub(super) tab_decorations: Vec<crate::fonts::tab_stops::PositionedTabDecoration>,
}

#[derive(Debug, Clone)]
pub(super) struct PositionedLineSegment {
    /// UTF-8 range relative to the parent line's logical text. The separating
    /// U+0009 is deliberately excluded and remains a logical carrier.
    pub(super) range: Range<usize>,
    pub(super) origin: f64,
    pub(super) line: Box<Line>,
}

impl Line {
    pub fn aligned_x(&self, left: f64, width: f64, align: TextAlign) -> f64 {
        left + self.metrics.left_pad
            + match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (width - self.metrics.width()) / 2.0,
                TextAlign::Right => width - self.metrics.width(),
            }
    }

    pub fn occupied_height(&self, line_height: f64) -> f64 {
        line_height.max(self.metrics.ascent + self.metrics.descent)
    }

    pub fn command(&self, x: f64, y: f64, style: &TextStyle) -> Result<PageCommand> {
        if !self.tab_segments.is_empty() || !self.tab_decorations.is_empty() {
            validate_size(style)?;
            if !x.is_finite() || !y.is_finite() {
                return Err(WellfriendError::invalid_input(
                    "authoring text position must be finite",
                ));
            }
            let mut runs = self.tab_decoration_commands(x, y, style);
            for segment in &self.tab_segments {
                if segment.line.visual.is_empty() {
                    continue;
                }
                match segment.line.command(
                    x + segment.origin + segment.line.metrics.left_pad,
                    y,
                    style,
                )? {
                    command @ PageCommand::Text { .. } => runs.push(command),
                    PageCommand::TextGroup {
                        runs: child_runs, ..
                    } => runs.extend(child_runs),
                    PageCommand::LogicalBreak { .. } => {}
                    _ => {
                        return Err(WellfriendError::invalid_input(
                            "tabbed authoring produced a non-text child command",
                        ))
                    }
                }
            }
            if runs.iter().all(|command| {
                matches!(
                    command,
                    PageCommand::BeginArtifact
                        | PageCommand::EndArtifact
                        | PageCommand::Path { .. }
                )
            }) && crate::fonts::logical_carrier::is_text(&self.logical)
            {
                runs.push(PageCommand::LogicalBreak {
                    text: self.logical.clone(),
                    x,
                    y,
                    size: style.size,
                });
            }
            if runs.is_empty() {
                return Ok(PageCommand::LogicalBreak {
                    text: self.logical.clone(),
                    x,
                    y,
                    size: style.size,
                });
            }
            return Ok(PageCommand::TextGroup {
                logical_text: self.logical.clone(),
                runs,
            });
        }
        validate_single_line(&self.visual, x, y, style)?;
        if crate::fonts::logical_carrier::is_text(&self.logical) {
            return Ok(PageCommand::LogicalBreak {
                text: self.logical.clone(),
                x,
                y,
                size: style.size,
            });
        }
        if let Some(fallback) = &self.fallback {
            return fallback.command(x, y, style);
        }
        Ok(PageCommand::Text {
            text: self.visual.clone(),
            x,
            y,
            style: style.clone(),
            bidi: self.bidi.clone(),
            logical_text: (self.logical != self.visual).then(|| self.logical.clone()),
            suppress_actual_text: false,
            font_asset: self.font_asset.clone(),
            shaped: None,
        })
    }

    fn tab_decoration_commands(&self, x: f64, y: f64, style: &TextStyle) -> Vec<PageCommand> {
        use crate::fonts::tab_stops::{PositionedTabDecoration, TabLeader};
        let width = (style.size / 18.0).clamp(0.35, 1.5);
        let mut paths = Vec::new();
        for decoration in &self.tab_decorations {
            let (path, graphics) = match *decoration {
                PositionedTabDecoration::Leader { from, to, leader } => {
                    let inset = width.max(style.size * 0.08);
                    if to - from <= inset * 2.0 || leader == TabLeader::None {
                        continue;
                    }
                    let baseline = y + style.size * 0.08;
                    let graphics = match leader {
                        TabLeader::Dots => GraphicsStyle::stroke(style.fill.clone(), width)
                            .line_cap(LineCap::Round)
                            .dash(vec![width * 0.01, style.size * 0.30], 0.0),
                        TabLeader::Dashes => GraphicsStyle::stroke(style.fill.clone(), width)
                            .dash(vec![style.size * 0.34, style.size * 0.22], 0.0),
                        TabLeader::Solid => GraphicsStyle::stroke(style.fill.clone(), width),
                        TabLeader::None => unreachable!(),
                    };
                    (
                        PathBuilder::new()
                            .move_to(x + from + inset, baseline)
                            .line_to(x + to - inset, baseline),
                        graphics,
                    )
                }
                PositionedTabDecoration::Bar { position } => (
                    PathBuilder::new()
                        .move_to(x + position, y - self.metrics.descent)
                        .line_to(x + position, y + self.metrics.ascent),
                    GraphicsStyle::stroke(style.fill.clone(), width),
                ),
            };
            paths.push(PageCommand::Path {
                path,
                style: graphics,
            });
        }
        if paths.is_empty() {
            return paths;
        }
        let mut output = Vec::with_capacity(paths.len() + 2);
        output.push(PageCommand::BeginArtifact);
        output.extend(paths);
        output.push(PageCommand::EndArtifact);
        output
    }

    /// Emit one semantic owner per exact logical range while retaining the
    /// shaping result of the complete line. A requested boundary that divides
    /// an OpenType cluster fails closed instead of reshaping a substring.
    pub(super) fn owned_commands(
        &self,
        page: &PdfPageBuilder,
        x: f64,
        y: f64,
        style: &TextStyle,
        spans: &[OwnedTextSpan],
    ) -> Result<Vec<PageCommand>> {
        validate_owned_spans(&self.logical, spans)?;
        if !self.tab_segments.is_empty() || !self.tab_decorations.is_empty() {
            validate_size(style)?;
            if !x.is_finite() || !y.is_finite() {
                return Err(WellfriendError::invalid_input(
                    "authoring text position must be finite",
                ));
            }
            let mut output = self.tab_decoration_commands(x, y, style);
            let mut cursor = 0usize;
            for segment in &self.tab_segments {
                if segment.range.start < cursor || segment.range.end > self.logical.len() {
                    return Err(WellfriendError::invalid_input(
                        "positioned tab field has invalid semantic source provenance",
                    ));
                }
                append_owned_logical_carriers(
                    &mut output,
                    &self.logical,
                    spans,
                    cursor..segment.range.start,
                    x,
                    y,
                    style.size,
                )?;
                let local_spans = intersect_owned_spans(spans, segment.range.clone());
                if !segment.range.is_empty() {
                    output.extend(segment.line.owned_commands(
                        page,
                        x + segment.origin + segment.line.metrics.left_pad,
                        y,
                        style,
                        &local_spans,
                    )?);
                }
                cursor = segment.range.end;
            }
            append_owned_logical_carriers(
                &mut output,
                &self.logical,
                spans,
                cursor..self.logical.len(),
                x,
                y,
                style.size,
            )?;
            return Ok(output);
        }
        validate_single_line(&self.visual, x, y, style)?;
        if let Some(fallback) = &self.fallback {
            return fallback.owned_commands(x, y, style, spans);
        }
        if crate::fonts::logical_carrier::is_text(&self.logical) {
            return wrap_owned_paints(&self.logical, spans, Vec::new(), x, y, style.size);
        }
        let visible = self.visual.as_str();
        let paints = match style.font {
            FontFace::Standard(font) => {
                let mut paints = Vec::new();
                for span in spans {
                    let start = span.range.start.min(visible.len());
                    let end = span.range.end.min(visible.len());
                    if start >= end {
                        continue;
                    }
                    let prefix = standard_metrics(font, &visible[..start], style.size)?.advance;
                    paints.push(OwnedPaint {
                        element: span.element,
                        command: PageCommand::Text {
                            text: visible[start..end].to_owned(),
                            x: x + prefix,
                            y,
                            style: style.clone(),
                            bidi: None,
                            logical_text: None,
                            suppress_actual_text: true,
                            font_asset: self.font_asset.clone(),
                            shaped: None,
                        },
                    });
                }
                paints
            }
            FontFace::BuiltinUnicode | FontFace::Custom(_) => {
                let bytes = page
                    .font_program(style.font)?
                    .ok_or_else(|| WellfriendError::invalid_input("missing authored font"))?;
                let bidi = self.bidi.as_ref().ok_or_else(|| {
                    WellfriendError::invalid_input("authored semantic line lost bidi context")
                })?;
                let shaped = TextShaper::shape_resolved(bytes, visible, bidi, &Default::default())?;
                shaped_owned_paints(
                    visible,
                    spans,
                    x,
                    y,
                    style,
                    self.font_asset.clone(),
                    bidi,
                    &shaped,
                )?
            }
            FontFace::Fallback(_) => unreachable!("fallback handled above"),
        };
        wrap_owned_paints(&self.logical, spans, paints, x, y, style.size)
    }
}

fn intersect_owned_spans(spans: &[OwnedTextSpan], range: Range<usize>) -> Vec<OwnedTextSpan> {
    spans
        .iter()
        .filter_map(|span| {
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            (start < end).then(|| OwnedTextSpan {
                range: start - range.start..end - range.start,
                element: span.element,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn append_owned_logical_carriers(
    output: &mut Vec<PageCommand>,
    logical: &str,
    spans: &[OwnedTextSpan],
    range: Range<usize>,
    x: f64,
    y: f64,
    size: f64,
) -> Result<()> {
    if range.is_empty() {
        return Ok(());
    }
    for span in spans {
        let start = span.range.start.max(range.start);
        let end = span.range.end.min(range.end);
        if start >= end {
            continue;
        }
        let carrier = &logical[start..end];
        if !crate::fonts::logical_carrier::is_text(carrier) {
            return Err(WellfriendError::invalid_input(
                "positioned tab gap contains paintable text without a field plan",
            ));
        }
        output.push(PageCommand::BeginStructure(span.element));
        output.push(PageCommand::LogicalBreak {
            text: carrier.to_owned(),
            x,
            y,
            size,
        });
        output.push(PageCommand::EndStructure(span.element));
    }
    Ok(())
}

fn validate_owned_spans(text: &str, spans: &[OwnedTextSpan]) -> Result<()> {
    let mut cursor = 0usize;
    let mut identities = BTreeSet::new();
    for span in spans {
        if span.range.start != cursor
            || span.range.start >= span.range.end
            || span.range.end > text.len()
            || !text.is_char_boundary(span.range.start)
            || !text.is_char_boundary(span.range.end)
            || !identities.insert(span.element)
        {
            return Err(WellfriendError::invalid_input(
                "authored semantic spans must be unique, contiguous UTF-8 ranges",
            ));
        }
        cursor = span.range.end;
    }
    if cursor != text.len() {
        return Err(WellfriendError::invalid_input(
            "authored semantic spans do not cover their complete line",
        ));
    }
    Ok(())
}

pub(super) fn owner_for_cluster(spans: &[OwnedTextSpan], cluster: Range<usize>) -> Result<u64> {
    spans
        .iter()
        .find(|span| cluster.start >= span.range.start && cluster.end <= span.range.end)
        .map(|span| span.element)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "footnote marker boundary divides a shaped text cluster".into(),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn shaped_owned_paints(
    text: &str,
    spans: &[OwnedTextSpan],
    x: f64,
    y: f64,
    style: &TextStyle,
    font_asset: Option<Arc<[u8]>>,
    bidi: &LineBidi,
    shaped: &crate::fonts::ShapedRun,
) -> Result<Vec<OwnedPaint>> {
    let boundaries = super::cluster_boundaries(text, shaped);
    let mut paints = Vec::new();
    let mut pen = x;
    let mut cursor = 0usize;
    let mut seen = BTreeSet::new();
    while cursor < shaped.glyphs.len() {
        let cluster = shaped.glyphs[cursor].cluster;
        if !seen.insert(cluster) {
            return Err(WellfriendError::UnsupportedFeature(
                "noncontiguous shaped cluster cannot be semantically partitioned".into(),
            ));
        }
        let first_pen = pen;
        let mut end_glyph = cursor;
        let mut glyphs = Vec::new();
        while end_glyph < shaped.glyphs.len() && shaped.glyphs[end_glyph].cluster == cluster {
            let mut glyph = shaped.glyphs[end_glyph].clone();
            glyph.cluster = 0;
            pen += glyph.advance * style.size / 1000.0;
            glyphs.push(glyph);
            end_glyph += 1;
        }
        let start = usize::try_from(cluster)
            .map_err(|_| WellfriendError::invalid_input("shaped cluster offset overflow"))?;
        let end = boundaries
            .iter()
            .copied()
            .find(|candidate| *candidate > start)
            .unwrap_or(text.len());
        if start >= end || end > text.len() || !text.is_char_boundary(start) {
            return Err(WellfriendError::invalid_input(
                "invalid shaped cluster boundary",
            ));
        }
        let owner = owner_for_cluster(spans, start..end)?;
        let resolved = bidi.slice(
            text,
            start..end,
            shaped.direction == crate::fonts::TextDirection::RightToLeft,
        )?;
        paints.push(OwnedPaint {
            element: owner,
            command: PageCommand::Text {
                text: text[start..end].to_owned(),
                x: first_pen,
                y,
                style: style.clone(),
                bidi: Some(resolved),
                logical_text: None,
                suppress_actual_text: true,
                font_asset: font_asset.clone(),
                shaped: Some(Arc::new(crate::fonts::ShapedRun {
                    glyphs,
                    direction: shaped.direction,
                    used_complex_shaping: true,
                })),
            },
        });
        cursor = end_glyph;
    }
    if !pen.is_finite() {
        return Err(WellfriendError::invalid_input(
            "authored semantic glyph position overflow",
        ));
    }
    Ok(paints)
}

pub(super) fn wrap_owned_paints(
    logical: &str,
    spans: &[OwnedTextSpan],
    mut paints: Vec<OwnedPaint>,
    x: f64,
    y: f64,
    size: f64,
) -> Result<Vec<PageCommand>> {
    let mut output = Vec::new();
    for span in spans {
        output.push(PageCommand::BeginStructure(span.element));
        let mut indices = paints
            .iter()
            .enumerate()
            .filter_map(|(index, paint)| (paint.element == span.element).then_some(index))
            .collect::<Vec<_>>();
        if let Some(first) = indices.first().copied() {
            let PageCommand::Text {
                logical_text,
                suppress_actual_text,
                ..
            } = &mut paints[first].command
            else {
                return Err(WellfriendError::invalid_input(
                    "authored semantic paint is not text",
                ));
            };
            *logical_text = Some(logical[span.range.clone()].to_owned());
            *suppress_actual_text = false;
            for (ordinal, index) in indices.drain(..).enumerate() {
                if ordinal != 0 {
                    let PageCommand::Text { logical_text, .. } = &mut paints[index].command else {
                        return Err(WellfriendError::invalid_input(
                            "authored semantic paint is not text",
                        ));
                    };
                    // The first paint owns the span's complete logical value.
                    // Retained cluster paints are visual-only; make that
                    // explicit in the in-memory command model as well as in
                    // the serialized suppress_actual_text flag.
                    *logical_text = Some(String::new());
                }
                output.push(paints[index].command.clone());
            }
        } else {
            let carrier = &logical[span.range.clone()];
            if !crate::fonts::logical_carrier::is_text(carrier) {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored semantic span has no paintable or logical carrier".into(),
                ));
            }
            output.push(PageCommand::LogicalBreak {
                text: carrier.to_owned(),
                x,
                y,
                size,
            });
        }
        output.push(PageCommand::EndStructure(span.element));
    }
    if paints
        .iter()
        .any(|paint| !spans.iter().any(|span| span.element == paint.element))
    {
        return Err(WellfriendError::invalid_input(
            "authored semantic paint has no registered owner",
        ));
    }
    Ok(output)
}

fn validate_size(style: &TextStyle) -> Result<()> {
    if !style.size.is_finite() || style.size <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "authoring font size must be finite and positive",
        ));
    }
    Ok(())
}

pub(super) fn validate_single_line(text: &str, x: f64, y: f64, style: &TextStyle) -> Result<()> {
    crate::cancel::check_current_cancel("authoring text validation")?;
    validate_size(style)?;
    if text.len() > 16 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "authoring text exceeds 16 MiB".into(),
        ));
    }
    if !x.is_finite() || !y.is_finite() {
        return Err(WellfriendError::invalid_input(
            "authoring text position must be finite",
        ));
    }
    for (index, ch) in text.chars().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("authoring text validation")?;
        }
        if crate::fonts::hard_break::is_hard_break(ch) {
            return Err(WellfriendError::invalid_input(
                "authoring draw_text/text_width require one line; use draw_paragraph for hard separators"));
        }
        if ch == '\t' {
            return Err(WellfriendError::invalid_input(
                "authoring draw_text/text_width do not define tab stops; use draw_paragraph with ParagraphStyle::tab_stops"));
        }
    }
    Ok(())
}

fn standard_metrics(font: StandardFont, text: &str, size: f64) -> Result<LineMetrics> {
    let bytes = get_fallback_font(font.fallback_font_name()).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("authoring Standard-14 metrics unavailable".into())
    })?;
    let metrics = TrueTypeMetrics::parse(bytes)?;
    let mut advance = 0.0;
    // These scalars have an explicit separate logical carrier, not Standard-14
    // character codes. Do not measure a font-dependent visible placeholder.
    let painted = if crate::fonts::logical_carrier::is_text(text) {
        ""
    } else {
        text
    };
    for (index, ch) in painted.chars().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("authoring standard metrics")?;
        }
        encode_standard_char(font, ch).ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "authoring: character {ch:?} is not encodable in {}",
                font.base_font_name()
            ))
        })?;
        advance += metrics.glyph_width(ch) * size / 1000.0;
    }
    checked_metrics(LineMetrics {
        advance,
        left_pad: 0.0,
        right_pad: 0.0,
        ascent: (metrics.ascender * size / 1000.0).max(0.0),
        descent: (-metrics.descender * size / 1000.0).max(0.0),
    })
}

pub(super) fn checked_metrics(metrics: LineMetrics) -> Result<LineMetrics> {
    if ![
        metrics.width(),
        metrics.left_pad,
        metrics.right_pad,
        metrics.ascent,
        metrics.descent,
        metrics.ascent + metrics.descent,
    ]
    .iter()
    .all(|n| n.is_finite() && *n >= 0.0)
        || !metrics.advance.is_finite()
    {
        return Err(WellfriendError::invalid_input(
            "invalid authored line metrics",
        ));
    }
    Ok(metrics)
}

pub(super) fn text_width(page: &PdfPageBuilder, text: &str, style: &TextStyle) -> Result<f64> {
    validate_single_line(text, 0.0, 0.0, style)?;
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::width(page, text, style);
    }
    if let FontFace::Standard(font) = style.font {
        return Ok(standard_metrics(font, text, style.size)?.width());
    }
    let bytes = page
        .font_program(style.font)?
        .ok_or_else(|| WellfriendError::invalid_input("missing authoring font program"))?;
    let run = TextShaper::shape(bytes, text, ShapeOptions::default())?;
    if crate::fonts::shaper::has_missing_glyphs(bytes, text, &run)? {
        return Err(WellfriendError::UnsupportedFeature(
            "authoring font lacks shaped glyph coverage".into(),
        ));
    }
    Ok(checked_metrics(measure_run(bytes, &run, style.size)?)?.width())
}

fn measure_nonfallback_range(
    page: &PdfPageBuilder,
    text: &str,
    range: Range<usize>,
    style: &TextStyle,
    paragraph: &PreparedParagraph<'_>,
) -> Result<LineMetrics> {
    match style.font {
        FontFace::Standard(font) => standard_metrics(font, &text[range], style.size),
        FontFace::BuiltinUnicode | FontFace::Custom(_) => {
            let bytes = page
                .font_program(style.font)?
                .ok_or_else(|| WellfriendError::invalid_input("missing paragraph font"))?;
            let bidi = paragraph.bidi.line(range.clone())?;
            let run = TextShaper::shape_resolved(
                bytes,
                &text[range.clone()],
                &bidi,
                &Default::default(),
            )?;
            if crate::fonts::shaper::has_missing_glyphs(bytes, &text[range], &run)? {
                return Err(WellfriendError::UnsupportedFeature(
                    "authoring font lacks shaped glyph coverage".into(),
                ));
            }
            checked_metrics(measure_run(bytes, &run, style.size)?)
        }
        FontFace::Fallback(_) => Err(WellfriendError::invalid_input(
            "fallback range reached the single-font authoring planner",
        )),
    }
}

fn prepare_nonfallback_range(
    page: &PdfPageBuilder,
    text: &str,
    range: Range<usize>,
    style: &TextStyle,
    paragraph: &PreparedParagraph<'_>,
) -> Result<Line> {
    let logical = &text[range.clone()];
    let metrics = measure_nonfallback_range(page, text, range.clone(), style, paragraph)?;
    let bidi = match style.font {
        FontFace::Standard(_) => None,
        FontFace::BuiltinUnicode | FontFace::Custom(_) => Some(paragraph.bidi.line(range.clone())?),
        FontFace::Fallback(_) => unreachable!("validated by caller"),
    };
    Ok(Line {
        logical: logical.to_owned(),
        visual: logical.to_owned(),
        metrics,
        bidi,
        font_asset: page.custom_font_asset(style.font),
        fallback: None,
        tab_segments: Vec::new(),
        tab_decorations: Vec::new(),
    })
}

pub(super) fn tabbed_metrics(
    plan: &crate::fonts::tab_stops::TabLinePlan,
    segments: &[PositionedLineSegment],
) -> Result<LineMetrics> {
    let ascent = segments
        .iter()
        .map(|segment| segment.line.metrics.ascent)
        .fold(0.0, f64::max);
    let descent = segments
        .iter()
        .map(|segment| segment.line.metrics.descent)
        .fold(0.0, f64::max);
    checked_metrics(LineMetrics {
        advance: plan.advance,
        left_pad: 0.0,
        right_pad: 0.0,
        ascent,
        descent,
    })
}

/// Prepare a paragraph whose U+0009 characters are positioned against exact
/// tab stops. Tabs remain in logical `/ActualText`; they are never sent to a
/// font shaper or expanded into guessed spaces.
pub(super) fn prepare_with_tabs(
    page: &PdfPageBuilder,
    text: &str,
    width: f64,
    style: &TextStyle,
    tabs: &crate::fonts::tab_stops::TabStops,
) -> Result<Vec<Line>> {
    if !text.contains('\t') {
        return prepare(page, text, width, style);
    }
    validate_size(style)?;
    if !width.is_finite() || width <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "authoring paragraph width must be finite and positive",
        ));
    }
    tabs.validate()?;
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::prepare_with_tabs(page, text, width, style, tabs);
    }
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
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
                Ok(measure_nonfallback_range(page, text, segment, style, &paragraph)?.width())
            })?;
        Ok(plan.advance)
    };
    let measured = paragraph.break_lines_measured(0, width, 100_000, measure_line)?;
    if measured.last().map_or(0, |line| line.bytes.end) != text.len() {
        return Err(WellfriendError::ResourceLimit(
            "authoring tabbed paragraph line budget exhausted".into(),
        ));
    }
    let mut output = Vec::with_capacity(measured.len());
    for measured_line in measured {
        crate::cancel::check_current_cancel("authoring tabbed final line plan")?;
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
            |segment| {
                Ok(measure_nonfallback_range(page, text, segment, style, &paragraph)?.width())
            },
        )?;
        if plan.advance > width + 1e-7 {
            return Err(WellfriendError::UnsupportedFeature(
                "resolved tab fields exceed the authoring paragraph width".into(),
            ));
        }
        let mut segments = Vec::with_capacity(plan.segments.len());
        for segment in &plan.segments {
            if segment.range.is_empty() {
                continue;
            }
            segments.push(PositionedLineSegment {
                range: segment.range.start - logical_range.start
                    ..segment.range.end - logical_range.start,
                origin: segment.origin,
                line: Box::new(prepare_nonfallback_range(
                    page,
                    text,
                    segment.range.clone(),
                    style,
                    &paragraph,
                )?),
            });
        }
        let metrics = tabbed_metrics(&plan, &segments)?;
        output.push(Line {
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

pub(super) fn prepare(
    page: &PdfPageBuilder,
    text: &str,
    width: f64,
    style: &TextStyle,
) -> Result<Vec<Line>> {
    if text.contains('\t') {
        return prepare_with_tabs(
            page,
            text,
            width,
            style,
            &crate::fonts::tab_stops::TabStops::default(),
        );
    }
    validate_size(style)?;
    if !width.is_finite() || width <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "authoring paragraph width must be finite and positive",
        ));
    }
    if !width.is_finite() || width <= 0.0 {
        return Err(WellfriendError::invalid_input(
            "authoring paragraph width must be finite and positive",
        ));
    }
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::prepare(page, text, width, style);
    }
    let bytes = page.font_program(style.font)?;
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
    let measure = |range: std::ops::Range<usize>| -> Result<LineMetrics> {
        let visual = text[range.clone()].trim_end_matches(crate::fonts::hard_break::is_hard_break);
        match style.font {
            FontFace::Standard(font) => standard_metrics(font, visual, style.size),
            _ => {
                let bytes = bytes
                    .ok_or_else(|| WellfriendError::invalid_input("missing paragraph font"))?;
                let bidi = paragraph
                    .bidi
                    .line(range.start..range.start + visual.len())?;
                let run = TextShaper::shape_resolved(bytes, visual, &bidi, &Default::default())?;
                checked_metrics(measure_run(bytes, &run, style.size)?)
            }
        }
    };
    let measured =
        paragraph.break_lines_measured(0, width, 100_000, |range| Ok(measure(range)?.width()))?;
    if measured.last().map_or(0, |line| line.bytes.end) != text.len() {
        return Err(WellfriendError::ResourceLimit(
            "authoring paragraph line budget exhausted".into(),
        ));
    }
    let mut lines = Vec::with_capacity(measured.len());
    for line in measured {
        crate::cancel::check_current_cancel("authoring final line plan")?;
        let logical = &text[line.bytes.clone()];
        let visual = logical.trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let bidi = if let Some(bytes) = bytes {
            let resolved = paragraph
                .bidi
                .line(line.bytes.start..line.bytes.start + visual.len())?;
            let shaped = TextShaper::shape_resolved(bytes, visual, &resolved, &Default::default())?;
            if crate::fonts::shaper::has_missing_glyphs(bytes, visual, &shaped)? {
                return Err(WellfriendError::UnsupportedFeature(
                    "authoring font lacks shaped glyph coverage".into(),
                ));
            }
            Some(resolved)
        } else {
            None
        };
        let metrics = measure(line.bytes)?;
        lines.push(Line {
            logical: logical.into(),
            visual: visual.into(),
            metrics,
            bidi,
            font_asset: page.custom_font_asset(style.font),
            fallback: None,
            tab_segments: Vec::new(),
            tab_decorations: Vec::new(),
        });
    }
    Ok(lines)
}

/// Shape caller-owned exact logical ranges with whole-paragraph bidi and
/// fallback context. This does not select new line breaks.
pub(super) fn prepare_ranges(
    page: &PdfPageBuilder,
    text: &str,
    ranges: &[std::ops::Range<usize>],
    style: &TextStyle,
) -> Result<Vec<Line>> {
    validate_size(style)?;
    let mut cursor = 0usize;
    for range in ranges {
        if range.start != cursor
            || range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(WellfriendError::invalid_input(
                "authored exact line ranges must be contiguous UTF-8 boundaries",
            ));
        }
        cursor = range.end;
    }
    if cursor != text.len() {
        return Err(WellfriendError::invalid_input(
            "authored exact line ranges do not cover their paragraph",
        ));
    }
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::prepare_ranges(page, text, ranges, style);
    }
    let bytes = page.font_program(style.font)?;
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
    let mut output = Vec::with_capacity(ranges.len());
    for range in ranges {
        crate::cancel::check_current_cancel("authoring exact line shaping")?;
        let logical = &text[range.clone()];
        let visual = logical.trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let (metrics, bidi) = match style.font {
            FontFace::Standard(font) => (standard_metrics(font, visual, style.size)?, None),
            _ => {
                let bytes = bytes
                    .ok_or_else(|| WellfriendError::invalid_input("missing paragraph font"))?;
                let bidi = paragraph
                    .bidi
                    .line(range.start..range.start + visual.len())?;
                let shaped = TextShaper::shape_resolved(bytes, visual, &bidi, &Default::default())?;
                if crate::fonts::shaper::has_missing_glyphs(bytes, visual, &shaped)? {
                    return Err(WellfriendError::UnsupportedFeature(
                        "authoring font lacks shaped glyph coverage".into(),
                    ));
                }
                (
                    checked_metrics(measure_run(bytes, &shaped, style.size)?)?,
                    Some(bidi),
                )
            }
        };
        output.push(Line {
            logical: logical.into(),
            visual: visual.into(),
            metrics,
            bidi,
            font_asset: page.custom_font_asset(style.font),
            fallback: None,
            tab_segments: Vec::new(),
            tab_decorations: Vec::new(),
        });
    }
    Ok(output)
}

pub(super) fn prepare_ranges_with_tabs(
    page: &PdfPageBuilder,
    text: &str,
    ranges: &[Range<usize>],
    style: &TextStyle,
    tabs: &crate::fonts::tab_stops::TabStops,
) -> Result<Vec<Line>> {
    if !text.contains('\t') {
        return prepare_ranges(page, text, ranges, style);
    }
    validate_size(style)?;
    let mut cursor = 0usize;
    for range in ranges {
        if range.start != cursor
            || range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(WellfriendError::invalid_input(
                "authored exact tabbed line ranges must be contiguous UTF-8 boundaries",
            ));
        }
        cursor = range.end;
    }
    if cursor != text.len() {
        return Err(WellfriendError::invalid_input(
            "authored exact tabbed line ranges do not cover their paragraph",
        ));
    }
    tabs.validate()?;
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::prepare_ranges_with_tabs(page, text, ranges, style, tabs);
    }
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
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
            |segment| {
                Ok(measure_nonfallback_range(page, text, segment, style, &paragraph)?.width())
            },
        )?;
        let mut segments = Vec::with_capacity(plan.segments.len());
        for segment in &plan.segments {
            if segment.range.is_empty() {
                continue;
            }
            segments.push(PositionedLineSegment {
                range: segment.range.start - logical_range.start
                    ..segment.range.end - logical_range.start,
                origin: segment.origin,
                line: Box::new(prepare_nonfallback_range(
                    page,
                    text,
                    segment.range.clone(),
                    style,
                    &paragraph,
                )?),
            });
        }
        let metrics = tabbed_metrics(&plan, &segments)?;
        output.push(Line {
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

/// Horizontal advance-cell bounds for one exact source range within a prepared
/// line. Coordinates are relative to the line's emitted text origin.
pub(super) fn range_x_bounds(
    page: &PdfPageBuilder,
    text: &str,
    line: std::ops::Range<usize>,
    field: std::ops::Range<usize>,
    style: &TextStyle,
) -> Result<(f64, f64)> {
    if line.start > field.start
        || field.start >= field.end
        || field.end > line.end
        || line.end > text.len()
        || !text.is_char_boundary(line.start)
        || !text.is_char_boundary(line.end)
        || !text.is_char_boundary(field.start)
        || !text.is_char_boundary(field.end)
    {
        return Err(WellfriendError::invalid_input(
            "invalid authored field geometry range",
        ));
    }
    if matches!(style.font, FontFace::Fallback(_)) {
        return super::fallback::range_x_bounds(page, text, line, field, style);
    }
    let visible_end = line.end
        - text[line.clone()]
            .chars()
            .rev()
            .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
            .map(char::len_utf8)
            .sum::<usize>();
    if field.end > visible_end {
        return Err(WellfriendError::invalid_input(
            "body field overlaps a nonpainting hard separator",
        ));
    }
    if let FontFace::Standard(font) = style.font {
        let prefix = standard_metrics(font, &text[line.start..field.start], style.size)?.width();
        let width = standard_metrics(font, &text[field.clone()], style.size)?.width();
        return Ok((prefix, prefix + width));
    }
    let bytes = page
        .font_program(style.font)?
        .ok_or_else(|| WellfriendError::invalid_input("missing body-field font"))?;
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
    let bidi = paragraph.bidi.line(line.start..visible_end)?;
    let visual = &text[line.start..visible_end];
    let shaped = TextShaper::shape_resolved(bytes, visual, &bidi, &Default::default())?;
    let relative = field.start - line.start..field.end - line.start;
    let mut clusters = shaped
        .glyphs
        .iter()
        .filter_map(|glyph| usize::try_from(glyph.cluster).ok())
        .filter(|offset| *offset <= visual.len() && visual.is_char_boundary(*offset))
        .collect::<Vec<_>>();
    clusters.push(0);
    clusters.push(visual.len());
    clusters.sort_unstable();
    clusters.dedup();
    let mut pen = 0.0f64;
    let mut left = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    for glyph in &shaped.glyphs {
        let start = usize::try_from(glyph.cluster)
            .map_err(|_| WellfriendError::invalid_input("body-field glyph cluster overflow"))?;
        let end = clusters
            .iter()
            .copied()
            .find(|offset| *offset > start)
            .unwrap_or(visual.len());
        let next = pen + glyph.advance * style.size / 1000.0;
        let glyph_x = pen + glyph.offset_x * style.size / 1000.0;
        if start < relative.end && end > relative.start {
            left = left.min(glyph_x.min(next));
            right = right.max(glyph_x.max(next));
        }
        pen = next;
    }
    if !left.is_finite() || !right.is_finite() || right < left {
        return Err(WellfriendError::invalid_input(
            "body field has no measurable glyph interval",
        ));
    }
    Ok((left, right))
}

pub(super) fn range_x_bounds_with_tabs(
    page: &PdfPageBuilder,
    text: &str,
    line: Range<usize>,
    field: Range<usize>,
    style: &TextStyle,
    tabs: &crate::fonts::tab_stops::TabStops,
) -> Result<(f64, f64)> {
    if !text[line.clone()].contains('\t') {
        return range_x_bounds(page, text, line, field, style);
    }
    let visible_end = line.end
        - text[line.clone()]
            .chars()
            .rev()
            .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
            .map(char::len_utf8)
            .sum::<usize>();
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default())?;
    let plan =
        crate::fonts::tab_stops::plan_line(text, line.start..visible_end, tabs, |segment| {
            if matches!(style.font, FontFace::Fallback(_)) {
                Ok(super::fallback::range_metrics(page, text, segment, style)?.width())
            } else {
                Ok(measure_nonfallback_range(page, text, segment, style, &paragraph)?.width())
            }
        })?;
    let segment = plan
        .segments
        .iter()
        .find(|segment| field.start >= segment.range.start && field.end <= segment.range.end)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "body field cannot cross a positioned tab separator".into(),
            )
        })?;
    let metrics = if matches!(style.font, FontFace::Fallback(_)) {
        super::fallback::range_metrics(page, text, segment.range.clone(), style)?
    } else {
        measure_nonfallback_range(page, text, segment.range.clone(), style, &paragraph)?
    };
    let (left, right) = range_x_bounds(page, text, segment.range.clone(), field, style)?;
    Ok((
        segment.origin + metrics.left_pad + left,
        segment.origin + metrics.left_pad + right,
    ))
}
