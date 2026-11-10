//! `pdftotext`-style text outputs: physical layout (`-layout`), raw
//! content-stream order (`-raw`) and the word-level TSV table (`-tsv`).
//!
//! These work on the same positioned [`TextChunk`]s as every other text path;
//! they differ only in how the chunks are arranged into output.

use crate::analysis::layout::{analyze_page, LayoutConfig};
use crate::text::semantic_model::TextSemanticPage;
use crate::text::TextChunk;

/// Baseline difference, in font sizes, at which raw and layout output start a
/// new line (Poppler's `maxIntraLineDelta`).
const MAX_INTRA_LINE_DELTA: f64 = 0.5;
/// Gap, in font sizes, above which raw output separates two runs with a space.
const RAW_SPACE_GAP: f64 = 0.2;

/// Text in content-stream order: each run is written in the order it was
/// shown, with a line break when the baseline moves by half a font size or
/// more and a space when consecutive runs are separated by a gap.
pub fn raw_order_text(chunks: &[TextChunk]) -> String {
    let mut out = String::new();
    let mut previous: Option<&TextChunk> = None;
    for chunk in chunks {
        if chunk.text.is_empty() {
            continue;
        }
        if let Some(prev) = previous {
            let size = prev.font_size.abs().max(1.0);
            let (along, across) = if prev.is_vertical || chunk.is_vertical {
                // Vertical runs advance down the page; columns move in x.
                (prev.y - prev.width.max(0.0) - chunk.y, chunk.x - prev.x)
            } else {
                (chunk.x - (prev.x + prev.width.max(0.0)), chunk.y - prev.y)
            };
            if across.abs() >= MAX_INTRA_LINE_DELTA * size {
                trim_trailing_spaces(&mut out);
                out.push('\n');
            } else if along.abs() > RAW_SPACE_GAP * size
                && !out.ends_with(char::is_whitespace)
                && !chunk.text.starts_with(char::is_whitespace)
            {
                out.push(' ');
            }
        }
        out.push_str(&chunk.text);
        previous = Some(chunk);
    }
    trim_trailing_spaces(&mut out);
    out
}

fn trim_trailing_spaces(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
}

struct Fragment {
    base: f64,
    x0: f64,
    width: f64,
    font_size: f64,
    text: String,
}

/// Physical layout: every text line is placed on the output row of its
/// baseline and indented to the character column of its x position, so
/// side-by-side columns stay side by side (Poppler's `-layout`). Vertical gaps
/// become 1 to 5 line breaks depending on their size. `page_x0` is the left
/// edge of the page box.
pub fn physical_layout_text(chunks: &[TextChunk], page_x0: f64) -> String {
    let horizontal: Vec<TextChunk> = chunks
        .iter()
        .filter(|chunk| !chunk.is_vertical)
        .cloned()
        .collect();
    let layout = analyze_page(&horizontal, &LayoutConfig::default());
    let mut fragments: Vec<Fragment> = layout
        .blocks
        .iter()
        .flat_map(|block| block.lines.iter())
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| Fragment {
            base: line.bbox.y0,
            x0: line.bbox.x0,
            width: line.bbox.width(),
            font_size: line.bbox.height().max(1.0),
            text: line.text.clone(),
        })
        .collect();

    // Character pitch: the average advance per character over the page.
    let (total_width, total_chars) = fragments.iter().fold((0.0, 0usize), |acc, f| {
        (acc.0 + f.width, acc.1 + f.text.chars().count())
    });
    let mut pitch = if total_chars > 0 {
        total_width / total_chars as f64
    } else {
        0.0
    };
    if !pitch.is_finite() || pitch <= 0.1 {
        let mut sizes: Vec<f64> = fragments.iter().map(|f| f.font_size).collect();
        sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        pitch = sizes.get(sizes.len() / 2).copied().unwrap_or(10.0) * 0.5;
    }

    // Top to bottom, then rows of fragments whose baselines agree.
    fragments.sort_by(|a, b| {
        b.base
            .partial_cmp(&a.base)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut rows: Vec<Vec<Fragment>> = Vec::new();
    for fragment in fragments {
        let joins = rows.last().is_some_and(|row| {
            let first = &row[0];
            (first.base - fragment.base).abs() < MAX_INTRA_LINE_DELTA * first.font_size
        });
        if joins {
            rows.last_mut().expect("row exists").push(fragment);
        } else {
            rows.push(vec![fragment]);
        }
    }

    let mut out = String::new();
    let mut previous: Option<(f64, f64)> = None;
    for mut row in rows {
        row.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((base, size)) = previous {
            let breaks = ((base - row[0].base) / size).floor().clamp(1.0, 5.0) as usize;
            for _ in 0..breaks {
                out.push('\n');
            }
        }
        let mut col = 0usize;
        for fragment in &row {
            let target = ((fragment.x0 - page_x0) / pitch).round().max(0.0) as usize;
            if col > 0 && target <= col {
                out.push(' ');
                col += 1;
            } else {
                for _ in col..target {
                    out.push(' ');
                }
                col = col.max(target);
            }
            out.push_str(&fragment.text);
            col += fragment.text.chars().count();
        }
        previous = Some((row[0].base, row[0].font_size));
    }

    // Vertical runs have no horizontal rows; list their columns afterwards.
    let vertical: Vec<TextChunk> = chunks
        .iter()
        .filter(|chunk| chunk.is_vertical && !chunk.text.trim().is_empty())
        .cloned()
        .collect();
    if !vertical.is_empty() {
        let reader = crate::text::reading_order::ReadingOrderReconstructor::default();
        for (index, group) in reader.group_into_columns(vertical).into_iter().enumerate() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(reader.vertical_column_line(group, index).text.trim());
        }
    }
    out
}

/// Header row of the `-tsv` table.
pub const TSV_HEADER: &str =
    "level\tpage_num\tpar_num\tblock_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";

fn tsv_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if matches!(ch, '\t' | '\n' | '\r') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// Rows of Poppler's `-tsv` table for one page (without the header): a page
/// row (level 1), then for each block a level-3 row, and for each line a
/// level-4 row followed by its words (level 5, confidence 100). Coordinates
/// are in points from the top-left corner of the page box. Each semantic block
/// is reported as its own flow (`par_num`), with `block_num` 0.
pub fn tsv_rows(page: &TextSemanticPage) -> String {
    let [bx0, by0, bx1, by1] = page.page_box;
    let (left_edge, top_edge) = (bx0.min(bx1), by0.max(by1));
    let (width, height) = ((bx1 - bx0).abs(), (by1 - by0).abs());
    let mut out = format!(
        "1\t{}\t0\t0\t0\t0\t{:.6}\t{:.6}\t{width:.6}\t{height:.6}\t-1\t###PAGE###\n",
        page.page, 0.0, 0.0
    );
    let rect = |quad: &crate::text::semantic_model::TextQuad| {
        let left = quad.x0.min(quad.x1) - left_edge;
        let top = top_edge - quad.y0.max(quad.y1);
        (
            left,
            top,
            (quad.x1 - quad.x0).abs(),
            (quad.y1 - quad.y0).abs(),
        )
    };
    for (flow, block) in page.blocks.iter().enumerate() {
        let (l, t, w, h) = rect(&block.quad);
        out.push_str(&format!(
            "3\t{}\t{flow}\t0\t0\t0\t{l:.6}\t{t:.6}\t{w:.6}\t{h:.6}\t-1\t###FLOW###\n",
            page.page
        ));
        for (line_num, line) in block.lines.iter().enumerate() {
            let words: Vec<_> = line
                .words
                .iter()
                .filter(|word| !word.text.trim().is_empty())
                .collect();
            if words.is_empty() {
                continue;
            }
            let (l, t, w, h) = rect(&line.quad);
            out.push_str(&format!(
                "4\t{}\t{flow}\t0\t{line_num}\t0\t{l:.6}\t{t:.6}\t{w:.6}\t{h:.6}\t-1\t###LINE###\n",
                page.page
            ));
            for (word_num, word) in words.iter().enumerate() {
                let (l, t, w, h) = rect(&word.quad);
                out.push_str(&format!(
                    "5\t{}\t{flow}\t0\t{line_num}\t{word_num}\t{l:.2}\t{t:.2}\t{w:.2}\t{h:.2}\t100\t{}\n",
                    page.page,
                    tsv_text(&word.text)
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: &str, x: f64, y: f64, width: f64) -> TextChunk {
        TextChunk {
            text: text.to_string(),
            x,
            y,
            font_size: 10.0,
            font_name: "F1".to_string(),
            width,
            is_rtl: false,
            is_vertical: false,
            is_invisible: false,
            is_actual_text: false,
            mapping_sources: Vec::new(),
            glyph_run: None,
        }
    }

    #[test]
    fn raw_order_follows_the_content_stream() {
        // Shown bottom line first, then the top line: raw order keeps that.
        let chunks = vec![
            chunk("second", 10.0, 100.0, 30.0),
            chunk("line", 45.0, 100.0, 20.0),
            chunk("first", 10.0, 120.0, 25.0),
            chunk("line", 35.0, 120.0, 20.0),
        ];
        assert_eq!(raw_order_text(&chunks), "second line\nfirstline");
    }

    #[test]
    fn layout_places_columns_side_by_side() {
        // Two columns of two lines each: rows must hold both columns.
        let chunks = vec![
            chunk("Left one", 0.0, 700.0, 40.0),
            chunk("Left two", 0.0, 688.0, 40.0),
            chunk("Right one", 300.0, 700.0, 45.0),
            chunk("Right two", 300.0, 688.0, 45.0),
        ];
        let text = physical_layout_text(&chunks, 0.0);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert!(lines[0].starts_with("Left one"));
        assert!(lines[0].trim_end().ends_with("Right one"));
        assert!(lines[1].starts_with("Left two"));
        assert!(lines[1].trim_end().ends_with("Right two"));
        let col_one = lines[0].find("Right").unwrap();
        let col_two = lines[1].find("Right").unwrap();
        assert_eq!(col_one, col_two, "same column position on both rows");
        assert!(col_one > 20, "right column is indented: {col_one}");
    }

    #[test]
    fn layout_turns_large_gaps_into_blank_lines() {
        let chunks = vec![
            chunk("Top", 0.0, 700.0, 15.0),
            chunk("Bottom", 0.0, 640.0, 30.0),
        ];
        let text = physical_layout_text(&chunks, 0.0);
        assert_eq!(text, "Top\n\n\n\n\nBottom");
    }
}
