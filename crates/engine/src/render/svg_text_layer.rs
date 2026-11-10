//! Invisible, selectable text layer for SVG output.
//!
//! The SVG backend draws text as glyph outlines, which keeps the page exact
//! but leaves nothing to search or copy. Like a PDF viewer's text layer, this
//! module adds the decoded Unicode on top as transparent `<text>` elements:
//! glyphs that continue one another on a baseline become one element whose
//! characters are placed at their glyphs' positions (a per-character `x`
//! list), with a space inserted where the gap between glyphs reads as a word
//! break. The text is painted with zero opacity, so the rendered image is
//! unchanged, while browsers still select, find and copy it.

use crate::render::transform::{Transform2D, Viewport};

/// One shown glyph, in SVG device space.
pub(super) struct LayerGlyph<'a> {
    /// Unicode text the glyph decodes to.
    pub text: &'a str,
    /// Device position of the glyph origin.
    pub origin: (f64, f64),
    /// Device position of the pen after the glyph's advance.
    pub end: (f64, f64),
    /// Device vector of one em along the text-space x axis.
    pub em_x: (f64, f64),
    /// Device vector of one em along the text-space y axis (upwards).
    pub em_y: (f64, f64),
    /// Whether the font writes vertically (glyphs advance down the column).
    pub vertical: bool,
}

/// A gap wider than this fraction of the font size reads as a word break.
const WORD_GAP_EM: f64 = 0.15;
/// Glyphs further apart than this (in ems) start a separate element.
const MAX_JOIN_GAP_EM: f64 = 3.0;
/// The run moves backwards by more than this (in ems): a separate element.
const MAX_OVERLAP_EM: f64 = 0.6;
/// Baselines further apart than this (in ems) belong to separate lines.
const MAX_BASELINE_SHIFT_EM: f64 = 0.25;

struct Run {
    origin: (f64, f64),
    /// Unit vector along the run's baseline, in device space.
    dir: (f64, f64),
    /// Font size in device units.
    size: f64,
    text: String,
    /// Offset along `dir` of each UTF-16 unit of `text`.
    offsets: Vec<f64>,
    /// Offset along `dir` of the pen after the last glyph.
    end: f64,
}

#[derive(Default)]
pub(super) struct TextLayer {
    elements: String,
    run: Option<Run>,
}

fn dot(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn length(v: (f64, f64)) -> f64 {
    v.0.hypot(v.1)
}

fn unit(v: (f64, f64)) -> Option<(f64, f64)> {
    let len = length(v);
    (len.is_finite() && len > 1e-9).then(|| (v.0 / len, v.1 / len))
}

impl TextLayer {
    /// Add one glyph, extending the current element when the glyph continues
    /// it on the same baseline.
    pub(super) fn push(&mut self, glyph: &LayerGlyph<'_>) {
        let size = length(glyph.em_y);
        // Writing direction: the text-space x axis, or down the column.
        let dir = if glyph.vertical {
            unit((-glyph.em_y.0, -glyph.em_y.1))
        } else {
            unit(glyph.em_x)
        };
        let (Some(dir), true) = (dir, size.is_finite() && size > 1e-6) else {
            return;
        };
        if glyph.text.is_empty() {
            return;
        }
        let continues = self.run.as_ref().and_then(|run| {
            let delta = (glyph.origin.0 - run.origin.0, glyph.origin.1 - run.origin.1);
            let along = dot(delta, run.dir);
            let gap = along - run.end;
            let joins = dot(dir, run.dir) > 0.995
                && (size - run.size).abs() <= 0.2 * run.size
                && cross(run.dir, delta).abs() <= MAX_BASELINE_SHIFT_EM * run.size
                && gap >= -MAX_OVERLAP_EM * run.size
                && gap <= MAX_JOIN_GAP_EM * run.size;
            joins.then_some((along, gap))
        });
        let (along, gap) = match continues {
            Some(position) => position,
            None => {
                self.flush();
                self.run = Some(Run {
                    origin: glyph.origin,
                    dir,
                    size,
                    text: String::new(),
                    offsets: Vec::new(),
                    end: 0.0,
                });
                (0.0, 0.0)
            }
        };
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let starts_with_space = glyph.text.starts_with(char::is_whitespace);
        let ends_with_space = run.text.ends_with(char::is_whitespace);
        if gap > WORD_GAP_EM * run.size
            && !run.text.is_empty()
            && !starts_with_space
            && !ends_with_space
        {
            run.text.push(' ');
            run.offsets.push(run.end);
        }
        let delta = (glyph.end.0 - glyph.origin.0, glyph.end.1 - glyph.origin.1);
        let advance = dot(delta, run.dir).max(0.0);
        let chars: Vec<char> = glyph
            .text
            .chars()
            .filter_map(|ch| match ch {
                ch if ch.is_whitespace() => Some(' '),
                ch if ch.is_control() || ch == '\u{FFFE}' || ch == '\u{FFFF}' => None,
                ch => Some(ch),
            })
            .collect();
        let step = advance / chars.len().max(1) as f64;
        for (index, ch) in chars.iter().enumerate() {
            let offset = along + step * index as f64;
            run.text.push(*ch);
            for _ in 0..ch.len_utf16() {
                run.offsets.push(offset);
            }
        }
        run.end = run.end.max(along + advance);
    }

    /// Close the current element (at the end of a text object).
    pub(super) fn flush(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        if run.text.trim().is_empty() {
            return;
        }
        let (ox, oy) = run.origin;
        let (dx, dy) = run.dir;
        let mut el = String::with_capacity(run.text.len() * 8 + 64);
        if (dx - 1.0).abs() < 1e-6 && dy.abs() < 1e-6 {
            // Upright text: absolute positions, no transform.
            el.push_str("<text x=\"");
            push_offsets(&mut el, &run.offsets, ox);
            el.push_str(&format!("\" y=\"{oy:.2}\""));
        } else {
            // The element's own frame: x along the baseline, y down from it.
            el.push_str(&format!(
                "<text transform=\"matrix({dx:.5} {dy:.5} {:.5} {dx:.5} {ox:.2} {oy:.2})\" x=\"",
                -dy
            ));
            push_offsets(&mut el, &run.offsets, 0.0);
            el.push_str("\" y=\"0\"");
        }
        el.push_str(&format!(" font-size=\"{:.2}\">", run.size));
        push_escaped(&mut el, &run.text);
        el.push_str("</text>\n");
        self.elements.push_str(&el);
    }

    /// Add one positioned word whose box is given in page space (used for
    /// pages embedded as a raster image, where no glyph geometry is replayed).
    pub(super) fn push_word_box(&mut self, text: &str, page_box: [f64; 4], viewport: &Viewport) {
        let [x0, y0, x1, y1] = page_box;
        let (ax, ay) = viewport.page_to_pixel_f64(x0, y0);
        let (bx, by) = viewport.page_to_pixel_f64(x1, y1);
        let (left, right) = (ax.min(bx), ax.max(bx));
        let (top, bottom) = (ay.min(by), ay.max(by));
        let (width, height) = (right - left, bottom - top);
        let text = text.trim();
        if text.is_empty() || !(width.is_finite() && height.is_finite()) || height <= 0.0 {
            return;
        }
        // The word is one wide "glyph" on a baseline about a fifth of the box
        // height above its bottom; words of a line join into one element with
        // a space where they are apart (CJK words, one per character, abut).
        let baseline = bottom - 0.2 * height;
        self.push(&LayerGlyph {
            text,
            origin: (left, baseline),
            end: (right, baseline),
            em_x: (height, 0.0),
            em_y: (0.0, -height),
            vertical: false,
        });
    }

    /// The layer as one transparent group, or nothing when no text was shown.
    pub(super) fn finish(mut self) -> String {
        self.flush();
        if self.elements.is_empty() {
            return String::new();
        }
        format!(
            "<g class=\"text-layer\" fill=\"#000\" fill-opacity=\"0\" font-family=\"sans-serif\" \
             xml:space=\"preserve\" style=\"white-space:pre\">\n{}</g>\n",
            self.elements
        )
    }
}

fn push_offsets(out: &mut String, offsets: &[f64], base: f64) {
    for (index, offset) in offsets.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{:.2}", base + offset));
    }
}

fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            ch => out.push(ch),
        }
    }
}

/// Device vectors of one em along the text-space axes for a glyph drawn with
/// `glyph_device`, the glyph-space to device matrix whose unit is one em.
pub(super) fn em_vectors(glyph_device: &Transform2D) -> ((f64, f64), (f64, f64)) {
    let [a, b, c, d, _, _] = glyph_device.to_array();
    ((a, b), (c, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(text: &str, x: f64, advance: f64) -> LayerGlyph<'_> {
        LayerGlyph {
            text,
            origin: (x, 100.0),
            end: (x + advance, 100.0),
            em_x: (10.0, 0.0),
            em_y: (0.0, -10.0),
            vertical: false,
        }
    }

    #[test]
    fn glyphs_on_a_baseline_form_one_element_with_word_breaks() {
        let mut layer = TextLayer::default();
        layer.push(&glyph("H", 10.0, 6.0));
        layer.push(&glyph("i", 16.0, 3.0));
        // A 3-unit gap at size 10 is a word break.
        layer.push(&glyph("y", 22.0, 5.0));
        layer.push(&glyph("o", 27.0, 5.0));
        let svg = layer.finish();
        assert!(svg.contains("fill-opacity=\"0\""), "{svg}");
        assert!(svg.contains("xml:space=\"preserve\""), "{svg}");
        assert!(
            svg.contains("<text x=\"10.00 16.00 19.00 22.00 27.00\" y=\"100.00\" font-size=\"10.00\">Hi yo</text>"),
            "{svg}"
        );
    }

    #[test]
    fn new_lines_and_rotated_text_get_their_own_elements() {
        let mut layer = TextLayer::default();
        layer.push(&glyph("a", 10.0, 5.0));
        let mut next_line = glyph("b", 10.0, 5.0);
        next_line.origin.1 = 120.0;
        next_line.end.1 = 120.0;
        layer.push(&next_line);
        layer.push(&LayerGlyph {
            text: "c&<",
            origin: (50.0, 50.0),
            end: (50.0, 40.0),
            em_x: (0.0, -10.0),
            em_y: (-10.0, 0.0),
            vertical: false,
        });
        let svg = layer.finish();
        assert_eq!(svg.matches("<text").count(), 3, "{svg}");
        assert!(svg.contains(">c&amp;&lt;</text>"), "{svg}");
        assert!(
            svg.contains("transform=\"matrix(0.00000 -1.00000 1.00000 0.00000 50.00 50.00)\""),
            "{svg}"
        );
    }

    #[test]
    fn word_boxes_join_into_lines_with_spaces_only_between_separate_words() {
        let viewport = Viewport::new([0.0, 0.0, 612.0, 792.0], 72);
        let mut layer = TextLayer::default();
        layer.push_word_box("Hello", [72.0, 720.0, 104.0, 734.0], &viewport);
        layer.push_word_box("ASCII", [107.8, 720.0, 144.4, 734.0], &viewport);
        layer.push_word_box("\u{65E5}", [72.0, 690.0, 86.0, 704.0], &viewport);
        layer.push_word_box("\u{672C}", [86.0, 690.0, 100.0, 704.0], &viewport);
        let svg = layer.finish();
        assert_eq!(svg.matches("<text").count(), 2, "{svg}");
        assert!(svg.contains(">Hello ASCII</text>"), "{svg}");
        assert!(svg.contains(">\u{65E5}\u{672C}</text>"), "{svg}");
        assert!(svg.contains("y=\"69.20\""), "{svg}");
    }

    #[test]
    fn empty_layers_emit_nothing() {
        let mut layer = TextLayer::default();
        layer.push(&glyph(" ", 10.0, 3.0));
        assert_eq!(layer.finish(), "");
    }
}
