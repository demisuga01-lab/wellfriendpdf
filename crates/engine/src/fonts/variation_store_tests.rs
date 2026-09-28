//! Unexecuted synthetic regressions for the shared variation arithmetic.
use super::*;
struct Data {
    regions: Vec<u16>,
    rows: Vec<Vec<i32>>,
    words: u16,
    long: bool,
}
fn fixture(axes: u16, regions: &[[i16; 3]], sets: &[Option<Data>]) -> Vec<u8> {
    let region_at = 8 + sets.len() * 4;
    let count = if axes == 0 {
        regions.len()
    } else {
        regions.len() / usize::from(axes)
    };
    let mut out = vec![0; region_at];
    out[0..2].copy_from_slice(&1u16.to_be_bytes());
    out[2..6].copy_from_slice(&(region_at as u32).to_be_bytes());
    out[6..8].copy_from_slice(&(sets.len() as u16).to_be_bytes());
    out.extend_from_slice(&axes.to_be_bytes());
    out.extend_from_slice(&(count as u16).to_be_bytes());
    if axes != 0 {
        for region in regions {
            for value in region {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    for (i, set) in sets.iter().enumerate() {
        let Some(set) = set else {
            continue;
        };
        let at = out.len() as u32;
        out[8 + i * 4..12 + i * 4].copy_from_slice(&at.to_be_bytes());
        out.extend_from_slice(&(set.rows.len() as u16).to_be_bytes());
        out.extend_from_slice(&(set.words | if set.long { 0x8000 } else { 0 }).to_be_bytes());
        out.extend_from_slice(&(set.regions.len() as u16).to_be_bytes());
        for region in &set.regions {
            out.extend_from_slice(&region.to_be_bytes());
        }
        for row in &set.rows {
            assert_eq!(row.len(), set.regions.len());
            for (i, delta) in row.iter().enumerate() {
                match (set.long, i < usize::from(set.words)) {
                    (true, true) => out.extend_from_slice(&delta.to_be_bytes()),
                    (true, false) | (false, true) => {
                        out.extend_from_slice(&(*delta as i16).to_be_bytes())
                    }
                    _ => out.push(*delta as i8 as u8),
                }
            }
        }
    }
    out
}
fn parse(bytes: Vec<u8>) -> ItemVariationStore {
    let len = bytes.len();
    ItemVariationStore::parse(bytes.into(), 0..len).unwrap()
}
fn coordinates(values: &[i16]) -> Vec<ttf_parser::NormalizedCoordinate> {
    values
        .iter()
        .copied()
        .map(ttf_parser::NormalizedCoordinate::from)
        .collect()
}
#[test]
fn row_work_is_available_before_evaluation_and_respects_null_and_sentinel_rows() {
    let store = parse(fixture(
        1,
        &[[0, 16384, 16384], [0, 16384, 16384]],
        &[
            Some(Data {
                regions: vec![0, 1],
                rows: vec![vec![3, 7]],
                words: 2,
                long: false,
            }),
            None,
        ],
    ));
    let prepared = store.prepare(&coordinates(&[8192])).unwrap();
    assert_eq!(prepared.delta_work(0, 0).unwrap(), 2);
    assert_eq!(prepared.delta(0, 0).unwrap(), 5.);
    assert_eq!(prepared.delta_work(1, 200).unwrap(), 0);
    assert_eq!(prepared.delta_work(0xffff, 0xffff).unwrap(), 0);
    assert!(prepared.delta_work(0, 1).is_err());
    assert!(prepared.delta_work(2, 0).is_err());
}
#[test]
fn signed_word_and_byte_deltas_use_the_right_region_columns() {
    let store = parse(fixture(
        1,
        &[[0, 16384, 16384], [-16384, -16384, 0]],
        &[Some(Data {
            regions: vec![0, 1],
            rows: vec![vec![-1000, -120], vec![321, 25]],
            words: 1,
            long: false,
        })],
    ));
    assert_eq!(
        store
            .prepare(&coordinates(&[8192]))
            .unwrap()
            .delta(0, 0)
            .unwrap(),
        -500.
    );
    assert_eq!(
        store
            .prepare(&coordinates(&[-8192]))
            .unwrap()
            .delta(0, 0)
            .unwrap(),
        -60.
    );
    assert_eq!(
        store
            .prepare(&coordinates(&[16384]))
            .unwrap()
            .delta(0, 1)
            .unwrap(),
        321.
    );
}
#[test]
fn long_words_preserve_signed_32_bit_and_signed_short_values() {
    let store = parse(fixture(
        0,
        &[[0; 3]; 2],
        &[Some(Data {
            regions: vec![0, 1],
            rows: vec![vec![-2_000_000_000, -32000]],
            words: 1,
            long: true,
        })],
    ));
    assert_eq!(
        store.prepare(&[]).unwrap().delta(0, 0).unwrap(),
        -2_000_032_000.
    );
    assert!(store.require_cff2_regions_only().is_err());
}
#[test]
fn multi_axis_scalars_multiply_and_outside_axes_zero_the_delta() {
    let store = parse(fixture(
        2,
        &[[0, 16384, 16384], [-16384, -16384, 0]],
        &[Some(Data {
            regions: vec![0],
            rows: vec![vec![100]],
            words: 1,
            long: false,
        })],
    ));
    assert_eq!(
        store
            .prepare(&coordinates(&[8192, -8192]))
            .unwrap()
            .delta(0, 0)
            .unwrap(),
        25.
    );
    assert_eq!(
        store
            .prepare(&coordinates(&[-8192, -8192]))
            .unwrap()
            .delta(0, 0)
            .unwrap(),
        0.
    );
    assert!(store.prepare(&coordinates(&[8192])).is_err());
}
#[test]
fn null_subtables_and_no_variation_sentinel_are_not_invalid_indices() {
    let store = parse(fixture(0, &[], &[None]));
    let prepared = store.prepare(&[]).unwrap();
    assert_eq!(prepared.delta(0, u32::MAX).unwrap(), 0.);
    assert_eq!(prepared.delta(0xffff, 0xffff).unwrap(), 0.);
    assert!(prepared.delta(1, 0).is_err());
    assert!(prepared.delta(0xffff, 0).is_err());
    assert!(store.scalars(0, &[]).unwrap().is_empty());
    store.require_cff2_regions_only().unwrap();
}
#[test]
fn nonparticipating_axes_follow_opentype_rules_without_division_by_zero() {
    for region in [[-16384, 0, 16384], [100, 50, 200], [-100, 50, 100]] {
        assert_eq!(axis_scalar(region, 0), 1.);
    }
    assert_eq!(axis_scalar([0, 8192, 16384], 4096), 0.5);
    assert_eq!(axis_scalar([0, 8192, 16384], 12288), 0.5);
    assert_eq!(axis_scalar([0, 16384, 16384], 16384), 1.);
    assert_eq!(axis_scalar([-16384, -16384, 0], -16384), 1.);
}
#[test]
fn aliased_delta_subtables_share_metadata_and_the_original_allocation() {
    let mut data = fixture(
        0,
        &[[0; 3]],
        &[
            Some(Data {
                regions: vec![0],
                rows: vec![vec![9]],
                words: 0,
                long: false,
            }),
            None,
        ],
    );
    let pointer = data[8..12].to_vec();
    data[12..16].copy_from_slice(&pointer);
    let source: Arc<[u8]> = data.into();
    let store = ItemVariationStore::parse(Arc::clone(&source), 0..source.len()).unwrap();
    assert!(Arc::ptr_eq(&store.source, &source));
    assert!(Arc::ptr_eq(
        store.sets[0].as_ref().unwrap(),
        store.sets[1].as_ref().unwrap()
    ));
    assert_eq!(store.prepare(&[]).unwrap().delta(1, 0).unwrap(), 9.);
}
#[test]
fn store_subrange_is_relative_to_its_own_header_not_the_font() {
    let data = fixture(
        0,
        &[[0; 3]],
        &[Some(Data {
            regions: vec![0],
            rows: vec![vec![7]],
            words: 0,
            long: false,
        })],
    );
    let mut font = vec![0xee; 43];
    font.extend_from_slice(&data);
    let store = ItemVariationStore::parse(font.into(), 43..43 + data.len()).unwrap();
    assert_eq!(store.prepare(&[]).unwrap().delta(0, 0).unwrap(), 7.);
}
#[test]
fn malformed_counts_regions_rows_and_extents_fail_closed() {
    let data = fixture(
        1,
        &[[0, 16384, 16384]],
        &[Some(Data {
            regions: vec![0],
            rows: vec![vec![100]],
            words: 1,
            long: false,
        })],
    );
    let at = u32_at(&data, 8).unwrap() as usize;
    for (offset, value) in [(at + 2, 2u16), (at + 6, 1), (at, 2), (12, 65)] {
        let mut bad = data.clone();
        bad[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
        let len = bad.len();
        assert!(ItemVariationStore::parse(bad.into(), 0..len).is_err());
    }
    assert!(ItemVariationStore::parse(Arc::from(data.clone()), 1..usize::MAX).is_err());
    assert!(parse(data)
        .prepare(&coordinates(&[0]))
        .unwrap()
        .delta(0, 1)
        .is_err());
}
#[test]
fn declared_delta_work_is_limited_before_row_allocation() {
    let mut data = fixture(
        0,
        &[],
        &[Some(Data {
            regions: vec![],
            rows: vec![],
            words: 0,
            long: false,
        })],
    );
    let at = u32_at(&data, 8).unwrap() as usize;
    data[at..at + 2].copy_from_slice(&65535u16.to_be_bytes());
    data[at + 4..at + 6].copy_from_slice(&65535u16.to_be_bytes());
    let len = data.len();
    assert!(matches!(
        ItemVariationStore::parse(data.into(), 0..len),
        Err(WellfriendError::ResourceLimit(_))
    ));
}
#[test]
fn delta_maps_handle_both_counts_packed_widths_and_last_entry_extension() {
    let map = DeltaSetIndexMap::parse(&[0, 0x13, 0, 2, 0, 0x23, 0, 0x45]).unwrap();
    assert_eq!(map.get(0).unwrap(), (2, 3));
    assert_eq!(map.get(9000).unwrap(), (4, 5));
    let map = DeltaSetIndexMap::parse(&[1, 0x3f, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff]).unwrap();
    assert_eq!(map.get(0).unwrap(), (65535, 65535));
    assert!(DeltaSetIndexMap::parse(&[0, 0x80, 0, 0]).is_err());
    assert!(DeltaSetIndexMap::parse(&[0, 0, 0, 0])
        .unwrap()
        .get(0)
        .is_err());
    assert!(DeltaSetIndexMap::parse(&[0, 0, 0, 1]).is_err());
}
#[test]
fn integer_rounding_and_overflow_follow_the_instancing_contract() {
    assert_eq!(round_i32(1.5).unwrap(), 2);
    assert_eq!(round_i32(-1.5).unwrap(), -1);
    assert_eq!(round_i32(-0.5).unwrap(), 0);
    assert_eq!(round_i32(f64::from(i32::MIN)).unwrap(), i32::MIN);
    for value in [f64::NAN, f64::INFINITY, f64::from(i32::MAX) + 1.] {
        assert!(round_i32(value).is_err());
    }
}
#[test]
fn cancellation_prevents_preparation_and_delta_publication() {
    let data = fixture(0, &[], &[None]);
    let store = parse(data.clone());
    let prepared = store.prepare(&[]).unwrap();
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    let len = data.len();
    assert!(cancel
        .scope(|| ItemVariationStore::parse(data.into(), 0..len))
        .is_err());
    assert!(cancel.scope(|| store.prepare(&[])).is_err());
    assert!(cancel.scope(|| prepared.delta(0, 0)).is_err());
}
