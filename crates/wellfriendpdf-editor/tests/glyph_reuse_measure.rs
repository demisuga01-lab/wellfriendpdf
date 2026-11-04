//! Fallback-rate measurement for edits that only use glyphs the source font
//! already has. Run on the build machine:
//!
//! ```text
//! WF_GLYPH_REUSE_PDFS=a.pdf,b.pdf WF_GLYPH_REUSE_PAGES=3 \
//!   cargo test --release -p wellfriendpdf-editor --test glyph_reuse_measure -- --ignored --nocapture
//! ```
//!
//! For every editable paragraph of the first pages, one word of four or
//! more letters is replaced by its reverse (characters the paragraph already
//! draws). Each edit is counted as kept in the source font, drawn with a
//! fallback font (the report adds a font), or refused; per source font type.

use std::collections::BTreeMap;

use wellfriendpdf_editor::{apply_operation, fonts::page_fonts, EditorOperation, MemoryFonts};
use wellfriendpdf_engine::PageTextLayout;

const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");

#[derive(Default, Debug)]
struct Counts {
    source: usize,
    fallback: usize,
    refused: usize,
}

#[test]
#[ignore = "measurement over real PDFs; run with --ignored"]
fn measure_glyph_reuse() {
    let paths = std::env::var("WF_GLYPH_REUSE_PDFS").unwrap_or_default();
    let max_pages: usize = std::env::var("WF_GLYPH_REUSE_PAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    let mut total: BTreeMap<String, Counts> = BTreeMap::new();
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    for path in paths.split(',').filter(|p| !p.is_empty()) {
        let bytes = std::fs::read(path).unwrap();
        let pages = wellfriendpdf_engine::ContentEngine::open_bytes(bytes.clone()).unwrap().page_count().unwrap();
        let document = lopdf::Document::load_mem(&bytes).unwrap();
        let mut file: BTreeMap<String, Counts> = BTreeMap::new();
        for page in 1..=pages.min(max_pages) {
            let Ok(layout) = PageTextLayout::from_bytes(&bytes, page) else { continue };
            if !layout.editable {
                continue;
            }
            let subtypes: BTreeMap<String, String> = page_fonts(&document, page as u32)
                .into_iter()
                .map(|f| (f.resource, f.subtype))
                .collect();
            for paragraph in &layout.paragraphs {
                if paragraph.range[1] > layout.page_text_len() {
                    continue;
                }
                // The first word of 4+ letters drawn by one font.
                let glyphs: Vec<_> = layout
                    .glyphs
                    .iter()
                    .filter(|g| g.range[0] >= paragraph.range[0] && g.range[1] <= paragraph.range[1])
                    .collect();
                let mut word = None;
                let mut start = 0;
                while start < glyphs.len() {
                    let mut end = start;
                    while end < glyphs.len()
                        && glyphs[end].text.chars().count() == 1
                        && glyphs[end].text.chars().all(char::is_alphabetic)
                        && glyphs[end].font_resource == glyphs[start].font_resource
                        && glyphs[end].range[1] - glyphs[end].range[0] == 1
                        && (end == start || glyphs[end].range[0] == glyphs[end - 1].range[1])
                    {
                        end += 1;
                    }
                    if end - start >= 4 {
                        word = Some((glyphs[start].range[0], glyphs[end - 1].range[1], glyphs[start].font_resource.clone()));
                        break;
                    }
                    start = end.max(start + 1);
                }
                let Some((s, e, resource)) = word else { continue };
                let text = layout.text_in([s, e]);
                let reversed: String = text.chars().rev().collect();
                if reversed == text {
                    continue;
                }
                let kind = subtypes.get(&resource).cloned().unwrap_or_else(|| "?".into());
                let operation = EditorOperation::ReplaceText {
                    page: page as u32,
                    start: s,
                    end: e,
                    expected: text.clone(),
                    text: reversed,
                    fallback_font: Some("liberation-sans-regular".into()),
                    overflow: Default::default(),
                };
                let counts = file.entry(kind.clone()).or_default();
                match apply_operation(&bytes, &operation, &fonts, true) {
                    Ok((_, report)) if report.fonts_added.is_empty() => counts.source += 1,
                    Ok(_) => {
                        counts.fallback += 1;
                        if std::env::var("WF_GLYPH_REUSE_WHY").is_ok() {
                            // Why the source font was not kept: the engine's
                            // per-run serializer, asked directly.
                            let request: wellfriendpdf_engine::text_reflow::GeometricReflowRequest =
                                serde_json::from_value(serde_json::json!({
                                    "page": page,
                                    "source_text": text,
                                    "replacement_text": text.chars().rev().collect::<String>(),
                                    "target_logical_scalar_range": [s, e],
                                    "font_policy": "preserve_original_per_run",
                                }))
                                .unwrap();
                            let why = match wellfriendpdf_engine::text_reflow::apply_reflow_region(&bytes, &request) {
                                Ok(_) => "per-run serializer succeeded".to_string(),
                                Err(error) => error.to_string().chars().take(110).collect(),
                            };
                            *refusals.entry(format!("WHY {kind}: {why}")).or_default() += 1;
                        }
                    }
                    Err(error) => {
                        counts.refused += 1;
                        let message: String = error.message.chars().take(90).collect();
                        *refusals.entry(format!("{kind}: {message}")).or_default() += 1;
                    }
                }
            }
        }
        let name = path.rsplit('/').next().unwrap();
        for (kind, counts) in &file {
            println!("FILE {name} {kind}: source {} fallback {} refused {}", counts.source, counts.fallback, counts.refused);
            let t = total.entry(kind.clone()).or_default();
            t.source += counts.source;
            t.fallback += counts.fallback;
            t.refused += counts.refused;
        }
    }
    let mut all = Counts::default();
    for (kind, counts) in &total {
        let edits = counts.source + counts.fallback + counts.refused;
        println!(
            "TOTAL {kind}: {edits} edits, source font {} ({:.1}%), fallback font {} ({:.1}%), refused {}",
            counts.source,
            100.0 * counts.source as f64 / edits.max(1) as f64,
            counts.fallback,
            100.0 * counts.fallback as f64 / edits.max(1) as f64,
            counts.refused
        );
        all.source += counts.source;
        all.fallback += counts.fallback;
        all.refused += counts.refused;
    }
    let edits = all.source + all.fallback + all.refused;
    println!(
        "TOTAL all: {edits} edits, source font {}, fallback font {} ({:.1}% of edits, {:.1}% of applied), refused {}",
        all.source,
        all.fallback,
        100.0 * all.fallback as f64 / edits.max(1) as f64,
        100.0 * all.fallback as f64 / (all.source + all.fallback).max(1) as f64,
        all.refused
    );
    for (reason, count) in refusals {
        println!("REFUSED {count} x {reason}");
    }
}
