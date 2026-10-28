//! Per-glyph page geometry over the editing model's logical text.
//!
//! The glyphs are read with the same string-token scanner that builds
//! [`MultiRunRangeModel`], so every glyph carries the exact logical scalar
//! range that the editing transactions accept: a selection made against these
//! glyphs can be passed to them unchanged.

use super::*;
use crate::content::state::{concat_matrix, translate_matrix};
use std::collections::HashMap;

/// One painted source glyph, in PDF user space of its page.
#[derive(Debug, Clone)]
pub(crate) struct SourceGlyph {
    /// Logical text this glyph contributes (may be empty for glyphs inside an
    /// `/ActualText` span whose text was already emitted).
    pub text: String,
    /// `[start, end)` scalar range in the page's logical text.
    pub logical_range: [usize; 2],
    /// Corners: baseline start at the descender, baseline end at the
    /// descender, top end, top start (counter-clockwise for upright text).
    pub quad: [[f64; 2]; 4],
    /// Pen position before and after the glyph, on the baseline.
    pub baseline: [[f64; 2]; 2],
    pub font_resource: String,
    /// Font size scaled into user space (points on an unscaled page).
    pub font_size: f64,
    pub vertical: bool,
    pub invisible: bool,
}

/// A page's logical text with the geometry of its glyphs.
pub(crate) struct PageGlyphLayout {
    pub logical_text: String,
    /// The text is the editing model's exact logical text, so its offsets
    /// are accepted by the editing transactions. `false` when the editing
    /// model refuses the page (an ambiguous font encoding, say): the layout
    /// then comes from a tolerant scan that skips undecodable strings and is
    /// good for selection and hit-testing only.
    pub editable: bool,
    pub glyphs: Vec<SourceGlyph>,
}

/// One text-showing string's share of the logical text.
struct SpanText {
    logical_range: [usize; 2],
    text: String,
    source_text: String,
}

/// Glyph geometry for every page-content glyph whose position is known.
/// Glyphs whose text position cannot be tracked exactly (an unsupported
/// font encoding, a text-showing operator without a usable text matrix) are
/// absent; their logical text is still part of `logical_text`.
pub(crate) fn page_glyph_layout(
    engine: &ContentEngine,
    page_number: usize,
) -> Result<PageGlyphLayout> {
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut metrics = inline_text::Metrics::new(&resources, reader);
    let tokens = scan_page_text_string_tokens_with_metrics(
        reader,
        &page.contents,
        ScannedTextTokenState::default(),
        &mut metrics,
    )?;
    let (logical_text, spans, editable) =
        match analyze_multi_run_source(engine, &page, ScannedTextTokenState::default()) {
            Ok(model) => {
                let spans = model
                    .source_spans
                    .into_iter()
                    .map(|span| {
                        (
                            (
                                span.stream_object,
                                span.stream_generation,
                                span.byte_range[0],
                            ),
                            SpanText {
                                logical_range: span.logical_range,
                                text: span.text,
                                source_text: span.source_text,
                            },
                        )
                    })
                    .collect::<HashMap<_, _>>();
                (model.logical_text, spans, true)
            }
            Err(WellfriendError::UnsupportedFeature(_) | WellfriendError::MalformedPdf(_)) => {
                let (text, spans) = tolerant_spans(&tokens, &resources, reader);
                (text, spans, false)
            }
            Err(error) => return Err(error),
        };
    let mut resolvers: BTreeMap<String, (FontResolver, f64, f64)> = BTreeMap::new();
    let mut glyphs = Vec::new();
    for PageContentStringToken {
        object,
        generation,
        token,
        ..
    } in tokens
    {
        let Some(span) = spans.get(&(object, generation, token.token_start)) else {
            continue;
        };
        let Some(position) = token.source_position.as_ref() else {
            continue;
        };
        let Some(font) = resources.fonts.get(&token.font_name) else {
            continue;
        };
        if !resolvers.contains_key(&token.font_name) {
            let (ascent, descent) = vertical_extent(font, reader);
            resolvers.insert(
                token.font_name.clone(),
                (FontResolver::new(font, reader), ascent, descent),
            );
        }
        let (resolver, ascent, descent) = &resolvers[&token.font_name];
        let codes: Vec<_> = resolver
            .codes(&token.decoded)
            .filter_map(std::result::Result::ok)
            .map(|decoded| decoded.code)
            .collect();
        if codes.is_empty() {
            continue;
        }
        let text_to_page = concat_matrix(&position.matrix(), &token.source_ctm);
        let size = token.font_size;
        // Tz is stored in percent.
        let scaling = token.horizontal_scaling / 100.0;
        let rise = token.text_rise;
        let vertical = resolver.is_vertical();
        let user_size = size * text_to_page[2].hypot(text_to_page[3]);
        let span_length = span.logical_range[1] - span.logical_range[0];
        let decoded_texts: Vec<String> = codes
            .iter()
            .map(|code| resolver.decode_code(*code))
            .collect();
        // Without an /ActualText substitution the span's logical text is the
        // concatenation of the glyphs' decoded text; otherwise the span's
        // range is shared out over its glyphs in order.
        let direct = span.text == span.source_text
            && decoded_texts
                .iter()
                .map(|text| text.chars().count())
                .sum::<usize>()
                == span_length;
        let mut cursor = span.logical_range[0];
        let mut pen = 0.0_f64;
        let count = codes.len();
        for (index, (code, decoded)) in codes.iter().zip(&decoded_texts).enumerate() {
            let logical_range = if direct {
                let start = cursor;
                cursor += decoded.chars().count();
                [start, cursor]
            } else {
                [
                    span.logical_range[0] + index * span_length / count,
                    span.logical_range[0] + (index + 1) * span_length / count,
                ]
            };
            let word_space = code.is_word_space();
            let (corners, start, end, advance) = if vertical {
                let (w1y, _, _) = resolver.vertical_metrics_for_code(*code);
                let advance = crate::fonts::resolver::vertical_text_advance(
                    w1y,
                    size,
                    token.character_spacing,
                    token.word_spacing,
                    word_space,
                );
                let drawn = w1y / 1000.0 * size;
                (
                    [
                        [-0.5 * size, pen + drawn],
                        [0.5 * size, pen + drawn],
                        [0.5 * size, pen],
                        [-0.5 * size, pen],
                    ],
                    [0.0, pen],
                    [0.0, pen + drawn],
                    advance,
                )
            } else {
                let width = resolver.width_for_code(*code) / 1000.0 * size * scaling;
                let advance = (resolver.width_for_code(*code) / 1000.0 * size
                    + token.character_spacing
                    + if word_space { token.word_spacing } else { 0.0 })
                    * scaling;
                let bottom = rise + descent * size;
                let top = rise + ascent * size;
                (
                    [
                        [pen, bottom],
                        [pen + width, bottom],
                        [pen + width, top],
                        [pen, top],
                    ],
                    [pen, rise],
                    [pen + width, rise],
                    advance,
                )
            };
            pen += advance;
            let to_page = |point: [f64; 2]| -> [f64; 2] {
                let m = concat_matrix(&translate_matrix(point[0], point[1]), &text_to_page);
                [m[4], m[5]]
            };
            let quad = corners.map(to_page);
            if !quad.iter().flatten().all(|value| value.is_finite()) {
                continue;
            }
            glyphs.push(SourceGlyph {
                text: if direct {
                    decoded.clone()
                } else {
                    logical_text
                        .chars()
                        .skip(logical_range[0])
                        .take(logical_range[1] - logical_range[0])
                        .collect()
                },
                logical_range,
                quad,
                baseline: [to_page(start), to_page(end)],
                font_resource: token.font_name.clone(),
                font_size: user_size,
                vertical,
                invisible: token.text_render_mode == 3,
            });
        }
    }
    Ok(PageGlyphLayout {
        logical_text,
        editable,
        glyphs,
    })
}

/// Logical text from the scanned strings without the editing model's
/// refusals: a string whose font cannot decode it contributes nothing, and
/// an `/ActualText` value replaces the strings it covers once, as in the
/// editing model.
fn tolerant_spans(
    tokens: &[PageContentStringToken],
    resources: &PageResources,
    reader: &crate::PdfReader,
) -> (String, HashMap<(u32, u16, usize), SpanText>) {
    let mut logical_text = String::new();
    let mut offset = 0usize;
    let mut spans = HashMap::new();
    let mut emitted_actual_text = BTreeSet::<(u32, u16, usize, usize)>::new();
    let mut resolvers: BTreeMap<String, FontResolver> = BTreeMap::new();
    for page_token in tokens {
        let token = &page_token.token;
        let Some(font) = resources.fonts.get(&token.font_name) else {
            continue;
        };
        let resolver = resolvers
            .entry(token.font_name.clone())
            .or_insert_with(|| FontResolver::new(font, reader));
        let source_text = resolver
            .try_decode_string(&token.decoded)
            .unwrap_or_default();
        let text = match token.actual_text_sources.last() {
            Some(source) => {
                let key = (
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end,
                );
                if emitted_actual_text.insert(key) {
                    source.logical_text.to_string()
                } else {
                    String::new()
                }
            }
            None => source_text.clone(),
        };
        let start = offset;
        offset += text.chars().count();
        logical_text.push_str(&text);
        spans.insert(
            (page_token.object, page_token.generation, token.token_start),
            SpanText {
                logical_range: [start, offset],
                text,
                source_text,
            },
        );
    }
    (logical_text, spans)
}

/// FontDescriptor `/Ascent` and `/Descent` as fractions of the em, with the
/// customary 0.8 / -0.2 when absent or implausible.
fn vertical_extent(font: &crate::PdfDictionary, reader: &crate::PdfReader) -> (f64, f64) {
    let descriptor_owner = if font.get_name("Subtype") == Some("Type0") {
        crate::fonts::resolver::get_descendant_font(font, reader)
    } else {
        Some(font.clone())
    };
    let descriptor = descriptor_owner
        .as_ref()
        .and_then(|owner| owner.get("FontDescriptor"))
        .and_then(|value| reader.resolve(value.clone()).ok());
    let number = |key: &str| {
        descriptor
            .as_ref()
            .and_then(PdfObject::as_dict)
            .and_then(|dict| dict.get(key))
            .and_then(PdfObject::as_number)
            .filter(|value| value.is_finite())
    };
    let ascent = number("Ascent")
        .map(|value| value / 1000.0)
        .filter(|value| (0.3..=1.5).contains(value))
        .unwrap_or(0.8);
    let descent = number("Descent")
        .map(|value| value / 1000.0)
        .filter(|value| (-0.6..=0.0).contains(value) && *value < 0.0)
        .unwrap_or(-0.2);
    (ascent, descent)
}
