//! Unexecuted geometry and dictionary regression source.
use super::*;

fn viewport() -> Viewport {
    Viewport::new([0.0, 0.0, 10.0, 10.0], 72)
}
fn region(bbox: [f64; 4], ctm: Transform2D) -> PaintRegion {
    PaintRegion::new(Some(bbox), &ctm, &viewport(), (0, 0, 10, 10)).unwrap()
}
fn reader() -> PdfReader {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>",
        "[8 9 2 3]",
        ".375",
        "[5 0 R]",
        "null",
        "true",
        "9 0 R",
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    PdfReader::from_bytes(bytes).unwrap()
}
fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

#[test]
fn common_arrays_and_numbers_resolve_references_and_normalize_rectangles() {
    let reader = reader();
    let mut dict = PdfDictionary::empty();
    dict.insert("BBox", reference(4));
    dict.insert("Background", reference(6));
    dict.insert("AntiAlias", reference(8));
    let common = CommonEntries::read(&dict, &reader).unwrap();
    assert_eq!(common.bbox, Some([2.0, 3.0, 8.0, 9.0]));
    assert_eq!(common.background, Some(vec![0.375]));
    dict.insert("BBox", reference(7));
    dict.insert("Background", PdfObject::Null);
    dict.insert("AntiAlias", PdfObject::Null);
    let common = CommonEntries::read(&dict, &reader).unwrap();
    assert!(common.bbox.is_none());
    assert!(common.background.is_none());
    dict.insert("BBox", reference(9));
    assert!(CommonEntries::read(&dict, &reader).is_err());
}

#[test]
fn axis_aligned_bbox_has_fractional_coverage_without_a_dense_mask() {
    let r = region([2.25, 2.25, 6.75, 6.75], Transform2D::identity());
    assert_eq!(r.bounds(), Some((2, 3, 7, 8)));
    assert_eq!(r.coverage(2, 3).unwrap(), 0.5625);
    assert_eq!(r.coverage(4, 4).unwrap(), 1.0);
    assert_eq!(r.coverage(1, 4).unwrap(), 0.0);
    let sum: f32 = (0..10)
        .flat_map(|y| (0..10).map(move |x| r.coverage(x, y).unwrap()))
        .sum();
    assert_eq!(sum, 20.25);
}

#[test]
fn sheared_bbox_is_a_polygon_not_its_axis_aligned_envelope() {
    let r = region(
        [0.0, 0.0, 1.0, 1.0],
        Transform2D::from([1.0, 1.0, -1.0, 1.0, 5.0, 3.0]),
    );
    assert_eq!(r.bounds(), Some((4, 5, 6, 7)));
    for (x, y) in [(4, 5), (5, 5), (4, 6), (5, 6)] {
        assert_eq!(r.coverage(x, y).unwrap(), 0.5);
    }
    assert_eq!(r.coverage(3, 5).unwrap(), 0.0);
    let sum: f32 = (0..10)
        .flat_map(|y| (0..10).map(move |x| r.coverage(x, y).unwrap()))
        .sum();
    assert_eq!(sum, 2.0);
}

#[test]
fn reflected_reversed_and_degenerate_bbox_geometry_is_consistent() {
    let reflected = region(
        [2.0, 2.0, 6.0, 6.0],
        Transform2D::from([-1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
    );
    assert_eq!(reflected.bounds(), Some((4, 4, 8, 8)));
    assert_eq!(reflected.coverage(4, 4).unwrap(), 1.0);
    for bbox in [
        [2.0, 2.0, 2.0, 7.0],
        [2.0, 3.0, 6.0, 3.0],
        [20.0, 20.0, 30.0, 30.0],
    ] {
        assert!(region(bbox, Transform2D::identity()).bounds().is_none());
    }
    assert!(region([0.0, 0.0, 1.0, 1.0], Transform2D::scale(0.0, 1.0))
        .bounds()
        .is_none());
}

#[test]
fn cropped_and_rotated_viewports_keep_bbox_coverage_coordinates() {
    let bbox = Some([1.25, 2.25, 8.75, 7.5]);
    let ctm = Transform2D::from([0.8, 0.2, -0.1, 0.9, 1.0, 0.0]);
    for rotation in [0, 90, 180, 270] {
        let vp = Viewport::new_rotated([0.0, 0.0, 10.0, 10.0], 72, rotation);
        let full = PaintRegion::new(bbox, &ctm, &vp, (0, 0, 10, 10)).unwrap();
        let shifted = full.shifted(2, 3);
        let tile =
            PaintRegion::new(bbox, &ctm, &vp.pixel_window(2, 3, 6, 5), (0, 0, 6, 5)).unwrap();
        for y in 0..5 {
            for x in 0..6 {
                let expected = full.coverage(x + 2, y + 3).unwrap();
                assert!((shifted.coverage(x, y).unwrap() - expected).abs() < 1e-5);
                assert!((tile.coverage(x, y).unwrap() - expected).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn extreme_finite_bbox_is_clipped_before_pixel_area_computation() {
    let r = region([-1e200, -1e200, 1e200, 1e200], Transform2D::identity());
    assert_eq!(r.bounds(), Some((0, 0, 10, 10)));
    for y in 0..10 {
        for x in 0..10 {
            assert_eq!(r.coverage(x, y).unwrap(), 1.0);
        }
    }
    assert!(PaintRegion::new(
        Some([0.0, 0.0, f64::MAX, f64::MAX]),
        &Transform2D::scale(2.0, 2.0),
        &viewport(),
        (0, 0, 10, 10)
    )
    .is_err());
}

#[test]
fn malformed_common_entries_are_rejected_without_large_component_allocations() {
    let reader = reader();
    for (key, value) in [
        ("BBox", PdfObject::Array(vec![PdfObject::Integer(0); 3])),
        ("BBox", PdfObject::Array(vec![PdfObject::Real(f64::NAN); 4])),
        (
            "Background",
            PdfObject::Array(vec![PdfObject::Integer(0); 17]),
        ),
        ("Background", PdfObject::Name("Black".into())),
        ("AntiAlias", PdfObject::Integer(1)),
    ] {
        let mut dict = PdfDictionary::empty();
        dict.insert(key, value);
        assert!(CommonEntries::read(&dict, &reader).is_err(), "{key}");
    }
}
