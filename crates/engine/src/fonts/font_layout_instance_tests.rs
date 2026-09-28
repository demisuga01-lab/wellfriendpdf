//! Regression source only: no compiler, tests or font workloads run in this change.
use super::*;
use crate::fonts::{
    position_instance::{signed, Resolver, Writer},
    variation_store::{u16_at, u32_at},
};
fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_be_bytes()).collect()
}
fn set(out: &mut [u8], field: usize, value: usize) {
    out[field..field + 2].copy_from_slice(&u16::try_from(value).unwrap().to_be_bytes());
}
fn set32(out: &mut [u8], field: usize, value: usize) {
    out[field..field + 4].copy_from_slice(&u32::try_from(value).unwrap().to_be_bytes());
}
fn append(out: &mut Vec<u8>, field: usize, owner: usize, child: &[u8]) -> usize {
    let at = out.len();
    set(out, field, at - owner);
    out.extend_from_slice(child);
    at
}
fn coverage(gids: &[u16]) -> Vec<u8> {
    let mut out = words(&[1, gids.len() as u16]);
    out.extend(words(gids));
    out
}
fn index() -> Vec<u8> {
    words(&[0, 0, 0x8000])
}
fn store(delta: i16) -> Vec<u8> {
    words(&[
        1,
        0,
        12,
        1,
        0,
        22,
        1,
        1,
        0,
        16384,
        16384,
        1,
        1,
        1,
        0,
        delta as u16,
    ])
}
fn gdef(delta: i16) -> Vec<u8> {
    let mut data = vec![0; 18];
    set32(&mut data, 0, 0x10003);
    set32(&mut data, 14, 18);
    data.extend(store(delta));
    data
}
fn layout(programs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0; 14];
    set32(&mut out, 0, 0x10001);
    let mut scripts = words(&[1]);
    scripts.extend(b"DFLT");
    scripts.extend(words(&[8, 4, 0, 0, 0xffff, 1, 0]));
    append(&mut out, 4, 0, &scripts);
    let mut features = words(&[1]);
    features.extend(b"kern");
    features.extend(words(&[8, 0, 1, 0]));
    append(&mut out, 6, 0, &features);
    let mut lookups = words(&[programs.len() as u16]);
    lookups.resize(2 + programs.len() * 2, 0);
    let root = append(&mut out, 8, 0, &lookups);
    for (i, (kind, program)) in programs.iter().enumerate() {
        let at = append(&mut out, root + 2 + i * 2, root, &words(&[*kind, 0, 1, 8]));
        assert_eq!(out.len(), at + 8);
        out.extend(program);
    }
    out
}
fn single(base: Option<i16>) -> Vec<u8> {
    let mut out = words(&[1, 0, if base.is_some() { 0x44 } else { 0x40 }]);
    if let Some(base) = base {
        out.extend(words(&[base as u16]));
    }
    let field = out.len();
    out.extend(words(&[0]));
    append(&mut out, field, 0, &index());
    append(&mut out, 2, 0, &coverage(&[1]));
    out
}
fn stage(programs: &[(u16, Vec<u8>)], delta: i16, coordinate: i16) -> LayoutStage {
    freeze(
        &BTreeMap::from([
            (*b"GPOS", Arc::from(layout(programs))),
            (*b"GDEF", Arc::from(gdef(delta))),
        ]),
        &[ttf_parser::NormalizedCoordinate::from(coordinate)],
    )
    .unwrap()
}
fn child(data: &[u8], base: usize, field: usize) -> usize {
    base + usize::from(u16_at(data, field).unwrap())
}
fn subtable(data: &[u8], i: usize) -> usize {
    let list = child(data, 0, 8);
    let lookup = child(data, list, list + 2 + i * 2);
    assert_eq!(u16_at(data, lookup).unwrap(), 9);
    let extension = child(data, lookup, lookup + 6);
    extension + u32_at(data, extension + 4).unwrap() as usize
}
#[test]
fn single_value_without_base_gets_a_scalar_field_and_zero_device_offset() {
    let stage = stage(&[(1, single(None))], 40, 16384);
    let data = &stage.tables[b"GPOS"];
    let at = subtable(data, 0);
    assert_eq!(u16_at(data, at + 4).unwrap(), 0x44);
    assert_eq!(signed(data, at + 6).unwrap(), 40);
    assert_eq!(u16_at(data, at + 8).unwrap(), 0);
    assert_eq!(stage.resolved_adjustments, 1);
    assert!(!stage.retained_gdef_store);
    assert_eq!(stage.retired_variation_stores, [*b"GDEF"]);
}
#[test]
fn fractional_position_deltas_round_towards_positive_infinity_at_halves() {
    for (delta, expected) in [(11, 106), (-11, 95)] {
        let stage = stage(&[(1, single(Some(100)))], delta, 8192);
        let data = &stage.tables[b"GPOS"];
        assert_eq!(signed(data, subtable(data, 0) + 6).unwrap(), expected);
    }
}
#[test]
fn single_per_glyph_records_sharing_one_index_apply_once_per_value() {
    let mut program = words(&[2, 0, 0x44, 2, 10, 0, 20, 0]);
    let device = append(&mut program, 10, 0, &index());
    set(&mut program, 14, device);
    append(&mut program, 2, 0, &coverage(&[1, 2]));
    let result = stage(&[(1, program)], 40, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    assert_eq!(signed(data, at + 8).unwrap(), 50);
    assert_eq!(signed(data, at + 12).unwrap(), 60);
    assert_eq!(result.resolved_adjustments, 2);
    assert_eq!(result.distinct_variation_indices, 1);
    assert_eq!(result.evaluated_delta_cells, 1);
}
#[test]
fn pair_set_device_offsets_are_owned_by_the_pair_set_not_the_lookup() {
    let mut program = words(&[1, 0, 0x44, 0x40, 1, 0]);
    let pair = append(&mut program, 10, 0, &words(&[1, 2, 10, 0, 0]));
    let device = append(&mut program, pair + 6, pair, &index());
    set(&mut program, pair + 8, device - pair);
    append(&mut program, 2, 0, &coverage(&[1]));
    let result = stage(&[(2, program)], -30, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    let pair = child(data, at, at + 10);
    assert_eq!(u16_at(data, at + 6).unwrap(), 0x44); // Keep second-record presence.
    assert_eq!(signed(data, pair + 4).unwrap(), -20);
    assert_eq!(signed(data, pair + 8).unwrap(), -30);
    assert_eq!(u16_at(data, pair + 6).unwrap(), 0);
    assert_eq!(u16_at(data, pair + 10).unwrap(), 0);
}
#[test]
fn class_pair_matrix_keeps_classes_and_per_cell_values() {
    let mut program = words(&[2, 0, 0x44, 0, 0, 0, 2, 2, 1, 0, 2, 0, 3, 0, 4, 0]);
    let device = append(&mut program, 18, 0, &index());
    for field in [22, 26, 30] {
        set(&mut program, field, device);
    }
    append(&mut program, 2, 0, &coverage(&[1]));
    append(&mut program, 8, 0, &words(&[1, 1, 1, 1]));
    append(&mut program, 10, 0, &words(&[2, 1, 2, 2, 1]));
    let result = stage(&[(2, program)], 10, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    for i in 0..4 {
        assert_eq!(signed(data, at + 16 + i * 4).unwrap(), 11 + i as i16);
    }
    assert_eq!(u16_at(data, child(data, at, at + 8) + 6).unwrap(), 1);
}
fn anchor() -> Vec<u8> {
    words(&[3, 100, 200, 10, 10, 0, 0, 0x8000])
}
#[test]
fn cursive_anchor_variations_are_anchor_relative_and_null_anchors_survive() {
    let mut program = words(&[1, 0, 1, 0, 0]);
    append(&mut program, 6, 0, &anchor());
    append(&mut program, 2, 0, &coverage(&[1]));
    let result = stage(&[(3, program)], 30, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    let anchor = child(data, at, at + 6);
    assert_eq!(signed(data, anchor + 2).unwrap(), 130);
    assert_eq!(signed(data, anchor + 4).unwrap(), 230);
    assert_eq!(u16_at(data, anchor + 6).unwrap(), 0);
    assert_eq!(u16_at(data, anchor + 8).unwrap(), 0);
    assert_eq!(u16_at(data, at + 8).unwrap(), 0);
}
fn mark_program(kind: u16) -> Vec<u8> {
    let mut program = words(&[1, 0, 0, 2, 0, 0]);
    let mark = append(&mut program, 8, 0, &words(&[1, 1, 0]));
    append(&mut program, mark + 4, mark, &anchor());
    let base_header = if kind == 5 {
        words(&[1, 0])
    } else {
        words(&[1, 0, 0])
    };
    let base = append(&mut program, 10, 0, &base_header);
    let matrix = if kind == 5 {
        append(&mut program, base + 2, base, &words(&[2, 0, 0, 0, 0]))
    } else {
        base
    };
    append(&mut program, matrix + 4, matrix, &anchor());
    if kind == 5 {
        append(&mut program, matrix + 8, matrix, &words(&[1, 300, 400]));
    }
    append(&mut program, 2, 0, &coverage(&[3]));
    append(&mut program, 4, 0, &coverage(&[1]));
    program
}
#[test]
fn all_mark_lookup_kinds_relocate_their_distinct_anchor_owners() {
    for kind in [4, 5, 6] {
        let result = stage(&[(kind, mark_program(kind))], -20, 16384);
        let data = &result.tables[b"GPOS"];
        let at = subtable(data, 0);
        let mark = child(data, at, at + 8);
        let mark_anchor = child(data, mark, mark + 4);
        assert_eq!(signed(data, mark_anchor + 2).unwrap(), 80);
        let base = child(data, at, at + 10);
        let matrix = if kind == 5 {
            child(data, base, base + 2)
        } else {
            base
        };
        assert_eq!(u16_at(data, matrix + 2).unwrap(), 0);
        let base_anchor = child(data, matrix, matrix + 4);
        assert_eq!(signed(data, base_anchor + 4).unwrap(), 180);
        if kind == 5 {
            let second = child(data, matrix, matrix + 8);
            assert_eq!(signed(data, second + 2).unwrap(), 300);
        }
    }
}
#[test]
fn contour_point_anchor_is_preserved_and_reported_not_flattened() {
    let mut program = words(&[1, 0, 1, 0, 0]);
    append(&mut program, 6, 0, &words(&[2, 100, 200, 17]));
    append(&mut program, 2, 0, &coverage(&[1]));
    let result = stage(&[(3, program)], 0, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    let anchor = child(data, at, at + 6);
    assert_eq!(&data[anchor..anchor + 8], &words(&[2, 100, 200, 17]));
    assert_eq!(result.contour_point_references, 1);
}
#[test]
fn pixel_device_program_is_preserved_instead_of_evaluated_as_a_variation() {
    let hint = words(&[10, 13, 1, 0x6c00]);
    let mut program = words(&[1, 0, 0x44, 7, 0]);
    append(&mut program, 8, 0, &hint);
    append(&mut program, 2, 0, &coverage(&[1]));
    let result = stage(&[(1, program)], 99, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    assert_eq!(signed(data, at + 6).unwrap(), 7);
    let device = child(data, at, at + 8);
    assert_eq!(&data[device..device + hint.len()], hint);
    assert_eq!(result.retained_device_hints, 1);
    assert_eq!(result.resolved_adjustments, 0);
}
#[test]
fn missing_owning_store_fails_but_no_variation_sentinel_needs_no_store() {
    let mut program = single(None);
    let input = BTreeMap::from([(*b"GPOS", Arc::from(layout(&[(1, program.clone())])))]);
    assert!(freeze(&input, &[]).is_err());
    let at = usize::from(u16_at(&program, 6).unwrap());
    set(&mut program, at, 65535);
    set(&mut program, at + 2, 65535);
    let input = BTreeMap::from([(*b"GPOS", Arc::from(layout(&[(1, program)])))]);
    let result = freeze(&input, &[]).unwrap();
    let data = &result.tables[b"GPOS"];
    assert_eq!(signed(data, subtable(data, 0) + 6).unwrap(), 0);
}
fn caret_gdef() -> Vec<u8> {
    let mut data = vec![0; 18];
    set32(&mut data, 0, 0x10003);
    let list = append(&mut data, 8, 0, &words(&[0, 1, 0]));
    let glyph = append(&mut data, list + 4, list, &words(&[3, 0, 0, 0]));
    let caret = append(
        &mut data,
        glyph + 2,
        glyph,
        &words(&[3, 100, 6, 0, 0, 0x8000]),
    );
    set(&mut data, glyph + 4, caret - glyph); // Exact source alias.
    append(&mut data, glyph + 6, glyph, &words(&[2, 17]));
    append(&mut data, list, list, &coverage(&[1]));
    let store_at = data.len();
    set32(&mut data, 14, store_at);
    data.extend(store(20));
    data
}
#[test]
fn aliased_carets_read_immutable_values_before_shared_store_retirement() {
    let original = caret_gdef();
    let input = BTreeMap::from([
        (*b"GDEF", Arc::from(original.clone())),
        (*b"JSTF", Arc::from(words(&[1, 0, 0]))),
    ]);
    let result = freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).unwrap();
    let data = &result.tables[b"GDEF"];
    let list = child(data, 0, 8);
    let glyph = child(data, list, list + 4);
    for field in [2, 4] {
        let caret = child(data, glyph, glyph + field);
        assert_eq!(signed(data, caret + 2).unwrap(), 120);
        assert_eq!(u16_at(data, caret + 4).unwrap(), 0);
    }
    assert_eq!(result.contour_point_references, 1);
    assert_eq!(result.retired_variation_stores, [*b"GDEF"]);
    assert!(!result.retained_gdef_store);
    assert_eq!(u32_at(data, 0).unwrap(), 0x10002);
    assert_eq!(&*input[b"GDEF"], &original);
}
#[test]
fn gdef_class_attachment_and_mark_set_graphs_remain_owner_relative() {
    let mut data = vec![0; 14];
    set32(&mut data, 0, 0x10002);
    append(&mut data, 4, 0, &words(&[1, 1, 2, 1, 3]));
    append(&mut data, 10, 0, &words(&[2, 1, 3, 3, 7]));
    let attach = append(&mut data, 6, 0, &words(&[0, 1, 0]));
    append(&mut data, attach + 4, attach, &words(&[2, 2, 5]));
    append(&mut data, attach, attach, &coverage(&[1]));
    let marks = append(&mut data, 12, 0, &words(&[1, 1, 0, 8]));
    data.extend(coverage(&[3]));
    assert_eq!(u32_at(&data, marks + 4).unwrap(), 8);
    let result = freeze(&BTreeMap::from([(*b"GDEF", Arc::from(data))]), &[]).unwrap();
    let data = &result.tables[b"GDEF"];
    let attach = child(data, 0, 6);
    let points = child(data, attach, attach + 4);
    assert_eq!(&data[points..points + 6], &words(&[2, 2, 5]));
    let marks = child(data, 0, 12);
    let covered = marks + u32_at(data, marks + 4).unwrap() as usize;
    assert_eq!(&data[covered..covered + 6], &coverage(&[3]));
    assert_eq!(result.contour_point_references, 2);
    assert!(!result.retained_gdef_store);
}
#[test]
fn contextual_dispatch_and_lookup_identity_survive_position_rewriting() {
    let mut context = words(&[3, 1, 1, 12, 0, 1]);
    context.extend(coverage(&[1]));
    let result = stage(&[(7, context.clone()), (1, single(Some(10)))], 20, 16384);
    let data = &result.tables[b"GPOS"];
    let at = subtable(data, 0);
    assert_eq!(&data[at..at + context.len()], context);
    assert_eq!(signed(data, subtable(data, 1) + 6).unwrap(), 30);
    assert_eq!(result.opaque_layout_sources, [*b"GPOS"]);
}
#[test]
fn feature_selection_composes_with_variable_position_resolution() {
    let mut data = layout(&[(1, single(Some(10))), (1, single(Some(30)))]);
    let variations = data.len();
    set32(&mut data, 10, variations);
    // Null condition set matches; replacement feature names lookup index 1.
    data.extend(words(&[
        1, 0, 0, 1, 0, 0, 0, 16, 1, 0, 1, 0, 0, 12, 0, 1, 1,
    ]));
    let result = freeze(
        &BTreeMap::from([(*b"GPOS", Arc::from(data)), (*b"GDEF", Arc::from(gdef(20)))]),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
    )
    .unwrap();
    let data = &result.tables[b"GPOS"];
    let features = child(data, 0, 6);
    let feature = child(data, features, features + 6);
    assert_eq!(u16_at(data, feature + 4).unwrap(), 1);
    assert_eq!(result.selected_features[b"GPOS"], Some(0));
    assert_eq!(signed(data, subtable(data, 1) + 6).unwrap(), 50);
}
fn font_with(tables: BTreeMap<[u8; 4], Vec<u8>>, variable: bool) -> Vec<u8> {
    let source = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let container = crate::fonts::font_container::Container::parse(&source).unwrap();
    let mut out = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, source[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    out.extend(tables);
    if variable {
        let mut fvar = words(&[1, 0, 16, 2, 1, 20, 0, 8]);
        fvar.extend(b"TEST");
        for n in [-65536i32, 0, 65536] {
            fvar.extend(n.to_be_bytes());
        }
        fvar.extend(words(&[0, 256]));
        out.insert(*b"fvar", fvar);
    }
    crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", out).unwrap()
}
#[test]
fn reopened_static_fixture_shapes_resolved_positions_not_just_status_flags() {
    let result = stage(&[(1, single(Some(10)))], 40, 8192);
    let font = font_with(result.tables, false);
    let shaped = crate::fonts::TextShaper::shape(&font, "AB", Default::default()).unwrap();
    assert_eq!(shaped.glyphs[0].advance, 630.);
    assert_eq!(shaped.glyphs[1].advance, 600.);
}
#[test]
fn metric_preparation_stages_layout_with_identical_selected_coordinates() {
    let source = font_with(
        BTreeMap::from([
            (*b"GDEF", gdef(40)),
            (*b"GPOS", layout(&[(1, single(Some(10)))])),
        ]),
        true,
    );
    let request =
        crate::fonts::VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 0.5);
    let prepared = crate::fonts::font_metric_instance::prepare(&source, 0, &request).unwrap();
    assert_eq!(prepared.coordinates[0].get(), 8192);
    let data = &prepared.layout.as_ref().unwrap().tables[b"GPOS"];
    assert_eq!(signed(data, subtable(data, 0) + 6).unwrap(), 30);
    let mut tables = prepared.layout.unwrap().tables;
    tables.extend(prepared.metrics.tables);
    let font = font_with(tables, false);
    let shaped = crate::fonts::TextShaper::shape(&font, "A", Default::default()).unwrap();
    assert_eq!(shaped.glyphs[0].advance, 630.);
}
#[test]
fn nonvariable_font_does_not_activate_stray_variable_positioning_data() {
    let source = font_with(
        BTreeMap::from([(*b"GPOS", layout(&[(1, single(Some(10)))]))]),
        false,
    );
    let stage = crate::fonts::font_metric_instance::prepare(
        &source,
        0,
        &crate::fonts::VariationRequest::none(),
    )
    .unwrap();
    assert!(stage.layout.is_none());
}
#[test]
fn invalid_position_counts_formats_and_overflow_fail_without_source_changes() {
    let original = single(Some(32767));
    let input = BTreeMap::from([
        (*b"GPOS", Arc::from(layout(&[(1, original)]))),
        (*b"GDEF", Arc::from(gdef(1))),
    ]);
    let snapshot = input.clone();
    assert!(freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).is_err());
    assert_eq!(input, snapshot);
    for program in [words(&[1, 0, 0x100, 0]), words(&[2, 10, 0, 2, 0, 1, 1, 1])] {
        assert!(freeze(
            &BTreeMap::from([(*b"GPOS", Arc::from(layout(&[(1, program)])))]),
            &[]
        )
        .is_err());
    }
}
#[test]
fn limits_and_cancellation_prevent_partial_layout_publication() {
    let mut writer = Writer::new();
    writer.reserve(4).unwrap();
    assert!(writer.link(0, 0, 65536).is_err());
    assert!(writer.reserve(64 * 1024 * 1024).is_err());
    let mut resolver = Resolver::new(None);
    assert!(resolver.charge(1_000_001).is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&BTreeMap::new(), &[]).is_err()));
}

#[test]
fn already_extended_lookup_is_unwrapped_and_rewritten_once() {
    let mut extended = words(&[1, 1, 0, 8]);
    extended.extend(single(Some(10)));
    let result = stage(&[(9, extended)], 20, 16384);
    let data = &result.tables[b"GPOS"];
    assert_eq!(signed(data, subtable(data, 0) + 6).unwrap(), 30);
    assert_eq!(result.resolved_adjustments, 1);
}

#[test]
fn extension_lookup_with_conflicting_inner_kinds_is_rejected() {
    let mut data = layout(&[(9, Vec::new())]);
    let list = child(&data, 0, 8);
    let lookup = child(&data, list, list + 2);
    data.truncate(lookup);
    data.extend(words(&[9, 0, 2, 10, 18]));
    data.extend(words(&[1, 1, 0, 16, 1, 3, 0, 8]));
    data.extend(single(Some(10)));
    let input = BTreeMap::from([(*b"GPOS", Arc::from(data)), (*b"GDEF", Arc::from(gdef(1)))]);
    assert!(freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).is_err());
}

#[test]
fn gdef_pixel_caret_adjustment_preserves_its_packed_device_program() {
    let mut source = vec![0; 12];
    set32(&mut source, 0, 0x10000);
    let list = append(&mut source, 8, 0, &words(&[0, 1, 0]));
    let glyph = append(&mut source, list + 4, list, &words(&[1, 0]));
    let caret = append(&mut source, glyph + 2, glyph, &words(&[3, 80, 0]));
    let hint = words(&[12, 13, 3, 0xff01]);
    append(&mut source, caret + 4, caret, &hint);
    append(&mut source, list, list, &coverage(&[1]));
    let result = freeze(&BTreeMap::from([(*b"GDEF", Arc::from(source))]), &[]).unwrap();
    let data = &result.tables[b"GDEF"];
    let list = child(data, 0, 8);
    let glyph = child(data, list, list + 4);
    let caret = child(data, glyph, glyph + 2);
    let device = child(data, caret, caret + 4);
    assert_eq!(signed(data, caret + 2).unwrap(), 80);
    assert_eq!(&data[device..device + hint.len()], hint);
    assert_eq!(result.retained_device_hints, 1);
}

#[test]
fn malformed_device_ranges_and_formats_are_not_ignored() {
    for device in [
        words(&[13, 12, 1, 0]),
        words(&[12, 12, 4, 0]),
        words(&[12, 30, 3]),
    ] {
        let mut data = words(&[1, 0, 0x44, 0, 0]);
        // Put coverage first so a truncated Device cannot read coverage as data.
        append(&mut data, 2, 0, &coverage(&[1]));
        append(&mut data, 8, 0, &device);
        assert!(freeze(
            &BTreeMap::from([(*b"GPOS", Arc::from(layout(&[(1, data)])))]),
            &[]
        )
        .is_err());
    }
}

#[test]
fn class_indices_coverage_order_and_required_mark_anchors_are_validated() {
    let mut missing_anchor = mark_program(4);
    let mark = child(&missing_anchor, 0, 8);
    set(&mut missing_anchor, mark + 4, 0);
    let mut invalid_class = mark_program(4);
    let mark = child(&invalid_class, 0, 8);
    set(&mut invalid_class, mark + 2, 2);
    let mut bad_coverage = single(None);
    let cov = child(&bad_coverage, 0, 2);
    bad_coverage.truncate(cov);
    bad_coverage.extend(coverage(&[2, 1]));
    for (kind, program) in [(4, missing_anchor), (4, invalid_class), (1, bad_coverage)] {
        let input = BTreeMap::from([
            (*b"GPOS", Arc::from(layout(&[(kind, program)]))),
            (*b"GDEF", Arc::from(gdef(1))),
        ]);
        assert!(freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).is_err());
    }
}

#[test]
fn gdef_store_requires_matching_axes_short_rows_and_valid_header_ownership() {
    assert!(freeze(&BTreeMap::from([(*b"GDEF", Arc::from(gdef(1)))]), &[]).is_err());
    let mut long = gdef(1);
    // ItemVariationData begins at store + 22, wordDeltaCount follows itemCount.
    set(&mut long, 18 + 22 + 2, 0x8001);
    long.extend([0, 0]);
    let mut header_overlap = gdef(1);
    set32(&mut header_overlap, 14, 4);
    for data in [long, header_overlap] {
        assert!(freeze(
            &BTreeMap::from([(*b"GDEF", Arc::from(data))]),
            &[ttf_parser::NormalizedCoordinate::from(16384)]
        )
        .is_err());
    }
}

#[test]
fn contextual_positioning_still_dispatches_to_frozen_lookup_after_font_reopen() {
    let mut context = words(&[3, 1, 1, 12, 0, 1]);
    context.extend(coverage(&[1]));
    let result = stage(&[(7, context), (1, single(Some(10)))], 40, 8192);
    let font = font_with(result.tables, false);
    let shaped = crate::fonts::TextShaper::shape(&font, "AB", Default::default()).unwrap();
    assert_eq!(shaped.glyphs[0].advance, 630.);
    assert_eq!(shaped.glyphs[1].advance, 600.);
}

#[path = "base_jstf_instance_tests.rs"]
mod baseline_and_justification;

#[path = "layout_point_instance_tests.rs"]
mod outline_point_identity;
