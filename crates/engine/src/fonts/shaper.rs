//! OpenType shaping for generated text; source PDF glyphs are never reshaped.
pub use super::coverage::missing_glyph_clusters;
pub use super::shaping_context::ShapingContext;
use crate::error::{Result, WellfriendError};
use rustybuzz::{Direction, UnicodeBuffer};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use unicode_bidi::{BidiInfo, Level};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDirection {
    LeftToRight,
    RightToLeft,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShapeOptions {
    pub direction: Option<TextDirection>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    /// UTF-8 offset in the original logical text, not a visual-run index.
    pub cluster: u32,
    pub advance: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub direction: TextDirection,
    /// Compatibility field: OpenType shaping was used, including for Latin.
    pub used_complex_shaping: bool,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct TextShaper;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenTypeSettings {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

/// Resolved UAX #9 levels after paragraph analysis and line-specific rule L1.
/// Byte indexing matches unicode-bidi; continuation lines must not re-resolve
/// neutrals or isolate controls without their preceding paragraph context.
/// Non-emitting logical neighbours separately preserve cross-boundary joining.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct LineBidi {
    pub levels: Vec<u8>,
    pub rtl: bool,
    #[serde(default, skip_serializing_if = "ShapingContext::is_empty")]
    pub context: ShapingContext,
}
impl LineBidi {
    /// Slice resolved levels without losing the logical neighbours of a font,
    /// bidi or vertical-orientation run. This does not re-run bidi analysis.
    pub fn slice(&self, text: &str, range: Range<usize>, rtl: bool) -> Result<Self> {
        if self.levels.len() != text.len() {
            return Err(WellfriendError::invalid_input(
                "bidi context length mismatch",
            ));
        }
        let context = self.context.slice(text, range.clone())?;
        Ok(Self {
            levels: self.levels[range].to_vec(),
            rtl,
            context,
        })
    }
}

/// A retained cmap can still point at empty glyph slots in an embedded subset.
/// Treat a nonzero GID without paintable outlines as missing for visible text;
/// spaces and default-ignorable controls are allowed to have no outline.
pub fn has_missing_glyphs(font: &[u8], text: &str, run: &ShapedRun) -> Result<bool> {
    Ok(!missing_glyph_clusters(font, text, run)?.is_empty())
}

pub struct ParagraphBidi<'a> {
    info: BidiInfo<'a>,
    base: Option<Level>,
    joining: super::shaping_context::JoiningIndex<'a>,
}
impl<'a> ParagraphBidi<'a> {
    /// Visit visible hard-line ranges with their original paragraph's resolved
    /// bidi levels. False stops coverage probing early; no fresh paragraph is
    /// inferred merely because LS/VT/FF ends a line.
    pub(crate) fn all_hard_lines(
        &self,
        mut visit: impl FnMut(Range<usize>, LineBidi) -> Result<bool>,
    ) -> Result<bool> {
        crate::cancel::check_current_cancel("paragraph hard-line visit")?;
        if self.info.text.is_empty() {
            let result = visit(0..0, self.line(0..0)?)?;
            crate::cancel::check_current_cancel("empty hard-line result")?;
            return Ok(result);
        }
        for paragraph in &self.info.paragraphs {
            // unicode-bidi keeps CR and LF as separate B paragraphs. The line
            // policy treats the CRLF pair as one separator, not an empty line.
            if &self.info.text[paragraph.range.clone()] == "\n"
                && paragraph.range.start > 0
                && self.info.text.as_bytes()[paragraph.range.start - 1] == b'\r'
            {
                continue;
            }
            for local in super::hard_break::logical_lines(&self.info.text[paragraph.range.clone()])
            {
                let local = local?;
                let visible = paragraph.range.start + local.logical.start
                    ..paragraph.range.start + local.visible.end;
                let bidi = self.line(visible.clone())?;
                let keep_going = visit(visible, bidi)?;
                crate::cancel::check_current_cancel("paragraph hard-line result")?;
                if !keep_going {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub fn new(text: &'a str, options: ShapeOptions) -> Result<Self> {
        crate::cancel::check_current_cancel("paragraph bidi analysis")?;
        if text.len() > 4_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "paragraph bidi input exceeds 4 MB".into(),
            ));
        }
        let base = options.direction.map(|d| {
            if d == TextDirection::RightToLeft {
                Level::rtl()
            } else {
                Level::ltr()
            }
        });
        let joining = super::shaping_context::JoiningIndex::new(text)?;
        Ok(Self {
            info: BidiInfo::new(text, base),
            base,
            joining,
        })
    }
    pub fn line(&self, line: Range<usize>) -> Result<LineBidi> {
        let text = self.info.text;
        if line.start > line.end
            || line.end > text.len()
            || !text.is_char_boundary(line.start)
            || !text.is_char_boundary(line.end)
        {
            return Err(WellfriendError::invalid_input(
                "invalid paragraph line range",
            ));
        }
        if line.is_empty() {
            return Ok(LineBidi {
                levels: Vec::new(),
                rtl: self.base.is_some_and(|l| l.is_rtl()),
                context: Default::default(),
            });
        }
        let paragraph_index = self
            .info
            .paragraphs
            .partition_point(|p| p.range.end <= line.start);
        let paragraph = self
            .info
            .paragraphs
            .get(paragraph_index)
            .filter(|p| p.range.start <= line.start && line.end <= p.range.end)
            .ok_or_else(|| {
                WellfriendError::invalid_input("line crosses a hard paragraph boundary")
            })?;
        // Use the library's L1 implementation on a line-sized view of the
        // resolved paragraph. Do not clone all N paragraph levels per width probe.
        let info = BidiInfo {
            text: &text[line.clone()],
            original_classes: self.info.original_classes[line.clone()].to_vec(),
            levels: self.info.levels[line.clone()].to_vec(),
            paragraphs: vec![unicode_bidi::ParagraphInfo {
                range: 0..line.len(),
                level: paragraph.level,
            }],
        };
        let levels = info.reordered_levels(&info.paragraphs[0], 0..line.len());
        Ok(LineBidi {
            levels: levels.iter().map(Level::number).collect(),
            rtl: paragraph.level.is_rtl(),
            context: self.joining.context(line)?,
        })
    }
}
pub fn resolve_line_bidi(
    text: &str,
    line: Range<usize>,
    options: ShapeOptions,
) -> Result<LineBidi> {
    ParagraphBidi::new(text, options)?.line(line)
}

// Bounded thread-local LRU: no global rendering lock and no pointer/family-name
// aliasing between font revisions. Keys hash actual font bytes, text, direction.
struct ShapeCache {
    entries: VecDeque<([u8; 32], ShapedRun)>,
    bytes: usize,
}

thread_local! {
    static SHAPE_CACHE: RefCell<ShapeCache> = const { RefCell::new(ShapeCache {
        entries: VecDeque::new(),
        bytes: 0,
    }) };
}
const CACHE_BYTES: usize = 4 * 1024 * 1024;
// The byte budget is authoritative. A low entry cap caused cache thrashing for
// line-layout searches because hundreds of small candidate runs fit well below
// 4 MiB but evicted one another on every width probe.
const CACHE_ENTRIES: usize = 4_096;

fn cached_shape(key: &[u8; 32]) -> Option<ShapedRun> {
    SHAPE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache
            .entries
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = cache.entries.remove(index)?;
        let result = entry.1.clone();
        cache.entries.push_front(entry);
        Some(result)
    })
}
fn cache_shape(key: [u8; 32], result: &ShapedRun) {
    let bytes = shaped_run_bytes(result);
    if bytes > CACHE_BYTES / 4 {
        return;
    }
    SHAPE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        while cache.entries.len() >= CACHE_ENTRIES
            || bytes.saturating_add(cache.bytes) > CACHE_BYTES
        {
            let Some((_, evicted)) = cache.entries.pop_back() else {
                break;
            };
            cache.bytes = cache.bytes.saturating_sub(shaped_run_bytes(&evicted));
        }
        cache.entries.push_front((key, result.clone()));
        cache.bytes = cache.bytes.saturating_add(bytes);
    });
}

#[inline]
fn shaped_run_bytes(run: &ShapedRun) -> usize {
    run.glyphs.len() * std::mem::size_of::<ShapedGlyph>()
}

/// Itemize without splitting graphemes. Common and inherited characters remain
/// with their context rather than becoming one-character fallback runs.
pub(crate) fn script_ranges(text: &str) -> Vec<(Range<usize>, Script)> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = Script::Common;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let strong = grapheme
            .chars()
            .map(|c| c.script())
            .find(|s| !matches!(s, Script::Common | Script::Inherited | Script::Unknown));
        if let Some(script) = strong {
            if current != Script::Common && script != current {
                runs.push((start..offset, current));
                start = offset;
            }
            current = script;
        }
    }
    if start < text.len() {
        runs.push((start..text.len(), current));
    }
    runs
}
impl TextShaper {
    pub fn shape_with_settings(
        font_bytes: &[u8],
        text: &str,
        options: ShapeOptions,
        settings: &OpenTypeSettings,
    ) -> Result<ShapedRun> {
        if settings == &OpenTypeSettings::default() {
            return Self::shape(font_bytes, text, options);
        }
        shape_paragraphs(font_bytes, text, options, settings)
    }

    /// Shape an already-broken logical line using its parent paragraph levels.
    pub fn shape_resolved(
        font_bytes: &[u8],
        text: &str,
        bidi: &LineBidi,
        settings: &OpenTypeSettings,
    ) -> Result<ShapedRun> {
        let font_digest: [u8; 32] = Sha256::digest(font_bytes).into();
        Self::shape_resolved_prehashed(font_bytes, &font_digest, text, bidi, settings)
    }

    /// Shape a resolved line while reusing a digest of the immutable font
    /// program. Candidate-based layout can shape thousands of spans from the
    /// same font; hashing a multi-megabyte font for every span turns the
    /// dynamic-programming search into avoidable font-size multiplied work.
    /// The digest is request-local provenance and callers must derive it from
    /// exactly `font_bytes` before the first call.
    pub(crate) fn shape_resolved_prehashed(
        font_bytes: &[u8],
        font_digest: &[u8; 32],
        text: &str,
        bidi: &LineBidi,
        settings: &OpenTypeSettings,
    ) -> Result<ShapedRun> {
        crate::cancel::check_current_cancel("contextual line shaping")?;
        if text.len() > 4_000_000
            || bidi.levels.len() != text.len()
            || settings.features.len() > 64
            || settings.features.iter().any(|f| f.len() > 128)
            || settings.language.as_ref().is_some_and(|s| s.len() > 128)
        {
            return Err(WellfriendError::invalid_input(
                "invalid contextual shaping budget/levels",
            ));
        }
        bidi.context.validate()?;
        if text.chars().any(super::hard_break::is_hard_break) {
            return Err(WellfriendError::invalid_input(
                "resolved shaping expects one visible logical line without hard separators",
            ));
        }
        let mut digest = Sha256::new();
        digest.update(b"resolved-line-v5-prehashed-font-hard-lines");
        let context_bytes = serde_json::to_vec(&bidi.context)
            .map_err(|e| WellfriendError::invalid_input(e.to_string()))?;
        for bytes in [
            font_digest.as_slice(),
            text.as_bytes(),
            bidi.levels.as_slice(),
            context_bytes.as_slice(),
        ] {
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        }
        digest.update([u8::from(bidi.rtl)]);
        let settings_bytes = serde_json::to_vec(settings)
            .map_err(|e| WellfriendError::invalid_input(e.to_string()))?;
        digest.update(settings_bytes);
        let key: [u8; 32] = digest.finalize().into();
        if let Some(hit) = cached_shape(&key) {
            return Ok(hit);
        }
        let face = rustybuzz::Face::from_slice(font_bytes, 0).ok_or_else(|| {
            WellfriendError::UnsupportedFeature("font.shaper.invalid_font".into())
        })?;
        let language = settings
            .language
            .as_deref()
            .map(|s| s.parse::<rustybuzz::Language>())
            .transpose()
            .map_err(|_| WellfriendError::invalid_input("invalid shaping language"))?;
        let features = settings
            .features
            .iter()
            .map(|s| s.parse::<rustybuzz::Feature>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| WellfriendError::invalid_input("invalid OpenType feature"))?;
        if features.iter().any(|f| f.start != 0 || f.end != u32::MAX) {
            return Err(WellfriendError::invalid_input(
                "line shaping features must apply to the complete run",
            ));
        }
        let mut runs: Vec<(Range<usize>, Level)> = Vec::new();
        for (offset, ch) in text.char_indices() {
            let level = Level::new(bidi.levels[offset])
                .map_err(|_| WellfriendError::invalid_input("invalid bidi level"))?;
            let end = offset + ch.len_utf8();
            if bidi.levels[offset..end]
                .iter()
                .any(|l| *l != level.number())
            {
                return Err(WellfriendError::invalid_input(
                    "bidi level divides a Unicode scalar",
                ));
            }
            if let Some((range, previous)) = runs.last_mut() {
                if *previous == level {
                    range.end = end;
                    continue;
                }
            }
            runs.push((offset..end, level));
        }
        let order = BidiInfo::reorder_visual(&runs.iter().map(|(_, l)| *l).collect::<Vec<_>>());
        let mut glyphs = Vec::new();
        let upem = f64::from(face.units_per_em()).max(1.0);
        for index in order {
            let (range, level) = &runs[index];
            let mut scripts = script_ranges(&text[range.clone()]);
            if level.is_rtl() {
                scripts.reverse();
            }
            for (script_range, script) in scripts {
                crate::cancel::check_current_cancel("contextual script shaping")?;
                let start = range.start + script_range.start;
                let end = range.start + script_range.end;
                let mut buffer = UnicodeBuffer::new();
                // Joiners still participate in GSUB/GPOS. Remove only the
                // default-ignorable output carriers, using shaper provenance,
                // instead of guessing whether a blank glyph in a cluster is missing.
                buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
                buffer.push_str(&text[start..end]);
                // Soft line breaks and font/script itemization are not a new
                // word. Neighbours affect forms but never emit extra glyphs.
                bidi.context.apply(&mut buffer, text, start..end)?;
                buffer.set_direction(if level.is_rtl() {
                    Direction::RightToLeft
                } else {
                    Direction::LeftToRight
                });
                if let Some(language) = &language {
                    buffer.set_language(language.clone());
                }
                if let Ok(script) = script.short_name().parse::<rustybuzz::Script>() {
                    buffer.set_script(script);
                }
                buffer.guess_segment_properties();
                let shaped = rustybuzz::shape(&face, &features, buffer);
                for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                    glyphs.push(ShapedGlyph {
                        glyph_id: u16::try_from(info.glyph_id).map_err(|_| {
                            WellfriendError::MalformedPdf("glyph id exceeds sfnt range".into())
                        })?,
                        cluster: start as u32 + info.cluster,
                        advance: f64::from(pos.x_advance) / upem * 1000.0,
                        offset_x: f64::from(pos.x_offset) / upem * 1000.0,
                        offset_y: f64::from(pos.y_offset) / upem * 1000.0,
                    });
                }
            }
        }
        let result = ShapedRun {
            glyphs,
            direction: if bidi.rtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            },
            used_complex_shaping: !text.is_empty(),
        };
        cache_shape(key, &result);
        Ok(result)
    }

    pub fn shape(font_bytes: &[u8], text: &str, options: ShapeOptions) -> Result<ShapedRun> {
        crate::cancel::check_current_cancel("OpenType shaping")?;
        if text.is_empty() {
            return Ok(ShapedRun {
                glyphs: Vec::new(),
                direction: options.direction.unwrap_or(TextDirection::LeftToRight),
                used_complex_shaping: false,
            });
        }
        if text.len() > 4_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "shaping input exceeds 4 MB".into(),
            ));
        }
        let mut hash = Sha256::new();
        hash.update(b"paragraph-v4-hard-lines");
        hash.update((font_bytes.len() as u64).to_le_bytes());
        hash.update(font_bytes);
        hash.update([match options.direction {
            None => 0,
            Some(TextDirection::LeftToRight) => 1,
            Some(TextDirection::RightToLeft) => 2,
        }]);
        hash.update(text.as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        if let Some(hit) = cached_shape(&key) {
            return Ok(hit);
        }
        let result = shape_uncached(font_bytes, text, options)?;
        cache_shape(key, &result);
        Ok(result)
    }
}
fn shape_uncached(font_bytes: &[u8], text: &str, options: ShapeOptions) -> Result<ShapedRun> {
    shape_paragraphs(font_bytes, text, options, &OpenTypeSettings::default())
}

/// Analyze the full input once, then use the same itemization, context, flags
/// and cluster conversion as a measured/generated line. This avoids a separate
/// default-settings shaper with different boundary behavior.
fn shape_paragraphs(
    font_bytes: &[u8],
    text: &str,
    options: ShapeOptions,
    settings: &OpenTypeSettings,
) -> Result<ShapedRun> {
    let prepared = ParagraphBidi::new(text, options)?;
    let direction = options.direction.unwrap_or_else(|| {
        if prepared
            .info
            .paragraphs
            .first()
            .is_some_and(|p| p.level.is_rtl())
        {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }
    });
    let mut glyphs = Vec::new();
    prepared.all_hard_lines(|range, line| {
        crate::cancel::check_current_cancel("OpenType paragraph shaping")?;
        let shaped = TextShaper::shape_resolved(font_bytes, &text[range.clone()], &line, settings)?;
        for mut glyph in shaped.glyphs {
            glyph.cluster += range.start as u32;
            glyphs.push(glyph);
        }
        Ok(true)
    })?;
    Ok(ShapedRun {
        glyphs,
        direction,
        used_complex_shaping: !text.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::font_rasterizer::get_fallback_font;

    #[test]
    fn continuation_line_retains_isolate_embedding_levels() {
        let text = "English \u{2067}אבג 123 more\u{2069} end";
        let start = text.find("123").unwrap();
        let options = ShapeOptions {
            direction: Some(TextDirection::LeftToRight),
        };
        let inherited = resolve_line_bidi(text, start..start + 3, options).unwrap();
        let isolated = resolve_line_bidi("123", 0..3, options).unwrap();
        assert!(inherited.levels.iter().all(|level| *level == 2));
        assert!(isolated.levels.iter().all(|level| *level == 0));
        let font = get_fallback_font("Symbol").unwrap();
        let shaped =
            TextShaper::shape_resolved(font, "123", &inherited, &OpenTypeSettings::default())
                .unwrap();
        assert_eq!(
            shaped.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn retained_cmap_does_not_make_removed_subset_outlines_editable() {
        let font = get_fallback_font("Helvetica").unwrap();
        let face = ttf_parser::Face::parse(font, 0).unwrap();
        let keep = std::collections::BTreeSet::from([face.glyph_index('A').unwrap().0]);
        let subset = crate::fonts::sfnt_subset::subset_glyf_preserving_gids(font, &keep).unwrap();
        let present = TextShaper::shape(&subset.bytes, "A", ShapeOptions::default()).unwrap();
        assert!(!has_missing_glyphs(&subset.bytes, "A", &present).unwrap());
        let missing = TextShaper::shape(&subset.bytes, "Z", ShapeOptions::default()).unwrap();
        assert!(has_missing_glyphs(&subset.bytes, "Z", &missing).unwrap());
    }

    #[test]
    fn latin_uses_opentype_shaping() {
        let font = get_fallback_font("Helvetica").expect("bundled font");
        let run = TextShaper::shape(font, "Hello", ShapeOptions::default()).expect("shape latin");

        assert_eq!(run.glyphs.len(), 5);
        assert_eq!(run.direction, TextDirection::LeftToRight);
        assert!(run.used_complex_shaping);
        assert!(run.glyphs.iter().all(|glyph| glyph.advance >= 0.0));
    }

    #[test]
    fn arabic_uses_complex_rtl_shaping_when_font_supports_it() {
        let font = get_fallback_font("Symbol").expect("DejaVu fallback");
        let run = TextShaper::shape(
            font,
            "\u{0633}\u{0644}\u{0627}\u{0645}",
            ShapeOptions::default(),
        )
        .expect("shape arabic");

        assert!(run.used_complex_shaping);
        assert_eq!(run.direction, TextDirection::RightToLeft);
        assert!(!run.glyphs.is_empty());
        assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id > 0));
    }

    #[test]
    fn hebrew_uses_complex_rtl_shaping_when_font_supports_it() {
        let font = get_fallback_font("Symbol").expect("DejaVu fallback");
        let run = TextShaper::shape(
            font,
            "\u{05E9}\u{05DC}\u{05D5}\u{05DD}",
            ShapeOptions::default(),
        )
        .expect("shape hebrew");

        assert!(run.used_complex_shaping);
        assert_eq!(run.direction, TextDirection::RightToLeft);
        assert!(!run.glyphs.is_empty());
        assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id > 0));
    }

    #[test]
    fn invalid_font_bytes_fail_cleanly() {
        let err = TextShaper::shape(b"not a font", "Hello", ShapeOptions::default())
            .expect_err("invalid font should fail");
        assert_eq!(err.code(), "unsupported_feature");
    }

    #[test]
    fn joiner_carriers_do_not_make_visible_coverage_fail() {
        let font = get_fallback_font("Symbol").unwrap();
        for text in ["A\u{200D}B", "ب\u{200C}ب", "ب\u{200D}ب", "A\u{FE0F}"] {
            let shaped = TextShaper::shape(font, text, ShapeOptions::default()).unwrap();
            assert!(
                !has_missing_glyphs(font, text, &shaped).unwrap(),
                "{text:?}"
            );
        }
    }
}
