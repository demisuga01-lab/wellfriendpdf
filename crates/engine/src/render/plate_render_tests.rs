//! End-to-end native CMYK / DeviceN plate renders of small synthetic pages.

use crate::cancel::CancelToken;
use crate::render::{OverprintPolicy, RenderDocumentCache, RenderMode};
use crate::ContentEngine;

fn pdf(content: &str, resources: &str, extra: &[&[u8]]) -> Vec<u8> {
    let content = content.as_bytes();
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << {resources} >> >>"
        )
        .into_bytes(),
    ];
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    objects.push(stream);
    objects.extend(extra.iter().map(|object| object.to_vec()));
    let mut data = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        data.extend_from_slice(body);
        data.extend_from_slice(b"\nendobj\n");
    }
    let xref = data.len();
    data.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        data.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    data
}

fn render_cmyk(bytes: Vec<u8>, overprint: OverprintPolicy) -> crate::render::CmykRaster {
    let engine = ContentEngine::open_bytes(bytes).expect("open synthetic PDF");
    let mut contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .expect("contract");
    contract.overprint = overprint;
    engine
        .render_page_cmyk_with_contract(&contract, &CancelToken::none())
        .expect("cmyk render")
}

/// CMYK of the device pixel at PDF point (x, y) of a 100 pt page at 72 dpi.
fn at(raster: &crate::render::CmykRaster, x: u32, y: u32) -> [u8; 4] {
    let row = raster.height - 1 - y;
    let offset = (row * raster.width + x) as usize * 4;
    raster.pixels[offset..offset + 4].try_into().unwrap()
}

const SPOT: &[u8] = b"[/Separation /PANTONE#20123#20C /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.24 0.94 0] /N 1 >>]";
const OVERPRINT_GS: &[u8] = b"<< /Type /ExtGState /OP true /op true /OPM 1 >>";

#[test]
fn device_cmyk_paint_is_exact_and_rgb_gray_use_undercolour_removal() {
    let raster = render_cmyk(
        pdf(
            "0.2 0.4 0 0.1 k 0 0 50 50 re f 1 0 0 rg 50 0 50 50 re f 0.5 g 0 50 50 50 re f",
            "",
            &[],
        ),
        OverprintPolicy::Preview,
    );
    assert!(raster.native, "{:?}", raster.diagnostics);
    assert_eq!((raster.width, raster.height), (100, 100));
    assert_eq!(at(&raster, 25, 25), [51, 102, 0, 26]);
    assert_eq!(at(&raster, 75, 25), [0, 255, 255, 0]);
    // Retained device-gray fills carry an 8-bit RGB colour: 0.5 is 128,
    // i.e. 127 or 128 units of black ink.
    let [c, m, y, k] = at(&raster, 25, 75);
    assert!([c, m, y] == [0, 0, 0] && (127..=128).contains(&k), "{k}");
    // Unpainted paper is zero ink.
    assert_eq!(at(&raster, 75, 75), [0, 0, 0, 0]);
}

#[test]
fn overprint_mode_one_and_spot_alternates_overprint_per_plate() {
    let resources = "/ExtGState << /GSop 6 0 R >> /ColorSpace << /CS0 5 0 R >>";
    let content = "1 0 0 0 k 0 0 100 100 re f \
                   q /GSop gs 0 1 0 0 k 0 0 50 50 re f /CS0 cs 1 scn 50 0 50 50 re f Q \
                   /CS0 cs 0.5 scn 0 50 50 50 re f";
    let bytes = pdf(content, resources, &[SPOT, OVERPRINT_GS]);
    let raster = render_cmyk(bytes.clone(), OverprintPolicy::Preview);
    assert!(raster.native, "{:?}", raster.diagnostics);
    // OPM 1: the zero cyan component leaves the cyan plate untouched.
    assert_eq!(at(&raster, 25, 25), [255, 255, 0, 0]);
    // The spot names the plates its alternate inks at full tint (M, Y).
    assert_eq!(at(&raster, 75, 25), [255, 61, 240, 0]);
    // Without overprint the spot knocks the cyan out.
    assert_eq!(at(&raster, 25, 75), [0, 31, 120, 0]);
    // Overprint disabled by policy: knockout everywhere.
    let knockout = render_cmyk(bytes, OverprintPolicy::Disabled);
    assert_eq!(at(&knockout, 25, 25), [0, 255, 0, 0]);
    assert_eq!(at(&knockout, 75, 25), [0, 61, 240, 0]);
}

#[test]
fn constant_alpha_and_multiply_composite_per_plate() {
    let resources = "/ExtGState << /A 5 0 R /M 6 0 R >>";
    let content = "0 0 1 0 k 0 0 100 100 re f q /A gs 1 0 0 0 k 0 0 50 100 re f Q \
                   q /M gs 0 1 0 0 k 50 0 50 100 re f Q";
    let raster = render_cmyk(
        pdf(
            content,
            resources,
            &[
                b"<< /Type /ExtGState /ca 0.5 >>",
                b"<< /Type /ExtGState /BM /Multiply >>",
            ],
        ),
        OverprintPolicy::Preview,
    );
    assert!(raster.native, "{:?}", raster.diagnostics);
    // Normal blend at 50 %: cyan half-inked, yellow half-knocked-out.
    let [c, m, y, k] = at(&raster, 25, 50);
    assert!((127..=129).contains(&c) && m == 0 && (127..=129).contains(&y) && k == 0);
    // Multiply on complements (PDF subtractive rule) keeps the yellow.
    assert_eq!(at(&raster, 75, 50), [0, 255, 255, 0]);
}

#[test]
fn device_cmyk_images_and_shadings_keep_components() {
    let resources = "/XObject << /Im0 5 0 R >> /Shading << /Sh0 6 0 R >>";
    let content = "q 50 0 0 50 0 0 cm /Im0 Do Q q 50 0 50 100 re W n /Sh0 sh Q";
    let mut image = b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceCMYK /Length 8 >>\nstream\n".to_vec();
    image.extend_from_slice(&[10, 20, 30, 40, 200, 0, 100, 0]);
    image.extend_from_slice(b"\nendstream");
    let shading: &[u8] = b"<< /ShadingType 2 /ColorSpace /DeviceCMYK /Coords [50 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0.4 0 0 1] /C1 [0.4 0 0 1] /N 1 >> >>";
    let raster = render_cmyk(
        pdf(content, resources, &[&image, shading]),
        OverprintPolicy::Preview,
    );
    assert!(raster.native, "{:?}", raster.diagnostics);
    assert_eq!(at(&raster, 5, 25), [10, 20, 30, 40]);
    assert_eq!(at(&raster, 45, 25), [200, 0, 100, 0]);
    assert_eq!(at(&raster, 75, 50), [102, 0, 0, 255]);
}

fn assert_near(actual: [u8; 4], expected: [u8; 4]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (i16::from(*a) - i16::from(e)).abs() <= 1),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn non_separable_blends_composite_natively_in_cmyk() {
    // Backdrop C/M/Y/K = 0.2/0.4/0.6/0.3. The modes act on the complements
    // of C, M and Y (0.8, 0.6, 0.4); K comes from the backdrop for Hue,
    // Saturation and Color and from the source for Luminosity.
    let resources = "/ExtGState << /H 5 0 R /L 6 0 R >>";
    let content = "0.2 0.4 0.6 0.3 k 0 0 100 100 re f \
                   q /H gs 1 0 0 0 k 0 0 50 50 re f Q \
                   q /L gs 0 0 0 0.5 k 50 0 50 50 re f Q";
    let raster = render_cmyk(
        pdf(
            content,
            resources,
            &[
                b"<< /Type /ExtGState /BM /Hue >>",
                b"<< /Type /ExtGState /BM /Luminosity >>",
            ],
        ),
        OverprintPolicy::Preview,
    );
    assert!(raster.native, "{:?}", raster.diagnostics);
    // Hue of pure cyan, saturation and luminosity of the backdrop:
    // SetLum(SetSat((0, 1, 1), 0.4), 0.638) = (0.358, 0.758, 0.758).
    assert_near(at(&raster, 25, 25), [164, 62, 62, 77]);
    // Luminosity of paper-white C/M/Y lifts the backdrop to white; K from
    // the source.
    assert_near(at(&raster, 75, 25), [0, 0, 0, 128]);
    // Untouched backdrop.
    assert_eq!(at(&raster, 25, 75), [51, 102, 153, 77]);
}

#[test]
fn non_separable_blend_with_spot_plates_renders_process_plates_together() {
    let resources = "/ColorSpace << /CS0 5 0 R >> /ExtGState << /H 6 0 R >>";
    let content = "0 0 0 1 k 0 0 50 100 re f /CS0 cs 0.5 scn 50 0 50 100 re f \
                   /H gs 1 0 0 0 k 0 0 100 50 re f";
    let engine = ContentEngine::open_bytes(pdf(
        content,
        resources,
        &[SPOT, b"<< /Type /ExtGState /BM /Hue >>"],
    ))
    .unwrap();
    let contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .unwrap();
    let mut cache = RenderDocumentCache::new();
    let raster = engine
        .render_page_separations_with_contract_and_cache(
            &contract,
            &CancelToken::none(),
            8,
            &mut cache,
        )
        .unwrap();
    assert!(raster.native, "{:?}", raster.diagnostics);
    assert_eq!(raster.plates.len(), 5);
    // Device row 75 is PDF y = 24 (the Hue paint); row 25 is above it.
    let pixel =
        |plate: usize, x: u32, row: u32| raster.plates[plate].pixels[(row * 100 + x) as usize];
    // A grey backdrop has no hue to take: C/M/Y stay at the backdrop's.
    assert_eq!(pixel(0, 25, 75), 0);
    assert_eq!(pixel(0, 75, 75), 0);
    // Black and the spot keep the backdrop.
    assert_eq!(pixel(3, 25, 75), 255);
    assert_eq!(pixel(4, 75, 75), 128);
    assert_eq!(pixel(4, 75, 25), 128);
}

#[test]
fn non_separable_blend_with_per_plate_overprint_renders_natively() {
    // OPM 1 overprint names only magenta (C/M/Y can then not share a pass
    // unmasked) and a Hue paint needs C, M and Y together: the C/M/Y pass
    // masks the overprint per plate.
    let resources = "/ExtGState << /OP 5 0 R /H 6 0 R >>";
    let content = "0.2 0.4 0.6 0.3 k 0 0 100 100 re f \
                   q /OP gs 0 1 0 0 k 0 0 50 100 re f Q \
                   q /H gs 1 0 0 0 k 0 0 100 50 re f Q";
    let bytes = pdf(
        content,
        resources,
        &[OVERPRINT_GS, b"<< /Type /ExtGState /BM /Hue >>"],
    );
    let raster = render_cmyk(bytes.clone(), OverprintPolicy::Preview);
    assert!(raster.native, "{:?}", raster.diagnostics);
    // Overprint alone: magenta replaced, C, Y and K kept.
    assert_eq!(at(&raster, 25, 75), [51, 255, 153, 77]);
    assert_eq!(at(&raster, 75, 75), [51, 102, 153, 77]);
    // Hue of pure cyan over the overprinted backdrop, complements
    // (0.8, 0, 0.4): SetLum(SetSat((0, 1, 1), 0.8), 0.284) = (0, 0.406,
    // 0.406); K from the backdrop.
    assert_near(at(&raster, 25, 25), [255, 151, 151, 77]);
    // Hue over the plain backdrop, as without overprint.
    assert_near(at(&raster, 75, 25), [164, 62, 62, 77]);
    // Overprint disabled: the magenta paint knocks C, Y and K out.
    let knockout = render_cmyk(bytes, OverprintPolicy::Disabled);
    assert!(knockout.native, "{:?}", knockout.diagnostics);
    assert_eq!(at(&knockout, 25, 75), [0, 255, 0, 0]);
}

#[test]
fn transfer_functions_apply_per_plate() {
    // A single curve y = x^2 on the additive value (255 - ink), and a
    // four-curve array that only changes the black plate.
    let resources = "/ExtGState << /T 5 0 R /T4 6 0 R >>";
    let content = "q /T gs 0 0 0 0.5 k 0 0 50 100 re f Q \
                   q /T4 gs 0.5 0 0 0.5 k 50 0 50 100 re f Q";
    let raster = render_cmyk(
        pdf(
            content,
            resources,
            &[
                b"<< /Type /ExtGState /TR << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 2 >> >>",
                b"<< /Type /ExtGState /TR [/Identity /Identity /Identity << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 2 >>] >>",
            ],
        ),
        OverprintPolicy::Preview,
    );
    assert!(raster.native, "{:?}", raster.diagnostics);
    // K = 128 ink is 127 additive: (127/255)^2 * 255 = 63, i.e. 192 ink.
    assert_near(at(&raster, 25, 50), [0, 0, 0, 192]);
    assert_near(at(&raster, 75, 50), [128, 0, 0, 192]);
}

#[test]
fn separations_render_process_and_spot_plates() {
    let resources = "/ColorSpace << /CS0 5 0 R >>";
    let content = "0 0 0 1 k 0 0 50 100 re f /CS0 cs 0.5 scn 50 0 50 100 re f";
    let engine = ContentEngine::open_bytes(pdf(content, resources, &[SPOT])).unwrap();
    let contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .unwrap();
    let mut cache = RenderDocumentCache::new();
    let raster = engine
        .render_page_separations_with_contract_and_cache(
            &contract,
            &CancelToken::none(),
            8,
            &mut cache,
        )
        .unwrap();
    assert!(raster.native, "{:?}", raster.diagnostics);
    let names: Vec<&str> = raster
        .plates
        .iter()
        .map(|plate| plate.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Cyan", "Magenta", "Yellow", "Black", "PANTONE 123 C"]
    );
    let pixel = |plate: usize, x: u32| raster.plates[plate].pixels[(50 * 100 + x) as usize];
    assert_eq!(pixel(3, 25), 255);
    assert_eq!(pixel(4, 25), 0);
    // The separated spot paints only its own plate.
    assert_eq!(pixel(4, 75), 128);
    assert_eq!((pixel(1, 75), pixel(2, 75), pixel(3, 75)), (0, 0, 0));
    // With no spot plates the spot maps to CMYK through its alternate.
    let cmyk = engine
        .render_page_cmyk_with_contract(&contract, &CancelToken::none())
        .unwrap();
    assert_eq!(at(&cmyk, 75, 50), [0, 31, 120, 0]);
}

#[test]
fn rgb_render_is_unchanged_by_plate_support() {
    let bytes = pdf("0.2 0.4 0 0.1 k 0 0 50 50 re f", "", &[]);
    let engine = ContentEngine::open_bytes(bytes).unwrap();
    let contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .unwrap();
    let rgb = engine
        .render_page_with_contract(&contract, &CancelToken::none())
        .unwrap();
    let expected = crate::render::cmm::device_cmyk_to_srgb(0.2, 0.4, 0.0, 0.1)
        .map(|value| (value * 255.0).round() as u8);
    let pixel = rgb.get_pixel(25, 75);
    for channel in 0..3 {
        assert!((i16::from(pixel[channel]) - i16::from(expected[channel])).abs() <= 1);
    }
}
