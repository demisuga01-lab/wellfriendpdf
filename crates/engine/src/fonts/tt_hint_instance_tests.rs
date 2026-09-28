//! Hint preservation and instance fallback regression source; not executed.
use super::*;
fn coords(values: &[i16]) -> Vec<ttf_parser::NormalizedCoordinate> {
    values
        .iter()
        .copied()
        .map(ttf_parser::NormalizedCoordinate::from)
        .collect()
}
fn source(programs: &[&[u8]]) -> Tables {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for program in programs {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend([0; 10]);
        glyf.extend((program.len() as u16).to_be_bytes());
        glyf.extend_from_slice(program);
        if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    loca.extend((glyf.len() as u32).to_be_bytes());
    let mut head = vec![0; 54];
    head[1] = 1;
    head[51] = 1;
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut maxp = vec![0; 32];
    maxp[1] = 1;
    maxp[4..6].copy_from_slice(&(programs.len() as u16).to_be_bytes());
    maxp[15] = 2;
    maxp[25] = 8;
    [
        (*b"glyf", glyf.into()),
        (*b"loca", loca.into()),
        (*b"head", head.into()),
        (*b"maxp", maxp.into()),
    ]
    .into_iter()
    .collect()
}
fn prepare(tables: &Tables, values: &[i16]) -> HintStage {
    let outlines = crate::fonts::glyf_instance::freeze(tables, None).unwrap();
    freeze(tables, &outlines, &coords(values)).unwrap()
}
#[test]
fn prep_append_preserves_source_offsets_and_axis_order_with_signed_values() {
    let mut tables = source(&[&[0x91, 0x22]]);
    tables.insert(*b"prep", Arc::from([0xb0, 7, 0x21]));
    let original = tables.clone();
    let stage = prepare(&tables, &[-16384, 0, 8192]);
    let expected = [
        0xb0, 7, 0x21, 0xb0, 0x91, 0x89, 0x41, 3, 0xc0, 0, 0, 0, 0x20, 0, 0x2d,
    ];
    assert_eq!(stage.tables[b"prep"], expected);
    assert_eq!(stage.appended_definition, 3..expected.len());
    assert_eq!(stage.normalized_values, [-16384, 0, 8192]);
    assert_eq!(stage.instruction_capacity, 1);
    assert_eq!(stage.stack_capacity, 11);
    assert!(stage.review.is_empty());
    assert!(stage.scan_work > 0);
    assert_eq!(tables, original);
    assert!(!stage.tables.contains_key(b"glyf"));
    assert!(!stage.tables.contains_key(b"fpgm"));
}
#[test]
fn existing_variation_idef_is_overridden_without_allocating_a_duplicate_identity() {
    let mut tables = source(&[&[0x91, 0x21]]);
    let old = [0xb0, 0x91, 0x89, 0xb0, 0, 0x2d];
    tables.insert(*b"prep", Arc::from(old));
    let stage = prepare(&tables, &[8192]);
    assert_eq!(&stage.tables[b"prep"][..old.len()], old);
    assert_eq!(stage.instruction_capacity, 1);
    let scan = tt_bytecode::scan(
        &stage.tables[b"prep"],
        Owner::Preparation,
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(scan.definitions.len(), 2);
    assert!(scan.definitions.iter().all(|d| d.identifier == Some(145)));
    assert!(stage.review.is_empty());
}
#[test]
fn custom_idef_capacity_and_recomputed_glyph_maxima_are_retained() {
    let mut tables = source(&[&[0xb0, 1, 0x2f]]);
    tables.insert(*b"fpgm", Arc::from([0xb0, 0x8f, 0x89, 0x2d]));
    let outlines = crate::fonts::glyf_instance::freeze(&tables, None).unwrap();
    assert_eq!(u16_at(&outlines.tables[b"maxp"], 26).unwrap(), 3);
    let stage = freeze(&tables, &outlines, &coords(&[0])).unwrap();
    assert_eq!(stage.instruction_capacity, 2);
    assert_eq!(u16_at(&stage.tables[b"maxp"], 26).unwrap(), 3);
    assert_eq!(
        &stage.tables[b"maxp"][..22],
        &outlines.tables[b"maxp"][..22]
    );
    assert_eq!(
        &stage.tables[b"maxp"][26..],
        &outlines.tables[b"maxp"][26..]
    );
}
#[test]
fn initialization_queries_and_calls_are_reported_not_claimed_frozen() {
    let mut tables = source(&[&[]]);
    tables.insert(
        *b"fpgm",
        Arc::from([0xb0, 0, 0x2c, 0x91, 0x2d, 0xb0, 0, 0x2b]),
    );
    tables.insert(*b"prep", Arc::from([0x91, 0x22]));
    let stage = prepare(&tables, &[8192]);
    assert!(stage.review.iter().any(|r| r.owner == Owner::Font
        && r.kind == ReviewKind::InitializationCallsWithVariationDefinitions));
    assert!(stage.review.iter().any(|r| r.owner == Owner::Preparation
        && r.kind == ReviewKind::InitializationVariationQuery
        && r.offset == Some(0)));
}
#[test]
fn variation_capability_queries_are_separate_from_unrelated_getinfo_selectors() {
    let tables = source(&[&[0xb0, 1, 0x88, 0xb0, 8, 0x88, 0x88]]);
    let stage = prepare(&tables, &[0]);
    let queries = stage
        .review
        .iter()
        .filter(|r| r.kind == ReviewKind::FontVariationCapabilityQuery)
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[0].offset, Some(5));
    assert_eq!(queries[1].offset, Some(6));
}
#[test]
fn dynamic_idefs_reserve_opcode_capacity_without_inventing_an_identifier() {
    let mut tables = source(&[&[]]);
    tables.insert(*b"fpgm", Arc::from([0xb0, 145, 0x20, 0x89, 0x2d]));
    let stage = prepare(&tables, &[0]);
    assert_eq!(stage.instruction_capacity, 256);
    assert!(stage
        .review
        .iter()
        .any(|r| r.kind == ReviewKind::DynamicInstructionDefinition));
}
#[test]
fn jumps_unknown_opcodes_and_legacy_variation_data_remain_explicit_reviews() {
    let mut tables = source(&[&[0x92, 0x8f]]);
    tables.insert(*b"prep", Arc::from([0xb0, 1, 0x1c]));
    let stage = prepare(&tables, &[0]);
    for kind in [
        ReviewKind::LegacyVariationOpcode,
        ReviewKind::UnknownInstructionSemantics,
        ReviewKind::InitializationRelativeControlFlow,
    ] {
        assert!(stage.review.iter().any(|r| r.kind == kind));
    }
    assert_eq!(&stage.tables[b"prep"][..3], [0xb0, 1, 0x1c]);
}
#[test]
fn instruction_ranges_belong_to_the_written_glyphs_and_are_bounded() {
    let tables = source(&[&[0x91], &[], &[0xb0, 0x88]]);
    let mut outlines = crate::fonts::glyf_instance::freeze(&tables, None).unwrap();
    let stage = freeze(&tables, &outlines, &coords(&[0])).unwrap();
    assert_eq!(
        stage.programs.iter().map(|p| p.owner).collect::<Vec<_>>(),
        [Owner::Glyph(0), Owner::Glyph(2)]
    );
    assert!(stage.programs[1].queries.is_empty());
    outlines.instruction_ranges[0].1.start = 0;
    assert!(freeze(&tables, &outlines, &coords(&[0])).is_err());
}
#[test]
fn malformed_source_programs_cannot_hide_the_appended_definition_in_a_scope() {
    for prep in [vec![0x58], vec![0xb0, 145, 0x89], vec![0x40, 3, 0x91]] {
        let mut tables = source(&[&[]]);
        tables.insert(*b"prep", prep.into());
        let outlines = crate::fonts::glyf_instance::freeze(&tables, None).unwrap();
        assert!(freeze(&tables, &outlines, &coords(&[0])).is_err());
    }
}
#[test]
fn stack_overflow_axis_budgets_and_cancellation_return_no_stage() {
    let tables = source(&[&[]]);
    let mut outlines = crate::fonts::glyf_instance::freeze(&tables, None).unwrap();
    assert!(freeze(&tables, &outlines, &[]).is_err());
    assert!(freeze(&tables, &outlines, &coords(&[0; 65])).is_err());
    outlines.tables.get_mut(b"maxp").unwrap()[24..26].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(freeze(&tables, &outlines, &coords(&[0])).is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&tables, &outlines, &coords(&[0])).is_err()));
}
#[test]
fn generated_tail_has_balanced_definition_and_exact_selected_word_payload() {
    let values = [-16384, -8192, -1, 0, 1, 8192, 16384];
    let output = fallback(&coords(&values));
    let scan = tt_bytecode::scan(&output, Owner::Preparation, &mut Budget::default()).unwrap();
    let body = &output[scan.definitions[0].body.clone()];
    assert_eq!(&body[..2], &[0x41, 7]);
    let decoded = body[2..]
        .chunks_exact(2)
        .map(|v| i16::from_be_bytes(v.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(decoded, values);
    assert!(scan.queries.is_empty());
}
#[test]
fn combined_source_preparation_keeps_fallback_and_profile_after_save_reopen() {
    let mut owners = source(&[&[], &[0x91, 0x21], &[], &[]]);
    let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
    fvar.extend(b"TEST");
    for n in [-65536i32, 0, 65536] {
        fvar.extend(n.to_be_bytes());
    }
    fvar.extend([0, 0, 1, 0]);
    owners.insert(*b"fvar", fvar.into());
    let base = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let mut tables = crate::fonts::font_container::Container::parse(&base)
        .unwrap()
        .faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, base[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    tables.remove(b"CFF ");
    tables.extend(owners.iter().map(|(tag, bytes)| (*tag, bytes.to_vec())));
    let source = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables.clone()).unwrap();
    let request = crate::fonts::VariationRequest::none()
        .with_axis(ttf_parser::Tag::from_bytes(b"TEST"), -0.5);
    let stage = crate::fonts::font_metric_instance::prepare(&source, 0, &request).unwrap();
    let hints = stage.hints.unwrap();
    assert_eq!(
        stage.outlines.as_ref().unwrap().tables[b"maxp"],
        hints.tables[b"maxp"]
    );
    assert_eq!(hints.normalized_values, [-8192]);
    assert!(hints.review.is_empty());
    let expected_prep = hints.tables[b"prep"].clone();
    let expected_profile = hints.tables[b"maxp"].clone();
    tables.extend(stage.outlines.unwrap().tables);
    tables.extend(stage.metrics.tables);
    tables.extend(stage.layout.unwrap().tables);
    tables.extend(hints.tables);
    tables.remove(b"fvar");
    let output = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables).unwrap();
    let reopened = ttf_parser::Face::parse(&output, 0).unwrap();
    assert!(!reopened.is_variable());
    assert_eq!(
        reopened
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"prep"))
            .unwrap(),
        expected_prep
    );
    assert_eq!(
        reopened
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"maxp"))
            .unwrap(),
        expected_profile
    );
}
