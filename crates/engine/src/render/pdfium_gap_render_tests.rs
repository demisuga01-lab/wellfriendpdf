// PDFium pixel-suite gaps (R10): pattern-stroked text, Type 3 glyphs painted
// with a pattern, zero-area fills, Link borders, inline images without EI,
// truncated image data, out-of-range palette indices and soft-mask strokes.
mod pdfium_gap_rendering {
    use super::*;

    fn stream(dict: &str, content: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< {dict} /Length {} >>\nstream\n", content.len()).into_bytes();
        bytes.extend_from_slice(content);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }

    /// A 100x100 page whose resources are `resources` (objects from 5 on).
    fn render(resources: &str, content: &[u8], extra_objects: Vec<Vec<u8>>) -> PixelBuffer {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << {resources} >> >>"
            )
            .into_bytes(),
            stream("", content),
        ];
        objects.extend(extra_objects);
        let engine = ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap();
        engine.render_page(1, 72).unwrap()
    }

    fn green_shading_pattern() -> Vec<u8> {
        b"<< /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 1 0] /C1 [0 1 0] /N 1 >> /Extend [true true] >> >>".to_vec()
    }

    fn count(buf: &PixelBuffer, wanted: impl Fn(PixelColor) -> bool) -> usize {
        (0..buf.height as i32)
            .flat_map(|y| (0..buf.width as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| wanted(buf.get_pixel(x, y)))
            .count()
    }

    #[test]
    fn text_stroked_with_a_shading_pattern_paints_the_pattern() {
        let buf = render(
            "/Font << /F1 6 0 R >> /Pattern << /P1 5 0 R >>",
            b"BT /Pattern CS /P1 SCN /F1 40 Tf 1 Tr 2 w 10 30 Td (H) Tj ET",
            vec![
                green_shading_pattern(),
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            ],
        );
        let green = count(&buf, |p| p[1] > 200 && p[0] < 100 && p[2] < 100);
        assert!(
            green > 20,
            "pattern-stroked glyph outline is painted: {green}"
        );
    }

    #[test]
    fn type3_image_mask_glyphs_take_the_fill_pattern() {
        let glyph =
            b"10 0 0 0 10 10 d1 q 10 0 0 10 0 0 cm BI /IM true /W 1 /H 1 /BPC 1 ID \x00 EI Q";
        let buf = render(
            "/Font << /F1 6 0 R >> /Pattern << /P1 5 0 R >>",
            b"BT /Pattern cs /P1 scn /F1 4 Tf 30 30 Td (a) Tj ET",
            vec![
                green_shading_pattern(),
                b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [1 0 0 1 0 0] /CharProcs << /a 7 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [10] >>".to_vec(),
                stream("", glyph),
            ],
        );
        let green = count(&buf, |p| p[1] > 200 && p[0] < 100 && p[2] < 100);
        assert!(
            green >= 1000,
            "the 40x40 glyph is painted with the pattern: {green}"
        );
    }

    #[test]
    fn two_point_fills_draw_a_one_pixel_line() {
        let buf = render("", b"0 0 1 rg 10 50.5 m 90 50.5 l f", Vec::new());
        // A full-alpha cosmetic line along device row 49.
        let blue = count(&buf, |p| p[2] > 200 && p[0] < 128);
        assert!((70..=180).contains(&blue), "{blue}");
    }

    #[test]
    fn link_borders_are_stroked_unless_underlined() {
        let link = |extra: &str| {
            let objects = vec![
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Annots [4 0 R] >>".to_vec(),
                format!("<< /Type /Annot /Subtype /Link /Rect [10 10 90 30] /C [0 1 1] {extra} >>")
                    .into_bytes(),
            ];
            let engine = ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap();
            engine.render_page(1, 72).unwrap()
        };
        let cyan = |p: PixelColor| p[0] < 50 && p[1] > 200 && p[2] > 200;
        assert!(count(&link("/Border [0 0 1]"), cyan) > 150);
        assert_eq!(count(&link("/BS << /S /U /W 1 >>"), cyan), 0);
        assert_eq!(count(&link(""), cyan), 0);
    }

    #[test]
    fn inline_image_without_ei_is_not_drawn() {
        let buf = render(
            "",
            b"q 100 0 0 100 0 0 cm BI /W 1 /H 1 /BPC 8 /CS /G ID \x00",
            Vec::new(),
        );
        assert_eq!(count(&buf, |p| p[0] < 128), 0);
    }

    #[test]
    fn rows_missing_from_image_data_are_black() {
        // 1x2 RGB image with only its first (white) row present.
        let buf = render(
            "/XObject << /Im 5 0 R >>",
            b"q 100 0 0 100 0 0 cm /Im Do Q",
            vec![stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                &[255, 255, 255],
            )],
        );
        assert_eq!(buf.get_pixel(50, 20), [255, 255, 255, 255]);
        assert_eq!(buf.get_pixel(50, 80), [0, 0, 0, 255]);
    }

    #[test]
    fn palette_indices_above_hival_are_black() {
        let buf = render(
            "/XObject << /Im 5 0 R >>",
            b"q 100 0 0 100 0 0 cm /Im Do Q",
            vec![stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 2 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceGray 0 <CC>]",
                &[0, 1],
            )],
        );
        assert_eq!(buf.get_pixel(50, 20), [0xCC, 0xCC, 0xCC, 255]);
        assert_eq!(buf.get_pixel(50, 80), [0, 0, 0, 255]);
    }

    #[test]
    fn soft_mask_group_strokes_are_not_stroke_adjusted() {
        // pdfium bug_1995: the mask's hole edge falls on a half pixel and must
        // stay anti-aliased instead of snapping to a whole pixel.
        let objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /ExtGState << /G << /SMask << /Type /Mask /S /Alpha /G 5 0 R >> >> >> >> >>".to_vec(),
            stream("", b"q 1 0 0 1 5 5 cm 1 0 0 rg 0 0 30 30 re f /G gs 0 0 1 rg 0 0 30 30 re f Q"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 30 30] /Group << /Type /Group /S /Transparency /I true >>",
                b"q 0 0 0 RG 15 w 0 0 30 30 re S Q",
            ),
        ];
        let engine = ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap();
        let buf = engine.render_page(1, 72).unwrap();
        let edge = buf.get_pixel(12, 20);
        assert!(
            (100..=160).contains(&edge[0]) && (100..=160).contains(&edge[2]),
            "half-covered mask edge blends red and blue: {edge:?}"
        );
    }

    #[test]
    fn malformed_decode_and_short_data_draw_like_pdfium() {
        // pdfium bug_554151: a one-entry /Decode reads its missing entries as
        // 0 and the rows past the data are black.
        let buf = render(
            "/XObject << /Im 5 0 R >>",
            b"q 100 0 0 100 0 0 cm /Im Do Q",
            vec![stream(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 4 /ColorSpace /DeviceRGB /Decode [1.0]",
                &[0xFF; 6],
            )],
        );
        assert_eq!(count(&buf, |p| p[0] > 64 || p[1] > 64 || p[2] > 64), 0);
    }

    #[test]
    fn scn_naming_a_pattern_in_a_device_space_paints_that_pattern() {
        // pdfium bug_1099446: `/P1 SCN` without `/Pattern CS` selects P1, a
        // tiling pattern without /BBox, so nothing is drawn.
        let buf = render(
            "/Pattern << /P1 5 0 R >>",
            b"4 w 0 0 m 100 100 l /P1 SCN S",
            vec![stream("/PatternType 1 /PaintType 1", b"0 0 10 10 re f")],
        );
        assert_eq!(count(&buf, |p| p[0] < 128), 0);
    }
}
