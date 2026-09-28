//! Point-preserving outline/metric composition regressions, deliberately unrun.
use super::super::variation_store::u32_at;
use super::*;
fn words(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_be_bytes()).collect()
}
fn simple(points: &[[i16; 2]], ends: &[u16], flags: &[u8], instructions: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let bounds = if points.is_empty() {
        None
    } else {
        Some(ttf_parser::Rect {
            x_min: points.iter().map(|p| p[0]).min().unwrap(),
            y_min: points.iter().map(|p| p[1]).min().unwrap(),
            x_max: points.iter().map(|p| p[0]).max().unwrap(),
            y_max: points.iter().map(|p| p[1]).max().unwrap(),
        })
    };
    program::header(&mut out, ends.len() as i16, bounds);
    for end in ends {
        out.extend(end.to_be_bytes());
    }
    out.extend((instructions.len() as u16).to_be_bytes());
    out.extend(instructions);
    out.extend(flags.iter().map(|f| f & 0x41));
    for axis in 0..2 {
        let mut previous = 0;
        for point in points {
            out.extend((point[axis] - previous).to_be_bytes());
            previous = point[axis];
        }
    }
    out
}
fn triangle() -> Vec<u8> {
    simple(&[[0, 0], [100, 0], [0, 100]], &[2], &[1, 1, 1], &[])
}
fn component(glyph: u16, flags: u16, args: [u16; 2], matrix: &[i16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(flags.to_be_bytes());
    out.extend(glyph.to_be_bytes());
    if flags & 1 != 0 {
        for a in args {
            out.extend(a.to_be_bytes());
        }
    } else {
        for a in args {
            out.push(a as u8);
        }
    }
    out.extend(words(matrix));
    out
}
fn composite(parts: &[Vec<u8>], instructions: &[u8]) -> Vec<u8> {
    let mut out = words(&[-1, 0, 0, 100, 100]);
    for part in parts {
        out.extend(part);
    }
    if !instructions.is_empty() {
        out.extend((instructions.len() as u16).to_be_bytes());
        out.extend(instructions);
    }
    out
}
fn owners(glyphs: &[Vec<u8>], long: bool) -> Tables {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in glyphs {
        if long {
            loca.extend((glyf.len() as u32).to_be_bytes());
        } else {
            loca.extend(((glyf.len() / 2) as u16).to_be_bytes());
        }
        glyf.extend(glyph);
        if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    if long {
        loca.extend((glyf.len() as u32).to_be_bytes());
    } else {
        loca.extend(((glyf.len() / 2) as u16).to_be_bytes());
    }
    let mut head = vec![0; 54];
    head[1] = 1;
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = u8::from(long);
    let mut maxp = vec![0; 32];
    maxp[1] = 1;
    maxp[4..6].copy_from_slice(&(glyphs.len() as u16).to_be_bytes());
    maxp[15] = 2;
    [
        (*b"glyf", glyf.into()),
        (*b"loca", loca.into()),
        (*b"head", head.into()),
        (*b"maxp", maxp.into()),
    ]
    .into_iter()
    .collect()
}
fn parse(tables: &Tables) -> Program {
    Program::parse(
        Arc::clone(&tables[b"glyf"]),
        &tables[b"loca"],
        &tables[b"head"],
        &tables[b"maxp"],
        &mut Budget::default(),
    )
    .unwrap()
}
fn saved(stage: &OutlineStage) -> Tables {
    stage
        .tables
        .iter()
        .map(|(tag, data)| (*tag, Arc::from(data.as_slice())))
        .collect()
}
fn points(program: &Program, id: usize) -> &[[i32; 2]] {
    match &program.glyphs[id].kind {
        Kind::Simple { points, .. } => points,
        _ => panic!("expected simple"),
    }
}
fn bbox(stage: &OutlineStage, id: usize) -> Option<[i16; 4]> {
    stage.geometry[id]
        .instance
        .map(|r| [r.x_min, r.y_min, r.x_max, r.y_max])
}
fn delta_payload(indices: &[u16], x: &[i16], y: &[i16]) -> Vec<u8> {
    let mut data = vec![indices.len() as u8];
    if !indices.is_empty() {
        data.push(0x80 | (indices.len() as u8 - 1));
        let mut prior = 0;
        for i in indices {
            data.extend((i - prior).to_be_bytes());
            prior = *i;
        }
    }
    for values in [x, y] {
        for chunk in values.chunks(64) {
            data.push(0x40 | (chunk.len() as u8 - 1));
            data.extend(words(chunk));
        }
    }
    data
}
fn store(payloads: &[Vec<u8>]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend((payloads.len() as u16).to_be_bytes());
    data.extend(((4 + payloads.len() * 6) as u16).to_be_bytes());
    for payload in payloads {
        data.extend((payload.len() as u16).to_be_bytes());
        data.extend(0xa000u16.to_be_bytes());
        data.extend(16384i16.to_be_bytes());
    }
    for payload in payloads {
        data.extend(payload);
    }
    data
}
fn gvar(stores: &[Vec<u8>]) -> Vec<u8> {
    let mut out = words(&[1, 0, 1, 0]);
    out.extend(0u32.to_be_bytes());
    out.extend((stores.len() as u16).to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend(((20 + (stores.len() + 1) * 4) as u32).to_be_bytes());
    let mut offset = 0u32;
    for source in stores {
        out.extend(offset.to_be_bytes());
        offset += source.len() as u32;
    }
    out.extend(offset.to_be_bytes());
    for source in stores {
        out.extend(source);
    }
    out
}
fn varied(tables: &Tables, stores: &[Vec<u8>], coordinate: i16) -> OutlineStage {
    let g = PreparedGvar::prepare(
        gvar(stores).into(),
        &[ttf_parser::NormalizedCoordinate::from(coordinate)],
        stores.len() as u16,
    )
    .unwrap();
    freeze(tables, Some(&g)).unwrap()
}

#[test]
fn compact_flags_decode_repeated_points_short_vectors_and_signs() {
    let mut glyph = words(&[1, 0, -50, 100, 0, 2, 0]);
    glyph.extend([0x39, 1, 0x17, 100, 50]);
    let tables = owners(&[glyph], false);
    let parsed = parse(&tables);
    assert_eq!(points(&parsed, 0), [[0, 0], [0, 0], [100, -50]]);
    let output = freeze(&tables, None).unwrap();
    assert_eq!(points(&parse(&saved(&output)), 0), points(&parsed, 0));
}
#[test]
fn point_flags_contour_ends_and_instruction_bytes_survive_emission() {
    let instructions = [0xb0, 1, 0x2f];
    let source = simple(
        &[[0, 0], [50, 100], [100, 0], [0, 10], [100, 10]],
        &[2, 4],
        &[1, 0, 1, 1, 1],
        &instructions,
    );
    let tables = owners(&[source], true);
    let original = parse(&tables);
    let stage = freeze(&tables, None).unwrap();
    let reopened = parse(&saved(&stage));
    assert_eq!(points(&reopened, 0), points(&original, 0));
    if let Kind::Simple { flags, ends, .. } = &reopened.glyphs[0].kind {
        assert_eq!(
            flags.iter().map(|f| f & 1).collect::<Vec<_>>(),
            [1, 0, 1, 1, 1]
        );
        assert_eq!(ends, &[2, 4]);
    }
    assert_eq!(
        &reopened.source[reopened.glyphs[0].instructions.clone()],
        instructions
    );
    assert_eq!(stage.retained_instruction_glyphs, [0]);
    assert_eq!(stage.point_counts, [5]);
    assert_eq!(stage.contour_counts, [2]);
}
#[test]
fn duplicate_variations_infer_on_original_points_then_round_once() {
    let tables = owners(
        &[simple(
            &[[0, 0], [50, 50], [100, 100]],
            &[2],
            &[1, 0, 1],
            &[],
        )],
        false,
    );
    let payload = delta_payload(&[0, 0, 2], &[10, 10, 40], &[20, -10, 30]);
    let stage = varied(&tables, &[store(&[payload])], 8192);
    let reopened = parse(&saved(&stage));
    assert_eq!(points(&reopened, 0), [[10, 5], [65, 60], [120, 115]]);
    assert_eq!(bbox(&stage, 0), Some([10, 5, 120, 115]));
    if let Kind::Simple { flags, .. } = &reopened.glyphs[0].kind {
        assert_ne!(flags[0] & 0x40, 0);
    }
}
#[test]
fn fractional_tuple_contributions_are_not_individually_rounded() {
    let tables = owners(&[simple(&[[0, 0]], &[0], &[1], &[])], false);
    let payload = delta_payload(&[0], &[1], &[-1]);
    let stage = varied(&tables, &[store(&[payload.clone(), payload])], 8192);
    assert_eq!(points(&parse(&saved(&stage)), 0), [[1, -1]]);
    assert_eq!((stage.tuple_count, stage.active_tuple_count), (2, 2));
}
#[test]
fn component_matrix_uses_the_declared_column_mapping() {
    let source = simple(&[[10, 20]], &[0], &[1], &[]);
    let parent = composite(&[component(0, 0x83, [0, 0], &[0, 16384, 8192, 0])], &[]);
    let stage = freeze(&owners(&[source, parent], false), None).unwrap();
    assert_eq!(bbox(&stage, 1), Some([10, 10, 10, 10]));
    let reopened = parse(&saved(&stage));
    if let Kind::Composite(parts) = &reopened.glyphs[1].kind {
        assert_eq!(parts[0].matrix, [0, 16384, 8192, 0]);
    } else {
        panic!();
    }
}
#[test]
fn scaled_unscaled_and_default_offsets_have_distinct_placement() {
    let source = simple(&[[10, 20]], &[0], &[1], &[]);
    let parents = [0x800, 0x1000, 0, 0x1800]
        .map(|flag| composite(&[component(0, 0xb | flag, [100, 50], &[8192])], &[]));
    let mut glyphs = vec![source];
    glyphs.extend(parents);
    let stage = freeze(&owners(&glyphs, false), None).unwrap();
    assert_eq!(bbox(&stage, 1), Some([55, 35, 55, 35]));
    for id in [2, 3, 4] {
        assert_eq!(bbox(&stage, id), Some([105, 60, 105, 60]));
    }
    assert_eq!(stage.default_offset_components, [(3, 0), (4, 0)]);
    let reopened = parse(&saved(&stage));
    for id in [3, 4] {
        if let Kind::Composite(c) = &reopened.glyphs[id].kind {
            assert_eq!(c[0].flags & 0x1800, 0x1000);
        }
    }
}
#[test]
fn point_attached_components_retain_indices_and_ignore_component_deltas() {
    let parent = composite(
        &[
            component(0, 0x23, [0, 0], &[]),
            component(0, 1, [1, 0], &[]),
        ],
        &[],
    );
    let tables = owners(&[triangle(), parent], false);
    let stage = varied(
        &tables,
        &[
            Vec::new(),
            store(&[delta_payload(&[0, 1], &[20, 100], &[0, 200])]),
        ],
        16384,
    );
    assert_eq!(bbox(&stage, 1), Some([20, 0, 220, 100]));
    assert_eq!(stage.ignored_attachment_deltas, [(1, 1)]);
    let reopened = parse(&saved(&stage));
    if let Kind::Composite(c) = &reopened.glyphs[1].kind {
        assert_eq!(c[0].arguments, Arguments::Offset([20, 0]));
        assert_eq!(c[1].arguments, Arguments::Points([1, 0]));
    }
}
#[test]
fn component_offset_variations_are_applied_before_scaled_offset_transform() {
    let parent = composite(&[component(0, 0x80b, [100, 0], &[8192])], &[]);
    let tables = owners(&[triangle(), parent], false);
    let stage = varied(
        &tables,
        &[Vec::new(), store(&[delta_payload(&[0], &[20], &[40])])],
        16384,
    );
    assert_eq!(bbox(&stage, 1), Some([60, 20, 110, 70]));
}
#[test]
fn empty_glyphs_and_empty_composites_keep_phantom_deltas_and_hints() {
    let parent = composite(&[component(0, 0x303, [0, 0], &[])], &[0xb0, 0, 0x2f]);
    let tables = owners(&[Vec::new(), parent], false);
    let stage = varied(
        &tables,
        &[
            store(&[delta_payload(&[0, 1], &[10, 40], &[0, 0])]),
            Vec::new(),
        ],
        16384,
    );
    assert_eq!(bbox(&stage, 0), None);
    assert_eq!(bbox(&stage, 1), None);
    let p = stage.geometry[0].phantom.unwrap();
    assert_eq!((p.left, p.right), (10., 40.));
    assert_eq!(stage.retained_instruction_glyphs, [1]);
    assert_eq!(stage.use_my_metrics_glyphs, [1]);
    let reopened = parse(&saved(&stage));
    assert_eq!(
        &reopened.source[reopened.glyphs[1].instructions.clone()],
        [0xb0, 0, 0x2f]
    );
}
#[test]
fn component_dependencies_do_not_reorder_glyph_ids() {
    let parent = composite(&[component(1, 3, [200, 0], &[])], &[]);
    let stage = freeze(&owners(&[parent, triangle()], false), None).unwrap();
    assert_eq!(bbox(&stage, 0), Some([200, 0, 300, 100]));
    let reopened = parse(&saved(&stage));
    assert!(matches!(&reopened.glyphs[0].kind, Kind::Composite(_)));
    assert!(matches!(&reopened.glyphs[1].kind, Kind::Simple { .. }));
}
#[test]
fn cyclic_out_of_range_and_overdeep_components_are_rejected() {
    let self_cycle = composite(&[component(0, 3, [0, 0], &[])], &[]);
    assert!(freeze(&owners(&[self_cycle], false), None).is_err());
    let a = composite(&[component(1, 3, [0, 0], &[])], &[]);
    let b = composite(&[component(0, 3, [0, 0], &[])], &[]);
    assert!(freeze(&owners(&[a, b], false), None).is_err());
    assert!(freeze(
        &owners(&[composite(&[component(1, 3, [0, 0], &[])], &[])], false),
        None
    )
    .is_err());
    let mut glyphs = vec![triangle()];
    for id in 0..65 {
        glyphs.push(composite(&[component(id, 3, [0, 0], &[])], &[]));
    }
    assert!(freeze(&owners(&glyphs, false), None).is_err());
}
#[test]
fn malformed_headers_locations_contours_and_flags_do_not_publish_tables() {
    let original = owners(&[triangle()], false);
    for (tag, at, value) in [
        (*b"head", 50, 2u16),
        (*b"head", 52, 1),
        (*b"maxp", 0, 0),
        (*b"loca", 0, 0xffff),
    ] {
        let mut source = original.clone();
        let mut table = source[&tag].to_vec();
        table[at..at + 2].copy_from_slice(&value.to_be_bytes());
        source.insert(tag, table.into());
        assert!(freeze(&source, None).is_err());
    }
    let mut repeated = words(&[1, 0, 0, 0, 0, 0, 0]);
    repeated.extend([0x39, 1]);
    assert!(freeze(&owners(&[repeated], false), None).is_err());
    let mut reserved = triangle();
    reserved[14] |= 0x80;
    assert!(freeze(&owners(&[reserved], false), None).is_err());
    let malformed = simple(&[[0, 0], [1, 0]], &[1, 1], &[1, 1], &[]);
    assert!(freeze(&owners(&[malformed], false), None).is_err());
}
#[test]
fn invalid_component_flags_and_unavailable_attachment_points_fail_closed() {
    for flags in [0x2013, 0x4b, 1] {
        let parent = composite(&[component(0, flags, [0, 0], &[8192, 8192])], &[]);
        assert!(freeze(&owners(&[triangle(), parent], false), None).is_err());
    }
    let parent = composite(
        &[
            component(0, 0x23, [0, 0], &[]),
            component(0, 1, [0, 3], &[]),
        ],
        &[],
    );
    assert!(freeze(&owners(&[triangle(), parent], false), None).is_err()); // child phantom needs the later hint-aware stage
}
#[test]
fn zero_contour_instruction_programs_and_header_only_glyphs_are_distinct() {
    let header_only = words(&[0, 0, 0, 0, 0]);
    let mut instructed = header_only.clone();
    instructed.extend(words(&[3]));
    instructed.extend([0xb0, 0, 0x2f]);
    let stage = freeze(&owners(&[header_only, instructed, Vec::new()], false), None).unwrap();
    assert_eq!(stage.retained_instruction_glyphs, [1]);
    assert_eq!(u16_at(&stage.tables[b"maxp"], 26).unwrap(), 3);
    let parsed = parse(&saved(&stage));
    assert!(matches!(&parsed.glyphs[0].kind, Kind::Simple { .. }));
    assert!(matches!(&parsed.glyphs[2].kind, Kind::Empty));
}
#[test]
fn global_bounds_exclude_outlineless_glyph_headers() {
    let empty = words(&[0, -20000, -20000, 20000, 20000]);
    let stage = freeze(&owners(&[empty, triangle()], false), None).unwrap();
    assert_eq!(&stage.tables[b"head"][36..44], words(&[0, 0, 100, 100]));
}
#[test]
fn maxp_rebuilds_expanded_counts_and_preserves_instruction_resource_maxima() {
    let parent = composite(
        &[
            component(0, 0x23, [0, 0], &[]),
            component(0, 3, [200, 0], &[]),
        ],
        &[],
    );
    let nested = composite(&[component(1, 3, [0, 0], &[])], &[]);
    let mut tables = owners(&[triangle(), parent, nested], false);
    let mut maxp = tables[b"maxp"].to_vec();
    maxp[16..26].copy_from_slice(&words(&[7, 8, 9, 10, 11]));
    tables.insert(*b"maxp", maxp.into());
    let stage = freeze(&tables, None).unwrap();
    let out = &stage.tables[b"maxp"];
    assert_eq!(&out[6..14], words(&[3, 1, 6, 2]));
    assert_eq!(&out[16..26], words(&[7, 8, 9, 10, 11]));
    assert_eq!(&out[28..32], words(&[2, 2]));
}
#[test]
fn source_bearing_basis_and_repaired_bounds_are_reported_separately() {
    let mut glyph = triangle();
    glyph[2..4].copy_from_slice(&(-20i16).to_be_bytes());
    let mut stage = freeze(&owners(&[glyph], false), None).unwrap();
    assert_eq!(stage.geometry[0].default.unwrap().x_min, -20);
    assert_eq!(bbox(&stage, 0), Some([0, 0, 100, 100]));
    assert_eq!(stage.repaired_default_bounds, [0]);
    stage
        .synchronize_sidebearing_flag(&[Metric {
            advance: 600,
            bearing: 20,
        }])
        .unwrap();
    assert_eq!(u16_at(&stage.tables[b"head"], 16).unwrap() & 2, 0);
    stage
        .synchronize_sidebearing_flag(&[Metric {
            advance: 600,
            bearing: 0,
        }])
        .unwrap();
    assert_eq!(u16_at(&stage.tables[b"head"], 16).unwrap() & 2, 2);
    assert!(stage.synchronize_sidebearing_flag(&[]).is_err());
}
#[test]
fn varied_coordinate_and_relative_displacement_overflow_are_not_saturated() {
    let tables = owners(&[simple(&[[32767, 0]], &[0], &[1], &[])], false);
    let g = PreparedGvar::prepare(
        gvar(&[store(&[delta_payload(&[0], &[1], &[0])])]).into(),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
        1,
    )
    .unwrap();
    assert!(freeze(&tables, Some(&g)).is_err());
    let tables = owners(
        &[simple(&[[-16000, 0], [16000, 0]], &[1], &[1, 1], &[])],
        false,
    );
    let g = PreparedGvar::prepare(
        gvar(&[store(&[delta_payload(&[0, 1], &[-16000, 16000], &[0, 0])])]).into(),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
        1,
    )
    .unwrap();
    assert!(freeze(&tables, Some(&g)).is_err());
}
#[test]
fn cancellation_and_expanded_point_budgets_prevent_output_publication() {
    let tables = owners(&[triangle()], false);
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&tables, None).is_err()));
    let child = simple(&vec![[0, 0]; 40000], &[39999], &vec![1; 40000], &[]);
    let parent = composite(
        &[
            component(0, 0x23, [0, 0], &[]),
            component(0, 3, [0, 0], &[]),
        ],
        &[],
    );
    assert!(freeze(&owners(&[child, parent], true), None).is_err());
}
#[test]
fn wide_glyph_storage_switches_to_long_loca_without_reordering() {
    let glyph = simple(
        &vec![[0, 0]; 1000],
        &(0..1000).collect::<Vec<_>>(),
        &vec![1; 1000],
        &[],
    );
    let source = owners(&vec![glyph; 70], true);
    let stage = freeze(&source, None).unwrap();
    assert_eq!(u16_at(&stage.tables[b"head"], 50).unwrap(), 1);
    assert_eq!(
        u32_at(&stage.tables[b"loca"], 70 * 4).unwrap() as usize,
        stage.tables[b"glyf"].len()
    );
    assert_eq!(parse(&saved(&stage)).glyphs.len(), 70);
}
#[test]
fn complete_internal_preparation_saves_actual_points_and_matching_metrics() {
    let mut tables = owners(&[Vec::new(), triangle(), triangle(), Vec::new()], false);
    let gv = gvar(&[
        Vec::new(),
        store(&[delta_payload(
            &[0, 1, 1, 2, 4],
            &[0, 100, 40, 0, 80],
            &[0; 5],
        )]),
        Vec::new(),
        Vec::new(),
    ]);
    tables.insert(*b"gvar", gv.into());
    let mut fv = words(&[1, 0, 16, 2, 1, 20, 0, 8]);
    fv.extend(b"TEST");
    for n in [-65536i32, 0, 65536] {
        fv.extend(n.to_be_bytes());
    }
    fv.extend(words(&[0, 256]));
    tables.insert(*b"fvar", fv.into());
    // GDEF caret point 2 belongs to glyph 1's exact three explicit points.
    tables.insert(
        *b"GDEF",
        words(&[1, 0, 0, 0, 12, 0, 6, 1, 12, 1, 1, 1, 1, 4, 2, 2]).into(),
    );
    let base = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let mut source = crate::fonts::font_container::Container::parse(&base)
        .unwrap()
        .faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, base[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    source.remove(b"CFF ");
    source.extend(tables.iter().map(|(tag, bytes)| (*tag, bytes.to_vec())));
    let font = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], source.clone()).unwrap();
    let request =
        crate::fonts::VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 0.5);
    let stage = crate::fonts::font_metric_instance::prepare(&font, 0, &request).unwrap();
    assert_eq!(stage.layout.as_ref().unwrap().checked_point_references, 1);
    assert_eq!(stage.layout.as_ref().unwrap().unchecked_point_references, 0);
    let mut invalid_source = source.clone();
    invalid_source.get_mut(b"GDEF").unwrap()[30..32].copy_from_slice(&3u16.to_be_bytes());
    let invalid_font = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], invalid_source).unwrap();
    assert!(crate::fonts::font_metric_instance::prepare(&invalid_font, 0, &request).is_err());
    assert_eq!(stage.metrics.horizontal[1].advance, 640);
    assert_eq!(stage.metrics.horizontal[1].bearing, 0);
    assert_eq!(stage.geometry[1].instance.unwrap().x_max, 170);
    source.extend(stage.outlines.unwrap().tables);
    source.extend(stage.metrics.tables);
    source.extend(stage.layout.unwrap().tables);
    source.extend(stage.hints.unwrap().tables);
    source.remove(b"gvar");
    source.remove(b"fvar");
    let output = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], source).unwrap();
    let reopened = ttf_parser::Face::parse(&output, 0).unwrap();
    assert_eq!(reopened.glyph_index('A'), Some(ttf_parser::GlyphId(1)));
    assert_eq!(
        reopened.glyph_hor_advance(ttf_parser::GlyphId(1)),
        Some(640)
    );
    assert_eq!(
        reopened
            .glyph_bounding_box(ttf_parser::GlyphId(1))
            .unwrap()
            .x_max,
        170
    );
    let again = crate::fonts::font_metric_instance::prepare(
        &output,
        0,
        &crate::fonts::VariationRequest::none(),
    )
    .unwrap();
    assert_eq!(again.metrics.horizontal[1].advance, 640);
    assert_eq!(again.geometry[1].instance.unwrap().x_max, 170);
}

#[test]
fn instructions_flagged_on_an_early_component_are_preserved_after_the_last() {
    let parent = composite(
        &[
            component(0, 0x123, [0, 0], &[]),
            component(0, 3, [200, 0], &[]),
        ],
        &[0xb0, 0, 0x2f],
    );
    let stage = freeze(&owners(&[triangle(), parent], false), None).unwrap();
    let reopened = parse(&saved(&stage));
    let g = &reopened.glyphs[1];
    assert_eq!(&reopened.source[g.instructions.clone()], [0xb0, 0, 0x2f]);
    if let Kind::Composite(parts) = &g.kind {
        assert_eq!(parts[0].flags & 0x100, 0);
        assert_eq!(parts[1].flags & 0x100, 0x100);
    } else {
        panic!();
    }
}

#[test]
fn control_point_bounds_do_not_shrink_to_tight_bezier_ink_bounds() {
    let source = simple(&[[0, 0], [100, 100], [200, 0]], &[2], &[1, 0, 1], &[]);
    let stage = freeze(&owners(&[source], false), None).unwrap();
    assert_eq!(bbox(&stage, 0), Some([0, 0, 200, 100]));
    assert_eq!(&stage.tables[b"head"][36..44], words(&[0, 0, 200, 100]));
}

#[test]
fn child_point_is_transformed_before_attachment_alignment() {
    let child = simple(&[[10, 0], [20, 10]], &[1], &[1, 1], &[]);
    let parent = composite(
        &[
            component(0, 0x23, [0, 0], &[]),
            component(0, 9, [0, 1], &[8192]),
        ],
        &[],
    );
    let stage = freeze(&owners(&[child, parent], false), None).unwrap();
    assert_eq!(bbox(&stage, 1), Some([5, -5, 20, 10]));
}

#[test]
fn later_invalid_variation_payload_returns_no_partial_stage() {
    let source = owners(&[triangle(), triangle()], false);
    let unchanged = source.clone();
    let valid = store(&[delta_payload(&[0], &[20], &[10])]);
    let g = PreparedGvar::prepare(
        gvar(&[valid, vec![0, 1, 0, 4]]).into(),
        &[ttf_parser::NormalizedCoordinate::from(16384)],
        2,
    )
    .unwrap();
    assert!(freeze(&source, Some(&g)).is_err());
    assert_eq!(source, unchanged);
}
