use std::sync::Arc;

use super::*;
use crate::text::collector::{TextGlyph, TextGlyphRun};

/// A horizontal run with one glyph per character of `widths[i]` points each,
/// starting at (x, y), at `size` points, in stream order from `pos`.
fn run(text: &str, x: f64, y: f64, size: f64, widths: &[f64], pos: u32) -> TextChunk {
    run_with_matrix(text, x, y, size, widths, pos, [size, 0.0, 0.0, size])
}

fn run_with_matrix(
    text: &str,
    x: f64,
    y: f64,
    size: f64,
    widths: &[f64],
    pos: u32,
    font_matrix: [f64; 4],
) -> TextChunk {
    let len = font_matrix[0].hypot(font_matrix[1]).max(1e-9);
    let (ux, uy) = (font_matrix[0] / len, font_matrix[1] / len);
    let mut glyphs = Vec::new();
    let mut along = 0.0;
    for (index, (offset, ch)) in text.char_indices().enumerate() {
        let w = widths.get(index).copied().unwrap_or(size * 0.5);
        glyphs.push(TextGlyph {
            x: x + ux * along,
            y: y + uy * along,
            dx: ux * w,
            dy: uy * w,
            text_start: offset as u32,
            text_end: (offset + ch.len_utf8()) as u32,
            stream_pos: pos + index as u32,
            code_len: 1,
        });
        along += w;
    }
    TextChunk {
        text: text.to_string(),
        x,
        y,
        font_size: size,
        font_name: "F1".to_string(),
        width: along,
        is_rtl: crate::text::collector::is_rtl_dominant(text),
        is_vertical: false,
        is_invisible: false,
        is_actual_text: false,
        mapping_sources: Vec::new(),
        glyph_run: Some(Arc::new(TextGlyphRun {
            font_matrix,
            ascent: 0.95,
            descent: -0.35,
            glyphs,
        })),
    }
}

/// Words of equal 5-point glyphs separated by `gap` points, as one run with
/// explicit space glyphs.
fn line(text: &str, x: f64, y: f64, pos: u32) -> TextChunk {
    let widths: Vec<f64> = text.chars().map(|_| 5.0).collect();
    run(text, x, y, 10.0, &widths, pos)
}

fn frame() -> PageFrame {
    PageFrame::new([0.0, 0.0, 612.0, 792.0], 0)
}

fn text_of(chunks: &[TextChunk]) -> String {
    analyze_flows(chunks, Some(&frame()), &FlowLayoutOptions::default()).text()
}

#[test]
fn single_line_words_are_spaced_by_their_glyph_gaps() {
    // "Hello" then "world" 3pt further: real glyph boxes make the gap
    // visible even without a space glyph.
    let chunks = vec![
        run("Hello", 72.0, 700.0, 10.0, &[5.0; 5], 0),
        run("world", 72.0 + 25.0 + 3.0, 700.0, 10.0, &[5.0; 5], 5),
    ];
    assert_eq!(text_of(&chunks), "Hello world");
}

#[test]
fn kerned_glyphs_inside_a_word_stay_together() {
    // A 0.2pt kern (0.02 em) between two runs of one word: no space.
    let chunks = vec![
        run("exper", 72.0, 700.0, 10.0, &[5.0; 5], 0),
        run("iment", 72.0 + 25.2, 700.0, 10.0, &[5.0; 5], 5),
    ];
    assert_eq!(text_of(&chunks), "experiment");
}

#[test]
fn space_glyphs_break_words() {
    let chunks = vec![line("two words", 72.0, 700.0, 0)];
    assert_eq!(text_of(&chunks), "two words");
}

#[test]
fn character_spacing_does_not_widen_glyph_boxes() {
    // Letter-spaced capitals: glyphs 6pt wide placed every 9pt. Each letter is
    // its own word, and the per-line threshold (1.3 × the smallest gap) keeps
    // them together.
    let mut chunks = Vec::new();
    for (i, ch) in "SPACED".chars().enumerate() {
        chunks.push(run(&ch.to_string(), 72.0 + 9.0 * i as f64, 700.0, 10.0, &[6.0], i as u32));
    }
    assert_eq!(text_of(&chunks), "SPACED");
}

#[test]
fn ligature_glyph_is_split_into_characters() {
    // One "fi" glyph 10pt wide followed by "nd".
    let mut chunk = run("find", 72.0, 700.0, 10.0, &[10.0, 5.0, 5.0], 0);
    let run_data = Arc::get_mut(chunk.glyph_run.as_mut().unwrap()).unwrap();
    // Re-map: glyph 0 covers "fi", glyph 1 "n", glyph 2 "d".
    run_data.glyphs[0].text_end = 2;
    run_data.glyphs[1].text_start = 2;
    run_data.glyphs[1].text_end = 3;
    run_data.glyphs[1].x = 82.0;
    run_data.glyphs[2].text_start = 3;
    run_data.glyphs[2].text_end = 4;
    run_data.glyphs[2].x = 87.0;
    run_data.glyphs.truncate(3);
    let page = analyze_flows(&[chunk], Some(&frame()), &FlowLayoutOptions::default());
    assert_eq!(page.text(), "find");
    let word = &page.flows[0].blocks[0].lines[0].words[0];
    assert!((word.bbox[2] - 92.0).abs() < 1e-6, "{word:?}");
}

#[test]
fn spacing_accent_merges_with_its_letter() {
    // "e" then an acute accent (U+00B4) centred on it: one "é".
    let chunks = vec![
        run("caf", 72.0, 700.0, 10.0, &[5.0; 3], 0),
        run("e", 87.0, 700.0, 10.0, &[5.0], 3),
        run("\u{00B4}", 88.0, 700.0, 10.0, &[3.0], 4),
    ];
    assert_eq!(text_of(&chunks), "caf\u{00E9}");
}

#[test]
fn accent_before_its_letter_also_merges() {
    let chunks = vec![
        run("\u{00A8}", 73.0, 700.0, 10.0, &[3.0], 0),
        run("u", 72.0, 700.0, 10.0, &[5.0], 1),
    ];
    assert_eq!(text_of(&chunks), "\u{00FC}");
}

#[test]
fn dotless_i_with_dieresis_becomes_i_dieresis() {
    let chunks = vec![
        run("na\u{0131}", 72.0, 700.0, 10.0, &[5.0, 5.0, 3.0], 0),
        run("\u{00A8}", 82.2, 700.0, 10.0, &[2.6], 3),
        run("ve", 85.0, 700.0, 10.0, &[5.0, 5.0], 4),
    ];
    assert_eq!(text_of(&chunks), "na\u{00EF}ve");
}

#[test]
fn fake_bold_overprint_is_removed() {
    // The same word drawn twice, 0.3pt apart.
    let chunks = vec![
        run("Bold", 72.0, 700.0, 10.0, &[5.0; 4], 0),
        run("Bold", 72.3, 700.0, 10.0, &[5.0; 4], 4),
        run("text", 100.0, 700.0, 10.0, &[5.0; 4], 8),
    ];
    let page = analyze_flows(&chunks, Some(&frame()), &FlowLayoutOptions::default());
    assert_eq!(page.text(), "Bold text");
    assert_eq!(page.duplicates_removed, 1);
}

#[test]
fn interleaved_fake_bold_glyphs_are_removed() {
    // Each glyph drawn twice in a row, the copy offset by 0.25pt.
    let mut chunks = Vec::new();
    for (i, ch) in "Hi!".chars().enumerate() {
        let x = 72.0 + 5.0 * i as f64;
        chunks.push(run(&ch.to_string(), x, 700.0, 10.0, &[5.0], 2 * i as u32));
        chunks.push(run(&ch.to_string(), x + 0.25, 700.0, 10.0, &[5.0], 2 * i as u32 + 1));
    }
    assert_eq!(text_of(&chunks), "Hi!");
}

#[test]
fn hyphenated_line_is_joined_with_its_continuation() {
    let chunks = vec![
        line("an extra-", 72.0, 700.0, 0),
        line("ordinary day", 72.0, 688.0, 20),
    ];
    assert_eq!(text_of(&chunks), "an extraordinary day");
    let page = analyze_flows(&chunks, Some(&frame()), &FlowLayoutOptions::default());
    assert_eq!(page.text_with(false), "an extra-\nordinary day");
}

#[test]
fn hyphen_before_a_capitalised_line_is_kept() {
    let chunks = vec![
        line("the anti-", 72.0, 700.0, 0),
        line("American", 72.0, 688.0, 20),
    ];
    assert_eq!(text_of(&chunks), "the anti-\nAmerican");
}

#[test]
fn two_columns_are_read_one_after_the_other() {
    let mut chunks = Vec::new();
    let mut pos = 0;
    for row in 0..4 {
        let y = 700.0 - 12.0 * row as f64;
        chunks.push(line(&format!("left{row} text"), 72.0, y, pos));
        pos += 20;
        chunks.push(line(&format!("right{row} text"), 320.0, y, pos));
        pos += 20;
    }
    let text = text_of(&chunks);
    let lefts: Vec<usize> = (0..4).map(|r| text.find(&format!("left{r}")).unwrap()).collect();
    let rights: Vec<usize> = (0..4).map(|r| text.find(&format!("right{r}")).unwrap()).collect();
    assert!(lefts.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(rights.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(lefts[3] < rights[0], "left column first: {text}");
}

#[test]
fn heading_spanning_columns_comes_first_and_footer_last() {
    let mut chunks = vec![line("A wide heading across both columns", 72.0, 740.0, 0)];
    let mut pos = 100;
    for row in 0..3 {
        let y = 700.0 - 12.0 * row as f64;
        chunks.push(line(&format!("alpha{row}"), 72.0, y, pos));
        chunks.push(line(&format!("beta{row}"), 320.0, y, pos + 10));
        pos += 20;
    }
    chunks.push(line("a footer line that runs across the whole width of the page", 72.0, 600.0, 500));
    let text = text_of(&chunks);
    let heading = text.find("heading").unwrap();
    let alpha = text.find("alpha0").unwrap();
    let beta = text.find("beta2").unwrap();
    let footer = text.find("footer").unwrap();
    assert!(heading < alpha && alpha < beta && beta < footer, "{text}");
}

#[test]
fn rotated_text_is_grouped_in_its_own_pool() {
    // Body text plus a label rotated 90 degrees counter-clockwise (reading
    // upwards) in the margin: the label's glyphs form one line, not one
    // line per glyph.
    let mut chunks = vec![
        line("body text line one", 100.0, 700.0, 0),
        line("body text line two", 100.0, 688.0, 30),
    ];
    chunks.push(run_with_matrix(
        "MARGIN",
        40.0,
        500.0,
        10.0,
        &[6.0; 6],
        60,
        [0.0, 10.0, -10.0, 0.0],
    ));
    let page = analyze_flows(&chunks, Some(&frame()), &FlowLayoutOptions::default());
    let text = page.text();
    assert!(text.contains("MARGIN"), "{text}");
    assert_eq!(page.primary_rotation, 0);
    let rotated: Vec<&FlowBlock> = page
        .flows
        .iter()
        .flat_map(|f| f.blocks.iter())
        .filter(|b| b.rotation != 0)
        .collect();
    assert_eq!(rotated.len(), 1);
    assert_eq!(rotated[0].rotation, 270);
}

#[test]
fn upside_down_text_reads_in_its_own_direction() {
    // Text rotated 180 degrees: glyphs advance to the left.
    let chunk = run_with_matrix("upside", 300.0, 400.0, 10.0, &[5.0; 6], 0, [-10.0, 0.0, 0.0, -10.0]);
    let page = analyze_flows(&[chunk], Some(&frame()), &FlowLayoutOptions::default());
    assert_eq!(page.text(), "upside");
    assert_eq!(page.primary_rotation, 180);
}

#[test]
fn diagonal_text_is_flagged_and_can_be_discarded() {
    let c = 45f64.to_radians().cos() * 10.0;
    let chunks = vec![
        line("straight", 72.0, 700.0, 0),
        run_with_matrix("slanted", 100.0, 400.0, 10.0, &[5.0; 7], 20, [c, c, -c, c]),
    ];
    let page = analyze_flows(&chunks, Some(&frame()), &FlowLayoutOptions::default());
    assert!(page
        .flows
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.lines.iter())
        .flat_map(|l| l.words.iter())
        .any(|w| w.diagonal));
    let options = FlowLayoutOptions {
        discard_diagonal: true,
        ..Default::default()
    };
    let page = analyze_flows(&chunks, Some(&frame()), &options);
    assert_eq!(page.text(), "straight");
    assert!(page.diagonal_discarded > 0);
}

#[test]
fn rtl_line_is_emitted_in_logical_order() {
    // Hebrew alef-bet-gimel placed left to right visually as gimel bet alef.
    let chunks = vec![run("\u{05D2}\u{05D1}\u{05D0}", 72.0, 700.0, 10.0, &[5.0; 3], 0)];
    let page = analyze_flows(&chunks, Some(&frame()), &FlowLayoutOptions::default());
    assert_eq!(page.text(), "\u{05D0}\u{05D1}\u{05D2}");
    assert!(!page.primary_lr);
}

#[test]
fn glyphs_off_the_page_are_dropped() {
    let chunks = vec![line("visible", 72.0, 700.0, 0), line("hidden", 700.0, 700.0, 20)];
    assert_eq!(text_of(&chunks), "visible");
}

#[test]
fn chunks_without_glyph_geometry_use_equal_widths() {
    let mut chunk = line("plain words", 72.0, 700.0, 0);
    chunk.glyph_run = None;
    assert_eq!(text_of(&[chunk]), "plain words");
}

#[test]
fn page_frame_applies_rotation() {
    let f = PageFrame::new([0.0, 0.0, 600.0, 800.0], 90);
    assert_eq!((f.width, f.height), (800.0, 600.0));
    // The page's bottom-left corner is the displayed top-left.
    assert_eq!(f.point(0.0, 0.0), (0.0, 0.0));
    let (x, y) = f.point(100.0, 50.0);
    assert_eq!((x, y), (50.0, 100.0));
    assert_eq!(f.user_point(x, y), (100.0, 50.0));
}

#[test]
fn table_cells_are_read_row_by_row() {
    // A 2 x 2 grid of aligned cells with wide gaps.
    let chunks = vec![
        line("cellA", 72.0, 700.0, 0),
        line("cellB", 300.0, 700.0, 10),
        line("cellC", 72.0, 660.0, 20),
        line("cellD", 300.0, 660.0, 30),
    ];
    let text = text_of(&chunks);
    let order: Vec<usize> = ["cellA", "cellB", "cellC", "cellD"]
        .iter()
        .map(|t| text.find(t).unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
}
