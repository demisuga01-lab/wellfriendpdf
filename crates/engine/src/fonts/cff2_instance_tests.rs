//! Source-only static CFF2 regressions; no execution during implementation.
use super::super::tests as fixtures;
use super::*;
use crate::fonts::font_instance;

fn coord(n: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(n)]
}
fn n(code: &mut Vec<u8>, value: i16) {
    code.push(28);
    code.extend(value.to_be_bytes());
}
fn simple(code: Vec<u8>, variation: Option<Vec<u8>>) -> Vec<u8> {
    fixtures::fixture(
        &[vec![], code, vec![], vec![]],
        &[vec![]],
        &[None],
        &[0; 4],
        0,
        variation,
        &[],
    )
}
fn varied() -> Vec<u8> {
    simple(
        fixtures::variable_glyph(None),
        Some(fixtures::store(&[[0, 16384, 16384]], &[vec![0]])),
    )
}
pub(super) fn with_privates(mut bytes: Vec<u8>, private: &[Vec<u8>]) -> Vec<u8> {
    let program = Program::parse(&bytes).unwrap();
    assert_eq!(private.len(), program.font_dicts.len());
    for (range, value) in program.font_dicts.iter().zip(private) {
        assert_eq!(range.len(), 11);
        let at = bytes.len();
        bytes[range.start + 1..range.start + 5]
            .copy_from_slice(&(value.len() as u32).to_be_bytes());
        bytes[range.start + 6..range.start + 10]
            .copy_from_slice(&(if value.is_empty() { 0 } else { at } as u32).to_be_bytes());
        bytes.extend(value);
    }
    bytes
}
pub(super) fn read_index(bytes: &[u8], at: usize) -> (Vec<&[u8]>, usize) {
    let count = usize::from(super::super::u16_at(bytes, at).unwrap());
    if count == 0 {
        return (vec![], at + 2);
    }
    let size = usize::from(bytes[at + 2]);
    let base = at + 3 + (count + 1) * size;
    let offsets = bytes[at + 3..base]
        .chunks_exact(size)
        .map(|p| p.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b)))
        .collect::<Vec<_>>();
    (
        offsets
            .windows(2)
            .map(|p| &bytes[base + p[0] - 1..base + p[1] - 1])
            .collect(),
        base + offsets[count] - 1,
    )
}
pub(super) fn parts(bytes: &[u8]) -> (Vec<&[u8]>, Vec<&[u8]>) {
    let (_, at) = read_index(bytes, 4);
    let (top, _) = read_index(bytes, at);
    let top = dict(top[0])
        .unwrap()
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let glyphs = read_index(bytes, top[&17][0] as usize).0;
    let fds = read_index(bytes, top[&0x0c24][0] as usize).0;
    let private = fds
        .into_iter()
        .map(|fd| {
            let fields = dict(fd).unwrap().into_iter().collect::<BTreeMap<_, _>>();
            let owner = &fields[&18];
            &bytes[owner[1] as usize..(owner[1] + owner[0]) as usize]
        })
        .collect();
    (glyphs, private)
}
fn prepare(bytes: &[u8], coords: &[ttf_parser::NormalizedCoordinate], widths: &[u16]) -> CffStage {
    freeze(
        &Program::parse(bytes).unwrap(),
        coords,
        widths,
        [0, 0, 300, 100],
        "WFStatic-Cff",
        MAX_OUTPUT,
    )
    .unwrap()
}
pub(super) fn fields(bytes: &[u8]) -> BTreeMap<u16, Vec<f64>> {
    dict(bytes).unwrap().into_iter().collect()
}
pub(super) fn unpack(code: &[u8]) -> Vec<(u16, Vec<f64>, Vec<u8>)> {
    let mut at = 0;
    let mut stack = Vec::new();
    let mut stems = 0usize;
    let mut out = Vec::new();
    let mut first = true;
    while at < code.len() {
        let byte = code[at];
        at += 1;
        if let Some(value) = number(code, &mut at, byte, false).unwrap() {
            stack.push(value);
            continue;
        }
        let op = if byte == 12 {
            let b = code[at];
            at += 1;
            0x0c00 | u16::from(b)
        } else {
            u16::from(byte)
        };
        if first {
            stack.remove(0);
            first = false;
        }
        if matches!(op, 1 | 3 | 18 | 23 | 19 | 20) {
            stems += stack.len() / 2;
        }
        let mask = if matches!(op, 19 | 20) {
            let n = stems.div_ceil(8);
            let v = code[at..at + n].to_vec();
            at += n;
            v
        } else {
            vec![]
        };
        out.push((op, std::mem::take(&mut stack), mask));
    }
    out
}

#[test]
fn every_selected_glyph_and_cid_survives_complete_cff_publication() {
    let bytes = varied();
    let frozen = prepare(&bytes, &coord(8192), &[600, 620, 600, 300]);
    let table = ttf_parser::cff::Table::parse(&frozen.bytes).unwrap();
    assert_eq!(table.number_of_glyphs(), 4);
    for id in 0..4 {
        assert_eq!(table.glyph_cid(ttf_parser::GlyphId(id)), Some(id));
    }
    let (glyphs, private) = parts(&frozen.bytes);
    assert_eq!(glyphs.len(), 4);
    assert_eq!(private.len(), 1);
    assert_eq!(fields(private[0])[&21], [32768.]);
    assert_eq!(unpack(glyphs[1])[1].1, [150., 0.]);
    assert_eq!(frozen.report.glyph_blends, 1);
    assert!(frozen.report.contour_overlaps_checked);
    assert!(frozen.report.preserved_contour_check.is_some());
}
#[test]
fn charstring_widths_cover_zero_and_full_unsigned_hmtx_domain() {
    let frozen = prepare(&simple(vec![], None), &[], &[0, 32767, 32768, 65535]);
    let (glyphs, private) = parts(&frozen.bytes);
    let nominal = fields(private[0])[&21][0];
    for (glyph, width) in glyphs.iter().zip([0., 32767., 32768., 65535.]) {
        let mut at = 1;
        let delta = number(glyph, &mut at, glyph[0], false).unwrap().unwrap();
        assert_eq!(delta + nominal, width);
        assert_eq!(glyph[at..], [14]);
    }
}
#[test]
fn private_blends_preserve_prefix_operands_and_delta_array_semantics() {
    let mut private = Vec::new();
    for value in [-20, 20, 10, 1] {
        n(&mut private, value);
    }
    private.extend([23, 6]);
    entry(&mut private, 0x0c09, &[0.039625]).unwrap();
    let bytes = with_privates(varied(), &[private]);
    let frozen = prepare(&bytes, &coord(8192), &[600; 4]);
    let (_, private) = parts(&frozen.bytes);
    let f = fields(private[0]);
    assert_eq!(f[&6], [-20., 25.]);
    assert_eq!(f[&0x0c09], [0.039625]);
    assert!(!f.contains_key(&22));
    assert!(!f.contains_key(&23));
    assert!(!f.contains_key(&19));
    assert_eq!(frozen.report.private_blends, 1);
}
#[test]
fn private_vsindex_and_glyph_override_are_independent() {
    let store = fixtures::store(
        &[[0, 16384, 16384], [-16384, -16384, 0]],
        &[vec![0], vec![1]],
    );
    let mut private = Vec::new();
    entry(&mut private, 22, &[1.]).unwrap();
    for v in [20, 10, 1] {
        n(&mut private, v);
    }
    private.extend([23, 10]);
    let bytes = with_privates(
        simple(fixtures::variable_glyph(Some(0)), Some(store)),
        &[private],
    );
    let frozen = prepare(&bytes, &coord(8192), &[600; 4]);
    let (glyphs, private) = parts(&frozen.bytes);
    assert_eq!(fields(private[0])[&10], [20.]);
    assert_eq!(unpack(glyphs[1])[1].1, [150., 0.]);
}
#[test]
fn selected_stem_snap_crossings_are_sorted_without_losing_hint_widths() {
    let mut private = Vec::new();
    entry(&mut private, 10, &[0.]).unwrap(); // Zero is a valid dominant width.
    for v in [20, 20, 0, -60, 2] {
        n(&mut private, v);
    }
    private.extend([23, 12, 12]);
    let bytes = with_privates(varied(), &[private]);
    let frozen = prepare(&bytes, &coord(8192), &[600; 4]);
    let (_, private) = parts(&frozen.bytes);
    assert_eq!(fields(private[0])[&10], [0.]);
    assert_eq!(fields(private[0])[&0x0c0c], [10., 10.]); // Absolute set {10, 20}.
    assert_eq!(frozen.report.reordered_stem_snap_arrays, 1);
    let mut duplicates = vec![20., 0.];
    assert!(normalize_snap_widths(&mut duplicates).unwrap());
    assert_eq!(duplicates, [20.]);
    assert!(normalize_snap_widths(&mut vec![20., -30.]).is_err());
}
#[test]
fn nested_local_and_global_calls_are_expanded_under_the_actual_fd() {
    let globals = vec![vec![32, 10]];
    let locals = vec![vec![vec![239, 139, 5]], vec![vec![189, 139, 5]]];
    for format in [0, 3, 4] {
        let bytes = fixtures::fixture(
            &[
                vec![],
                vec![139, 139, 21, 32, 29],
                vec![139, 139, 21, 32, 29],
                vec![],
            ],
            &locals,
            &[None, None],
            &[0, 0, 1, 1],
            format,
            None,
            &globals,
        );
        let frozen = prepare(&bytes, &[], &[600; 4]);
        let (glyphs, private) = parts(&frozen.bytes);
        assert_eq!(private.len(), 1); // identical hints, different subr outlines
        assert_eq!(unpack(glyphs[1])[1].1, [100., 0.]);
        assert_eq!(unpack(glyphs[2])[1].1, [50., 0.]);
        assert!(glyphs.iter().all(|g| unpack(g)
            .iter()
            .all(|(op, _, _)| ![10, 15, 16, 29].contains(op))));
    }
}
#[test]
fn masks_and_blended_stems_remain_bound_to_the_original_bit_order() {
    let mut code = Vec::new();
    for v in [0, 20, 10, 1] {
        n(&mut code, v);
    }
    code.extend([16, 18]);
    code.extend([
        149, 159, 19, 0xc0, 139, 139, 21, 239, 139, 5, 19, 0x80, 139, 149, 5,
    ]);
    let bytes = simple(
        code,
        Some(fixtures::store(&[[0, 16384, 16384]], &[vec![0]])),
    );
    let frozen = prepare(&bytes, &coord(8192), &[600; 4]);
    let (glyphs, _) = parts(&frozen.bytes);
    let ops = unpack(glyphs[1]);
    assert_eq!(ops[0].0, 18);
    assert_eq!(ops[0].1, [0., 25.]);
    assert_eq!(ops[1].0, 23);
    assert_eq!(ops[1].1, [10., 20.]);
    assert_eq!(
        ops.iter()
            .filter(|(op, _, _)| *op == 19)
            .map(|(_, _, mask)| mask.clone())
            .collect::<Vec<_>>(),
        [vec![0xc0], vec![0x80]]
    );
    assert_eq!((frozen.report.stem_hints, frozen.report.masks), (2, 2));
}
#[test]
fn oversized_stem_operands_split_with_rebased_delta_origins_and_width_room() {
    let mut code = Vec::new();
    for _ in 0..30 {
        n(&mut code, 10);
        n(&mut code, 10);
    }
    code.push(18);
    code.extend([19, 255, 255, 255, 252, 139, 139, 21]);
    let frozen = prepare(&simple(code, None), &[], &[600; 4]);
    let (glyphs, _) = parts(&frozen.bytes);
    let ops = unpack(glyphs[1]);
    assert_eq!(ops[0].1.len(), 46);
    assert_eq!(ops[1].1[0], 470.);
    assert_eq!(ops[1].1.len(), 14);
    assert_eq!(ops[2].2, [255, 255, 255, 252]);
    assert!(ops.iter().all(|(_, v, _)| v.len() <= 48));
}
#[test]
fn stem_split_origin_uses_the_deltas_actually_encoded_after_rounding() {
    let mut code = vec![139; 60];
    for _ in 0..60 {
        code.extend([255, 0, 0, 0, 1]);
    }
    n(&mut code, 60);
    code.extend([16, 18, 19, 255, 255, 255, 252, 139, 139, 21]);
    let bytes = simple(
        code,
        Some(fixtures::store(&[[0, 16384, 16384]], &[vec![0]])),
    );
    let frozen = prepare(&bytes, &coord(8192), &[600; 4]);
    let (glyphs, _) = parts(&frozen.bytes);
    let ops = unpack(glyphs[1]);
    assert_eq!(ops[0].1[0], 1. / 65536.);
    assert_eq!(ops[1].1[0], 47. / 65536.);
}
#[test]
fn flex_operators_are_retained_not_replaced_with_unhinted_beziers() {
    let mut code = vec![139, 139, 21];
    for _ in 0..13 {
        n(&mut code, 1);
    }
    code.extend([12, 35]);
    let frozen = prepare(&simple(code, None), &[], &[600; 4]);
    let (glyphs, _) = parts(&frozen.bytes);
    assert_eq!(unpack(glyphs[1])[1].0, 0x0c23);
}
#[test]
fn persistence_rejects_unknown_semantics_that_outline_projection_can_skip() {
    let bytes = simple(vec![139, 139, 21, 200, 2, 239, 139, 5], None);
    let program = Program::parse(&bytes).unwrap();
    assert!(charstring::freeze(&program, 1, &[]).is_ok());
    assert!(freeze(&program, &[], &[600; 4], [0; 4], "Unknown", MAX_OUTPUT).is_err());
    let bytes = with_privates(simple(vec![], None), &[vec![139, 12, 14]]);
    assert!(freeze(
        &Program::parse(&bytes).unwrap(),
        &[],
        &[600; 4],
        [0; 4],
        "Unknown",
        MAX_OUTPUT
    )
    .is_err());
}
#[test]
fn malformed_hint_masks_and_late_stems_do_not_publish() {
    for code in [
        vec![19],
        vec![139, 159, 18, 19, 0x81],
        vec![139, 139, 21, 139, 159, 18],
        vec![139, 159, 18, 19, 0x80, 139, 159, 23],
    ] {
        let bytes = simple(code, None);
        let program = Program::parse(&bytes).unwrap();
        assert!(freeze(&program, &[], &[600; 4], [0; 4], "BadMask", MAX_OUTPUT).is_err());
    }
}
#[test]
fn private_unknown_duplicate_late_vsindex_and_nonblendable_owners_fail() {
    for private in [
        vec![139, 12, 14],
        vec![149, 10, 159, 10],
        vec![149, 149, 140, 23, 10, 139, 22],
        vec![149, 149, 140, 23, 12, 9],
        vec![139, 6],
    ] {
        let bytes = with_privates(varied(), &[private]);
        let program = Program::parse(&bytes).unwrap();
        assert!(freeze(
            &program,
            &coord(8192),
            &[600; 4],
            [0; 4],
            "BadPrivate",
            MAX_OUTPUT
        )
        .is_err());
    }
}
#[test]
fn dict_numbers_roundtrip_bounded_real_and_integer_extremes() {
    for value in [0., -0.001, 0.039625, 32768., i32::MAX as f64, 1e-200, 1e200] {
        let mut code = Vec::new();
        encode_dict(&mut code, value).unwrap();
        let mut at = 1;
        assert_eq!(number(&code, &mut at, code[0], true).unwrap(), Some(value));
        assert_eq!(at, code.len());
    }
    assert!(encode_dict(&mut Vec::new(), f64::INFINITY).is_err());
}
#[test]
fn publication_is_deterministic_and_budget_or_cancellation_leaves_source_unchanged() {
    let bytes = varied();
    let original = bytes.clone();
    let program = Program::parse(&bytes).unwrap();
    assert_eq!(
        prepare(&bytes, &coord(8192), &[600; 4]).bytes,
        prepare(&bytes, &coord(8192), &[600; 4]).bytes
    );
    assert!(freeze(&program, &coord(8192), &[600; 4], [0; 4], "Budget", 32).is_err());
    let token = crate::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(freeze(
            &program,
            &coord(8192),
            &[600; 4],
            [0; 4],
            "Cancelled",
            MAX_OUTPUT
        )
        .is_err())
    });
    assert_eq!(bytes, original);
}
#[test]
fn large_source_fd_domain_collapses_only_identical_frozen_hint_dictionaries() {
    let count = 257;
    let glyphs = vec![vec![]; count];
    let locals = vec![vec![]; count];
    let mapping = (0..count).collect::<Vec<_>>();
    let bytes = fixtures::fixture(&glyphs, &locals, &vec![None; count], &mapping, 4, None, &[]);
    let frozen = prepare(&bytes, &[], &vec![600; count]);
    assert_eq!(frozen.report.output_font_dicts, 1);
    let private = (0..count)
        .map(|i| {
            let mut out = Vec::new();
            entry(&mut out, 10, &[(i + 1) as f64]).unwrap();
            out
        })
        .collect::<Vec<_>>();
    let bytes = with_privates(bytes, &private);
    assert!(freeze(
        &Program::parse(&bytes).unwrap(),
        &[],
        &vec![600; count],
        [0; 4],
        "TooManyFDs",
        MAX_OUTPUT
    )
    .is_err());
}
#[test]
fn distinct_hint_owners_survive_selector_remapping_after_unused_fds_are_removed() {
    let bytes = fixtures::fixture(
        &[vec![], vec![], vec![], vec![]],
        &vec![vec![]; 4],
        &[None; 4],
        &[3, 1, 3, 1],
        4,
        None,
        &[],
    );
    let private = (0..4)
        .map(|i| {
            let mut value = Vec::new();
            entry(&mut value, 10, &[10. + i as f64]).unwrap();
            value
        })
        .collect::<Vec<_>>();
    let frozen = prepare(&with_privates(bytes, &private), &[], &[600; 4]);
    assert_eq!(frozen.report.source_font_dicts, 4);
    assert_eq!(frozen.report.output_font_dicts, 2);
    let (_, at) = read_index(&frozen.bytes, 4);
    let (top, _) = read_index(&frozen.bytes, at);
    let select = fields(top[0])[&0x0c25][0] as usize;
    assert_eq!(&frozen.bytes[select..select + 5], &[0, 1, 0, 1, 0]);
    let (_, private) = parts(&frozen.bytes);
    assert_eq!(fields(private[0])[&10], [11.]);
    assert_eq!(fields(private[1])[&10], [13.]);
}
#[test]
fn collection_selection_and_signature_decision_bind_cff2_publication_to_exact_face() {
    let first = fixtures::sfnt(simple(vec![139, 139, 21, 239, 139, 5], None), false);
    let second = fixtures::sfnt(varied(), true);
    let source = crate::fonts::font_asset::tests::collection(&[&first, &second], 0x00020000, true);
    let mut request = font_instance::tests::request(&source);
    request.selection.face_index = 1;
    assert!(font_instance::prepare_font_instance(&source, &request).is_err());
    request.selection.allow_signature_removal = true;
    let output = font_instance::prepare_font_instance(&source, &request).unwrap();
    assert_eq!(output.report.face_index, 1);
    assert_eq!(output.report.source_face_count, 2);
    assert!(output.report.removed_signature);
    assert!(!output.report.signature_verified);
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    assert_eq!(
        face.glyph_bounding_box(ttf_parser::GlyphId(1))
            .unwrap()
            .x_max,
        150
    );
    request.selection.face_index = 0;
    assert!(font_instance::prepare_font_instance(&source, &request).is_err()); // TEST is not an axis of face 0.
}
#[test]
fn public_font_reopens_with_selected_cff1_outlines_and_existing_tables() {
    let source = fixtures::sfnt(varied(), true);
    let request = font_instance::tests::request(&source);
    let asset = font_instance::prepare_font_instance(&source, &request).unwrap();
    let face = ttf_parser::Face::parse(&asset.bytes, 0).unwrap();
    assert!(!face.is_variable());
    assert_eq!(
        face.glyph_bounding_box(ttf_parser::GlyphId(1))
            .unwrap()
            .x_max,
        150
    );
    assert!(face.tables().cff.is_some());
    assert!(face.tables().cff2.is_none());
    assert_eq!(
        asset.report.output_outline_format,
        crate::fonts::font_asset::OutlineFormat::Cff1
    );
    assert_eq!(asset.report.cff2.as_ref().unwrap().glyphs, 4);
    let table = font_instance::tests::tables(&asset.bytes);
    assert_eq!(table[b"maxp"].len(), 6);
    assert_eq!(
        table[b"cmap"],
        font_instance::tests::tables(&source)[b"cmap"]
    );
}
#[test]
fn nonvariable_cff2_can_be_prepared_without_inventing_axes() {
    let source = fixtures::sfnt(simple(vec![139, 139, 21, 239, 139, 5], None), false);
    let mut request = font_instance::tests::request(&source);
    request.coordinates.clear();
    let output = font_instance::prepare_font_instance(&source, &request).unwrap();
    assert!(output.report.normalized_coordinates.is_empty());
    assert_eq!(output.report.cff2.unwrap().glyph_blends, 0);
}
#[test]
fn cff_publication_composes_hvar_and_mvar_before_serializing_glyph_widths_and_post() {
    let source = fixtures::sfnt(varied(), true);
    let mut tables = font_instance::tests::tables(&source);
    let mut hvar = vec![0; 20];
    hvar[1] = 1;
    hvar[7] = 20;
    for n in [
        1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 4, 1, 1, 0, 0, 40, 0, 0,
    ] {
        hvar.extend(n.to_be_bytes());
    }
    tables.insert(*b"HVAR", hvar);
    let mut mvar = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 1, 0, 20];
    mvar.extend(b"undo");
    mvar.extend([0; 4]);
    for n in [1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 1, 1, 1, 0, 20] {
        mvar.extend(n.to_be_bytes());
    }
    tables.insert(*b"MVAR", mvar);
    let previous = i16::from_be_bytes(tables[b"post"][8..10].try_into().unwrap());
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let output =
        font_instance::prepare_font_instance(&source, &font_instance::tests::request(&source))
            .unwrap();
    let saved = font_instance::tests::tables(&output.bytes);
    let (glyphs, _) = parts(&saved[b"CFF "]);
    let mut at = 1;
    assert_eq!(
        number(glyphs[1], &mut at, glyphs[1][0], false)
            .unwrap()
            .unwrap()
            + 32768.,
        620.
    );
    assert_eq!(
        i16::from_be_bytes(saved[b"post"][8..10].try_into().unwrap()),
        previous + 10
    );
}
#[test]
fn static_cff_publication_retains_vertical_metrics_and_per_glyph_origins() {
    let source = fixtures::sfnt(varied(), true);
    let mut tables = font_instance::tests::tables(&source);
    let mut header = tables[b"hhea"].clone();
    header[34..36].copy_from_slice(&4u16.to_be_bytes());
    tables.insert(*b"vhea", header);
    let mut metrics = Vec::new();
    for gid in 0..4u16 {
        metrics.extend((1000 + gid).to_be_bytes());
        metrics.extend(20i16.to_be_bytes());
    }
    tables.insert(*b"vmtx", metrics);
    tables.insert(*b"VORG", vec![0, 1, 0, 0, 3, 32, 0, 1, 0, 1, 3, 42]);
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let output =
        font_instance::prepare_font_instance(&source, &font_instance::tests::request(&source))
            .unwrap();
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    for gid in 0..4 {
        assert_eq!(
            face.glyph_ver_advance(ttf_parser::GlyphId(gid)),
            Some(1000 + gid)
        );
        assert_eq!(
            face.glyph_y_origin(ttf_parser::GlyphId(gid)),
            Some(if gid == 1 { 810 } else { 800 })
        );
    }
}
#[test]
fn authoring_with_cff2_prepared_asset_saves_searchable_cff1_pdf() {
    use crate::authoring::{PageSize, PdfBuilder, TextStyle};
    let source = fixtures::sfnt(varied(), true);
    let request = font_instance::tests::request(&source);
    let mut builder = PdfBuilder::new();
    let (face, report) = builder
        .register_font_instance_bytes("Frozen CFF2", &source, &request)
        .unwrap();
    assert!(report.cff2.is_some());
    builder
        .add_page(PageSize::custom(200., 200.))
        .draw_text("A A", 10., 50., &TextStyle::new(face, 12.))
        .unwrap();
    let bytes = builder.to_bytes().unwrap();
    assert!(crate::ContentEngine::open_bytes(bytes)
        .unwrap()
        .get_page_text(1)
        .unwrap()
        .contains("A A"));
}
