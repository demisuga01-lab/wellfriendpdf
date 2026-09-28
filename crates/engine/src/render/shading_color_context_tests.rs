//! Unexecuted regression source; not observed rendering evidence.
use super::*;
use crate::render::{buffer::RenderMode, cmm::ColorTransformBackend};

const BACKGROUND: PixelColor = [255, 0, 255, 255];
fn name(value: &str) -> PdfObject {
    PdfObject::Name(value.into())
}
fn numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}
fn reader() -> PdfReader {
    PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
}
fn icc(n: i64) -> PdfObject {
    let mut profile = PdfDictionary::empty();
    profile.insert("N", PdfObject::Integer(n));
    profile.insert(
        "Alternate",
        name(match n {
            1 => "DeviceGray",
            3 => "DeviceRGB",
            4 => "DeviceCMYK",
            _ => unreachable!(),
        }),
    );
    PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream {
            dict: profile,
            raw: vec![],
        },
    ])
}
fn palette(base: PdfObject) -> PdfObject {
    PdfObject::Array(vec![
        name("Indexed"),
        base,
        PdfObject::Integer(0),
        PdfObject::String(vec![64]),
    ])
}
fn domains() -> (PdfObject, PdfObject) {
    let mut base = icc(1);
    if let PdfObject::Array(items) = &mut base {
        if let PdfObject::Stream { dict, .. } = &mut items[1] {
            dict.insert("Range", numbers(&[0.25, 0.75]));
        }
    }
    (palette(base), palette(name("DeviceGray")))
}
fn function(inputs: usize, program: &[u8]) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0].repeat(inputs)));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    PdfObject::Stream {
        dict,
        raw: program.to_vec(),
    }
}
fn constant_function() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("C0", numbers(&[0.0]));
    dict.insert("C1", numbers(&[0.0]));
    dict.insert("N", PdfObject::Integer(1));
    PdfObject::Dictionary(dict)
}
fn shading(kind: i64, space: PdfObject) -> (PdfDictionary, Vec<u8>) {
    let mut dict = PdfDictionary::empty();
    let mut data = Vec::new();
    dict.insert("ShadingType", PdfObject::Integer(kind));
    dict.insert("ColorSpace", space);
    match kind {
        1 => {
            dict.insert("Domain", numbers(&[0.0, 10.0, 0.0, 10.0]));
            dict.insert("Function", function(2, b"{ pop pop 0 }"));
        }
        2 | 3 => {
            dict.insert(
                "Coords",
                numbers(if kind == 2 {
                    &[0.0, 0.0, 10.0, 0.0]
                } else {
                    &[5.0, 5.0, 0.0, 5.0, 5.0, 10.0]
                }),
            );
            dict.insert("Function", constant_function());
        }
        4..=7 => {
            dict.insert("BitsPerCoordinate", PdfObject::Integer(8));
            dict.insert("BitsPerComponent", PdfObject::Integer(8));
            dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 0.0]));
            if kind != 5 {
                dict.insert("BitsPerFlag", PdfObject::Integer(8));
            }
            match kind {
                4 => data.extend_from_slice(&[0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0]),
                5 => {
                    dict.insert("VerticesPerRow", PdfObject::Integer(2));
                    data.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255, 0, 255, 255, 0]);
                }
                6 | 7 => {
                    data.push(0);
                    data.extend_from_slice(&[
                        0, 0, 85, 0, 170, 0, 255, 0, 255, 85, 255, 170, 255, 255, 170, 255, 85,
                        255, 0, 255, 0, 170, 0, 85,
                    ]);
                    if kind == 7 {
                        data.extend_from_slice(&[85, 85, 170, 85, 170, 170, 85, 170]);
                    }
                    data.extend_from_slice(&[0; 4]);
                }
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
    (dict, data)
}
fn render(
    dict: &PdfDictionary,
    data: &[u8],
    source: Option<&PdfObject>,
) -> (PixelBuffer, Result<(), String>) {
    let mut buf = PixelBuffer::new_filled_with_mode(10, 10, BACKGROUND, RenderMode::Compat);
    let result = ShadingRenderer::paint_with_options_cancellable(
        dict,
        &Transform2D::identity(),
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut buf,
        &reader(),
        if data.is_empty() { None } else { Some(data) },
        ShadingRenderOptions::default().with_color_context(
            source,
            ColorTransformOptions {
                backend: ColorTransformBackend::DeterministicFallback,
                ..Default::default()
            },
        ),
        &CancelToken::none(),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    );
    (buf, result)
}

#[test]
fn all_seven_shading_families_keep_source_palette_domains() {
    let (target, source) = domains();
    for kind in 1..=7 {
        let (dict, data) = shading(kind, target.clone());
        let (paint, result) = render(&dict, &data, Some(&source));
        result.unwrap();
        assert_eq!(
            paint.get_pixel(2, 7),
            [64, 64, 64, 255],
            "source domain, shading {kind}"
        );
        let (own, result) = render(&dict, &data, None);
        result.unwrap();
        assert_eq!(
            own.get_pixel(2, 7),
            [96, 96, 96, 255],
            "ICC palette domain, shading {kind}"
        );
    }
}
#[test]
fn shading_validation_and_paint_both_obey_selected_icc_backend() {
    let (target, source) = domains();
    let (dict, data) = shading(2, target);
    let before = crate::render::icc_conversion::metrics();
    render(&dict, &data, Some(&source)).1.unwrap();
    let after = crate::render::icc_conversion::metrics();
    assert!(after.policy_alternates > before.policy_alternates);
    assert_eq!(
        after.unavailable_profile_alternates,
        before.unavailable_profile_alternates
    );
    assert_eq!(after.native_conversions, before.native_conversions);
}
#[test]
fn icc_mesh_component_counts_follow_n_for_gray_rgb_and_cmyk() {
    let reader = reader();
    for n in [1usize, 3, 4] {
        let space = icc(n as i64);
        let (mut dict, _) = shading(5, space.clone());
        let mut decode = vec![0.0, 10.0, 0.0, 10.0];
        decode.extend_from_slice(&[0.0, 1.0].repeat(n));
        dict.insert("Decode", numbers(&decode));
        assert_eq!(
            MeshDecode::from_dict(&dict, "ICCBased", Some(&space), &reader)
                .unwrap()
                .n_color,
            n
        );
        let mut data = Vec::new();
        for position in [[0, 0], [255, 0], [0, 255], [255, 255]] {
            data.extend_from_slice(&position);
            data.extend_from_slice(&vec![0; n]);
        }
        let (paint, result) = render(&dict, &data, None);
        result.unwrap();
        assert_ne!(paint.get_pixel(2, 7), BACKGROUND);
    }
}
#[test]
fn failed_function_at_a_real_sample_is_an_error_not_silent_missing_paint() {
    let (mut dict, data) = shading(2, name("DeviceGray"));
    dict.insert(
        "Function",
        function(1, b"{ dup 0.25 lt { pop pop } { pop 0.5 } ifelse }"),
    );
    crate::render::page_renderer::validate_shading_dictionary_for_paint(
        &dict,
        "fixture",
        &reader(),
    )
    .unwrap();
    assert!(render(&dict, &data, None).1.is_err());
}
#[test]
fn independent_type4_triangles_restart_and_ignore_the_two_following_flags() {
    let (mut dict, _) = shading(4, name("DeviceGray"));
    dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]));
    let data = [
        0, 0, 0, 0, 2, 102, 0, 0, 1, 0, 102, 0, 0, 153, 153, 255, 1, 255, 153, 255, 2, 255, 255,
        255,
    ];
    let (paint, result) = render(&dict, &data, None);
    result.unwrap();
    assert_eq!(paint.get_pixel(1, 8), [0, 0, 0, 255]);
    assert_eq!(paint.get_pixel(8, 2), [255, 255, 255, 255]);
    assert_eq!(paint.get_pixel(5, 5), BACKGROUND);
}
#[test]
fn truncated_or_unanchored_type4_records_fail_closed() {
    let (dict, _) = shading(4, name("DeviceGray"));
    for data in [&[0, 0, 0, 0][..], &[1, 0, 0, 0][..], &[3, 0, 0, 0][..]] {
        assert!(render(&dict, data, None).1.is_err());
    }
}
#[test]
fn lattice_vertices_skip_each_records_unused_low_bits() {
    let (mut dict, _) = shading(5, name("DeviceGray"));
    dict.insert("BitsPerCoordinate", PdfObject::Integer(1));
    dict.insert("BitsPerComponent", PdfObject::Integer(1));
    dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]));
    let (paint, result) = render(
        &dict,
        &[0b00111111, 0b10111111, 0b01111111, 0b11111111],
        None,
    );
    result.unwrap();
    assert_eq!(paint.get_pixel(2, 7), [255, 255, 255, 255]);
    assert_eq!(paint.get_pixel(7, 2), [255, 255, 255, 255]);
}
#[test]
fn oversized_lattice_rows_cannot_allocate_from_dictionary_metadata_alone() {
    let (mut dict, _) = shading(5, name("DeviceGray"));
    dict.insert("VerticesPerRow", PdfObject::Integer(i64::MAX));
    assert!(render(&dict, &[0, 0, 0], None).1.is_err());
}
#[test]
fn outer_cancellation_is_not_masked_by_a_local_none_token() {
    let (dict, data) = shading(2, name("DeviceGray"));
    let cancel = CancelToken::new();
    cancel.cancel();
    let (paint, result) = cancel.scope(|| render(&dict, &data, None));
    assert!(result.is_err());
    assert_eq!(paint.get_pixel(2, 7), BACKGROUND);
    assert!(render(&dict, &data, None).1.is_ok());
}

#[test]
fn missing_or_truncated_mesh_streams_do_not_report_success() {
    for kind in 4..=7 {
        let (dict, data) = shading(kind, name("DeviceGray"));
        assert!(render(&dict, &[], None).1.is_err());
        assert!(render(&dict, &data[..data.len() - 1], None).1.is_err());
    }
}

fn paint_into(
    dict: &PdfDictionary,
    data: &[u8],
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    options: ShadingRenderOptions<'_>,
    work: &AtomicU64,
) -> Result<(), String> {
    ShadingRenderer::paint_with_options_cancellable(
        dict,
        &Transform2D::identity(),
        viewport,
        buf,
        &reader(),
        if data.is_empty() { None } else { Some(data) },
        options,
        &CancelToken::none(),
        work,
    )
}

fn linear_mesh(kind: i64, space: PdfObject) -> (PdfDictionary, Vec<u8>) {
    let (mut dict, _) = shading(kind, space);
    dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]));
    let data = if kind == 4 {
        vec![0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 0]
    } else {
        vec![0, 0, 0, 255, 0, 255, 0, 255, 0, 255, 255, 255]
    };
    (dict, data)
}

#[test]
fn gouraud_meshes_evaluate_nonlinear_function_after_interpolating_parameter() {
    for kind in [4, 5] {
        let (mut dict, data) = linear_mesh(kind, name("DeviceGray"));
        dict.insert("Function", function(1, b"{ dup mul }"));
        let (paint, result) = render(&dict, &data, None);
        result.unwrap();
        // At device (2.5,7.5), source x=2.5, so t=.25 and f(t)=.0625.
        assert_eq!(paint.get_pixel(2, 7), [16, 16, 16, 255], "type {kind}");
        assert_ne!(paint.get_pixel(2, 7), [64, 64, 64, 255]);
    }
}

#[test]
fn patch_meshes_interpolate_four_source_corners_before_function() {
    for kind in [6, 7] {
        let (mut dict, mut data) = shading(kind, name("DeviceGray"));
        dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]));
        dict.insert("Function", function(1, b"{ dup mul }"));
        let last = data.len() - 4;
        data[last..].copy_from_slice(&[0, 0, 255, 0]);
        let (paint, result) = render(&dict, &data, None);
        result.unwrap();
        for (x, y) in [(2, 7), (4, 4), (7, 2)] {
            let t = (x as f64 + 0.5) * (9.5 - y as f64) / 100.0;
            let expected = (t * t * 255.0).round() as u8;
            let pixel = paint.get_pixel(x, y);
            assert!(
                (pixel[0] as i32 - expected as i32).abs() <= 1,
                "type {kind}, {x},{y}: {pixel:?}"
            );
            assert_eq!([pixel[0], pixel[0], pixel[0], 255], pixel);
        }
    }
}

#[test]
fn gouraud_color_conversion_follows_source_component_interpolation() {
    let mut params = PdfDictionary::empty();
    params.insert("WhitePoint", numbers(&[0.9505, 1.0, 1.089]));
    params.insert("Gamma", PdfObject::Real(2.0));
    let space = PdfObject::Array(vec![name("CalGray"), PdfObject::Dictionary(params)]);
    let expected = components_to_render_color_with_space(
        &[0.25],
        "CalGray",
        Some(&space),
        &reader(),
        ShadingRenderOptions::default(),
    )
    .unwrap()
    .to_pixel_color();
    assert_ne!(
        expected,
        [64, 64, 64, 255],
        "fixture must distinguish source from RGB interpolation"
    );
    for kind in [4, 5] {
        let (dict, data) = linear_mesh(kind, space.clone());
        let (paint, result) = render(&dict, &data, None);
        result.unwrap();
        assert_eq!(paint.get_pixel(2, 7), expected, "type {kind}");
    }
}

#[test]
fn shading_opacity_clip_and_softmask_are_applied_once_for_every_family() {
    use crate::render::buffer::{AlphaMask, ClipMask, WHITE};
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    for kind in 1..=7 {
        let (dict, data) = shading(kind, name("DeviceGray"));
        let mut paint = PixelBuffer::new_filled(10, 10, WHITE);
        paint.set_clip(ClipMask::from_alpha_bytes(10, 10, vec![128; 100]));
        paint.set_smask(AlphaMask::filled(10, 10, 128));
        let mut reference = paint.clone();
        reference.blend_pixel(2, 7, [0, 0, 0, 255], 0.5);
        paint_into(
            &dict,
            &data,
            &viewport,
            &mut paint,
            ShadingRenderOptions::default().with_opacity(0.5),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            paint.get_pixel(2, 7),
            reference.get_pixel(2, 7),
            "type {kind}"
        );
    }
}

#[test]
fn overlapping_mesh_triangles_do_not_accumulate_graphics_state_opacity() {
    let (dict, mut data) = shading(4, name("DeviceGray"));
    data.extend_from_within(..);
    let mut paint = PixelBuffer::new_filled(10, 10, crate::render::buffer::WHITE);
    paint_into(
        &dict,
        &data,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        ShadingRenderOptions::default().with_opacity(0.5),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    assert_eq!(paint.get_pixel(2, 7), [128, 128, 128, 255]);
}

#[test]
fn late_mesh_work_exhaustion_does_not_publish_partially_painted_scratch() {
    let (dict, mut data) = shading(4, name("DeviceGray"));
    data.extend_from_within(..);
    let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
    let before = paint.to_rgba_bytes();
    // Clear/composite=200; first triangle=3 vertices+100 bounds. The
    // second triangle exhausts this shared budget after the first was painted.
    let work = AtomicU64::new(350);
    assert!(paint_into(
        &dict,
        &data,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        ShadingRenderOptions::default(),
        &work
    )
    .is_err());
    assert_eq!(work.load(Ordering::Acquire), 44);
    assert_eq!(paint.rgba_bytes(), before.as_slice());
}

#[test]
fn actual_sample_function_failure_does_not_publish_partial_shading() {
    let (mut dict, data) = linear_mesh(5, name("DeviceGray"));
    dict.insert("Function", function(1, b"{ dup 0.7 gt { pop pop } if }"));
    let (paint, result) = render(&dict, &data, None);
    assert!(result.is_err());
    assert!(paint
        .rgba_bytes()
        .chunks_exact(4)
        .all(|pixel| pixel == BACKGROUND));
}

#[test]
fn shading_scratch_grid_and_rows_obey_local_byte_limit() {
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    for kind in 1..=7 {
        let (dict, data) = shading(kind, name("DeviceGray"));
        let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
        assert!(paint_into(
            &dict,
            &data,
            &viewport,
            &mut paint,
            ShadingRenderOptions::default().with_working_byte_limit(399),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS)
        )
        .is_err());
        assert!(paint
            .rgba_bytes()
            .chunks_exact(4)
            .all(|pixel| pixel == BACKGROUND));
    }
    for kind in [5, 6, 7] {
        let (dict, data) = shading(kind, name("DeviceGray"));
        let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
        // The fixed scratch fits; the two-row/grid allocation must not.
        assert!(paint_into(
            &dict,
            &data,
            &viewport,
            &mut paint,
            ShadingRenderOptions::default().with_working_byte_limit(400 + 4096),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS)
        )
        .is_err());
        assert!(paint
            .rgba_bytes()
            .chunks_exact(4)
            .all(|pixel| pixel == BACKGROUND));
    }
}

#[test]
fn shared_shading_memory_reservation_fails_without_waiting_and_releases_on_error() {
    use crate::decode_scheduler::DecodeMemoryBudget;
    use std::sync::Arc;
    let budget = Arc::new(DecodeMemoryBudget::new(10_000));
    let ancestor = budget.try_acquire(9_000).unwrap();
    let (dict, data) = shading(5, name("DeviceGray"));
    let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    let options = ShadingRenderOptions::default().with_memory_budget(&budget);
    assert!(paint_into(
        &dict,
        &data,
        &viewport,
        &mut paint,
        options,
        &AtomicU64::new(MAX_SHADING_WORK_UNITS)
    )
    .is_err());
    drop(ancestor);
    // Both a post-allocation error and a successful paint release all tokens.
    assert!(paint_into(
        &dict,
        &data,
        &viewport,
        &mut paint,
        options,
        &AtomicU64::new(205)
    )
    .is_err());
    drop(budget.try_acquire(10_000).unwrap());
    paint_into(
        &dict,
        &data,
        &viewport,
        &mut paint,
        options,
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    drop(budget.try_acquire(10_000).unwrap());
    assert!(budget.metrics().peak_reserved_bytes <= 10_000);
}

#[test]
fn cropped_shading_scratch_keeps_rotation_tile_coordinates_and_dither_phase() {
    use crate::render::buffer::ClipMask;
    for kind in [2, 4, 5, 6, 7] {
        let (mut dict, data) = shading(kind, name("DeviceGray"));
        if kind >= 4 {
            dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.371, 0.371]));
        } else {
            dict.insert("Function", function(1, b"{ pop 0.371 }"));
        }
        for rotation in [0, 90, 180, 270] {
            let viewport = Viewport::new_rotated([0.0, 0.0, 10.0, 10.0], 72, rotation);
            let mut full =
                PixelBuffer::new_filled_with_mode(10, 10, BACKGROUND, RenderMode::HighQuality);
            let mut cropped = full.clone();
            let mut rows = vec![Vec::new(); 10];
            for row in &mut rows[2..8] {
                *row = vec![(3, 9)];
            }
            cropped.set_clip(ClipMask::from_visible_runs(10, 10, rows));
            let mut tile =
                PixelBuffer::new_filled_with_mode(6, 6, BACKGROUND, RenderMode::HighQuality);
            let tile_viewport = viewport.pixel_window(3, 2, 6, 6);
            for (vp, buf) in [
                (&viewport, &mut full),
                (&viewport, &mut cropped),
                (&tile_viewport, &mut tile),
            ] {
                paint_into(
                    &dict,
                    &data,
                    vp,
                    buf,
                    ShadingRenderOptions::default(),
                    &AtomicU64::new(MAX_SHADING_WORK_UNITS),
                )
                .unwrap();
            }
            for y in 0..6 {
                for x in 0..6 {
                    assert_eq!(
                        cropped.get_pixel(x + 3, y + 2),
                        full.get_pixel(x + 3, y + 2),
                        "crop {kind}/{rotation}"
                    );
                    assert_eq!(
                        tile.get_pixel(x, y),
                        full.get_pixel(x + 3, y + 2),
                        "tile {kind}/{rotation}"
                    );
                }
            }
            assert_eq!(cropped.get_pixel(0, 0), BACKGROUND);
        }
    }
}

#[test]
fn invalid_opacity_and_nonfinite_mesh_samples_fail_without_paint() {
    let (mut dict, data) = shading(5, name("DeviceGray"));
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    for opacity in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
        assert!(paint_into(
            &dict,
            &data,
            &viewport,
            &mut paint,
            ShadingRenderOptions::default().with_opacity(opacity),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS)
        )
        .is_err());
        assert_eq!(paint.get_pixel(2, 7), BACKGROUND);
    }
    // Finite endpoints with a overflowing subtraction must not silently drop a row.
    dict.insert(
        "Decode",
        numbers(&[0.0, 10.0, 0.0, 10.0, -f64::MAX, f64::MAX]),
    );
    let (paint, result) = render(&dict, &data, None);
    assert!(result.is_err());
    assert_eq!(paint.get_pixel(2, 7), BACKGROUND);
}

#[test]
fn transparent_scratch_allocation_is_byte_bounded_and_overflow_checked() {
    assert!(PixelBuffer::try_new_transparent_with_mode(10, 10, RenderMode::Compat, 399).is_err());
    assert!(PixelBuffer::try_new_transparent_with_mode(
        u32::MAX,
        u32::MAX,
        RenderMode::Compat,
        usize::MAX
    )
    .is_err());
    let buf = PixelBuffer::try_new_transparent_with_mode(10, 10, RenderMode::Compat, 400).unwrap();
    assert_eq!(buf.rgba_bytes(), &[0; 400]);
}

#[test]
fn curved_reused_patch_edges_cover_the_page_without_opacity_seams() {
    let first = [
        (0u8, 0u8),
        (0, 85),
        (0, 170),
        (0, 255),
        (43, 255),
        (85, 255),
        (128, 255),
        (150, 170),
        (106, 85),
        (128, 0),
        (85, 0),
        (43, 0),
        (43, 85),
        (43, 170),
        (85, 170),
        (85, 85),
    ];
    // Flag 2 reuses p7..p10: its direction is reversed relative to the first
    // patch's positive-v edge, and its varying axis becomes the next patch's v.
    let next = [
        (170u8, 0u8),
        (213, 0),
        (255, 0),
        (255, 85),
        (255, 170),
        (255, 255),
        (213, 255),
        (170, 255),
        (205, 190),
        (170, 85),
        (213, 85),
        (213, 170),
    ];
    for kind in [6, 7] {
        let (dict, _) = shading(kind, name("DeviceGray"));
        let count = if kind == 6 { 12 } else { 16 };
        let mut data = vec![0];
        for (x, y) in &first[..count] {
            data.extend_from_slice(&[*x, *y]);
        }
        data.extend_from_slice(&[0; 4]);
        data.push(2);
        for (x, y) in &next[..count - 4] {
            data.extend_from_slice(&[*x, *y]);
        }
        data.extend_from_slice(&[0; 2]);
        let mut paint = PixelBuffer::new_filled(10, 10, crate::render::buffer::WHITE);
        paint_into(
            &dict,
            &data,
            &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            &mut paint,
            ShadingRenderOptions::default().with_opacity(0.5),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS),
        )
        .unwrap();
        assert!(
            paint
                .rgba_bytes()
                .chunks_exact(4)
                .all(|pixel| pixel == [128, 128, 128, 255]),
            "type {kind}"
        );
    }
}

#[test]
fn batch_planning_preserves_source_order_for_overlapping_patches() {
    for kind in [6, 7] {
        let (mut dict, mut data) = shading(kind, name("DeviceGray"));
        dict.insert("Decode", numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]));
        let mut second = data.clone();
        let tail = second.len() - 4;
        second[tail..].fill(255);
        data.extend_from_slice(&second);
        let (paint, result) = render(&dict, &data, None);
        result.unwrap();
        assert_eq!(paint.get_pixel(2, 7), [255, 255, 255, 255], "type {kind}");
    }
}

#[test]
fn background_is_used_for_patterns_only_and_bbox_clips_every_family() {
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    for kind in 1..=7 {
        let (mut dict, data) = shading(kind, name("DeviceGray"));
        match kind {
            1 => {
                dict.insert("Domain", numbers(&[3.0, 7.0, 3.0, 7.0]));
            }
            2 => {
                dict.insert("Coords", numbers(&[3.0, 0.0, 7.0, 0.0]));
            }
            3 => {
                dict.insert("Coords", numbers(&[5.0, 5.0, 0.0, 5.0, 5.0, 2.0]));
            }
            5..=7 => {
                dict.insert("Decode", numbers(&[0.0, 5.0, 0.0, 5.0, 0.0, 0.0]));
            }
            _ => {}
        }
        dict.insert("Background", numbers(&[1.0]));
        dict.insert("BBox", numbers(&[1.0, 1.0, 9.0, 9.0]));
        for pattern in [false, true] {
            let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
            let options = if pattern {
                ShadingRenderOptions::default().for_pattern()
            } else {
                ShadingRenderOptions::default()
            };
            paint_into(
                &dict,
                &data,
                &viewport,
                &mut paint,
                options,
                &AtomicU64::new(MAX_SHADING_WORK_UNITS),
            )
            .unwrap();
            assert_eq!(
                paint.get_pixel(4, 5),
                [0, 0, 0, 255],
                "foreground {kind}/{pattern}"
            );
            assert_eq!(
                paint.get_pixel(8, 1),
                if pattern {
                    [255, 255, 255, 255]
                } else {
                    BACKGROUND
                },
                "outside shading {kind}/{pattern}"
            );
            assert_eq!(
                paint.get_pixel(0, 0),
                BACKGROUND,
                "outside BBox {kind}/{pattern}"
            );
        }
    }
}

#[test]
fn type1_bbox_uses_target_space_not_the_function_matrix() {
    let (mut dict, data) = shading(1, name("DeviceGray"));
    dict.insert("Matrix", numbers(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0]));
    dict.insert("BBox", numbers(&[4.0, 4.0, 2.0, 2.0]));
    let (paint, result) = render(&dict, &data, None);
    result.unwrap();
    assert_eq!(paint.get_pixel(2, 7), [0, 0, 0, 255]);
    assert_eq!(paint.get_pixel(5, 4), BACKGROUND);
}

#[test]
fn fractional_bbox_and_existing_clip_use_intersection_not_double_alpha() {
    use crate::render::buffer::{AlphaMask, ClipMask, WHITE};
    let (mut dict, data) = shading(2, name("DeviceGray"));
    dict.insert("BBox", numbers(&[2.5, 0.0, 8.0, 10.0]));
    let mut paint = PixelBuffer::new_filled(10, 10, WHITE);
    paint.set_clip(ClipMask::from_alpha_bytes(10, 10, vec![128; 100]));
    paint.set_smask(AlphaMask::filled(10, 10, 128));
    let original_clip = paint.clip_mask().unwrap().opacity_byte(2, 5);
    // BBox coverage=.5; existing clip=128/255. Their intersection is .5,
    // then opacity=.5 and soft mask=128/255 apply once each.
    let mut reference = PixelBuffer::new_filled(10, 10, WHITE);
    reference.set_smask(AlphaMask::filled(10, 10, 128));
    reference.blend_pixel(2, 5, [0, 0, 0, 255], 0.25);
    paint_into(
        &dict,
        &data,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        ShadingRenderOptions::default().with_opacity(0.5),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    assert_eq!(paint.get_pixel(2, 5), reference.get_pixel(2, 5));
    assert_eq!(paint.get_pixel(1, 5), WHITE);
    assert_eq!(paint.clip_mask().unwrap().opacity_byte(2, 5), original_clip);
}

#[test]
fn sheared_bbox_preserves_fractional_polygon_coverage_through_paint() {
    use crate::render::buffer::WHITE;
    let (mut dict, data) = shading(2, name("DeviceGray"));
    dict.insert("BBox", numbers(&[0.0, 0.0, 1.0, 1.0]));
    dict.insert(
        "Extend",
        PdfObject::Array(vec![PdfObject::Boolean(true); 2]),
    );
    let mut paint = PixelBuffer::new_filled(10, 10, WHITE);
    ShadingRenderer::paint_with_options_cancellable(
        &dict,
        &Transform2D::from([1.0, 1.0, -1.0, 1.0, 5.0, 3.0]),
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        &reader(),
        Some(&data),
        ShadingRenderOptions::default(),
        &CancelToken::none(),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    for (x, y) in [(4, 5), (5, 5), (4, 6), (5, 6)] {
        assert_eq!(paint.get_pixel(x, y), [128, 128, 128, 255]);
    }
    assert_eq!(paint.get_pixel(3, 5), WHITE);
}

#[test]
fn pattern_background_keeps_original_palette_domain_and_single_opacity() {
    let (target, source) = domains();
    let (mut dict, data) = shading(2, target);
    dict.insert("Coords", numbers(&[3.0, 0.0, 7.0, 0.0]));
    dict.insert("Background", numbers(&[0.0]));
    for original in [None, Some(&source)] {
        let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
        let value = if original.is_some() { 64 } else { 96 };
        let mut reference = paint.clone();
        reference.blend_pixel(1, 5, [value, value, value, 255], 0.5);
        let options = ShadingRenderOptions::default()
            .for_pattern()
            .with_opacity(0.5)
            .with_color_context(
                original,
                ColorTransformOptions {
                    backend: ColorTransformBackend::DeterministicFallback,
                    ..Default::default()
                },
            );
        paint_into(
            &dict,
            &data,
            &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            &mut paint,
            options,
            &AtomicU64::new(MAX_SHADING_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(paint.get_pixel(1, 5), reference.get_pixel(1, 5));
        assert_eq!(
            paint.get_pixel(5, 5),
            reference.get_pixel(1, 5),
            "foreground must not accumulate alpha over background"
        );
    }
}

#[test]
fn clipped_out_function_samples_are_not_evaluated_through_scratch_holes() {
    use crate::render::buffer::ClipMask;
    let (mut dict, data) = shading(2, name("DeviceGray"));
    dict.insert(
        "Function",
        function(1, b"{ dup 0.2 gt exch 0.4 lt and { pop } { 0 } ifelse }"),
    );
    let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
    paint.set_clip(ClipMask::from_visible_runs(
        10,
        10,
        vec![vec![(0, 2), (4, 10)]; 10],
    ));
    paint_into(
        &dict,
        &data,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        ShadingRenderOptions::default(),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    assert_eq!(paint.get_pixel(1, 5), [0, 0, 0, 255]);
    assert_eq!(paint.get_pixel(3, 5), BACKGROUND);
}

#[test]
fn bbox_bounds_reduce_scratch_allocation_and_empty_boxes_do_not_paint() {
    let (mut dict, data) = shading(1, name("DeviceGray"));
    dict.insert("BBox", numbers(&[2.0, 2.0, 3.0, 3.0]));
    let mut paint = PixelBuffer::new_filled(10, 10, BACKGROUND);
    paint_into(
        &dict,
        &data,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        ShadingRenderOptions::default().with_working_byte_limit(4096 + 4),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    )
    .unwrap();
    assert_eq!(paint.get_pixel(2, 7), [0, 0, 0, 255]);
    assert_eq!(paint.get_pixel(3, 7), BACKGROUND);
    dict.insert("BBox", numbers(&[2.0, 2.0, 2.0, 3.0]));
    let (paint, result) = render(&dict, &data, None);
    result.unwrap();
    assert!(paint.rgba_bytes().chunks_exact(4).all(|p| p == BACKGROUND));
}

#[test]
fn common_entry_validation_rejects_bad_bbox_background_and_antialias() {
    for (key, value) in [
        ("BBox", numbers(&[1.0, 2.0, 3.0])),
        ("BBox", numbers(&[0.0, 0.0, f64::INFINITY, 1.0])),
        ("Background", numbers(&[0.0, 1.0])),
        ("Background", numbers(&[])),
        ("AntiAlias", PdfObject::Name("true".into())),
    ] {
        let (mut dict, data) = shading(2, name("DeviceGray"));
        dict.insert(key, value);
        let (paint, result) = render(&dict, &data, None);
        assert!(result.is_err(), "{key}");
        assert!(paint.rgba_bytes().chunks_exact(4).all(|p| p == BACKGROUND));
    }
}

#[test]
fn shading_cache_collisions_do_not_reuse_another_function_parameter() {
    let mut cache = ShadingColorCache::new();
    let (a, b) = (0.000001, 0.000002);
    assert_eq!(ShadingColorCache::bucket(a), ShadingColorCache::bucket(b));
    cache.set(a, RenderColor::black());
    assert_eq!(cache.get(a), Some(RenderColor::black()));
    assert_eq!(cache.get(b), None);
    cache.set(b, RenderColor::white());
    assert_eq!(cache.get(b), Some(RenderColor::white()));
    assert_eq!(cache.get(a), None);
}

#[test]
fn axial_and_radial_discontinuities_survive_bucket_collisions_and_clipped_order() {
    use crate::render::buffer::ClipMask;
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    for kind in [2, 3] {
        let (mut dict, data) = shading(kind, name("DeviceGray"));
        dict.insert(
            "Coords",
            numbers(if kind == 2 {
                &[0.0, 0.0, 1e6, 0.0]
            } else {
                &[0.0, 0.0, 0.0, 0.0, 0.0, 1e6]
            }),
        );
        dict.insert(
            "Function",
            function(1, b"{ 0.000005 lt { 0 } { 1 } ifelse }"),
        );
        let mut full = PixelBuffer::new_filled(10, 10, BACKGROUND);
        let mut clipped = full.clone();
        clipped.set_clip(ClipMask::from_visible_runs(10, 10, vec![vec![(2, 9)]; 10]));
        for paint in [&mut full, &mut clipped] {
            paint_into(
                &dict,
                &data,
                &viewport,
                paint,
                ShadingRenderOptions::default(),
                &AtomicU64::new(MAX_SHADING_WORK_UNITS),
            )
            .unwrap();
        }
        for y in 0..10 {
            for x in 2..9 {
                let (ux, uy) = (x as f64 + 0.5, 9.5 - y as f64);
                let value = if (if kind == 2 { ux } else { ux.hypot(uy) }) < 5.0 {
                    0
                } else {
                    255
                };
                assert_eq!(
                    full.get_pixel(x, y),
                    [value, value, value, 255],
                    "{kind}/{x}/{y}"
                );
                assert_eq!(clipped.get_pixel(x, y), full.get_pixel(x, y));
            }
        }
    }
}

fn analytic_paint_with_transform(
    dict: &PdfDictionary,
    ctm: Transform2D,
) -> (PixelBuffer, Result<(), String>) {
    let mut paint = PixelBuffer::new_filled_with_mode(10, 10, BACKGROUND, RenderMode::Compat);
    let result = ShadingRenderer::paint_with_options_cancellable(
        dict,
        &ctm,
        &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
        &mut paint,
        &reader(),
        None,
        ShadingRenderOptions::default(),
        &CancelToken::none(),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    );
    (paint, result)
}

#[test]
fn analytic_shading_pixels_are_stable_under_coordinate_unit_changes() {
    for kind in [1, 2, 3] {
        let (mut base, _) = shading(kind, name("DeviceGray"));
        if kind != 1 {
            base.insert("Function", function(1, b"{ }"));
        }
        let (reference, result) = analytic_paint_with_transform(&base, Transform2D::identity());
        result.unwrap();
        assert!(reference
            .rgba_bytes()
            .chunks_exact(4)
            .any(|p| p != BACKGROUND));
        for scale in [1e-100, 1e-8, 1e100] {
            let mut dict = base.clone();
            if kind == 1 {
                dict.insert("Matrix", numbers(&[scale, 0.0, 0.0, scale, 0.0, 0.0]));
            } else {
                let coords = get_float_array(&base, "Coords").unwrap();
                dict.insert(
                    "Coords",
                    numbers(&coords.iter().map(|v| v * scale).collect::<Vec<_>>()),
                );
            }
            let (paint, result) =
                analytic_paint_with_transform(&dict, Transform2D::uniform_scale(1.0 / scale));
            result.unwrap();
            for (actual, expected) in paint.rgba_bytes().iter().zip(reference.rgba_bytes()) {
                assert!(
                    (*actual as i32 - *expected as i32).abs() <= 1,
                    "type {kind}, scale {scale}"
                );
            }
        }
    }
}

#[test]
fn analytic_domain_interpolation_and_validation_do_not_overflow() {
    let (mut dict, _) = shading(2, name("DeviceGray"));
    dict.insert("Domain", numbers(&[-f64::MAX, f64::MAX]));
    let mut program = function(1, b"{ 0 lt { 0 } { 1 } ifelse }");
    if let PdfObject::Stream { dict, .. } = &mut program {
        dict.insert("Domain", numbers(&[-f64::MAX, f64::MAX]));
    }
    dict.insert("Function", program);
    let (paint, result) = analytic_paint_with_transform(&dict, Transform2D::identity());
    result.unwrap();
    for x in 0..10 {
        let value = if x < 5 { 0 } else { 255 };
        assert_eq!(paint.get_pixel(x, 5), [value, value, value, 255]);
    }
}

#[test]
fn radial_negative_radii_and_zero_radius_pair_are_not_painted() {
    let (mut dict, _) = shading(3, name("DeviceGray"));
    dict.insert("Coords", numbers(&[0.0, 0.0, -1.0, 10.0, 0.0, 5.0]));
    let (paint, result) = analytic_paint_with_transform(&dict, Transform2D::identity());
    assert!(result.unwrap_err().contains("nonnegative"));
    assert!(paint.rgba_bytes().chunks_exact(4).all(|p| p == BACKGROUND));
    dict.insert("Coords", numbers(&[0.5, 0.5, 0.0, 9.5, 0.5, 0.0]));
    dict.insert(
        "Extend",
        PdfObject::Array(vec![PdfObject::Boolean(true); 2]),
    );
    let (paint, result) = analytic_paint_with_transform(&dict, Transform2D::identity());
    result.unwrap();
    assert!(paint.rgba_bytes().chunks_exact(4).all(|p| p == BACKGROUND));
}

#[test]
fn analytic_coordinate_failures_return_errors_without_compositing_scratch() {
    for kind in [1, 2, 3] {
        let (dict, _) = shading(kind, name("DeviceGray"));
        for ctm in [
            Transform2D::scale(0.0, 0.0),
            Transform2D::scale(f64::from_bits(1), 1.0),
        ] {
            let (paint, result) = analytic_paint_with_transform(&dict, ctm);
            assert!(result.is_err(), "type {kind}");
            assert!(paint.rgba_bytes().chunks_exact(4).all(|p| p == BACKGROUND));
        }
    }
}

#[test]
fn all_shading_families_accept_indirect_parameters_and_array_elements() {
    use crate::render::parameter_dictionary::tests::{reader_with_objects, reference};
    for kind in 1..=7 {
        let (direct, data) = shading(kind, name("DeviceGray"));
        let (expected, result) = render(&direct, &data, None);
        result.unwrap();
        let mut indirect = direct.clone();
        let mut extra = Vec::new();
        for (key, value) in direct.iter() {
            if key == "ColorSpace" {
                continue;
            }
            let value = if let PdfObject::Array(items) = value {
                let mut entries = Vec::new();
                for item in items {
                    let number = 4 + extra.len() as u32;
                    extra.push(item.clone());
                    entries.push(reference(number));
                }
                PdfObject::Array(entries)
            } else {
                value.clone()
            };
            let number = 4 + extra.len() as u32;
            extra.push(value);
            indirect.insert(key, reference(number));
        }
        let reader = reader_with_objects(&extra);
        let mut actual = PixelBuffer::new_filled_with_mode(10, 10, BACKGROUND, RenderMode::Compat);
        ShadingRenderer::paint_with_options_cancellable(
            &indirect,
            &Transform2D::identity(),
            &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            &mut actual,
            &reader,
            if data.is_empty() {
                None
            } else {
                Some(data.as_slice())
            },
            ShadingRenderOptions::default(),
            &CancelToken::none(),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            actual.rgba_bytes(),
            expected.rgba_bytes(),
            "shading type {kind}"
        );
    }
}

#[test]
fn optional_indirect_null_function_keeps_mesh_component_decode() {
    use crate::render::parameter_dictionary::tests::{reader_with_objects, reference};
    let reader = reader_with_objects(&[PdfObject::Null]);
    for kind in 4..=7 {
        let (mut dict, data) = shading(kind, name("DeviceGray"));
        let (expected, result) = render(&dict, &data, None);
        result.unwrap();
        dict.insert("Function", reference(4));
        let mut actual = PixelBuffer::new_filled_with_mode(10, 10, BACKGROUND, RenderMode::Compat);
        ShadingRenderer::paint_with_options_cancellable(
            &dict,
            &Transform2D::identity(),
            &Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            &mut actual,
            &reader,
            Some(data.as_slice()),
            ShadingRenderOptions::default(),
            &CancelToken::none(),
            &AtomicU64::new(MAX_SHADING_WORK_UNITS),
        )
        .unwrap();
        assert_eq!(
            actual.rgba_bytes(),
            expected.rgba_bytes(),
            "shading type {kind}"
        );
    }
}

#[test]
fn exponential_shading_uses_nonunit_function_domain_in_the_painted_result() {
    let (mut dict, _) = shading(2, name("DeviceGray"));
    dict.insert("Domain", numbers(&[2.0, 4.0]));
    let mut function = PdfDictionary::empty();
    function.insert("FunctionType", PdfObject::Integer(2));
    function.insert("Domain", numbers(&[2.0, 4.0]));
    function.insert("C0", numbers(&[0.0]));
    function.insert("C1", numbers(&[0.25]));
    function.insert("N", PdfObject::Integer(1));
    dict.insert("Function", PdfObject::Dictionary(function));
    let (paint, result) = render(&dict, &[], None);
    result.unwrap();
    for x in 0..10 {
        let expected = ((2.0 + (x as f64 + 0.5) * 0.2) * 0.25 * 255.0).round() as i32;
        let pixel = paint.get_pixel(x, 5);
        assert!((pixel[0] as i32 - expected).abs() <= 1);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 255);
    }
}
