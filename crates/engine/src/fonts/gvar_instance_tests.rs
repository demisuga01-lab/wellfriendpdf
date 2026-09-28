//! Directory ownership and tuple evaluation regression source; not executed.
use super::*;
fn words(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}
fn coords(n: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(n)]
}
fn glyph(shared_peak: bool) -> Vec<u8> {
    // One component plus four phantom points. Only component zero and the left
    // phantom point are explicit; component zero is intentionally repeated.
    let mut out = words(&[
        1,
        if shared_peak { 8 } else { 10 },
        13,
        if shared_peak { 0x2000 } else { 0xa000 },
    ]);
    if !shared_peak {
        out.extend(words(&[16384]));
    }
    out.extend([3, 2, 0, 0, 1, 2, 10, 20, 30, 2, 4, 6, 8]);
    out
}
fn table(long: bool, glyphs: &[Vec<u8>], shared: &[i16]) -> Vec<u8> {
    let header_end = 20 + (glyphs.len() + 1) * if long { 4 } else { 2 };
    let mut out = words(&[1, 0, 1, shared.len() as u16]);
    out.extend((header_end as u32).to_be_bytes());
    out.extend(words(&[glyphs.len() as u16, u16::from(long)]));
    out.extend(((header_end + shared.len() * 2) as u32).to_be_bytes());
    let mut relative = 0usize;
    let mut payload = Vec::new();
    for glyph in glyphs {
        if long {
            out.extend((relative as u32).to_be_bytes());
        } else {
            out.extend(((relative / 2) as u16).to_be_bytes());
        }
        payload.extend(glyph);
        if !long && payload.len() % 2 != 0 {
            payload.push(0);
        }
        relative = payload.len();
    }
    if long {
        out.extend((relative as u32).to_be_bytes());
    } else {
        out.extend(((relative / 2) as u16).to_be_bytes());
    }
    for value in shared {
        out.extend(value.to_be_bytes());
    }
    out.extend(payload);
    out
}
fn prepare(data: Vec<u8>, glyphs: u16) -> Result<PreparedGvar> {
    PreparedGvar::prepare(data.into(), &coords(8192), glyphs)
}
#[test]
fn short_and_long_directories_resolve_shared_peaks_and_empty_entries() {
    for long in [false, true] {
        let source: Arc<[u8]> =
            table(long, &[Vec::new(), glyph(true), Vec::new()], &[16384]).into();
        let stage = PreparedGvar::prepare(Arc::clone(&source), &coords(8192), 3).unwrap();
        assert!(Arc::ptr_eq(&source, &stage.data));
        assert!(stage.directory_work > 0);
        let mut budget = Budget::default();
        let out = stage
            .resolve(1, Domain::Components(1), &mut budget)
            .unwrap();
        assert_eq!(
            out.values,
            [[15., 5.], [15., 4.], [0., 0.], [0., 0.], [0., 0.]]
        );
        assert_eq!((out.tuples, out.active_tuples), (1, 1));
        for id in [0, 2] {
            let empty = stage
                .resolve(id, Domain::Components(0), &mut budget)
                .unwrap();
            assert_eq!(empty.values, [[0., 0.]; 4]);
            assert_eq!(empty.tuples, 0);
        }
    }
}
#[test]
fn no_shared_records_does_not_require_a_shared_offset_owner() {
    let mut data = table(true, &[glyph(false)], &[]);
    data[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    let prepared = prepare(data, 1).unwrap();
    assert_eq!(
        prepared
            .resolve(0, Domain::Components(1), &mut Budget::default())
            .unwrap()
            .values[0],
        [15., 5.]
    );
}
#[test]
fn offsets_bind_each_glyph_to_its_own_payload_extent() {
    let mut data = table(true, &[glyph(false), glyph(false)], &[]);
    // Cut the first store one byte short. The second store is not a source of
    // bytes for satisfying the first store's declared tuple payload length.
    let boundary = u32_at(&data, 24).unwrap() - 1;
    data[24..28].copy_from_slice(&boundary.to_be_bytes());
    let prepared = prepare(data, 2).unwrap();
    assert!(prepared
        .resolve(0, Domain::Components(1), &mut Budget::default())
        .is_err());
}
#[test]
fn wrong_axis_glyph_version_flags_and_descending_offsets_are_rejected() {
    let source = table(true, &[glyph(true), glyph(true)], &[16384]);
    for (offset, value) in [(0, 2u16), (4, 2), (12, 1), (14, 2)] {
        let mut data = source.clone();
        data[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
        assert!(prepare(data, 2).is_err());
    }
    let mut data = source.clone();
    data[20..24].copy_from_slice(&1u32.to_be_bytes());
    data[24..28].fill(0);
    assert!(prepare(data, 2).is_err());
    let mut data = source;
    data[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(prepare(data, 2).is_err());
}
#[test]
fn owners_cannot_overlap_headers_or_each_other() {
    let source = table(false, &[glyph(true)], &[16384]);
    for (offset, value) in [(8, 4u32), (16, 4), (8, 26), (16, u32::MAX)] {
        let mut data = source.clone();
        data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(prepare(data, 1).is_err());
    }
    let mut data = source;
    data[24..26].copy_from_slice(&20000u16.to_be_bytes());
    assert!(prepare(data, 1).is_err());
}
#[test]
fn unused_invalid_shared_peaks_are_not_silently_skipped() {
    let mut data = table(true, &[glyph(false)], &[16384, 8192]);
    let start = u32_at(&data, 8).unwrap() as usize;
    data[start + 2..start + 4].copy_from_slice(&20000u16.to_be_bytes());
    assert!(prepare(data, 1).is_err());
}
#[test]
fn selected_coordinates_are_owned_and_cannot_drift_between_glyphs() {
    let source: Arc<[u8]> = table(false, &[glyph(false)], &[]).into();
    let mut coordinate = coords(8192);
    let prepared = PreparedGvar::prepare(source, &coordinate, 1).unwrap();
    coordinate[0] = ttf_parser::NormalizedCoordinate::from(16384);
    assert_eq!(coordinate[0], ttf_parser::NormalizedCoordinate::from(16384));
    assert_eq!(
        prepared
            .resolve(0, Domain::Components(1), &mut Budget::default())
            .unwrap()
            .values[0][0],
        15.
    );
}
#[test]
fn directory_rejects_cvt_domains_invalid_contours_and_out_of_range_glyphs() {
    let prepared = prepare(table(false, &[Vec::new()], &[]), 1).unwrap();
    assert!(prepared
        .resolve(0, Domain::Cvt(1), &mut Budget::default())
        .is_err());
    assert!(prepared
        .resolve(1, Domain::Components(0), &mut Budget::default())
        .is_err());
    assert!(prepared
        .resolve(
            0,
            Domain::Simple {
                points: &[[0, 0]],
                contour_ends: &[]
            },
            &mut Budget::default()
        )
        .is_err());
    assert!(prepared
        .resolve(0, Domain::Components(usize::MAX), &mut Budget::default())
        .is_err());
}
#[test]
fn aggregate_evaluation_budget_and_cancel_apply_to_empty_glyphs_too() {
    let prepared = prepare(table(false, &[Vec::new()], &[]), 1).unwrap();
    let mut budget = Budget { work: 15_999_997 };
    assert!(prepared
        .resolve(0, Domain::Components(0), &mut budget)
        .is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(prepared
            .resolve(0, Domain::Components(0), &mut Budget::default())
            .is_err());
        assert!(prepare(table(false, &[Vec::new()], &[]), 1).is_err());
    });
}
#[test]
fn every_truncated_nonempty_directory_is_rejected_before_evaluation() {
    let data = table(true, &[glyph(true)], &[16384]);
    for end in 0..data.len() {
        assert!(prepare(data[..end].to_vec(), 1).is_err());
    }
}
