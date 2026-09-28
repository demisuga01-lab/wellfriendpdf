//! Source-only regressions: not compiled, executed or rendered in this change.
use super::*;

fn n(out: &mut Vec<u8>, value: i16) {
    out.push(28);
    out.extend_from_slice(&value.to_be_bytes());
}
fn d(out: &mut Vec<u8>, value: usize) {
    out.push(29);
    out.extend_from_slice(&(value as u32).to_be_bytes());
}
fn idx(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_be_bytes().to_vec();
    if entries.is_empty() {
        return out;
    }
    out.push(4);
    let mut at = 1u32;
    out.extend_from_slice(&at.to_be_bytes());
    for entry in entries {
        at += entry.len() as u32;
        out.extend_from_slice(&at.to_be_bytes());
    }
    for entry in entries {
        out.extend_from_slice(entry);
    }
    out
}
fn select(mapping: &[usize], format: u8) -> Vec<u8> {
    let mut out = vec![format];
    if format == 0 {
        out.extend(mapping.iter().map(|n| *n as u8));
        return out;
    }
    let ranges = mapping
        .iter()
        .enumerate()
        .filter(|(i, fd)| *i == 0 || mapping[*i - 1] != **fd)
        .map(|(i, fd)| (i, *fd))
        .collect::<Vec<_>>();
    if format == 3 {
        out.extend_from_slice(&(ranges.len() as u16).to_be_bytes());
        for (i, fd) in ranges {
            out.extend_from_slice(&(i as u16).to_be_bytes());
            out.push(fd as u8);
        }
        out.extend_from_slice(&(mapping.len() as u16).to_be_bytes());
    } else {
        out.extend_from_slice(&(ranges.len() as u32).to_be_bytes());
        for (i, fd) in ranges {
            out.extend_from_slice(&(i as u32).to_be_bytes());
            out.extend_from_slice(&(fd as u16).to_be_bytes());
        }
        out.extend_from_slice(&(mapping.len() as u32).to_be_bytes());
    }
    out
}
pub(crate) fn store(regions: &[[i16; 3]], sets: &[Vec<u16>]) -> Vec<u8> {
    let region_start = 8 + sets.len() * 4;
    let data_start = region_start + 4 + regions.len() * 6;
    let mut data = vec![0, 1];
    data.extend_from_slice(&(region_start as u32).to_be_bytes());
    data.extend_from_slice(&(sets.len() as u16).to_be_bytes());
    let mut at = data_start;
    for set in sets {
        data.extend_from_slice(&(at as u32).to_be_bytes());
        at += 6 + set.len() * 2;
    }
    data.extend_from_slice(&1u16.to_be_bytes());
    data.extend_from_slice(&(regions.len() as u16).to_be_bytes());
    for region in regions {
        for value in region {
            data.extend_from_slice(&value.to_be_bytes());
        }
    }
    for set in sets {
        data.extend_from_slice(&[0, 0, 0, 0]);
        data.extend_from_slice(&(set.len() as u16).to_be_bytes());
        for value in set {
            data.extend_from_slice(&value.to_be_bytes());
        }
    }
    let mut out = (data.len() as u16).to_be_bytes().to_vec();
    out.extend(data);
    out
}
/// Build offset-correct CFF2 with explicit local dictionaries, not a renamed CFF1.
pub(crate) fn fixture(
    glyphs: &[Vec<u8>],
    locals: &[Vec<Vec<u8>>],
    private_vs: &[Option<usize>],
    mapping: &[usize],
    format: u8,
    variation: Option<Vec<u8>>,
    globals: &[Vec<u8>],
) -> Vec<u8> {
    let globals = idx(globals);
    let chars = idx(glyphs);
    let selection = if locals.len() > 1 {
        select(mapping, format)
    } else {
        Vec::new()
    };
    let private = locals
        .iter()
        .zip(private_vs)
        .map(|(subs, vs)| {
            let mut out = Vec::new();
            if let Some(vs) = vs {
                d(&mut out, *vs);
                out.push(22);
            }
            if !subs.is_empty() {
                let size = out.len() + 6;
                d(&mut out, size);
                out.push(19);
            }
            out
        })
        .collect::<Vec<_>>();
    let fd = |offsets: &[usize]| {
        idx(&private
            .iter()
            .zip(offsets)
            .map(|(value, at)| {
                let mut out = Vec::new();
                d(&mut out, value.len());
                d(&mut out, if value.is_empty() { 0 } else { *at });
                out.push(18);
                out
            })
            .collect::<Vec<_>>())
    };
    let top = |chars: usize, fds: usize, selection: usize, var: usize| {
        let mut out = Vec::new();
        d(&mut out, chars);
        out.push(17);
        d(&mut out, fds);
        out.extend_from_slice(&[12, 36]);
        if !selection.eq(&0) {
            d(&mut out, selection);
            out.extend_from_slice(&[12, 37]);
        }
        if variation.is_some() {
            d(&mut out, var);
            out.push(24);
        }
        out
    };
    let top_size = top(0, 0, if selection.is_empty() { 0 } else { 1 }, 0).len();
    let chars_at = 5 + top_size + globals.len();
    let fds_at = chars_at + chars.len();
    let selection_at = fds_at + fd(&vec![0; locals.len()]).len();
    let var_at = selection_at + selection.len();
    let mut at = var_at + variation.as_ref().map_or(0, Vec::len);
    let mut offsets = Vec::new();
    for (value, subs) in private.iter().zip(locals) {
        offsets.push(at);
        at += value.len() + idx(subs).len();
    }
    let mut out = vec![2, 0, 5];
    out.extend_from_slice(&(top_size as u16).to_be_bytes());
    out.extend(top(
        chars_at,
        fds_at,
        if selection.is_empty() {
            0
        } else {
            selection_at
        },
        var_at,
    ));
    out.extend(globals);
    out.extend(chars);
    out.extend(fd(&offsets));
    out.extend(selection);
    if let Some(variation) = variation {
        out.extend(variation);
    }
    for (value, subs) in private.iter().zip(locals) {
        out.extend_from_slice(value);
        out.extend(idx(subs));
    }
    out
}
fn rect(width: i16) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [width, 0, 0, 10, -width, 0, 0, -10] {
        n(&mut out, value);
    }
    out.push(5);
    out
}
fn call() -> Vec<u8> {
    vec![139, 139, 21, 32, 10]
}
fn multfd(format: u8) -> Vec<u8> {
    fixture(
        &[vec![], call(), call(), vec![]],
        &[vec![rect(100)], vec![rect(200)]],
        &[None, None],
        &[0, 0, 1, 1],
        format,
        None,
        &[],
    )
}
#[derive(Default)]
struct Pen {
    points: Vec<(f32, f32)>,
}
impl ttf_parser::OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.points.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.points.push((x, y));
    }
    fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
        self.points.extend([(a, b), (x, y)]);
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.points.extend([(a, b), (c, d), (x, y)]);
    }
    fn close(&mut self) {}
}
fn bounds(program: &Program, gid: u16, coord: i16) -> Option<ttf_parser::Rect> {
    let coords = program
        .store
        .as_ref()
        .map(|_| vec![ttf_parser::NormalizedCoordinate::from(coord)])
        .unwrap_or_default();
    program.outline(gid, &coords, &mut Pen::default()).unwrap()
}
pub(crate) fn variable_glyph(explicit: Option<usize>) -> Vec<u8> {
    let mut code = Vec::new();
    if let Some(value) = explicit {
        n(&mut code, value as i16);
        code.push(15);
    }
    code.extend_from_slice(&[139, 139, 21]);
    for value in [100, 100, 1] {
        n(&mut code, value);
    }
    code.push(16);
    code.extend_from_slice(&[139, 5, 139, 149, 5]);
    code
}
pub(crate) fn sfnt(cff2: Vec<u8>, variable: bool) -> Vec<u8> {
    let font = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let container = crate::fonts::font_container::Container::parse(&font).unwrap();
    let mut tables = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, font[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    tables.remove(b"CFF ");
    tables.insert(*b"CFF2", cff2);
    if variable {
        let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
        fvar.extend_from_slice(b"TEST");
        for value in [-65536i32, 0, 65536] {
            fvar.extend_from_slice(&value.to_be_bytes());
        }
        fvar.extend_from_slice(&[0, 0, 1, 0]);
        tables.insert(*b"fvar", fvar);
    }
    crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap()
}

#[test]
fn source_bound_metric_stage_combines_cff2_geometry_with_hvar_advances() {
    let source = sfnt(
        fixture(
            &[vec![], variable_glyph(None), vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            Some(store(&[[0, 16384, 16384]], &[vec![0]])),
            &[],
        ),
        true,
    );
    let container = crate::fonts::font_container::Container::parse(&source).unwrap();
    let mut tables = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, source[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    let mut hvar = vec![0; 20];
    hvar[1] = 1;
    hvar[7] = 20;
    for value in [
        1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 4, 1, 1, 0, 0, 40, 0, 0,
    ] {
        hvar.extend_from_slice(&value.to_be_bytes());
    }
    tables.insert(*b"HVAR", hvar);
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let request =
        crate::fonts::VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 0.5);
    let stage = crate::fonts::font_metric_instance::prepare(&source, 0, &request).unwrap();
    assert_eq!(stage.geometry[1].default.unwrap().x_max, 100);
    assert_eq!(stage.geometry[1].instance.unwrap().x_max, 150);
    assert_eq!(stage.metrics.horizontal[1].advance, 620);
    assert_eq!(stage.coordinates[0].get(), 8192);
}

#[test]
fn static_cff2_without_variation_store_renders_both_actual_dictionaries() {
    for format in [0, 3, 4] {
        let program = Program::parse(&multfd(format)).unwrap();
        assert!(bounds(&program, 0, 0).is_none());
        assert_eq!(bounds(&program, 1, 0).unwrap().x_max, 100);
        assert_eq!(bounds(&program, 2, 0).unwrap().x_max, 200);
        assert!(bounds(&program, 3, 0).is_none());
    }
}
#[test]
fn private_vsindex_is_inherited_and_glyph_override_wins() {
    let variation = store(
        &[[0, 16384, 16384], [-16384, -16384, 0]],
        &[vec![0], vec![1]],
    );
    let bytes = fixture(
        &[
            vec![],
            variable_glyph(None),
            variable_glyph(Some(0)),
            vec![],
        ],
        &[vec![]],
        &[Some(1)],
        &[0; 4],
        0,
        Some(variation),
        &[],
    );
    let program = Program::parse(&bytes).unwrap();
    assert_eq!(bounds(&program, 1, -16384).unwrap().x_max, 200);
    assert_eq!(bounds(&program, 2, -16384).unwrap().x_max, 100);
    assert_eq!(bounds(&program, 1, 0).unwrap().x_max, 100);
    assert_eq!(bounds(&program, 2, 16384).unwrap().x_max, 200);
}
#[test]
fn more_than_64_regions_blend_without_the_old_fixed_scalar_limit() {
    let variation = store(&vec![[0, 16384, 16384]; 65], &[(0..65).collect()]);
    let mut code = vec![139, 139, 21];
    n(&mut code, 10);
    for _ in 0..65 {
        n(&mut code, 1);
    }
    n(&mut code, 1);
    code.extend_from_slice(&[16, 139, 5, 139, 149, 5]);
    let program = Program::parse(&fixture(
        &[vec![], code],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        Some(variation),
        &[],
    ))
    .unwrap();
    assert_eq!(bounds(&program, 1, 16384).unwrap().x_max, 75);
}
#[test]
fn global_subroutine_keeps_calling_glyphs_local_dictionary() {
    let glyph = vec![139, 139, 21, 32, 29];
    let bytes = fixture(
        &[vec![], glyph.clone(), glyph],
        &[vec![rect(100)], vec![rect(250)]],
        &[None, None],
        &[0, 0, 1],
        4,
        None,
        &[vec![32, 10]],
    );
    let program = Program::parse(&bytes).unwrap();
    assert_eq!(bounds(&program, 1, 0).unwrap().x_max, 100);
    assert_eq!(bounds(&program, 2, 0).unwrap().x_max, 250);
}
#[test]
fn large_cff2_path_stacks_are_split_into_valid_cff1_operations() {
    let mut code = vec![139, 139, 21];
    for _ in 0..100 {
        code.extend_from_slice(&[140, 140]);
    }
    code.push(5);
    let program = Program::parse(&fixture(
        &[vec![], code],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        None,
        &[],
    ))
    .unwrap();
    let b = bounds(&program, 1, 0).unwrap();
    assert_eq!((b.x_max, b.y_max), (100, 100));
}
#[test]
fn hint_masks_are_skipped_as_data_not_parsed_as_glyph_operators() {
    let mut code = [139, 149].repeat(8);
    code.extend_from_slice(&[18, 19, 255, 139, 139, 21]);
    code.extend(rect(100));
    let program = Program::parse(&fixture(
        &[vec![], code],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        None,
        &[],
    ))
    .unwrap();
    assert_eq!(bounds(&program, 1, 0).unwrap().x_max, 100);
}
#[test]
fn malformed_selections_and_indices_are_errors_not_first_dictionary_fallback() {
    assert!(Program::parse(&fixture(
        &[vec![], call()],
        &[vec![rect(10)], vec![rect(20)]],
        &[None, None],
        &[0, 2],
        0,
        None,
        &[]
    ))
    .is_err());
    let mut bytes = multfd(3);
    bytes.truncate(12);
    assert!(Program::parse(&bytes).is_err());
    let bytes = fixture(
        &[vec![], vec![139, 139, 21, 139, 10]],
        &[vec![rect(100)]],
        &[None],
        &[0; 2],
        0,
        None,
        &[],
    );
    assert!(Program::parse(&bytes)
        .unwrap()
        .outline(1, &[], &mut Pen::default())
        .is_err());
}
#[test]
fn recursion_stack_operand_arity_and_missing_variation_store_fail() {
    for (glyph, local) in [
        (call(), vec![vec![32, 10]]),
        (vec![139, 5], vec![]),
        (vec![139, 15], vec![]),
        (vec![139, 139, 140, 16], vec![]),
        (vec![139; 514], vec![]),
    ] {
        let bytes = fixture(&[vec![], glyph], &[local], &[None], &[0; 2], 0, None, &[]);
        let program = Program::parse(&bytes).unwrap();
        assert!(program.outline(1, &[], &mut Pen::default()).is_err());
    }
}
#[test]
fn resolved_glyph_and_coordinate_counts_are_checked() {
    let variation = store(&[[0, 16384, 16384]], &[vec![0]]);
    let bytes = fixture(
        &[vec![], variable_glyph(None)],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        Some(variation),
        &[],
    );
    let program = Program::parse(&bytes).unwrap();
    assert!(program.outline(2, &[], &mut Pen::default()).is_err());
    assert!(program.outline(1, &[], &mut Pen::default()).is_err());
}
#[test]
fn source_cache_is_shared_but_cancelled_work_is_not_published() {
    let bytes = multfd(4);
    // An isolated cache makes this independent of parallel test eviction.
    let cache = Mutex::new(VecDeque::new());
    let a = load_cached(&bytes, &cache).unwrap();
    let b = load_cached(&bytes, &cache).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    let mut other = bytes.clone();
    other.push(0);
    assert!(cancel.scope(|| load_cached(&other, &cache)).is_err());
    assert_eq!(cache.lock().unwrap().len(), 1);
    assert!(cancel
        .scope(|| a.outline(1, &[], &mut Pen::default()))
        .is_err());
}
#[test]
fn raster_vector_coverage_and_metrics_share_cff2_dictionary_selection() {
    let font = sfnt(multfd(4), false);
    let face = ttf_parser::Face::parse(&font, 0).unwrap();
    let outliner = crate::fonts::sfnt_outline::Outliner::new(&face).unwrap();
    assert_eq!(
        outliner
            .bounds(ttf_parser::GlyphId(2))
            .unwrap()
            .unwrap()
            .x_max,
        200
    );
    let (_, advance) =
        crate::render::glyph_outline::extract_glyph_path_by_gid_required_advance(&font, 2).unwrap();
    assert_eq!(advance, 600.0);
    assert!(
        crate::render::glyph_outline::extract_glyph_path_by_gid_mapped_outline(&font, 2)
            .unwrap()
            .is_some()
    );
    let run = crate::fonts::TextShaper::shape(&font, "AB", Default::default()).unwrap();
    assert!(
        crate::fonts::coverage::missing_glyph_clusters(&font, "AB", &run)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn malformed_cff2_is_not_reported_as_a_legitimate_blank_in_strict_paths() {
    let font = sfnt(
        fixture(
            &[vec![], vec![139, 5], vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            None,
            &[],
        ),
        false,
    );
    assert!(
        crate::render::glyph_outline::extract_glyph_path_by_gid_required_advance(&font, 1)
            .is_none()
    );
    assert!(
        crate::render::glyph_outline::extract_glyph_path_by_gid_mapped_outline(&font, 2)
            .unwrap()
            .is_none()
    );
}

#[test]
fn extended_variation_store_uses_checked_offsets_beyond_u16_length() {
    let mut variation = store(&[[0, 16384, 16384]], &[vec![0]]);
    let data_at = u32_at(&variation, 10).unwrap() as usize;
    let row = variation[2 + data_at..].to_vec();
    variation.resize(70002, 0);
    variation.extend_from_slice(&row);
    variation[0..2].copy_from_slice(&65535u16.to_be_bytes());
    variation[10..14].copy_from_slice(&70000u32.to_be_bytes());
    let program = Program::parse(&fixture(
        &[vec![], variable_glyph(None)],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        Some(variation),
        &[],
    ))
    .unwrap();
    assert_eq!(bounds(&program, 1, 16384).unwrap().x_max, 200);
}

#[test]
fn explicit_private_zero_vsindex_still_requires_a_store() {
    let bytes = fixture(&[vec![]], &[vec![]], &[Some(0)], &[0], 0, None, &[]);
    assert!(Program::parse(&bytes).is_err());
    let bytes = fixture(
        &[vec![]],
        &[vec![], vec![]],
        &[None, None],
        &[1],
        4,
        None,
        &[],
    );
    let program = Program::parse(&bytes).unwrap();
    assert!(Arc::ptr_eq(
        &program.dictionaries[0],
        &program.dictionaries[1]
    ));
}

fn glyph_program(code: Vec<u8>) -> Program {
    Program::parse(&fixture(
        &[vec![], code],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        None,
        &[],
    ))
    .unwrap()
}

#[test]
fn normalized_curve_operators_keep_optional_and_alternating_components() {
    // Assert actual emitted control/end points, not only the enclosing box.
    let cases: &[(u8, &[i16], &[(f32, f32)])] = &[
        (26, &[9, 1, 2, 3, 4], &[(9., 1.), (11., 4.), (11., 8.)]),
        (27, &[9, 1, 2, 3, 4], &[(1., 9.), (3., 12.), (7., 12.)]),
        (30, &[1, 2, 3, 4, 9], &[(0., 1.), (2., 4.), (6., 13.)]),
        (31, &[1, 2, 3, 4, 9], &[(1., 0.), (3., 3.), (12., 7.)]),
        (
            30,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[
                (0., 1.),
                (2., 4.),
                (6., 4.),
                (11., 4.),
                (17., 11.),
                (26., 19.),
            ],
        ),
        (
            31,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[
                (1., 0.),
                (3., 3.),
                (3., 7.),
                (3., 12.),
                (9., 19.),
                (17., 28.),
            ],
        ),
        (
            24,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[(1., 2.), (4., 6.), (9., 12.), (16., 20.)],
        ),
        (
            25,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[(1., 2.), (4., 6.), (9., 12.), (16., 20.)],
        ),
    ];
    for (op, values, expected) in cases {
        let mut code = vec![139, 139, 21];
        for value in *values {
            n(&mut code, *value);
        }
        code.push(*op);
        let program = glyph_program(code);
        let mut pen = Pen::default();
        program.outline(1, &[], &mut pen).unwrap().unwrap();
        assert_eq!(pen.points[0], (0., 0.));
        assert_eq!(&pen.points[1..], *expected, "operator {op}");
    }
}

#[test]
fn blend_preserves_stack_prefix_and_result_major_multi_region_deltas() {
    let variation = store(&[[0, 16384, 16384], [0, 8192, 16384]], &[vec![0, 1]]);
    let mut code = vec![139, 139, 21];
    // Prefix supplies (3, 4); results supply a second (dx, dy). At 0.5,
    // scalars are (0.5, 1): dx=10+2*.5+3=14, dy=20+4*.5+5=27.
    for value in [3, 4, 10, 20, 2, 3, 4, 5, 2] {
        n(&mut code, value);
    }
    code.extend_from_slice(&[16, 5]);
    let program = Program::parse(&fixture(
        &[vec![], code],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        Some(variation),
        &[],
    ))
    .unwrap();
    let mut pen = Pen::default();
    program
        .outline(1, &[ttf_parser::NormalizedCoordinate::from(8192)], &mut pen)
        .unwrap();
    assert_eq!(pen.points, vec![(0., 0.), (3., 4.), (17., 31.)]);
}

#[test]
fn shared_sfnt_gateway_uses_the_faces_normalized_axis_coordinates() {
    let font = sfnt(
        fixture(
            &[vec![], variable_glyph(None), vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            Some(store(&[[0, 16384, 16384]], &[vec![0]])),
            &[],
        ),
        true,
    );
    let mut face = ttf_parser::Face::parse(&font, 0).unwrap();
    face.set_variation(ttf_parser::Tag::from_bytes(b"TEST"), 0.5)
        .unwrap();
    let outliner = crate::fonts::sfnt_outline::Outliner::new(&face).unwrap();
    assert_eq!(
        outliner
            .bounds(ttf_parser::GlyphId(1))
            .unwrap()
            .unwrap()
            .x_max,
        150
    );
    let request =
        crate::fonts::VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 1.0);
    let (path, width) =
        crate::render::glyph_outline::extract_glyph_path_by_gid_required_advance_var(
            &font, 1, &request,
        )
        .unwrap();
    assert!(path.is_some());
    assert_eq!(width, 600.0);
}

#[test]
fn dictionary_work_is_bounded_before_repeated_expansion() {
    let mut work = 16 * 1024 * 1024;
    assert!(matches!(
        charge_dict_work(&mut work, 1),
        Err(WellfriendError::ResourceLimit(_))
    ));
    let mut work = usize::MAX;
    assert!(charge_dict_work(&mut work, 1).is_err());
}

#[test]
fn all_flex_variants_keep_their_control_points() {
    let cases: &[(u8, &[i16], &[(f32, f32)])] = &[
        (
            34,
            &[1, 2, 3, 4, 5, 6, 7],
            &[
                (1., 0.),
                (3., 3.),
                (7., 3.),
                (12., 3.),
                (18., 0.),
                (25., 0.),
            ],
        ),
        (
            35,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 50],
            &[
                (1., 2.),
                (4., 6.),
                (9., 12.),
                (16., 20.),
                (25., 30.),
                (36., 42.),
            ],
        ),
        (
            36,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[
                (1., 2.),
                (4., 6.),
                (9., 6.),
                (15., 6.),
                (22., 14.),
                (31., 0.),
            ],
        ),
        (
            37,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            &[
                (1., 2.),
                (4., 6.),
                (9., 12.),
                (16., 20.),
                (25., 30.),
                (0., 41.),
            ],
        ),
    ];
    for (op, values, expected) in cases {
        let mut code = vec![139, 139, 21];
        for value in *values {
            n(&mut code, *value);
        }
        code.extend_from_slice(&[12, *op]);
        let mut pen = Pen::default();
        glyph_program(code)
            .outline(1, &[], &mut pen)
            .unwrap()
            .unwrap();
        assert_eq!(&pen.points[1..], *expected, "flex {op}");
    }
}

#[test]
fn compact_subroutine_expansion_stops_at_the_instruction_budget() {
    let locals = vec![[33, 10].repeat(1001), [34, 10].repeat(1001), vec![]];
    let program = Program::parse(&fixture(
        &[vec![], call()],
        &[locals],
        &[None],
        &[0; 2],
        0,
        None,
        &[],
    ))
    .unwrap();
    let result = program.outline(1, &[], &mut Pen::default());
    assert!(matches!(result, Err(WellfriendError::ResourceLimit(_))));
}

#[test]
fn cff2_font_matrix_and_maxp_must_agree_with_sfnt_metadata() {
    let bytes = sfnt(multfd(4), false);
    let container = crate::fonts::font_container::Container::parse(&bytes).unwrap();
    let original = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, bytes[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    for (tag, at, value) in [(*b"head", 18, 2000u16), (*b"maxp", 4, 5u16)] {
        let mut tables = original.clone();
        tables.get_mut(&tag).unwrap()[at..at + 2].copy_from_slice(&value.to_be_bytes());
        let font = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
        let face = ttf_parser::Face::parse(&font, 0).unwrap();
        assert!(crate::fonts::sfnt_outline::Outliner::new(&face).is_err());
    }
}

#[test]
fn fdselect_ranges_require_exact_monotone_coverage() {
    let good = select(&[0, 0, 1, 1], 3);
    assert_eq!(fd_select(&good, Some(0), 4, 2).unwrap(), [0, 0, 1, 1]);
    for (at, value) in [(4, 1), (7, 0), (10, 3), (10, 5)] {
        let mut bad = good.clone();
        bad[at] = value;
        assert!(fd_select(&bad, Some(0), 4, 2).is_err());
    }
    assert!(fd_select(&[], None, 4, 2).is_err());
}

#[test]
fn unknown_operators_clear_operands_without_executing_cff1_return_or_endchar() {
    let mut code = vec![150, 9, 145, 12, 0, 139, 139, 21];
    code.extend(rect(100));
    code.extend_from_slice(&[145, 11, 145, 14, 139, 140, 5]);
    let mut pen = Pen::default();
    glyph_program(code)
        .outline(1, &[], &mut pen)
        .unwrap()
        .unwrap();
    assert_eq!(pen.points.last(), Some(&(0., 1.)));
    let entries = dict(&[150, 31, 145, 12, 0, 140, 17]).unwrap();
    assert_eq!(
        entries,
        vec![(31, vec![11.]), (0x0c00, vec![6.]), (17, vec![1.])]
    );
}

#[test]
fn bcd_numbers_accept_zero_spellings_and_reject_bad_padding_and_exponents() {
    for (bytes, expected) in [
        (vec![0xff], 0.),
        (vec![0xaf], 0.),
        (vec![0xa5, 0xff], 0.5),
        (vec![0xe2, 0xa2, 0x5f], -2.25),
        (vec![0x3c, 0x5f], 0.00003),
    ] {
        let mut at = 0;
        let actual = number(&bytes, &mut at, 30, true).unwrap().unwrap();
        assert!((actual - expected).abs() < 1e-12);
        assert_eq!(at, bytes.len());
    }
    for bytes in [
        vec![0xf1],
        vec![0xdf],
        vec![0x05, 0xff],
        vec![0x2b, 0x05, 0xff],
        vec![0xb5, 0xff],
    ] {
        assert!(number(&bytes, &mut 0, 30, true).is_err());
    }
}

#[test]
fn competing_outline_tables_cannot_silently_change_the_selected_program() {
    let original = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let container = crate::fonts::font_container::Container::parse(&original).unwrap();
    let mut tables = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, original[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    tables.insert(*b"CFF2", multfd(4));
    let font = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let face = ttf_parser::Face::parse(&font, 0).unwrap();
    assert!(crate::fonts::sfnt_outline::Outliner::new(&face).is_err());
}

#[test]
fn null_item_variation_subtable_leaves_blended_defaults_unchanged() {
    let mut variation = store(&[], &[vec![]]);
    variation[10..14].fill(0); // NULL ItemVariationData offset, not table zero.
    let program = Program::parse(&fixture(
        &[vec![], vec![139, 139, 21, 239, 140, 16, 139, 5]],
        &[vec![]],
        &[None],
        &[0; 2],
        0,
        Some(variation),
        &[],
    ))
    .unwrap();
    assert_eq!(bounds(&program, 1, 16384).unwrap().x_max, 100);
}
