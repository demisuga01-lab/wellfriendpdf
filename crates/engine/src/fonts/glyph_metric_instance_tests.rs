//! Synthetic metric transactions, deliberately unexecuted in this source phase.
use super::*;
fn set16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
fn set32(out: &mut [u8], at: usize, value: usize) {
    out[at..at + 4].copy_from_slice(&(value as u32).to_be_bytes());
}
fn word(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn coord(value: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(value)]
}
fn rect(x0: i16, y0: i16, x1: i16, y1: i16) -> Option<ttf_parser::Rect> {
    Some(ttf_parser::Rect {
        x_min: x0,
        y_min: y0,
        x_max: x1,
        y_max: y1,
    })
}
fn geometry(count: usize) -> Vec<GlyphGeometry> {
    vec![
        GlyphGeometry {
            default: rect(10, -20, 110, 180),
            instance: rect(10, -20, 110, 180),
            phantom: None
        };
        count
    ]
}
fn fixture(count: usize, vertical: bool) -> Tables {
    let mut maxp = vec![0, 0, 0x50, 0, 0, 0];
    set16(&mut maxp, 4, count as u16);
    let mut header = vec![0; 36];
    set32(&mut header, 0, 0x10000);
    set16(&mut header, 34, 1); // Shared advance, distinct per-glyph bearings.
    let mut hmtx = vec![2, 88, 0, 10]; // 600, 10
    let mut vmtx = vec![3, 232, 0, 20]; // 1000, 20
    for _ in 1..count {
        word(&mut hmtx, 10);
        word(&mut vmtx, 20);
    }
    let mut tables: Tables = [
        (*b"maxp", maxp),
        (*b"hhea", header.clone()),
        (*b"hmtx", hmtx),
    ]
    .into_iter()
    .map(|(tag, data)| (tag, Arc::<[u8]>::from(data)))
    .collect();
    if vertical {
        tables.insert(*b"vhea", header.into());
        tables.insert(*b"vmtx", vmtx.into());
    }
    tables
}
fn store(deltas: &[i16]) -> Vec<u8> {
    let mut out = Vec::new();
    for n in [
        1u16,
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
        deltas.len() as u16,
        1,
        1,
        0,
    ] {
        word(&mut out, n);
    }
    for delta in deltas {
        word(&mut out, *delta as u16);
    }
    out
}
fn variation(vertical: bool, deltas: &[i16], maps: [Option<Vec<u16>>; 4]) -> Vec<u8> {
    let mut out = vec![0; if vertical { 24 } else { 20 }];
    set32(&mut out, 0, 0x10000);
    for (i, map) in maps.iter().enumerate().take(if vertical { 4 } else { 3 }) {
        if let Some(entries) = map {
            let offset = out.len();
            set32(&mut out, 8 + i * 4, offset);
            out.extend_from_slice(&[0, 0x1f]); // Two-byte entries, 16 inner bits.
            word(&mut out, entries.len() as u16);
            for entry in entries {
                word(&mut out, *entry);
            }
        }
    }
    let offset = out.len();
    set32(&mut out, 4, offset);
    out.extend(store(deltas));
    out
}
fn insert_variation(
    tables: &mut Tables,
    vertical: bool,
    deltas: &[i16],
    maps: [Option<Vec<u16>>; 4],
) {
    tables.insert(
        if vertical { *b"VVAR" } else { *b"HVAR" },
        variation(vertical, deltas, maps).into(),
    );
}
fn no_maps() -> [Option<Vec<u16>>; 4] {
    [None, None, None, None]
}
fn stage(tables: &Tables, geo: &[GlyphGeometry]) -> GlyphMetricStage {
    freeze(tables, &coord(16384), OutlineKind::Cff, geo).unwrap()
}
fn read_stage(stage: &GlyphMetricStage, vertical: bool, count: usize) -> Vec<Metric> {
    let tables = stage
        .tables
        .iter()
        .map(|(tag, data)| (*tag, Arc::from(data.as_slice())))
        .collect();
    source_metrics(&tables, vertical, count).unwrap().unwrap()
}

#[test]
fn implicit_advance_indices_expand_a_compressed_metric_tail() {
    let mut source = fixture(3, false);
    insert_variation(&mut source, false, &[10, 20, 30], no_maps());
    let before = source.clone();
    let stage = stage(&source, &geometry(3));
    assert_eq!(
        stage
            .horizontal
            .iter()
            .map(|m| m.advance)
            .collect::<Vec<_>>(),
        vec![610, 620, 630]
    );
    assert_eq!(u16_at(&stage.tables[b"hhea"], 34).unwrap(), 3);
    assert_eq!(read_stage(&stage, false, 3), stage.horizontal);
    assert_eq!(source, before);
}

#[test]
fn explicit_last_entry_extension_and_map_aliases_preserve_every_gid() {
    let mut source = fixture(3, false);
    let mut data = variation(false, &[25, 50], [Some(vec![1]), Some(vec![0]), None, None]);
    // Reuse the leading-bearing map for the redundant trailing-bearing map.
    let leading = u32_at(&data, 12).unwrap() as usize;
    set32(&mut data, 16, leading);
    source.insert(*b"HVAR", data.into());
    let mut geo = geometry(3);
    for g in &mut geo {
        g.instance = rect(35, -20, 135, 180);
    }
    let stage = stage(&source, &geo);
    assert!(stage.horizontal.iter().all(|m| *m
        == Metric {
            advance: 650,
            bearing: 35
        }));
    assert_eq!(u16_at(&stage.tables[b"hhea"], 34).unwrap(), 1);
    assert!(stage.differences.is_empty()); // Width +50 is split equally between bearings.
}

#[test]
fn cff_without_leading_map_uses_instanced_outline_minimum() {
    let source = fixture(1, false);
    let mut geo = geometry(1);
    geo[0].instance = rect(-25, -20, 130, 180);
    let stage = stage(&source, &geo);
    assert_eq!(
        stage.horizontal[0],
        Metric {
            advance: 600,
            bearing: -25
        }
    );
    assert_eq!(signed(&stage.tables[b"hhea"], 12).unwrap(), -25);
    assert_eq!(signed(&stage.tables[b"hhea"], 14).unwrap(), 470);
    assert_eq!(signed(&stage.tables[b"hhea"], 16).unwrap(), 130);
}

#[test]
fn truetype_phantom_pairs_control_advances_without_hvar_or_vvar() {
    let mut source = fixture(1, true);
    source.insert(*b"gvar", Arc::from([])); // The outline transaction supplies resolved deltas.
    let mut geo = geometry(1);
    geo[0].instance = rect(30, -20, 150, 200);
    geo[0].phantom = Some(PhantomDeltas {
        left: 5.,
        right: 35.,
        top: 12.,
        bottom: -28.,
    });
    let stage = freeze(&source, &coord(16384), OutlineKind::TrueType, &geo).unwrap();
    assert_eq!(
        stage.horizontal[0],
        Metric {
            advance: 630,
            bearing: 25
        }
    );
    assert_eq!(
        stage.vertical.as_ref().unwrap()[0],
        Metric {
            advance: 1040,
            bearing: 12
        }
    );
    assert!(!stage.tables.contains_key(b"VORG"));
}

#[test]
fn explicit_variation_metrics_are_not_added_twice_to_phantom_metrics() {
    let mut source = fixture(1, true);
    source.insert(*b"gvar", Arc::from([]));
    insert_variation(
        &mut source,
        false,
        &[30, 15],
        [Some(vec![0]), Some(vec![1]), None, None],
    );
    insert_variation(
        &mut source,
        true,
        &[40, -8],
        [Some(vec![0]), Some(vec![1]), None, None],
    );
    let mut geo = geometry(1);
    geo[0].instance = rect(30, -20, 150, 200);
    geo[0].phantom = Some(PhantomDeltas {
        left: 5.,
        right: 35.,
        top: 12.,
        bottom: -28.,
    });
    let stage = freeze(&source, &coord(16384), OutlineKind::TrueType, &geo).unwrap();
    assert_eq!(
        stage.horizontal[0],
        Metric {
            advance: 630,
            bearing: 25
        }
    );
    assert_eq!(
        stage.vertical.as_ref().unwrap()[0],
        Metric {
            advance: 1040,
            bearing: 12
        }
    );
    assert!(stage.differences.is_empty());
}

#[test]
fn cff_vertical_origins_and_top_bearings_move_together() {
    let mut source = fixture(3, true);
    source.insert(*b"VORG", vec![0, 1, 0, 0, 0, 200, 0, 0].into());
    insert_variation(
        &mut source,
        true,
        &[0, 25, 50],
        [Some(vec![0]), None, None, Some(vec![1, 2, 1])],
    );
    let mut geo = geometry(3);
    geo[1].instance = rect(10, -20, 110, 210);
    let stage = stage(&source, &geo);
    assert_eq!(stage.origins.as_ref().unwrap(), &vec![225, 250, 225]);
    assert_eq!(
        stage
            .vertical
            .as_ref()
            .unwrap()
            .iter()
            .map(|m| m.bearing)
            .collect::<Vec<_>>(),
        vec![45, 40, 45]
    );
    // Most frequent origin is the default; the one exceptional GID stays explicit.
    assert_eq!(signed(&stage.tables[b"VORG"], 4).unwrap(), 225);
    assert_eq!(u16_at(&stage.tables[b"VORG"], 6).unwrap(), 1);
    assert_eq!(u16_at(&stage.tables[b"VORG"], 8).unwrap(), 1);
    let output = stage
        .tables
        .iter()
        .map(|(tag, data)| (*tag, Arc::from(data.as_slice())))
        .collect();
    assert_eq!(source_origins(&output, 3).unwrap(), stage.origins);
}

#[test]
fn vertical_origin_can_be_derived_then_varied_without_a_source_vorg() {
    let mut source = fixture(1, true);
    insert_variation(
        &mut source,
        true,
        &[0, -300],
        [Some(vec![0]), None, None, Some(vec![1])],
    );
    let stage = stage(&source, &geometry(1));
    assert_eq!(stage.origins, Some(vec![-100]));
    assert_eq!(stage.vertical.as_ref().unwrap()[0].bearing, -280);
}

#[test]
fn negative_fractional_metric_values_are_rounded_once() {
    let mut source = fixture(1, true);
    let mut vmtx = source[b"vmtx"].to_vec();
    vmtx[2..4].copy_from_slice(&(-190i16).to_be_bytes());
    source.insert(*b"vmtx", vmtx.into());
    source.insert(*b"VORG", vec![0, 1, 0, 0, 255, 246, 0, 0].into()); // -10
    insert_variation(
        &mut source,
        true,
        &[1, -1],
        [Some(vec![0]), None, None, Some(vec![1])],
    );
    let stage = freeze(&source, &coord(8192), OutlineKind::Cff, &geometry(1)).unwrap();
    assert_eq!(stage.origins, Some(vec![-10])); // -10.5 rounds toward positive infinity.
    assert_eq!(stage.vertical.as_ref().unwrap()[0].bearing, -190);
    assert_eq!(stage.vertical.as_ref().unwrap()[0].advance, 1001);
}

#[test]
fn redundant_bearing_conflicts_are_visible_in_the_stage_receipt() {
    let mut source = fixture(1, true);
    source.insert(*b"VORG", vec![0, 1, 0, 0, 0, 200, 0, 0].into());
    insert_variation(
        &mut source,
        true,
        &[0, 100],
        [Some(vec![0]), Some(vec![1]), Some(vec![1]), None],
    );
    let stage = stage(&source, &geometry(1));
    assert_eq!(stage.differences.len(), 2);
    assert_eq!(stage.differences[0].field, "leading_bearing");
    assert_eq!(stage.differences[0].declared, 120);
    assert_eq!(stage.differences[0].derived, 20);
    assert_eq!(stage.differences[1].field, "trailing_bearing");
}

#[test]
fn empty_glyphs_keep_advances_but_do_not_pollute_header_ink_extrema() {
    let mut source = fixture(2, false);
    insert_variation(&mut source, false, &[100, 0], no_maps());
    let mut geo = geometry(2);
    geo[0].default = None;
    geo[0].instance = None;
    let stage = stage(&source, &geo);
    assert_eq!(
        stage.horizontal[0],
        Metric {
            advance: 700,
            bearing: 0
        }
    );
    assert_eq!(u16_at(&stage.tables[b"hhea"], 10).unwrap(), 700);
    assert_eq!(signed(&stage.tables[b"hhea"], 12).unwrap(), 10);
}

#[test]
fn missing_nonfinite_phantoms_and_wrong_geometry_count_fail_closed() {
    let mut source = fixture(1, false);
    source.insert(*b"gvar", Arc::from([]));
    assert!(freeze(&source, &coord(16384), OutlineKind::TrueType, &[]).is_err());
    assert!(freeze(&source, &coord(16384), OutlineKind::TrueType, &geometry(1)).is_err());
    let mut geo = geometry(1);
    geo[0].phantom = Some(PhantomDeltas {
        left: f64::NAN,
        ..PhantomDeltas::default()
    });
    assert!(freeze(&source, &coord(16384), OutlineKind::TrueType, &geo).is_err());
}

#[test]
fn truetype_ignores_an_inapplicable_vorg_table() {
    let mut source = fixture(1, true);
    source.insert(*b"VORG", vec![0xff].into());
    let stage = freeze(&source, &coord(16384), OutlineKind::TrueType, &geometry(1)).unwrap();
    assert!(stage.origins.is_none());
    assert_eq!(stage.vertical.as_ref().unwrap()[0].bearing, 20);
}

#[test]
fn malformed_metric_lengths_counts_and_pairs_are_rejected() {
    for mode in 0..4 {
        let mut source = fixture(3, true);
        match mode {
            0 => {
                source.insert(*b"hmtx", vec![0, 1].into());
            }
            1 => {
                let mut h = source[b"hhea"].to_vec();
                set16(&mut h, 34, 4);
                source.insert(*b"hhea", h.into());
            }
            2 => {
                source.remove(b"vhea");
            }
            _ => {
                let mut h = source[b"hhea"].to_vec();
                set16(&mut h, 32, 1);
                source.insert(*b"hhea", h.into());
            }
        }
        assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(3)).is_err());
    }
}

#[test]
fn vorg_duplicate_unsorted_and_out_of_range_glyphs_are_rejected() {
    for ids in [[0, 0], [1, 0], [0, 3]] {
        let mut source = fixture(3, true);
        let mut vorg = vec![0, 1, 0, 0, 0, 200, 0, 2];
        for gid in ids {
            word(&mut vorg, gid);
            word(&mut vorg, 250);
        }
        source.insert(*b"VORG", vorg.into());
        assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(3)).is_err());
    }
}

#[test]
fn metric_field_overflow_is_not_saturated_and_input_remains_unchanged() {
    let mut source = fixture(1, false);
    let mut metrics = source[b"hmtx"].to_vec();
    set16(&mut metrics, 0, 65530);
    source.insert(*b"hmtx", metrics.into());
    insert_variation(&mut source, false, &[100], no_maps());
    let before = source.clone();
    assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(1)).is_err());
    assert_eq!(source, before);
}

#[test]
fn missing_implicit_delta_rows_and_empty_explicit_maps_are_not_zero_deltas() {
    for maps in [no_maps(), [Some(vec![]), None, None, None]] {
        let mut source = fixture(3, false);
        insert_variation(&mut source, false, &[5], maps);
        assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(3)).is_err());
    }
}

#[test]
fn malformed_variation_headers_maps_and_coordinate_counts_are_rejected() {
    for mode in 0..4 {
        let mut source = fixture(1, false);
        let mut data = variation(false, &[10], [Some(vec![0]), None, None, None]);
        match mode {
            0 => set32(&mut data, 4, 4),
            1 => set32(&mut data, 8, 8),
            2 => data[20] = 1, // Wrong map format for this parent.
            _ => {
                data.pop();
            }
        }
        source.insert(*b"HVAR", data.into());
        assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(1)).is_err());
    }
    let mut source = fixture(1, false);
    insert_variation(&mut source, false, &[10], no_maps());
    assert!(freeze(&source, &[], OutlineKind::Cff, &geometry(1)).is_err());
}

#[test]
fn global_caret_metrics_survive_per_glyph_header_rebuilding() {
    let mut source = fixture(1, false);
    let mut mvar = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 1, 0, 20];
    mvar.extend_from_slice(b"hcrs");
    mvar.extend_from_slice(&[0; 4]);
    mvar.extend(store(&[80]));
    source.insert(*b"MVAR", mvar.into());
    insert_variation(&mut source, false, &[25], no_maps());
    let stage = stage(&source, &geometry(1));
    assert_eq!(signed(&stage.tables[b"hhea"], 18).unwrap(), 80);
    assert_eq!(u16_at(&stage.tables[b"hhea"], 10).unwrap(), 625);
    assert_eq!(stage.global_changes.len(), 1);
}

#[test]
fn late_glyph_failure_discards_earlier_global_metric_stage() {
    let mut source = fixture(2, false);
    let mut mvar = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 1, 0, 20];
    mvar.extend_from_slice(b"hcrs");
    mvar.extend_from_slice(&[0; 4]);
    mvar.extend(store(&[80]));
    source.insert(*b"MVAR", mvar.into());
    insert_variation(&mut source, false, &[25], no_maps()); // GID 1 has no row.
    let before = source.clone();
    assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(2)).is_err());
    assert_eq!(source, before);
}

#[test]
fn cancellation_prevents_returning_any_partial_metric_transaction() {
    let source = fixture(1, true);
    let before = source.clone();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token
        .scope(|| assert!(freeze(&source, &coord(16384), OutlineKind::Cff, &geometry(1)).is_err()));
    assert_eq!(source, before);
}
