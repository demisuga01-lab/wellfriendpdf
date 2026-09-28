//! Unexecuted parser-to-paint regressions; no pixel rendering is performed.
use super::*;

fn word(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn dword(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn set32(out: &mut [u8], at: usize, n: usize) {
    out[at..at + 4].copy_from_slice(&(n as u32).to_be_bytes());
}
fn colr(paint: Vec<u8>) -> Vec<u8> {
    let mut out = vec![0; 34];
    out[1] = 1;
    set32(&mut out, 14, 34);
    dword(&mut out, 1);
    word(&mut out, 1);
    dword(&mut out, 10);
    out.extend(paint);
    out
}
fn palette() -> Vec<u8> {
    let mut out = Vec::new();
    for n in [0, 2, 1, 2] {
        word(&mut out, n);
    }
    dword(&mut out, 14);
    word(&mut out, 0);
    out.extend_from_slice(&[0, 0, 255, 255, 255, 0, 0, 255]);
    out
}
fn collect(data: &[u8], coords: &[ttf_parser::NormalizedCoordinate]) -> Vec<ColrPaintOp> {
    let cpal = palette();
    let table =
        ttf_parser::colr::Table::parse(ttf_parser::cpal::Table::parse(&cpal).unwrap(), data)
            .unwrap();
    let mut collector = ColrPaintCollector::new(255, coords);
    table
        .paint(
            GlyphId(1),
            0,
            &mut collector,
            coords,
            RgbaColor::new(0, 0, 0, 255),
        )
        .unwrap();
    assert!(!collector.unsupported, "{:?}", collector.unsupported_ops);
    collector.ops
}
fn solid_glyph() -> Vec<u8> {
    // PaintGlyph(gid 1) -> PaintSolid(first palette entry).
    vec![10, 0, 0, 6, 0, 1, 2, 0, 0, 0x40, 0]
}

#[test]
fn parser_centered_scale_keeps_its_center_fixed() {
    let mut paint = vec![18, 0, 0, 12]; // PaintScaleAroundCenter, child at +12.
    for n in [24576, 8192, 10, 20] {
        word(&mut paint, n);
    } // 1.5, 0.5, center.
    paint.extend(solid_glyph());
    let ops = collect(&colr(paint), &[]);
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].transform.transform_point(10., 20.), (10., 20.));
    assert_eq!(ops[0].transform.transform_point(12., 24.), (13., 22.));
}

#[test]
fn nested_child_scale_precedes_parent_translation_and_stack_restores() {
    let mut collector = ColrPaintCollector::new(255, &[]);
    collector.push_transform(Transform::new_translate(10., 20.));
    collector.push_transform(Transform::new_scale(2., 3.));
    assert_eq!(
        collector.current_transform.transform_point(1., 1.),
        (12., 23.)
    );
    collector.pop_transform();
    assert_eq!(
        collector.current_transform.transform_point(1., 1.),
        (11., 21.)
    );
    collector.pop_transform();
    assert_eq!(collector.current_transform, Transform2D::identity());
}

#[test]
fn variable_color_stop_offsets_and_alpha_use_the_selected_instance() {
    let mut paint = vec![10, 0, 0, 6, 0, 1, 5, 0, 0, 20];
    for n in [0, 0, 100, 0, 0, 100] {
        word(&mut paint, n);
    }
    dword(&mut paint, u32::MAX); // Geometry does not vary; the color line does.
    paint.push(0); // PAD
    word(&mut paint, 2);
    for (offset, color, delta) in [(0, 0, 0), (16384, 1, 2)] {
        word(&mut paint, offset);
        word(&mut paint, color);
        word(&mut paint, 16384);
        dword(&mut paint, delta);
    }
    let mut data = colr(paint);
    let map = data.len();
    set32(&mut data, 26, map);
    data.extend_from_slice(&[0, 1, 0, 4, 0, 1, 2, 3]);
    let store = data.len();
    set32(&mut data, 30, store);
    for n in [1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 4, 1, 1, 0] {
        word(&mut data, n);
    }
    for delta in [4096i16, -8192, 0, 0] {
        word(&mut data, delta as u16);
    }
    for (coordinate, offset, max_alpha) in [(0, 0., 255), (16384, 0.25, 128)] {
        let ops = collect(&data, &[ttf_parser::NormalizedCoordinate::from(coordinate)]);
        assert_eq!(ops.len(), 1);
        let ColrPaint::LinearGradient { stops, .. } = &ops[0].paint else {
            panic!("wrong paint");
        };
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].offset, offset);
        assert!((max_alpha - 1..=max_alpha).contains(&i32::from(stops[0].color[3])));
        assert_eq!(stops[1].offset, 1.);
        assert_eq!(stops[1].color[3], 255);
    }
}
