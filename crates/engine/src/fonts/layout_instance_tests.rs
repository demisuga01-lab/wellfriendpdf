//! Unexecuted structure/shaping regressions for FeatureVariations freezing.
use super::*;
fn word(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn set16(out: &mut [u8], at: usize, n: usize) {
    out[at..at + 2].copy_from_slice(&(n as u16).to_be_bytes());
}
fn set32(out: &mut [u8], at: usize, n: usize) {
    out[at..at + 4].copy_from_slice(&(n as u32).to_be_bytes());
}
#[derive(Clone)]
struct Rule {
    range: Option<(u16, i16, i16)>,
    format: u16,
    version: Option<u32>,
    lookup: u16,
}
fn rule(lookup: u16) -> Rule {
    Rule {
        range: Some((0, 8192, 16384)),
        format: 1,
        version: Some(0x10000),
        lookup,
    }
}
fn fixture(
    tag: [u8; 4],
    rules: &[Rule],
    parameter: Option<([u8; 4], Vec<u8>)>,
    extended: bool,
) -> Vec<u8> {
    let mut out = vec![0; 14];
    set32(&mut out, 0, 0x10001);
    let scripts = out.len();
    set16(&mut out, 4, scripts);
    word(&mut out, 1);
    out.extend_from_slice(b"DFLT");
    word(&mut out, 8);
    word(&mut out, 4);
    word(&mut out, 0); // default LangSys, no named languages
    for n in [0, 0xffff, 1, 0] {
        word(&mut out, n);
    }
    let features = out.len();
    set16(&mut out, 6, features);
    word(&mut out, 1);
    out.extend_from_slice(&parameter.as_ref().map_or(*b"liga", |p| p.0));
    word(&mut out, 8);
    for n in [0, 1, 0] {
        word(&mut out, n);
    }
    let lookups = out.len();
    set16(&mut out, 8, lookups);
    for n in [2, 0, 0] {
        word(&mut out, n);
    }
    for i in 0..2 {
        let at = out.len();
        set16(&mut out, lookups + 2 + i * 2, at - lookups);
        let extension = if tag == *b"GSUB" { 7 } else { 9 };
        for n in [if extended { extension } else { 1 }, 0x10, 1, 10, 3] {
            word(&mut out, n);
        }
        if extended {
            word(&mut out, 1);
            word(&mut out, 1);
            out.extend_from_slice(&8u32.to_be_bytes());
        }
        if tag == *b"GSUB" {
            for n in [2, 8, 1, 2 + i as u16] {
                word(&mut out, n);
            }
        } else {
            // SinglePos format 1; xAdvance is 10 or 20 font units.
            for n in [1, 8, 4, (i as u16 + 1) * 10] {
                word(&mut out, n);
            }
        }
        for n in [1, 1, 1] {
            word(&mut out, n);
        } // coverage gid 1
    }
    let variation = out.len();
    set32(&mut out, 10, variation);
    out.extend_from_slice(&0x10000u32.to_be_bytes());
    out.extend_from_slice(&(rules.len() as u32).to_be_bytes());
    out.resize(out.len() + rules.len() * 8, 0);
    for (i, rule) in rules.iter().enumerate() {
        let record = variation + 8 + i * 8;
        if let Some((axis, min, max)) = rule.range {
            let condition = out.len();
            set32(&mut out, record, condition - variation);
            word(&mut out, 1);
            out.extend_from_slice(&6u32.to_be_bytes());
            for n in [rule.format, axis, min as u16, max as u16] {
                word(&mut out, n);
            }
        }
        if let Some(version) = rule.version {
            let substitute = out.len();
            set32(&mut out, record + 4, substitute - variation);
            out.extend_from_slice(&version.to_be_bytes());
            word(&mut out, 1);
            word(&mut out, 0);
            out.extend_from_slice(&12u32.to_be_bytes());
            let feature = out.len();
            for n in [if parameter.is_some() { 6 } else { 0 }, 1, rule.lookup] {
                word(&mut out, n);
            }
            if let Some((_, params)) = &parameter {
                out.extend_from_slice(params);
            }
            assert_eq!(feature - substitute, 12);
        }
    }
    out
}
fn coordinate(value: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(value)]
}
fn chosen_lookup(data: &[u8]) -> u16 {
    let root = usize::from(u16_at(data, 6).unwrap());
    let feature = root + usize::from(u16_at(data, root + 6).unwrap());
    u16_at(data, feature + 4).unwrap()
}
fn font(table: Vec<u8>, tag: [u8; 4]) -> Vec<u8> {
    let input = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let container = crate::fonts::font_container::Container::parse(&input).unwrap();
    let mut tables = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, input[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    tables.insert(tag, table);
    // The fixture tests mark-filtering preservation structurally; shaping uses
    // ordinary glyphs and does not require a GDEF mark glyph set.
    crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap()
}
#[test]
fn first_matching_record_wins_and_range_endpoints_are_inclusive() {
    let data = fixture(*b"GSUB", &[rule(1), rule(0)], None, false);
    for value in [8192, 16384] {
        let frozen = freeze(&data, *b"GSUB", &coordinate(value)).unwrap();
        assert_eq!(frozen.selected_record, Some(0));
        assert_eq!(frozen.substituted_features, [0]);
        assert_eq!(chosen_lookup(&frozen.bytes), 1);
        assert_eq!(u32_at(&frozen.bytes, 0).unwrap(), 0x10000);
    }
    let frozen = freeze(&data, *b"GSUB", &coordinate(8191)).unwrap();
    assert_eq!(frozen.selected_record, None);
    assert_eq!(chosen_lookup(&frozen.bytes), 0);
}
#[test]
fn null_substitution_is_a_matching_noop_not_permission_to_choose_a_later_record() {
    let mut first = rule(1);
    first.range = None;
    first.version = None;
    let data = fixture(*b"GSUB", &[first, rule(1)], None, false);
    let frozen = freeze(&data, *b"GSUB", &coordinate(16384)).unwrap();
    assert_eq!(frozen.selected_record, Some(0));
    assert!(frozen.substituted_features.is_empty());
    assert_eq!(chosen_lookup(&frozen.bytes), 0);
}
#[test]
fn unknown_conditions_axes_and_substitution_versions_skip_to_supported_records() {
    for mut first in [rule(0), rule(0), rule(0)].into_iter().enumerate() {
        match first.0 {
            0 => first.1.format = 2,
            1 => first.1.range = Some((1, 0, 16384)),
            _ => first.1.version = Some(0x20000),
        }
        let data = fixture(*b"GSUB", &[first.1, rule(1)], None, false);
        assert_eq!(
            freeze(&data, *b"GSUB", &coordinate(16384))
                .unwrap()
                .selected_record,
            Some(1)
        );
    }
}
#[test]
fn emitted_lookups_point_into_one_exact_source_block_and_keep_flags_and_filter_sets() {
    for (tag, extended) in [
        (*b"GSUB", false),
        (*b"GSUB", true),
        (*b"GPOS", false),
        (*b"GPOS", true),
    ] {
        let data = fixture(tag, &[rule(1)], None, extended);
        let frozen = freeze(&data, tag, &coordinate(16384)).unwrap();
        assert!(frozen.retained_source_block);
        let base = frozen.bytes.len() - data.len();
        assert_eq!(&frozen.bytes[base..], &data);
        let root = usize::from(u16_at(&frozen.bytes, 8).unwrap());
        for i in 0..2 {
            let lookup = root + usize::from(u16_at(&frozen.bytes, root + 2 + i * 2).unwrap());
            assert_eq!(
                u16_at(&frozen.bytes, lookup).unwrap(),
                if tag == *b"GSUB" { 7 } else { 9 }
            );
            assert_eq!(u16_at(&frozen.bytes, lookup + 2).unwrap(), 0x10);
            assert_eq!(u16_at(&frozen.bytes, lookup + 8).unwrap(), 3);
            let extension = lookup + usize::from(u16_at(&frozen.bytes, lookup + 6).unwrap());
            assert_eq!(u16_at(&frozen.bytes, extension + 2).unwrap(), 1);
            let subtable = extension + u32_at(&frozen.bytes, extension + 4).unwrap() as usize;
            assert!(subtable >= base && subtable < frozen.bytes.len());
            assert_eq!(
                u16_at(&frozen.bytes, subtable).unwrap(),
                if tag == *b"GSUB" { 2 } else { 1 }
            );
        }
    }
}
#[test]
fn frozen_feature_selection_is_used_by_reopened_font_shaping() {
    let mut data = fixture(*b"GSUB", &[rule(1)], None, false);
    // Remove filtering for the shaping fixture, independently of preservation tests.
    let root = usize::from(u16_at(&data, 8).unwrap());
    for i in 0..2 {
        let at = root + usize::from(u16_at(&data, root + 2 + i * 2).unwrap());
        set16(&mut data, at + 2, 0);
    }
    for (value, expected) in [(0, 2), (16384, 3)] {
        let frozen = freeze(&data, *b"GSUB", &coordinate(value)).unwrap();
        let font = font(frozen.bytes, *b"GSUB");
        let shaped = crate::fonts::TextShaper::shape(&font, "A", Default::default()).unwrap();
        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].glyph_id, expected);
    }
}
#[test]
fn registered_feature_parameters_survive_relocation() {
    let mut cv = vec![0; 14];
    cv[12..14].copy_from_slice(&1u16.to_be_bytes());
    cv.extend_from_slice(&[0, 0, 65]);
    for (tag, params) in [
        (*b"size", vec![0; 10]),
        (*b"ss01", vec![0, 0, 1, 0]),
        (*b"cv01", cv),
    ] {
        let data = fixture(*b"GSUB", &[rule(1)], Some((tag, params.clone())), false);
        let frozen = freeze(&data, *b"GSUB", &coordinate(16384)).unwrap();
        let root = usize::from(u16_at(&frozen.bytes, 6).unwrap());
        let feature = root + usize::from(u16_at(&frozen.bytes, root + 6).unwrap());
        assert_eq!(
            parameters(&frozen.bytes, feature, &tag).unwrap().unwrap(),
            params
        );
    }
    let data = fixture(*b"GSUB", &[rule(1)], Some((*b"priv", vec![0; 4])), false);
    assert!(freeze(&data, *b"GSUB", &coordinate(16384)).is_err());
}
#[test]
fn truncated_offsets_invalid_indices_and_wrong_table_kinds_fail() {
    let data = fixture(*b"GSUB", &[rule(1)], None, false);
    for end in [0, 9, 13, data.len() - 1] {
        assert!(freeze(&data[..end], *b"GSUB", &coordinate(16384)).is_err());
    }
    let bad = fixture(*b"GSUB", &[rule(2)], None, false);
    assert!(freeze(&bad, *b"GSUB", &coordinate(16384)).is_err());
    assert!(freeze(&data, *b"GDEF", &coordinate(16384)).is_err());
    let mut bad = data;
    set16(&mut bad, 6, 0xffff);
    assert!(freeze(&bad, *b"GSUB", &coordinate(16384)).is_err());
}
#[test]
fn source_without_variations_retains_all_original_offsets_and_bytes() {
    let mut data = fixture(*b"GSUB", &[], None, false);
    set32(&mut data, 0, 0x10000);
    let frozen = freeze(&data, *b"GSUB", &[]).unwrap();
    assert_eq!(frozen.bytes, data);
    assert_eq!(frozen.selected_record, None);
}
#[test]
fn cancelling_an_instance_never_returns_a_serialized_table() {
    let data = fixture(*b"GSUB", &[rule(1)], None, false);
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| freeze(&data, *b"GSUB", &coordinate(16384)))
        .is_err());
}
#[test]
fn offset_and_output_budgets_refuse_instead_of_truncating() {
    let mut w = Writer { out: vec![0; 4] };
    assert!(w.link(0, 0, 65536).is_err());
    assert!(w.link(0, 4, 2).is_err());
    assert!(w.reserve(LIMIT).is_err());
}

#[test]
fn frozen_positioning_features_change_the_actual_shaped_advance() {
    let mut data = fixture(*b"GPOS", &[rule(1)], None, false);
    let features = usize::from(u16_at(&data, 6).unwrap());
    data[features + 2..features + 6].copy_from_slice(b"kern");
    let lookups = usize::from(u16_at(&data, 8).unwrap());
    for i in 0..2 {
        let lookup = lookups + usize::from(u16_at(&data, lookups + 2 + i * 2).unwrap());
        set16(&mut data, lookup + 2, 0);
    }
    for (value, expected) in [(0, 610.), (16384, 620.)] {
        let frozen = freeze(&data, *b"GPOS", &coordinate(value)).unwrap();
        let font = font(frozen.bytes, *b"GPOS");
        let shaped = crate::fonts::TextShaper::shape(&font, "A", Default::default()).unwrap();
        assert_eq!(shaped.glyphs.len(), 1);
        assert_eq!(shaped.glyphs[0].advance, expected);
    }
}
