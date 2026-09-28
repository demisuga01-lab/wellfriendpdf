//! Mixed-orientation vertical shaping. The output advances down the column;
//! offsets are in the glyph's unrotated horizontal-outline coordinate system.
//! Existing PDF glyph codes are not reshaped by this module.
use super::shaper::{script_ranges, OpenTypeSettings, ShapeOptions, ShapedGlyph, TextShaper};
use crate::{Result, WellfriendError};
use rustybuzz::{Direction, UnicodeBuffer};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;
#[path = "vertical_orientation_data.rs"]
mod data;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Upright,
    Rotated,
    TransformedUpright,
    TransformedRotated,
}
pub const UNICODE_VERSION: &str = "17.0.0";
pub fn orientation(ch: char) -> Orientation {
    let code = ch as u32;
    let index = data::RANGES.partition_point(|&(start, _, _)| start <= code);
    if index != 0 && code <= data::RANGES[index - 1].1 {
        data::RANGES[index - 1].2
    } else {
        Orientation::Rotated
    }
}
#[derive(Debug, Clone)]
pub struct VerticalGlyph {
    pub glyph: ShapedGlyph,
    pub rotate_clockwise: bool,
    pub vertical_alternate: bool,
    /// Physical horizontal pen movement, in 1000-unit em space.
    pub cross_advance: f64,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

/// Shape one already-broken vertical line. Graphemes keep one orientation;
/// sideways script runs retain horizontal shaping and bidi visual order.
/// Tr characters use a rotated fallback only when vertical GSUB did not supply
/// another glyph. Tu characters retain upright fallback. No font is substituted.
pub fn shape(font: &[u8], text: &str, settings: &OpenTypeSettings) -> Result<Vec<VerticalGlyph>> {
    let bidi = super::shaper::resolve_line_bidi(
        text,
        0..text.len(),
        ShapeOptions {
            direction: Some(super::TextDirection::LeftToRight),
        },
    )?;
    shape_resolved(font, text, &bidi, settings)
}
pub fn shape_resolved(
    font: &[u8],
    text: &str,
    bidi: &super::shaper::LineBidi,
    settings: &OpenTypeSettings,
) -> Result<Vec<VerticalGlyph>> {
    crate::cancel::check_current_cancel("vertical OpenType shaping")?;
    if text.len() > 4_000_000
        || bidi.levels.len() != text.len()
        || settings.features.len() > 64
        || settings.features.iter().any(|s| s.len() > 128)
        || settings.language.as_ref().is_some_and(|s| s.len() > 128)
    {
        return Err(fail("vertical shaping budget exceeded"));
    }
    bidi.context.validate()?;
    if text.chars().any(super::hard_break::is_hard_break) {
        return Err(fail("vertical shaping expects one logical column"));
    }
    for (offset, ch) in text.char_indices() {
        let level = bidi.levels[offset];
        if unicode_bidi::Level::new(level).is_err()
            || bidi.levels[offset..offset + ch.len_utf8()]
                .iter()
                .any(|v| *v != level)
        {
            return Err(fail(
                "vertical bidi levels divide a scalar or contain an invalid level",
            ));
        }
    }
    let face = rustybuzz::Face::from_slice(font, 0)
        .ok_or_else(|| fail("invalid vertical shaping font"))?;
    let upem = f64::from(face.units_per_em()).max(1.0);
    let features = settings
        .features
        .iter()
        .map(|s| s.parse::<rustybuzz::Feature>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| fail("invalid vertical OpenType feature"))?;
    if features.iter().any(|f| f.start != 0 || f.end != u32::MAX) {
        return Err(fail("vertical features must cover complete runs"));
    }
    let language = settings
        .language
        .as_deref()
        .map(str::parse::<rustybuzz::Language>)
        .transpose()
        .map_err(|_| fail("invalid vertical language"))?;
    let mut segments: Vec<(std::ops::Range<usize>, Orientation)> = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        let kind = orientation(grapheme.chars().next().unwrap());
        if let Some((range, last)) = segments.last_mut() {
            if *last == kind {
                range.end = start + grapheme.len();
                continue;
            }
        }
        segments.push((start..start + grapheme.len(), kind));
    }
    let center = (f64::from(face.ascender()) + f64::from(face.descender())) / upem * 500.0;
    let horizontal_shape = |range: std::ops::Range<usize>| {
        let line = bidi.slice(text, range.clone(), bidi.rtl)?;
        TextShaper::shape_resolved(font, &text[range], &line, settings)
    };
    let mut output = Vec::new();
    for (range, kind) in segments {
        crate::cancel::check_current_cancel("vertical orientation run")?;
        let slice = &text[range.clone()];
        if kind == Orientation::Rotated {
            let shaped = horizontal_shape(range.clone())?;
            for mut glyph in shaped.glyphs {
                glyph.cluster += range.start as u32;
                glyph.offset_y -= center;
                output.push(VerticalGlyph {
                    glyph,
                    rotate_clockwise: true,
                    vertical_alternate: false,
                    cross_advance: 0.0,
                });
            }
            continue;
        }
        for (script_range, script) in script_ranges(slice) {
            let start = range.start + script_range.start;
            let end = range.start + script_range.end;
            let run = &text[start..end];
            let mut buffer = UnicodeBuffer::new();
            buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
            buffer.push_str(run);
            bidi.context.apply(&mut buffer, text, start..end)?;
            buffer.set_direction(Direction::TopToBottom);
            if let Some(language) = &language {
                buffer.set_language(language.clone());
            }
            if let Ok(script) = script.short_name().parse::<rustybuzz::Script>() {
                buffer.set_script(script);
            }
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &features, buffer);
            // Compare complete cluster sequences, not just the first GID. This
            // keeps a transformed base plus marks together when choosing Tr's
            // fallback and does not confuse GPOS offsets with glyph substitution.
            let horizontal = if matches!(
                kind,
                Orientation::TransformedRotated | Orientation::TransformedUpright
            ) {
                Some(horizontal_shape(start..end)?)
            } else {
                None
            };
            let mut vertical_ids = BTreeMap::<u32, Vec<u16>>::new();
            let mut horizontal_ids = BTreeMap::<u32, Vec<u16>>::new();
            let mut horizontal_glyphs = BTreeMap::<u32, Vec<ShapedGlyph>>::new();
            for info in shaped.glyph_infos() {
                vertical_ids.entry(info.cluster).or_default().push(
                    u16::try_from(info.glyph_id)
                        .map_err(|_| fail("vertical glyph ID exceeds sfnt range"))?,
                );
            }
            if let Some(horizontal) = &horizontal {
                for glyph in &horizontal.glyphs {
                    horizontal_ids
                        .entry(glyph.cluster)
                        .or_default()
                        .push(glyph.glyph_id);
                    horizontal_glyphs
                        .entry(glyph.cluster)
                        .or_default()
                        .push(glyph.clone());
                }
            }
            let alternates = vertical_ids
                .iter()
                .filter_map(|(cluster, ids)| {
                    (horizontal.is_some() && horizontal_ids.get(cluster) != Some(ids))
                        .then_some(*cluster)
                })
                .collect::<BTreeSet<_>>();
            let mut fallback_emitted = BTreeSet::new();
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                crate::cancel::check_current_cancel("vertical glyph positioning")?;
                let alternate = alternates.contains(&info.cluster);
                if kind == Orientation::TransformedRotated && !alternate {
                    if fallback_emitted.insert(info.cluster) {
                        for mut glyph in horizontal_glyphs
                            .get(&info.cluster)
                            .into_iter()
                            .flatten()
                            .cloned()
                        {
                            glyph.cluster += start as u32;
                            glyph.offset_y -= center;
                            output.push(VerticalGlyph {
                                glyph,
                                rotate_clockwise: true,
                                vertical_alternate: false,
                                cross_advance: 0.0,
                            });
                        }
                    }
                } else {
                    output.push(VerticalGlyph {
                        glyph: ShapedGlyph {
                            glyph_id: u16::try_from(info.glyph_id)
                                .map_err(|_| fail("vertical glyph ID exceeds sfnt range"))?,
                            cluster: start as u32 + info.cluster,
                            advance: -f64::from(pos.y_advance) / upem * 1000.0,
                            offset_x: f64::from(pos.x_offset) / upem * 1000.0,
                            offset_y: f64::from(pos.y_offset) / upem * 1000.0,
                        },
                        rotate_clockwise: false,
                        vertical_alternate: alternate,
                        cross_advance: f64::from(pos.x_advance) / upem * 1000.0,
                    });
                }
            }
        }
    }
    if output.iter().any(|g| {
        !g.glyph.advance.is_finite() || g.glyph.advance < 0.0 || !g.cross_advance.is_finite()
    }) {
        return Err(fail("invalid vertical glyph advance"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn font() -> &'static [u8] {
        crate::render::get_fallback_font("Symbol").unwrap()
    }
    #[test]
    fn unicode_17_orientation_ranges_are_ordered_and_cover_non_ascii_scripts() {
        assert!(data::RANGES.windows(2).all(|w| w[0].1 < w[1].0));
        assert_eq!(orientation('A'), Orientation::Rotated);
        assert_eq!(orientation('\u{03B1}'), Orientation::Rotated);
        assert_eq!(orientation('\u{4E00}'), Orientation::Upright);
        assert_eq!(orientation('\u{1FAE9}'), Orientation::Upright);
        assert_eq!(orientation('\u{3001}'), Orientation::TransformedUpright);
        assert_eq!(orientation('\u{2329}'), Orientation::TransformedRotated);
    }
    #[test]
    fn sideways_run_preserves_kerning_ligatures_and_combining_clusters() {
        let text = "office AV e\u{301}";
        let horizontal = TextShaper::shape(font(), text, ShapeOptions::default()).unwrap();
        let vertical = shape(font(), text, &Default::default()).unwrap();
        assert_eq!(horizontal.glyphs.len(), vertical.len());
        for (h, v) in horizontal.glyphs.iter().zip(&vertical) {
            assert!(v.rotate_clockwise);
            assert_eq!(h.glyph_id, v.glyph.glyph_id);
            assert_eq!(h.cluster, v.glyph.cluster);
            assert_eq!(h.advance, v.glyph.advance);
            assert_eq!(h.offset_x, v.glyph.offset_x);
        }
    }
    #[test]
    fn upright_output_retains_y_advances_and_vertical_origins_from_shaper() {
        let text = "\u{00A7}\u{00A9}";
        let face = rustybuzz::Face::from_slice(font(), 0).unwrap();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(Direction::TopToBottom);
        buffer.guess_segment_properties();
        let expected = rustybuzz::shape(&face, &[], buffer);
        let output = shape(font(), text, &Default::default()).unwrap();
        assert_eq!(output.len(), expected.len());
        let scale = 1000.0 / f64::from(face.units_per_em());
        for ((glyph, pos), info) in output
            .iter()
            .zip(expected.glyph_positions())
            .zip(expected.glyph_infos())
        {
            assert!(!glyph.rotate_clockwise);
            assert_eq!(u32::from(glyph.glyph.glyph_id), info.glyph_id);
            assert!((glyph.glyph.advance + f64::from(pos.y_advance) * scale).abs() < 1e-7);
            assert!((glyph.glyph.offset_x - f64::from(pos.x_offset) * scale).abs() < 1e-7);
            assert!((glyph.glyph.offset_y - f64::from(pos.y_offset) * scale).abs() < 1e-7);
        }
    }
    #[test]
    fn sideways_continuation_uses_paragraph_levels_instead_of_resolving_isolates_again() {
        let text = "start \u{2067}\u{05D0}\u{05D1} 123 end\u{2069}";
        let start = text.find("123").unwrap();
        let visible = &text[start..];
        let bidi = super::super::shaper::resolve_line_bidi(
            text,
            start..text.len(),
            ShapeOptions::default(),
        )
        .unwrap();
        let expected =
            TextShaper::shape_resolved(font(), visible, &bidi, &Default::default()).unwrap();
        let output = shape_resolved(font(), visible, &bidi, &Default::default()).unwrap();
        assert_eq!(
            output
                .iter()
                .map(|g| (g.glyph.glyph_id, g.glyph.cluster))
                .collect::<Vec<_>>(),
            expected
                .glyphs
                .iter()
                .map(|g| (g.glyph_id, g.cluster))
                .collect::<Vec<_>>()
        );
    }
}
