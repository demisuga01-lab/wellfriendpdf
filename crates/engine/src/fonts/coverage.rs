//! Coverage of final shaped clusters, not a scalar-cmap proxy. The shaper
//! removes default-ignorable output after GSUB/GPOS; this checks the surviving
//! source clusters and detects retained cmap entries with absent subset outlines.
use super::ShapedRun;
use crate::{Result, WellfriendError};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use unicode_segmentation::UnicodeSegmentation;

const MAX_GLYPHS: usize = 1_000_000;

pub(crate) struct TextCoverage {
    pub missing_clusters: Vec<u32>,
    pub missing_scalars: Vec<char>,
    pub required_scalars: usize,
    pub covered_scalars: usize,
    pub advance_1000: f64,
}

/// Horizontal/default-feature approval summary. Final writers must repeat
/// coverage after line breaking, direction, feature or writing-mode changes.
pub(crate) fn analyze_text(font: &[u8], text: &str) -> Result<TextCoverage> {
    let run = super::TextShaper::shape(font, text, super::ShapeOptions::default())?;
    let missing_clusters = missing_glyph_clusters(font, text, &run)?;
    let mut boundaries = run
        .glyphs
        .iter()
        .map(|glyph| glyph.cluster as usize)
        .chain(missing_clusters.iter().map(|cluster| *cluster as usize))
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();
    let required = text
        .chars()
        .filter(|ch| outline_expected(*ch))
        .collect::<BTreeSet<_>>();
    let mut missing_scalars = BTreeSet::new();
    for cluster in &missing_clusters {
        crate::cancel::check_current_cancel("substitution coverage summary")?;
        let start = *cluster as usize;
        let end = boundaries
            .get(boundaries.partition_point(|offset| *offset <= start))
            .copied()
            .unwrap_or(text.len());
        missing_scalars.extend(text[start..end].chars().filter(|ch| outline_expected(*ch)));
    }
    let advance_1000 = run
        .glyphs
        .iter()
        .map(|glyph| glyph.advance.abs())
        .sum::<f64>();
    if !advance_1000.is_finite() {
        return Err(WellfriendError::invalid_input(
            "non-finite shaped coverage advance",
        ));
    }
    Ok(TextCoverage {
        covered_scalars: required.len().saturating_sub(missing_scalars.len()),
        required_scalars: required.len(),
        missing_clusters,
        missing_scalars: missing_scalars.into_iter().collect(),
        advance_1000,
    })
}

pub(crate) fn outline_expected(c: char) -> bool {
    !c.is_whitespace()
        && !c.is_control()
        && !matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F |
        0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF |
        0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF)
}

/// Sorted UTF-8 cluster starts requiring a missing/notdef glyph or an absent
/// visible outline. Invalid cluster offsets/positions are errors, not coverage.
/// This is not proof of intended typography or pixel-level visual equivalence.
pub fn missing_glyph_clusters(font: &[u8], text: &str, run: &ShapedRun) -> Result<Vec<u32>> {
    PreparedCoverage::new(font)?.missing_glyph_clusters(text, run)
}

/// Parsed coverage state for one immutable font program. Layout explores many
/// candidate lines from the same font, so reparsing sfnt/CFF2 outline state for
/// every candidate or emitted line is both unnecessary and a denial-of-service
/// multiplier on otherwise bounded text.
pub(crate) struct PreparedCoverage<'a> {
    face: ttf_parser::Face<'a>,
    outlines: super::sfnt_outline::PreparedOutlines,
}

impl<'a> PreparedCoverage<'a> {
    pub(crate) fn new(font: &'a [u8]) -> Result<Self> {
        let face = ttf_parser::Face::parse(font, 0)
            .map_err(|_| WellfriendError::invalid_input("invalid coverage font"))?;
        let outlines = super::sfnt_outline::PreparedOutlines::new(&face)?;
        Ok(Self { face, outlines })
    }

    pub(crate) fn has_missing_glyphs(&self, text: &str, run: &ShapedRun) -> Result<bool> {
        Ok(!self.missing_glyph_clusters(text, run)?.is_empty())
    }

    pub(crate) fn missing_glyph_clusters(&self, text: &str, run: &ShapedRun) -> Result<Vec<u32>> {
        crate::cancel::check_current_cancel("shaped coverage")?;
        if text.len() > 4_000_000 || run.glyphs.len() > MAX_GLYPHS {
            return Err(WellfriendError::ResourceLimit(
                "shaped coverage budget exceeded".into(),
            ));
        }
        let mut starts = BTreeSet::new();
        for (index, glyph) in run.glyphs.iter().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("coverage cluster indexing")?;
            }
            let start = glyph.cluster as usize;
            if start >= text.len()
                || !text.is_char_boundary(start)
                || ![glyph.advance, glyph.offset_x, glyph.offset_y]
                    .iter()
                    .all(|v| v.is_finite())
            {
                return Err(WellfriendError::invalid_input(
                    "invalid shaped coverage cluster or position",
                ));
            }
            starts.insert(start);
        }
        let mut missing = BTreeSet::new();
        // A completely empty run (or omitted visible prefix) must not pass visible
        // coverage vacuously. Removed controls/whitespace do not need an outline.
        let first = starts.first().copied().unwrap_or(text.len());
        for (index, (offset, grapheme)) in text[..first].grapheme_indices(true).enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("coverage omitted prefix")?;
            }
            if grapheme.chars().any(outline_expected) {
                missing.insert(offset as u32);
            }
            if missing.len() > MAX_GLYPHS {
                return Err(WellfriendError::ResourceLimit(
                    "coverage cluster budget exceeded".into(),
                ));
            }
        }
        let mut boundaries = starts.into_iter().collect::<Vec<_>>();
        boundaries.push(text.len());
        let mut cluster_policy = BTreeMap::new();
        for (index, pair) in boundaries.windows(2).enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("coverage cluster semantics")?;
            }
            let source = &text[pair[0]..pair[1]];
            let mut blanks = BTreeSet::new();
            for ch in source
                .chars()
                .filter(|ch| ch.is_whitespace() && !super::hard_break::is_hard_break(*ch))
            {
                if let Some(gid) = self.face.glyph_index(ch) {
                    blanks.insert(gid.0);
                }
                // The pinned normalizer can synthesize Unicode space advances
                // from the ordinary space glyph when a dedicated cmap is absent.
                if matches!(
                    ch as u32,
                    0x0020 | 0x00A0 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000
                ) {
                    if let Some(gid) = self.face.glyph_index(' ') {
                        blanks.insert(gid.0);
                    }
                }
            }
            cluster_policy.insert(pair[0], (source.chars().any(outline_expected), blanks));
        }
        let mut outlines = HashMap::new();
        for (index, glyph) in run.glyphs.iter().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("coverage outlines")?;
            }
            if glyph.glyph_id == 0 || glyph.glyph_id >= self.face.number_of_glyphs() {
                missing.insert(glyph.cluster);
            } else if cluster_policy[&(glyph.cluster as usize)].0 {
                let exists = if let Some(exists) = outlines.get(&glyph.glyph_id) {
                    *exists
                } else {
                    let exists = self
                        .outlines
                        .bounds(&self.face, ttf_parser::GlyphId(glyph.glyph_id))?
                        .is_some();
                    outlines.insert(glyph.glyph_id, exists);
                    exists
                };
                if !exists
                    && !cluster_policy[&(glyph.cluster as usize)]
                        .1
                        .contains(&glyph.glyph_id)
                {
                    missing.insert(glyph.cluster);
                }
            }
        }
        Ok(missing.into_iter().collect())
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
