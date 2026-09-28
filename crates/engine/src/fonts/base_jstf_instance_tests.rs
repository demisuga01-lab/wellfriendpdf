//! Source-only regressions; deliberately not executed in this implementation.
use super::*;

fn record(tag: &[u8; 4]) -> Vec<u8> {
    let mut data = tag.to_vec();
    data.extend(words(&[0]));
    data
}
fn base_coord(value: i16) -> Vec<u8> {
    words(&[3, value as u16, 6, 0, 0, 0x8000])
}
fn baseline(delta: Option<i16>, points: bool, both_axes: bool) -> Vec<u8> {
    let mut data = vec![0; 12];
    set32(&mut data, 0, 0x10001);
    let axis = append(&mut data, 4, 0, &words(&[0, 0]));
    if both_axes {
        set(&mut data, 6, axis);
    }
    let mut tags = words(&[2]);
    tags.extend(b"ideoromn");
    append(&mut data, axis, axis, &tags);
    let mut scripts = words(&[1]);
    scripts.extend(record(b"latn"));
    let scripts = append(&mut data, axis + 2, axis, &scripts);
    let mut script = words(&[0, 0, 1]);
    script.extend(record(b"FAR "));
    let script = append(&mut data, scripts + 6, scripts, &script);
    let values = append(&mut data, script, script, &words(&[1, 2, 0, 0]));
    append(&mut data, values + 4, values, &base_coord(100));
    let plain = if points {
        words(&[2, 200, 2, 7])
    } else {
        words(&[1, 200])
    };
    append(&mut data, values + 6, values, &plain);
    let mut minmax = words(&[0, 0, 1]);
    minmax.extend(b"kern");
    minmax.extend(words(&[0, 0]));
    let minmax = append(&mut data, script + 2, script, &minmax);
    set(&mut data, script + 10, minmax - script); // Same source owner reused by language.
    append(&mut data, minmax, minmax, &base_coord(-100));
    append(&mut data, minmax + 12, minmax, &base_coord(300));
    if let Some(delta) = delta {
        let at = data.len();
        set32(&mut data, 8, at);
        data.extend(store(delta));
    }
    data
}
fn base_script(data: &[u8], axis_field: usize) -> usize {
    let axis = child(data, 0, axis_field);
    let scripts = child(data, axis, axis + 2);
    child(data, scripts, scripts + 6)
}
fn baseline_value(data: &[u8], axis_field: usize, index: usize) -> usize {
    let script = base_script(data, axis_field);
    let values = child(data, script, script);
    child(data, values, values + 4 + index * 2)
}
fn prepare_base(data: Vec<u8>, coordinate: i16) -> Result<LayoutStage> {
    freeze(
        &BTreeMap::from([(*b"BASE", Arc::from(data))]),
        &[ttf_parser::NormalizedCoordinate::from(coordinate)],
    )
}

#[test]
fn base_uses_its_own_store_not_identically_indexed_gdef_deltas() {
    let input = BTreeMap::from([
        (*b"BASE", Arc::from(baseline(Some(30), false, true))),
        (*b"GDEF", Arc::from(gdef(80))),
        (*b"GPOS", Arc::from(layout(&[(1, single(Some(10)))]))),
    ]);
    let result = freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).unwrap();
    let data = &result.tables[b"BASE"];
    assert_eq!(u32_at(data, 0).unwrap(), 0x10000);
    for field in [4, 6] {
        assert_eq!(
            signed(data, baseline_value(data, field, 0) + 2).unwrap(),
            130
        );
        assert_eq!(u16_at(data, baseline_value(data, field, 0) + 4).unwrap(), 0);
        assert_eq!(
            signed(data, baseline_value(data, field, 1) + 2).unwrap(),
            200
        );
    }
    let gpos = &result.tables[b"GPOS"];
    assert_eq!(signed(gpos, subtable(gpos, 0) + 6).unwrap(), 90);
    assert_eq!(result.distinct_variation_indices, 2);
    assert_eq!(result.evaluated_delta_cells, 2);
    assert_eq!(result.retired_variation_stores, [*b"GDEF", *b"BASE"]);
}

#[test]
fn baseline_script_language_and_feature_extents_preserve_owner_semantics() {
    let source = baseline(Some(30), false, false);
    let input = BTreeMap::from([(*b"BASE", Arc::from(source.clone()))]);
    let result = freeze(&input, &[ttf_parser::NormalizedCoordinate::from(8192)]).unwrap();
    let data = &result.tables[b"BASE"];
    let script = base_script(data, 4);
    for field in [2, 10] {
        let minmax = child(data, script, script + field);
        let minimum = child(data, minmax, minmax);
        assert_eq!(signed(data, minimum + 2).unwrap(), -85);
        assert_eq!(u16_at(data, minmax + 2).unwrap(), 0);
        assert_eq!(u16_at(data, minmax + 10).unwrap(), 0);
        assert_eq!(&data[minmax + 6..minmax + 10], b"kern");
        let maximum = child(data, minmax, minmax + 12);
        assert_eq!(signed(data, maximum + 2).unwrap(), 315);
    }
    assert_eq!(&*input[b"BASE"], source);
}

#[test]
fn baseline_glyph_point_references_are_retained_and_reported() {
    let result = prepare_base(baseline(Some(0), true, false), 16384).unwrap();
    let data = &result.tables[b"BASE"];
    let at = baseline_value(data, 4, 1);
    assert_eq!(&data[at..at + 8], words(&[2, 200, 2, 7]));
    assert_eq!(result.contour_point_references, 1);
}

#[test]
fn baseline_reference_glyph_and_point_are_checked_on_both_axes() {
    let source = baseline(Some(0), true, true);
    let tables = BTreeMap::from([(*b"BASE", Arc::from(source))]);
    let coordinates = [ttf_parser::NormalizedCoordinate::from(0)];
    let stage = freeze_with_points(&tables, &coordinates, Some(Arc::from([0, 0, 8]))).unwrap();
    assert_eq!(stage.checked_point_references, 2);
    assert_eq!(stage.unchecked_point_references, 0);
    for counts in [vec![0, 0, 7], vec![0, 8]] {
        assert!(freeze_with_points(&tables, &coordinates, Some(counts.into())).is_err());
    }
}

#[test]
fn baseline_without_outline_authority_records_unchecked_identity() {
    let stage = prepare_base(baseline(Some(0), true, true), 0).unwrap();
    assert_eq!(stage.checked_point_references, 0);
    assert_eq!(stage.unchecked_point_references, 2);
}

#[test]
fn baseline_classic_pixel_device_is_not_resolved_through_the_variation_store() {
    let mut source = baseline(Some(20), false, false);
    let at = baseline_value(&source, 4, 0);
    let hint = words(&[12, 12, 3, 0x0100]);
    append(&mut source, at + 4, at, &hint);
    let result = prepare_base(source, 16384).unwrap();
    let data = &result.tables[b"BASE"];
    let at = baseline_value(data, 4, 0);
    assert_eq!(signed(data, at + 2).unwrap(), 100);
    let hint_at = child(data, at, at + 4);
    assert_eq!(&data[hint_at..hint_at + hint.len()], hint);
    assert_eq!(result.retained_device_hints, 1);
}

#[test]
fn baseline_missing_store_and_bad_default_do_not_produce_partial_tables() {
    assert!(prepare_base(baseline(None, false, false), 16384).is_err());
    let mut source = baseline(Some(20), false, false);
    let script = base_script(&source, 4);
    let values = child(&source, script, script);
    set(&mut source, values, 2);
    assert!(prepare_base(source, 16384).is_err());
}

#[test]
fn baseline_tags_coordinate_counts_and_null_required_values_are_checked() {
    let original = baseline(Some(20), false, false);
    let axis = child(&original, 0, 4);
    let tags = child(&original, axis, axis);
    let script = base_script(&original, 4);
    let values = child(&original, script, script);
    for case in 0..3 {
        let mut source = original.clone();
        match case {
            0 => {
                let first = source[tags + 2..tags + 6].to_vec();
                source[tags + 6..tags + 10].copy_from_slice(&first);
            }
            1 => set(&mut source, values + 2, 1),
            _ => set(&mut source, values + 4, 0),
        }
        assert!(prepare_base(source, 16384).is_err());
    }
}

#[test]
fn baseline_without_tags_can_preserve_only_extents_and_an_absent_axis() {
    let mut source = baseline(Some(20), false, false);
    let axis = child(&source, 0, 4);
    let script = base_script(&source, 4);
    set(&mut source, axis, 0);
    set(&mut source, script, 0);
    let result = prepare_base(source, 16384).unwrap();
    let data = &result.tables[b"BASE"];
    let axis = child(data, 0, 4);
    assert_eq!(u16_at(data, axis).unwrap(), 0);
    assert_eq!(u16_at(data, 6).unwrap(), 0);
    assert_eq!(u16_at(data, base_script(data, 4)).unwrap(), 0);
}

#[test]
fn baseline_coordinates_overflow_and_incompatible_axes_fail_closed() {
    let mut source = baseline(Some(1), false, false);
    let at = baseline_value(&source, 4, 0);
    set(&mut source, at + 2, 32767);
    assert!(prepare_base(source, 16384).is_err());
    let input = BTreeMap::from([(*b"BASE", Arc::from(baseline(Some(20), false, false)))]);
    assert!(freeze(&input, &[]).is_err());
}

fn justification(kind: u16, program: Vec<u8>, filter: bool) -> Vec<u8> {
    let mut data = words(&[1, 0, 1]);
    data.extend(record(b"latn"));
    let mut script = words(&[0, 0, 1]);
    script.extend(record(b"FAR "));
    let script = append(&mut data, 10, 0, &script);
    append(&mut data, script, script, &words(&[2, 1, 3]));
    let language = append(&mut data, script + 2, script, &words(&[2, 0, 0]));
    set(&mut data, script + 10, language - script); // Shared default/language suggestions.
    let first = append(&mut data, language + 2, language, &words(&[0; 10]));
    let second = append(&mut data, language + 4, language, &words(&[0; 10]));
    let maximum = append(&mut data, first + 8, first, &words(&[1, 0]));
    set(&mut data, second + 18, maximum - second); // Shrink and grow share source max.
    let lookup = append(
        &mut data,
        maximum + 2,
        maximum,
        &words(&[
            kind,
            if filter { 16 } else { 0 },
            1,
            if filter { 10 } else { 8 },
        ]),
    );
    if filter {
        data.extend(words(&[0]));
    }
    assert_eq!(data.len() - lookup, if filter { 10 } else { 8 });
    data.extend(program);
    data
}
fn jstf_priority(data: &[u8], language_field: usize, priority: usize) -> usize {
    let script = child(data, 0, 10);
    let language = child(data, script, script + language_field);
    child(data, language, language + 2 + priority * 2)
}
fn jstf_program(data: &[u8], language_field: usize, priority: usize) -> (usize, usize) {
    let priority_at = jstf_priority(data, language_field, priority);
    let maximum = child(
        data,
        priority_at,
        priority_at + if priority == 0 { 8 } else { 18 },
    );
    let lookup = child(data, maximum, maximum + 2);
    assert_eq!(u16_at(data, lookup).unwrap(), 9);
    let wrapper = child(data, lookup, lookup + 6);
    (
        lookup,
        wrapper + u32_at(data, wrapper + 4).unwrap() as usize,
    )
}
fn prepare_jstf(source: Vec<u8>, delta: i16) -> Result<LayoutStage> {
    freeze(
        &BTreeMap::from([
            (*b"JSTF", Arc::from(source)),
            (*b"GDEF", Arc::from(gdef(delta))),
        ]),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
    )
}

#[test]
fn justification_priorities_and_shared_language_owners_survive_gdef_retirement() {
    let result = prepare_jstf(justification(1, single(Some(10)), false), 40).unwrap();
    let data = &result.tables[b"JSTF"];
    for field in [2, 10] {
        for priority in [0, 1] {
            let (_, at) = jstf_program(data, field, priority);
            assert_eq!(signed(data, at + 6).unwrap(), 50);
        }
    }
    assert_eq!(result.resolved_adjustments, 1); // Same source program, one generated target.
    assert!(!result.retained_gdef_store);
    assert_eq!(u32_at(&result.tables[b"GDEF"], 0).unwrap(), 0x10002);
}

#[test]
fn embedded_justification_anchors_share_the_exact_outline_authority() {
    let mut program = words(&[1, 0, 1, 0, 0]);
    append(&mut program, 6, 0, &words(&[2, 10, 20, 3]));
    append(&mut program, 2, 0, &coverage(&[1]));
    let tables = BTreeMap::from([(*b"JSTF", Arc::from(justification(3, program, false)))]);
    let stage = freeze_with_points(&tables, &[], Some(Arc::from([0, 4, 0, 0]))).unwrap();
    assert_eq!(stage.checked_point_references, 1); // Same embedded program is cached.
    assert_eq!(stage.unchecked_point_references, 0);
    assert!(freeze_with_points(&tables, &[], Some(Arc::from([0, 3, 0, 0]))).is_err());
}

#[test]
fn justification_extenders_and_all_eight_lookup_modification_lists_keep_indices() {
    let mut source = justification(1, single(Some(10)), false);
    let priority = jstf_priority(&source, 2, 0);
    for i in [0, 1, 2, 3, 5, 6, 7, 8] {
        append(&mut source, priority + i * 2, priority, &words(&[1, 0]));
    }
    let input = BTreeMap::from([
        (*b"JSTF", Arc::from(source)),
        (*b"GDEF", Arc::from(gdef(40))),
        (*b"GPOS", Arc::from(layout(&[(1, single(Some(5)))]))),
        // Identity substitution; this case checks stable list identity.
        (
            *b"GSUB",
            Arc::from(layout(&[(1, words(&[1, 6, 0, 1, 1, 1]))])),
        ),
    ]);
    let result = freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).unwrap();
    let data = &result.tables[b"JSTF"];
    let priority = jstf_priority(data, 2, 0);
    for i in [0, 1, 2, 3, 5, 6, 7, 8] {
        let list = child(data, priority, priority + i * 2);
        assert_eq!(&data[list..list + 4], words(&[1, 0]));
    }
    let script = child(data, 0, 10);
    let extenders = child(data, script, script);
    assert_eq!(&data[extenders..extenders + 6], words(&[2, 1, 3]));
}

#[test]
fn justification_extensions_and_mark_filter_set_are_preserved() {
    let mut extended = words(&[1, 1, 0, 8]);
    extended.extend(single(Some(10)));
    let mut definitions = gdef(40);
    append(&mut definitions, 12, 0, &words(&[1, 1, 0, 8]));
    definitions.extend(coverage(&[1]));
    let result = freeze(
        &BTreeMap::from([
            (*b"JSTF", Arc::from(justification(9, extended, true))),
            (*b"GDEF", Arc::from(definitions)),
        ]),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
    )
    .unwrap();
    let data = &result.tables[b"JSTF"];
    let (lookup, at) = jstf_program(data, 2, 0);
    assert_eq!(u16_at(data, lookup + 2).unwrap(), 16);
    assert_eq!(u16_at(data, lookup + 8).unwrap(), 0);
    assert_eq!(signed(data, at + 6).unwrap(), 50);
}

#[test]
fn justification_cursive_and_each_mark_kind_share_the_position_writer() {
    let mut cursive = words(&[1, 0, 1, 0, 0]);
    append(&mut cursive, 6, 0, &anchor());
    append(&mut cursive, 2, 0, &coverage(&[1]));
    for (kind, source) in [
        (3, cursive),
        (4, mark_program(4)),
        (5, mark_program(5)),
        (6, mark_program(6)),
    ] {
        let result = prepare_jstf(justification(kind, source, false), 30).unwrap();
        let data = &result.tables[b"JSTF"];
        let (_, at) = jstf_program(data, 2, 0);
        let anchor = if kind == 3 {
            child(data, at, at + 6)
        } else {
            let mark = child(data, at, at + 8);
            child(data, mark, mark + 4)
        };
        assert_eq!(signed(data, anchor + 2).unwrap(), 130);
        assert_eq!(signed(data, anchor + 4).unwrap(), 230);
    }
}

#[test]
fn contextual_and_nested_extension_programs_are_invalid_in_jstf_max() {
    for (kind, program) in [
        (7, words(&[1])),
        (8, words(&[1])),
        (9, words(&[1, 7, 0, 8, 1])),
        (9, words(&[1, 9, 0, 8, 1])),
    ] {
        assert!(prepare_jstf(justification(kind, program, false), 1).is_err());
    }
}

#[test]
fn invalid_jstf_modification_indices_abort_the_whole_layout_stage() {
    let mut source = justification(1, single(Some(10)), false);
    let priority = jstf_priority(&source, 2, 0);
    append(&mut source, priority + 4, priority, &words(&[1, 0]));
    // There is no GPOS lookup list, so even index zero is invalid.
    assert!(prepare_jstf(source, 40).is_err());
}

#[test]
fn jstf_extender_glyphs_are_bounded_by_source_maxp_when_present() {
    let input = BTreeMap::from([
        (
            *b"JSTF",
            Arc::from(justification(1, single(Some(10)), false)),
        ),
        (*b"GDEF", Arc::from(gdef(40))),
        (*b"maxp", Arc::from(words(&[0, 0x5000, 3]))),
    ]);
    assert!(freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).is_err());
}

#[test]
fn malformed_jstf_prevents_store_retirement_or_any_output_publication() {
    let input = BTreeMap::from([
        (*b"GDEF", Arc::from(gdef(1))),
        (*b"JSTF", Arc::from(vec![0])),
    ]);
    let before = input.clone();
    assert!(freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).is_err());
    assert_eq!(input, before);
}

#[test]
fn complete_layout_stage_reopens_with_static_positioning_and_baseline_bytes() {
    let input = BTreeMap::from([
        (*b"BASE", Arc::from(baseline(Some(30), false, true))),
        (*b"GDEF", Arc::from(gdef(40))),
        (*b"GPOS", Arc::from(layout(&[(1, single(Some(10)))]))),
        (
            *b"JSTF",
            Arc::from(justification(1, single(Some(20)), false)),
        ),
    ]);
    let result = freeze(&input, &[ttf_parser::NormalizedCoordinate::from(16384)]).unwrap();
    let saved = font_with(result.tables, false);
    let face = ttf_parser::Face::parse(&saved, 0).unwrap();
    assert!(!face.is_variable());
    let base = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"BASE"))
        .unwrap();
    assert_eq!(signed(base, baseline_value(base, 6, 0) + 2).unwrap(), 130);
    let jstf = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"JSTF"))
        .unwrap();
    assert_eq!(signed(jstf, jstf_program(jstf, 2, 1).1 + 6).unwrap(), 60);
    let shaped = crate::fonts::TextShaper::shape(&saved, "AB", Default::default()).unwrap();
    assert_eq!(shaped.glyphs[0].advance, 650.);
    assert_eq!(shaped.glyphs[1].advance, 600.);
}

#[test]
fn cancellation_prevents_baseline_or_justification_publication() {
    let source = baseline(Some(20), false, false);
    let jstf = justification(1, single(Some(10)), false);
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(crate::fonts::base_instance::freeze(
            source.into(),
            &[ttf_parser::NormalizedCoordinate::from(16384)]
        )
        .is_err());
        let mut resolver = Resolver::new(None);
        assert!(crate::fonts::jstf_instance::freeze(&jstf, &mut resolver, [0, 0], 4).is_err());
    });
}

#[test]
fn immutable_leaf_reuse_finds_an_owner_reachable_copy_after_many_clones() {
    let mut writer = Writer::new();
    let mut recent = (0, 0);
    for _ in 0..2048 {
        let owner = writer.reserve(2).unwrap();
        let target = writer.leaf(&[1, 2], 0, 2, owner).unwrap();
        recent = (owner, target);
    }
    assert_eq!(writer.leaf(&[1, 2], 0, 2, recent.0).unwrap(), recent.1);
    // A much older owner still selects its earliest reachable target.
    assert_eq!(writer.leaf(&[1, 2], 0, 2, 0).unwrap(), 2);
}

#[test]
fn justification_reads_far_extension_offsets_without_a_16_bit_truncation() {
    let program = single(Some(10));
    let mut extended = words(&[1, 1, 0, 8]);
    extended.extend(&program);
    let mut source = justification(9, extended, false);
    let (lookup, _) = jstf_program(&source, 2, 0);
    let wrapper = child(&source, lookup, lookup + 6);
    set32(&mut source, wrapper + 4, 70000);
    source.resize(wrapper + 70000, 0);
    source.extend(program);
    let result = prepare_jstf(source, 20).unwrap();
    let data = &result.tables[b"JSTF"];
    assert_eq!(signed(data, jstf_program(data, 2, 0).1 + 6).unwrap(), 30);
    assert!(data.len() < 70000);
}

#[test]
fn baseline_version_one_and_explicit_zero_adjustments_need_no_store() {
    let mut source = baseline(None, false, false);
    set32(&mut source, 0, 0x10000); // Old header padding remains unreferenced.
    let script = base_script(&source, 4);
    let minmax = child(&source, script, script + 2);
    let minimum = child(&source, minmax, minmax);
    let maximum = child(&source, minmax, minmax + 12);
    let first = baseline_value(&source, 4, 0);
    for coord in [first, minimum, maximum] {
        set(&mut source, coord + 4, 0);
    }
    let result = prepare_base(source, 16384).unwrap();
    let data = &result.tables[b"BASE"];
    assert_eq!(signed(data, baseline_value(data, 4, 0) + 2).unwrap(), 100);
    assert!(result.retired_variation_stores.is_empty());
}

#[test]
fn justification_pair_sets_keep_device_ownership_inside_an_embedded_lookup() {
    let mut program = words(&[1, 0, 0x44, 0, 1, 0]);
    let pair = append(&mut program, 10, 0, &words(&[1, 2, 10, 0]));
    append(&mut program, pair + 6, pair, &index());
    append(&mut program, 2, 0, &coverage(&[1]));
    let result = prepare_jstf(justification(2, program, false), -20).unwrap();
    let data = &result.tables[b"JSTF"];
    let (_, at) = jstf_program(data, 2, 0);
    let pair = child(data, at, at + 10);
    assert_eq!(signed(data, pair + 4).unwrap(), -10);
    assert_eq!(u16_at(data, pair + 6).unwrap(), 0);
}
