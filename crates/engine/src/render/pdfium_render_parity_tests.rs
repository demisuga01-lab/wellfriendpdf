// PDFium rendering parity (WP7): forced colour schemes, optional-content
// print usage, windowed contract rendering, Type 3 bitmap-glyph baseline
// snapping, image overprint imitation, thumbnails and rendered image objects.
mod pdfium_render_parity {
    use super::*;
    use crate::render::contract::{ContractColor, DeviceMatrix, ForcedColorScheme};

    fn stream(dict: &str, content: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< {dict} /Length {} >>\nstream\n", content.len()).into_bytes();
        bytes.extend_from_slice(content);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }

    fn one_page_pdf(page_extra: &str, content: &[u8], extra_objects: Vec<Vec<u8>>) -> Vec<u8> {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R {page_extra} >>"
            )
            .into_bytes(),
            stream("", content),
        ];
        objects.extend(extra_objects);
        build_test_pdf_from_objects(&objects)
    }

    fn contract_for(engine: &ContentEngine) -> RenderContract {
        engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default contract")
    }

    fn render(engine: &ContentEngine, contract: &RenderContract) -> PixelBuffer {
        engine
            .render_page_with_contract(contract, &CancelToken::none())
            .expect("contract render")
    }

    fn forced_scheme(convert_fill_to_stroke: bool) -> ForcedColorScheme {
        ForcedColorScheme {
            path_fill: ContractColor::rgba(0, 255, 0, 255),
            path_stroke: ContractColor::rgba(255, 0, 255, 255),
            text_fill: ContractColor::rgba(0, 0, 255, 255),
            text_stroke: ContractColor::rgba(255, 255, 0, 255),
            convert_fill_to_stroke,
        }
    }

    /// Red filled square (path), a gray image, and a Type 3 text glyph
    /// filled red.
    fn forced_colour_fixture() -> ContentEngine {
        let content = b"1 0 0 rg 10 10 30 30 re f\n\
            q 30 0 0 30 60 10 cm /Im Do Q\n\
            BT /T3 20 Tf 10 60 Td (A) Tj ET\n";
        ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /XObject << /Im 5 0 R >> /Font << /T3 6 0 R >> >>",
            content,
            vec![
                stream(
                    "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
                    &[128],
                ),
                b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [.001 0 0 .001 0 0] /CharProcs << /A 7 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1000] >>".to_vec(),
                stream("", b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"),
            ],
        ))
        .expect("open forced colour fixture")
    }

    #[test]
    fn forced_colour_scheme_recolours_paths_and_text_but_not_images() {
        let engine = forced_colour_fixture();
        let plain = render(&engine, &contract_for(&engine));
        // Device y = 100 - user y.
        assert_eq!(plain.get_pixel(25, 75), [255, 0, 0, 255], "red path");
        assert_eq!(plain.get_pixel(75, 75), [128, 128, 128, 255], "gray image");
        assert_eq!(plain.get_pixel(20, 30), [0, 0, 0, 255], "black text");

        let mut forced = contract_for(&engine);
        forced.forced_colors = Some(forced_scheme(false));
        let out = render(&engine, &forced);
        assert_eq!(out.get_pixel(25, 75), [0, 255, 0, 255], "path fill forced");
        assert_eq!(
            out.get_pixel(75, 75),
            [128, 128, 128, 255],
            "images keep their colours"
        );
        assert_eq!(out.get_pixel(20, 30), [0, 0, 255, 255], "text fill forced");
    }

    #[test]
    fn forced_colour_fill_to_stroke_outlines_filled_paths() {
        let engine = forced_colour_fixture();
        let mut forced = contract_for(&engine);
        forced.forced_colors = Some(forced_scheme(true));
        let out = render(&engine, &forced);
        // The square's interior stays background; its edge (user x = 10)
        // carries the forced path-stroke colour.
        assert_eq!(out.get_pixel(25, 75), [255, 255, 255, 255]);
        let edge = out.get_pixel(10, 75);
        assert!(
            edge[0] > 128 && edge[1] < 128 && edge[2] > 128,
            "fill converted to a magenta stroke: {edge:?}"
        );
        // Text is not converted.
        assert_eq!(out.get_pixel(20, 30), [0, 0, 255, 255]);
    }

    #[test]
    fn forced_colour_scheme_is_part_of_cache_identity() {
        let engine = forced_colour_fixture();
        let mut cache = RenderDocumentCache::new();
        let plain_contract = contract_for(&engine);
        let mut forced_contract = plain_contract.clone();
        forced_contract.forced_colors = Some(forced_scheme(false));
        let cancel = CancelToken::none();
        let plain = engine
            .render_page_with_contract_and_cache(&plain_contract, &cancel, &mut cache)
            .unwrap();
        let forced = engine
            .render_page_with_contract_and_cache(&forced_contract, &cancel, &mut cache)
            .unwrap();
        let plain_again = engine
            .render_page_with_contract_and_cache(&plain_contract, &cancel, &mut cache)
            .unwrap();
        assert_ne!(plain.get_pixel(25, 75), forced.get_pixel(25, 75));
        assert_ne!(plain.get_pixel(20, 30), forced.get_pixel(20, 30));
        assert_same_pixels(&plain, &plain_again);

        // The retained progressive tile path keys its raster cache by the
        // contract fingerprint, so forced and plain tiles never alias.
        let tile_plain =
            PageRenderer::render_page_display_list_tile_cancellable_with_contract_and_cache(
                &engine,
                &plain_contract,
                &cancel,
                &mut cache,
            )
            .unwrap();
        let tile_forced =
            PageRenderer::render_page_display_list_tile_cancellable_with_contract_and_cache(
                &engine,
                &forced_contract,
                &cancel,
                &mut cache,
            )
            .unwrap();
        assert_eq!(tile_plain.get_pixel(25, 75), [255, 0, 0, 255]);
        assert_eq!(tile_forced.get_pixel(25, 75), [0, 255, 0, 255]);
        assert_eq!(tile_forced.get_pixel(20, 30), [0, 0, 255, 255]);
    }

    #[test]
    fn forced_colour_scheme_keeps_pattern_paints() {
        let content = b"/Pattern cs /P1 scn 10 10 80 80 re f\n";
        let engine = ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /Pattern << /P1 5 0 R >> >>",
            content,
            vec![stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources <<>>",
                b"1 0 0 rg 0 0 10 10 re f",
            )],
        ))
        .unwrap();
        let mut forced = contract_for(&engine);
        forced.forced_colors = Some(forced_scheme(true));
        let out = render(&engine, &forced);
        assert_eq!(
            out.get_pixel(50, 50),
            [255, 0, 0, 255],
            "pattern cells keep their real colours and are not converted to strokes"
        );
    }

    fn print_usage_fixture(page_content: &[u8], ocproperties: &str) -> ContentEngine {
        let objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R /OCProperties {ocproperties} >>").into_bytes(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /L1 5 0 R >> >> >>".to_vec(),
            stream("", page_content),
            b"<< /Type /OCG /Name (Layer) /Usage << /View << /ViewState /ON >> /Print << /PrintState /OFF >> >> >>".to_vec(),
        ];
        ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap()
    }

    #[test]
    fn print_profile_uses_optional_content_print_usage() {
        let content = b"/OC /L1 BDC 1 0 0 rg 0 0 100 100 re f EMC\n";
        let engine = print_usage_fixture(content, "<< /OCGs [5 0 R] /D << /BaseState /ON >> >>");
        let view = render(&engine, &contract_for(&engine));
        assert_eq!(
            view.get_pixel(50, 50),
            [255, 0, 0, 255],
            "visible on screen"
        );
        let mut print = contract_for(&engine);
        print.print_profile = PrintProfile::Print;
        let printed = render(&engine, &print);
        assert_eq!(
            printed.get_pixel(50, 50),
            [255, 255, 255, 255],
            "/Usage /Print /PrintState /OFF hides the layer when printing"
        );

        let view_context = OptionalContentContext::from_document(engine.document());
        let print_context = OptionalContentContext::from_document_with_usage(
            engine.document(),
            crate::optional_content::OptionalContentUsage::Print,
        );
        assert_ne!(
            view_context.visibility_fingerprint(),
            print_context.visibility_fingerprint()
        );
    }

    #[test]
    fn print_profile_applies_auto_state_print_events() {
        // The layer is OFF in the default configuration but carries an /AS
        // print event whose /Print category turns it ON for printing.
        let content = b"/OC /L1 BDC 0 0 1 rg 0 0 100 100 re f EMC\n";
        let objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /OFF [5 0 R] /AS [<< /Event /Print /OCGs [5 0 R] /Category [/Print] >> << /Event /View /OCGs [5 0 R] /Category [/View] >>] >> >> >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Properties << /L1 5 0 R >> >> >>".to_vec(),
            stream("", content),
            b"<< /Type /OCG /Name (Watermark) /Usage << /Print << /PrintState /ON >> >> >>".to_vec(),
        ];
        let engine = ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap();
        let view = render(&engine, &contract_for(&engine));
        assert_eq!(
            view.get_pixel(50, 50),
            [255, 255, 255, 255],
            "hidden on screen"
        );
        let mut print = contract_for(&engine);
        print.print_profile = PrintProfile::Print;
        assert_eq!(
            render(&engine, &print).get_pixel(50, 50),
            [0, 0, 255, 255],
            "/AS /Event /Print turns the layer on for printing"
        );
    }

    /// A page exercising anti-aliased paths, strokes, an image, a shading,
    /// a soft-masked group, text and an annotation.
    fn windowed_render_fixture() -> ContentEngine {
        let content = b"q 1 0 0 rg 5 5 m 95 20 l 50 95 l f Q\n\
            q 0 0 1 RG 3 w 10 90 m 90 10 l S Q\n\
            q 40 0 0 30 55 55 cm /Im Do Q\n\
            q 0 0 100 100 re W n /Sh sh Q\n\
            q /GS1 gs 0 1 0 rg 20 20 60 60 re f Q\n\
            BT /F1 18 Tf 12 40 Td (Window) Tj ET\n\
            q /Fm Do Q\n";
        let objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Annots [10 0 R] /Resources << /XObject << /Im 5 0 R /Fm 9 0 R >> /Shading << /Sh 6 0 R >> /ExtGState << /GS1 7 0 R >> /Font << /F1 8 0 R >> >> >>".to_vec(),
            stream("", content),
            stream(
                "/Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
            ),
            b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 100] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 0] /C1 [0 1 1] /N 1 >> /Extend [false false] >>".to_vec(),
            b"<< /Type /ExtGState /ca 0.5 /BM /Multiply >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            stream(
                "/Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /I true >> /Resources <<>>",
                b"0.5 g 30 60 25 25 re f",
            ),
            b"<< /Type /Annot /Subtype /Square /Rect [60 5 95 40] /C [1 0 1] /Border [0 0 2] >>".to_vec(),
        ];
        ContentEngine::open_bytes(build_test_pdf_from_objects(&objects)).unwrap()
    }

    #[test]
    fn clipped_contract_renders_only_the_window_pixel_identical_to_full_crop() {
        let engine = windowed_render_fixture();
        for dpi in [72, 150] {
            let mut full_contract = engine
                .default_render_contract(1, dpi, RenderMode::Compat)
                .unwrap();
            full_contract.annotations = AnnotationRenderPolicy::Include;
            let full = render(&engine, &full_contract);
            let (w, h) = (full.width, full.height);
            for tile in [
                RenderTile {
                    x: 0,
                    y: 0,
                    width: w / 3,
                    height: h / 4,
                },
                RenderTile {
                    x: w / 5,
                    y: h / 3,
                    width: w / 2,
                    height: h / 3,
                },
                RenderTile {
                    x: w - 7,
                    y: h - 9,
                    width: 7,
                    height: 9,
                },
                RenderTile {
                    x: 13,
                    y: 1,
                    width: 1,
                    height: h - 1,
                },
            ] {
                let mut clipped = full_contract.with_render_tile(tile);
                clipped.annotations = AnnotationRenderPolicy::Include;
                let window = render(&engine, &clipped);
                let expected = crop_buffer(&full, tile).unwrap();
                assert_same_pixels(&expected, &window);
            }
        }
    }

    #[test]
    fn clipped_contract_budget_covers_only_the_window() {
        let engine = windowed_render_fixture();
        let mut contract = contract_for(&engine).with_render_tile(RenderTile {
            x: 10,
            y: 10,
            width: 4,
            height: 4,
        });
        // A full-page intermediate (100x100x4 bytes) would not fit; the
        // window (8x8) plus the cropped surface (4x4) does.
        contract.resource_budget.max_temporary_bytes = 1024;
        render(&engine, &contract);
        contract.resource_budget.max_temporary_bytes = 200;
        let err = engine
            .render_page_with_contract(&contract, &CancelToken::none())
            .expect_err("window plus crop exceeds 200 bytes");
        assert!(err.to_string().contains("windowed intermediate"), "{err}");
    }

    #[test]
    fn unclipped_contract_renders_arbitrary_size_through_device_matrix() {
        let engine = windowed_render_fixture();
        let base = contract_for(&engine);
        // Scale by 1.5 and translate by (-20, -10): FPDF_RenderPageBitmapWithMatrix.
        let transform = DeviceMatrix::from_f64([1.5, 0.0, 0.0, 1.5, -20.0, -10.0]);
        let mut matrix_contract = base.clone();
        matrix_contract.transform = transform;
        matrix_contract.clip = None;
        matrix_contract.width = 64;
        matrix_contract.height = 48;
        matrix_contract.stride = 64 * 4;
        let small = render(&engine, &matrix_contract);
        assert_eq!((small.width, small.height), (64, 48));

        // The same matrix at the natural surface size, cropped, matches.
        let mut natural = base.clone();
        natural.transform = transform;
        let natural = render(&engine, &natural);
        let expected = crop_buffer(
            &natural,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 48,
            },
        )
        .unwrap();
        assert_same_pixels(&expected, &small);

        // An output larger than the page shows the background beyond it.
        let mut large = base.clone();
        large.clip = None;
        large.width = 140;
        large.height = 120;
        large.stride = 140 * 4;
        let large = render(&engine, &large);
        assert_eq!((large.width, large.height), (140, 120));
        assert_eq!(large.get_pixel(130, 110), [255, 255, 255, 255]);
        let full = render(&engine, &base);
        assert_eq!(large.get_pixel(50, 50), full.get_pixel(50, 50));
    }

    #[test]
    fn type3_adjust_blue_snaps_within_tolerance_and_caps_remembered_rows() {
        let mut blues = Vec::new();
        assert_eq!(type3_adjust_blue(10.2, &mut blues), 10);
        assert_eq!(type3_adjust_blue(10.7, &mut blues), 10, "within 0.8 px");
        assert_eq!(type3_adjust_blue(9.3, &mut blues), 10);
        assert_eq!(type3_adjust_blue(11.0, &mut blues), 11, "outside 0.8 px");
        assert_eq!(blues, vec![10, 11]);
        let mut many = Vec::new();
        for row in 0..20 {
            type3_adjust_blue(f64::from(row * 2), &mut many);
        }
        assert_eq!(many.len(), TYPE3_MAX_BLUES);
        assert_eq!(
            type3_adjust_blue(100.4, &mut many),
            100,
            "rounded, not stored"
        );
        assert_eq!(many.len(), TYPE3_MAX_BLUES);
    }

    #[test]
    fn type3_sole_image_mask_detection_requires_inked_edge_rows() {
        let parse = |program: &[u8]| crate::content::ContentParser::parse(program).unwrap();
        let glyph =
            parse(b"10 0 0 0 10 10 d1 q 10 0 0 10 0 0 cm BI /IM true /W 8 /H 2 ID \x00\x00 EI Q");
        assert!(type3_sole_image_mask_matrix(&glyph).is_some());
        // Top row blank (all ones do not paint with the default /Decode).
        let blank_top =
            parse(b"10 0 0 0 10 10 d1 q 10 0 0 10 0 0 cm BI /IM true /W 8 /H 2 ID \xff\x00 EI Q");
        assert!(type3_sole_image_mask_matrix(&blank_top).is_none());
        // Coloured glyphs and glyphs with paths are not bitmap glyphs.
        let coloured = parse(b"10 0 d0 q 10 0 0 10 0 0 cm BI /IM true /W 8 /H 2 ID \x00\x00 EI Q");
        assert!(type3_sole_image_mask_matrix(&coloured).is_none());
        let with_path = parse(
            b"10 0 0 0 10 10 d1 0 0 1 1 re f q 10 0 0 10 0 0 cm BI /IM true /W 8 /H 2 ID \x00\x00 EI Q",
        );
        assert!(type3_sole_image_mask_matrix(&with_path).is_none());
    }

    /// Bottom-most row (device y) with any ink in a column range.
    fn lowest_inked_row(buf: &PixelBuffer, x0: i32, x1: i32) -> Option<(i32, u8)> {
        (0..buf.height as i32).rev().find_map(|y| {
            let darkest = (x0..x1).map(|x| buf.get_pixel(x, y)[0]).min().unwrap();
            (darkest < 250).then_some((y, darkest))
        })
    }

    #[test]
    fn type3_bitmap_glyph_baselines_snap_to_remembered_rows() {
        // Glyph A's image sits on the baseline; glyph B's is raised by 0.3
        // glyph units (0.3 device px). PDFium snaps B's bottom row to A's.
        let glyph_a = stream(
            "",
            b"10 0 0 0 10 10 d1 q 10 0 0 10 0 0 cm BI /IM true /W 8 /H 8 ID \x00\x00\x00\x00\x00\x00\x00\x00 EI Q",
        );
        let glyph_b = stream(
            "",
            b"10 0 0 0 10 10 d1 q 10 0 0 10 0 0.3 cm BI /IM true /W 8 /H 8 ID \x00\x00\x00\x00\x00\x00\x00\x00 EI Q",
        );
        let engine = ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /Font << /T3 5 0 R >> >>",
            b"BT /T3 1 Tf 20 50 Td (AB) Tj ET\n",
            vec![
                b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [1 0 0 1 0 0] /CharProcs << /A 6 0 R /B 7 0 R >> /Encoding << /Type /Encoding /Differences [65 /A /B] >> /FirstChar 65 /LastChar 66 /Widths [20 20] >>".to_vec(),
                glyph_a,
                glyph_b,
            ],
        ))
        .unwrap();
        let out = render(&engine, &contract_for(&engine));
        let a = lowest_inked_row(&out, 21, 29).expect("glyph A ink");
        let b = lowest_inked_row(&out, 41, 49).expect("glyph B ink");
        assert_eq!(a, b, "bitmap glyph baselines are level");
        assert_eq!(a.0, 49, "baseline row sits just above device y = 50");
    }

    fn overprint_image_fixture(color_space: &str, sample: &[u8]) -> ContentEngine {
        ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /XObject << /Im 5 0 R >> /ExtGState << /OP 6 0 R >> /ColorSpace << /Sep 7 0 R >> >>",
            b"1 0 0 rg 0 0 100 100 re f /OP gs q 100 0 0 100 0 0 cm /Im Do Q\n",
            vec![
                stream(
                    &format!(
                        "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace {color_space}"
                    ),
                    sample,
                ),
                b"<< /Type /ExtGState /op true /OP true /OPM 0 >>".to_vec(),
                b"[/Separation /Spot /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0 0 0] /N 1 >>]".to_vec(),
            ],
        ))
        .unwrap()
    }

    #[test]
    fn overprint_preview_imitates_overprint_for_cmyk_and_separation_images() {
        for (space, sample) in [("/DeviceCMYK", vec![255, 0, 0, 0]), ("/Sep", vec![255])] {
            let engine = overprint_image_fixture(space, &sample);
            let knockout = render(&engine, &contract_for(&engine));
            let image = knockout.get_pixel(50, 50);
            assert!(image[1] > 100 && image[2] > 100, "cyan image: {image:?}");
            let mut preview = contract_for(&engine);
            preview.overprint = OverprintPolicy::Preview;
            let out = render(&engine, &preview);
            // Darken(red backdrop, cyan image).
            assert_eq!(
                out.get_pixel(50, 50),
                [image[0].min(255), image[1].min(0), image[2].min(0), 255],
                "{space}"
            );
        }
        // RGB images are composited normally.
        let engine = overprint_image_fixture("/DeviceRGB", &[0, 200, 255]);
        let mut preview = contract_for(&engine);
        preview.overprint = OverprintPolicy::Preview;
        assert_eq!(
            render(&engine, &preview).get_pixel(50, 50),
            [0, 200, 255, 255]
        );
    }

    #[test]
    fn page_thumbnail_decodes_embedded_thumb_image() {
        let engine = ContentEngine::open_bytes(one_page_pdf(
            "/Thumb 5 0 R",
            b"",
            vec![stream(
                "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                &[255, 0, 0, 0, 0, 255],
            )],
        ))
        .unwrap();
        let rgba = engine.page_thumbnail_rgba(1).unwrap().expect("thumbnail");
        assert_eq!((rgba.width, rgba.height, rgba.channels), (2, 1, 4));
        assert_eq!(rgba.pixels, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let raw = engine
            .decode_page_thumbnail(1)
            .unwrap()
            .expect("raw thumbnail");
        assert_eq!(raw.channels, 3);

        let without = ContentEngine::open_bytes(one_page_pdf("", b"", vec![])).unwrap();
        assert!(without.page_thumbnail_rgba(1).unwrap().is_none());
    }

    #[test]
    fn rendered_image_object_uses_its_matrix_size_and_soft_mask() {
        let engine = ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /XObject << /Im 5 0 R /Masked 6 0 R >> >>",
            b"q 20 0 0 10 5 5 cm /Im Do Q q 0 0 0.5 rg 30 0 0 -15 40 60 cm /Masked Do Q\n",
            vec![
                stream(
                    "/Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                    &[255, 0, 0, 0, 255, 0],
                ),
                stream(
                    "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /SMask 7 0 R",
                    &[0, 0, 255],
                ),
                stream(
                    "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
                    &[128],
                ),
            ],
        ))
        .unwrap();
        let first = engine
            .render_page_image_object(1, &PageImageObjectSelector::Index(0))
            .unwrap();
        assert_eq!((first.width, first.height), (20, 10));
        assert_eq!(first.get_pixel(2, 5), [255, 0, 0, 255]);
        assert_eq!(first.get_pixel(17, 5), [0, 255, 0, 255]);

        let by_name = engine
            .render_page_image_object(1, &PageImageObjectSelector::Name("Masked".into()))
            .unwrap();
        assert_eq!((by_name.width, by_name.height), (30, 15));
        let masked = by_name.get_pixel(15, 7);
        assert!(
            (120..=136).contains(&masked[3]),
            "soft mask alpha applied: {masked:?}"
        );
        assert!(engine
            .render_page_image_object(1, &PageImageObjectSelector::Index(2))
            .is_err());
    }

    #[test]
    fn postscript_passes_dct_images_through_and_ascii85_encodes_others() {
        let rgb = crate::images::decoder::RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let jpeg = crate::images::encoder::ImageEncoder::encode_jpeg(&rgb, 90).unwrap();
        let engine = ContentEngine::open_bytes(one_page_pdf(
            "/Resources << /XObject << /Jpg 5 0 R /Raw 6 0 R >> >>",
            b"q 40 0 0 40 10 10 cm /Jpg Do Q q 40 0 0 40 50 50 cm /Raw Do Q\n",
            vec![
                stream(
                    "/Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode",
                    &jpeg,
                ),
                stream(
                    "/Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                    &rgb.pixels,
                ),
            ],
        ))
        .unwrap();
        let page = engine.render_page_ps(1, 72).unwrap();
        assert!(!page.is_rasterized);
        assert!(page.has_regional_images);
        let body = &page.body;
        assert!(
            body.contains("% WellfriendImagePassthrough filter=DCTDecode"),
            "{body}"
        );
        assert!(body.contains("/regiondctsrc regionpicstr /DCTDecode filter def"));
        assert!(body.contains("/ImageType 1 /Width 2 /Height 2"));
        assert_eq!(
            body.matches("regionpicstr false 3 colorimage regionpicstr flushfile")
                .count(),
            1,
            "only the unfiltered image is re-encoded as samples"
        );
        assert_eq!(body.matches("/ASCII85Decode filter").count(), 2);
        assert!(!body.contains("readhexstring"));
        assert_eq!(body.matches("~>").count(), 2);
    }
}
