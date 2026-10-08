// Pixel assertions are unexecuted regression source, not observed evidence.
mod default_colour_rendering {
    use super::*;
    const GREEN: PixelColor = [0, 255, 0, 255];
    const DEFAULT: &str = "[/Separation /ReviewInk /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [0 1 0] /C1 [0 1 0] /N 1 >>]";
    fn stream(dict: &str, content: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< {dict} /Length {} >>\nstream\n", content.len()).into_bytes();
        bytes.extend_from_slice(content);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }
    fn fixture() -> ContentEngine {
        ContentEngine::open_bytes(build_test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R /Resources << /ColorSpace << /DefaultGray 6 0 R /Alias /DeviceGray /PatternGray [/Pattern /DeviceGray] >> /Font << /F1 5 0 R /T3 9 0 R >> /XObject << /Im 7 0 R /Nested 8 0 R >> /Pattern << /P1 11 0 R >> >> >>".to_string().into_bytes(),
            stream("",b""),b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),DEFAULT.as_bytes().to_vec(),
            stream("/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",&[128]),
            stream("/Subtype /Form /BBox [0 0 10 10] /Resources <<>>",b"0 0 10 10 re f"),
            b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [.001 0 0 .001 0 0] /CharProcs << /A 10 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1000] >>".to_vec(),
            stream("",b"1000 0 d0 0.4 g 0 0 1000 1000 re f"),
            stream("/Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 1 1] /XStep 1 /YStep 1 /Resources <<>>",b"0 0 1 1 re f")
        ])).unwrap()
    }
    fn state(engine: &ContentEngine) -> RenderState<'_> {
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        RenderState::new(
            PixelBuffer::new_filled_with_mode(10, 10, WHITE, RenderMode::Compat),
            viewport,
            engine.get_page_resources(1).unwrap(),
            engine,
            1,
        )
    }
    fn raw(state: &mut RenderState<'_>, program: &str) {
        for op in crate::content::ContentParser::parse(program.as_bytes()).unwrap() {
            state.dispatch(&op);
        }
        state.check_fatal_render_error().unwrap();
    }

    fn install_palette_domains(state: &mut RenderState<'_>) -> (PdfObject, PdfObject) {
        state.color_management_policy = ColorManagementPolicy::DeterministicFallback;
        let numbers = |values: &[f64]| {
            PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
        };
        let mut calibrated = PdfDictionary::empty();
        calibrated.insert("WhitePoint", numbers(&[0.9505, 1.0, 1.089]));
        let mut profile = PdfDictionary::empty();
        profile.insert("N", PdfObject::Integer(1));
        profile.insert("Range", numbers(&[0.25, 0.75]));
        profile.insert(
            "Alternate",
            PdfObject::Array(vec![
                PdfObject::Name("CalGray".into()),
                PdfObject::Dictionary(calibrated),
            ]),
        );
        let replacement = PdfObject::Array(vec![
            PdfObject::Name("ICCBased".into()),
            PdfObject::Stream {
                dict: profile,
                raw: vec![],
            },
        ]);
        let palette = PdfObject::Array(vec![
            PdfObject::Name("Indexed".into()),
            PdfObject::Name("DomainAlias".into()),
            PdfObject::Integer(0),
            PdfObject::String(vec![64]),
        ]);
        state
            .resources
            .color_spaces
            .insert("DefaultGray".into(), replacement.clone());
        state
            .resources
            .color_spaces
            .insert("DomainAlias".into(), PdfObject::Name("DeviceGray".into()));
        state
            .resources
            .color_spaces
            .insert("DomainPalette".into(), palette.clone());
        state.resources.color_spaces.insert(
            "PatternPalette".into(),
            PdfObject::Array(vec![PdfObject::Name("Pattern".into()), palette.clone()]),
        );
        (palette, replacement)
    }

    fn palette_expected(
        state: &RenderState<'_>,
        replacement: &PdfObject,
        source_is_icc: bool,
    ) -> PixelColor {
        let unit = 64.0 / 255.0;
        let value = if source_is_icc {
            0.25 + 0.5 * unit
        } else {
            unit
        };
        match crate::render::colorspace::resolve_named_color_with_options(
            replacement,
            &[value],
            1.0,
            state.engine.document().reader(),
            state.color_transform_options(),
        ) {
            crate::render::colorspace::NamedColor::Color(color) => color.to_pixel_color(),
            other => panic!("reference ICC alternate rejected: {other:?}"),
        }
    }

    fn constant_palette_shading() -> PdfDictionary {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Integer(1)]),
        );
        function.insert("C0", PdfObject::Array(vec![PdfObject::Integer(0)]));
        function.insert("C1", PdfObject::Array(vec![PdfObject::Integer(0)]));
        function.insert("N", PdfObject::Integer(1));
        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", PdfObject::Integer(2));
        shading.insert("ColorSpace", PdfObject::Name("DomainPalette".into()));
        shading.insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(10),
                PdfObject::Integer(0),
            ]),
        );
        shading.insert("Function", PdfObject::Dictionary(function));
        shading
    }

    #[test]
    fn raw_and_retained_shading_keep_original_palette_and_selected_backend() {
        let engine = fixture();
        for packed in [false, true] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let expected = palette_expected(&state, &replacement, false);
            let shading = PdfObject::Dictionary(constant_palette_shading());
            if packed {
                let viewport = state.viewport.clone();
                let mut adapter = RenderStatePlanAdapter {
                    state: &mut state,
                    viewport_ref: &viewport,
                    vector_ctm_base: None,
                    forced_vector_color: None,
                    forced_vector_fill_pixel_color: None,
                    forced_vector_stroke_pixel_color: None,
                    ignore_bounds: true,
                };
                adapter.dispatch_shading(
                    &ShadingDescriptor {
                        name: "DomainShading".into(),
                        object: Some(shading),
                    },
                    None,
                );
            } else {
                state
                    .resources
                    .shadings
                    .insert("DomainShading".into(), shading);
                raw(&mut state, "/DomainShading sh");
            }
            state.check_fatal_render_error().unwrap();
            assert_eq!(state.buf.get_pixel(5, 5), expected);
        }
    }

    #[test]
    fn shading_patterns_carry_source_domains_for_fill_and_stroke() {
        let engine = fixture();
        for program in [
            "/Pattern cs /DomainPattern scn 0 0 10 10 re f",
            "/Pattern CS /DomainPattern SCN 2 w 1 1 8 8 re S",
        ] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let expected = palette_expected(&state, &replacement, false);
            let mut pattern = PdfDictionary::empty();
            pattern.insert("PatternType", PdfObject::Integer(2));
            pattern.insert("Shading", PdfObject::Dictionary(constant_palette_shading()));
            state
                .resources
                .patterns
                .insert("DomainPattern".into(), PdfObject::Dictionary(pattern));
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(1, 5), expected);
        }
    }

    #[test]
    fn raw_and_retained_shading_apply_fill_alpha_once() {
        let engine = fixture();
        for packed in [false, true] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let color = palette_expected(&state, &replacement, false);
            let mut expected = state.buf.clone();
            expected.blend_pixel(5, 5, color, 0.25);
            state.gs.fill_alpha = 0.25;
            state.gs.stroke_alpha = 0.75;
            let shading = PdfObject::Dictionary(constant_palette_shading());
            if packed {
                let viewport = state.viewport.clone();
                let mut adapter = RenderStatePlanAdapter {
                    state: &mut state,
                    viewport_ref: &viewport,
                    vector_ctm_base: None,
                    forced_vector_color: None,
                    forced_vector_fill_pixel_color: None,
                    forced_vector_stroke_pixel_color: None,
                    ignore_bounds: true,
                };
                adapter.dispatch_shading(
                    &ShadingDescriptor {
                        name: "AlphaShading".into(),
                        object: Some(shading),
                    },
                    None,
                );
            } else {
                state
                    .resources
                    .shadings
                    .insert("AlphaShading".into(), shading);
                raw(&mut state, "/AlphaShading sh");
            }
            state.check_fatal_render_error().unwrap();
            assert_eq!(state.buf.get_pixel(5, 5), expected.get_pixel(5, 5));
            assert!(state.temporary_scheduler.metrics().peak_reserved_bytes > 0);
        }
    }

    #[test]
    fn shading_patterns_choose_fill_or_stroke_alpha_and_keep_clip() {
        let engine = fixture();
        for (program, alpha) in [
            ("/Pattern cs /DomainPattern scn 0 0 10 10 re f", 0.25),
            ("/Pattern CS /DomainPattern SCN 2 w 1 1 8 8 re S", 0.75),
        ] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let color = palette_expected(&state, &replacement, false);
            let mut expected = state.buf.clone();
            expected.blend_pixel(1, 5, color, alpha);
            state.gs.fill_alpha = 0.25;
            state.gs.stroke_alpha = 0.75;
            let mut pattern = PdfDictionary::empty();
            pattern.insert("PatternType", PdfObject::Integer(2));
            pattern.insert("Shading", PdfObject::Dictionary(constant_palette_shading()));
            state
                .resources
                .patterns
                .insert("DomainPattern".into(), PdfObject::Dictionary(pattern));
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(1, 5), expected.get_pixel(1, 5));
            if alpha == 0.75 {
                assert_eq!(state.buf.get_pixel(5, 5), WHITE);
            }
        }
    }

    #[test]
    fn public_render_budget_reaches_shading_scratch_allocation() {
        let engine = fixture();
        let mut state = state(&engine);
        install_palette_domains(&mut state);
        state.apply_resource_budget(RenderResourceBudget {
            max_temporary_bytes: 1,
            ..Default::default()
        });
        state.resources.shadings.insert(
            "BudgetShading".into(),
            PdfObject::Dictionary(constant_palette_shading()),
        );
        for op in crate::content::ContentParser::parse(b"/BudgetShading sh").unwrap() {
            state.dispatch(&op);
        }
        assert!(state.check_fatal_render_error().is_err());
        assert_eq!(state.buf.get_pixel(5, 5), WHITE);
    }

    fn bounded_palette_shading() -> PdfDictionary {
        let array = |values: &[i64]| {
            PdfObject::Array(values.iter().copied().map(PdfObject::Integer).collect())
        };
        let mut shading = constant_palette_shading();
        shading.insert("Coords", array(&[3, 0, 7, 0]));
        shading.insert("BBox", array(&[1, 1, 9, 9]));
        shading.insert("Background", array(&[0]));
        shading
    }

    #[test]
    fn raw_and_retained_shading_obey_bbox_but_ignore_pattern_background() {
        let engine = fixture();
        for packed in [false, true] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let expected = palette_expected(&state, &replacement, false);
            let shading = PdfObject::Dictionary(bounded_palette_shading());
            if packed {
                let viewport = state.viewport.clone();
                let mut adapter = RenderStatePlanAdapter {
                    state: &mut state,
                    viewport_ref: &viewport,
                    vector_ctm_base: None,
                    forced_vector_color: None,
                    forced_vector_fill_pixel_color: None,
                    forced_vector_stroke_pixel_color: None,
                    ignore_bounds: true,
                };
                adapter.dispatch_shading(
                    &ShadingDescriptor {
                        name: "Bounded".into(),
                        object: Some(shading),
                    },
                    None,
                );
            } else {
                state.resources.shadings.insert("Bounded".into(), shading);
                raw(&mut state, "/Bounded sh");
            }
            state.check_fatal_render_error().unwrap();
            assert_eq!(state.buf.get_pixel(5, 5), expected);
            assert_eq!(
                state.buf.get_pixel(8, 1),
                WHITE,
                "direct shading must ignore Background"
            );
            assert_eq!(
                state.buf.get_pixel(5, 0),
                WHITE,
                "BBox must clip even inside the axial range"
            );
        }
    }

    #[test]
    fn fill_and_stroke_patterns_apply_background_inside_bbox_and_selected_path() {
        let engine = fixture();
        for (program, stroke) in [
            ("/Pattern cs /Bounded scn 0 0 10 10 re f", false),
            ("/Pattern CS /Bounded SCN 2 w 1 1 8 8 re S", true),
        ] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let expected = palette_expected(&state, &replacement, false);
            let mut pattern = PdfDictionary::empty();
            pattern.insert("PatternType", PdfObject::Integer(2));
            pattern.insert("Shading", PdfObject::Dictionary(bounded_palette_shading()));
            state
                .resources
                .patterns
                .insert("Bounded".into(), PdfObject::Dictionary(pattern));
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(8, 1), expected);
            assert_eq!(state.buf.get_pixel(0, 0), WHITE);
            assert_eq!(
                state.buf.get_pixel(5, 5),
                if stroke { WHITE } else { expected }
            );
        }
    }

    #[test]
    fn pattern_bbox_uses_pattern_base_matrix_not_current_fill_ctm() {
        let engine = fixture();
        let mut state = state(&engine);
        let (_, replacement) = install_palette_domains(&mut state);
        let expected = palette_expected(&state, &replacement, false);
        let numbers = |values: &[i64]| {
            PdfObject::Array(values.iter().copied().map(PdfObject::Integer).collect())
        };
        let mut shading = constant_palette_shading();
        shading.insert("BBox", numbers(&[0, 0, 2, 2]));
        let mut pattern = PdfDictionary::empty();
        pattern.insert("PatternType", PdfObject::Integer(2));
        pattern.insert("Matrix", numbers(&[1, 0, 0, 1, 3, 4]));
        pattern.insert("Shading", PdfObject::Dictionary(shading));
        state
            .resources
            .patterns
            .insert("Translated".into(), PdfObject::Dictionary(pattern));
        raw(
            &mut state,
            "2 0 0 2 0 0 cm /Pattern cs /Translated scn 0 0 5 5 re f",
        );
        assert_eq!(state.buf.get_pixel(3, 4), expected);
        assert_eq!(state.buf.get_pixel(7, 1), WHITE);
    }

    #[test]
    fn indexed_fill_stroke_source_domain_survives_scope_restore_and_form_replay() {
        let engine = fixture();
        let mut state = state(&engine);
        let (_, replacement) = install_palette_domains(&mut state);
        let expected = palette_expected(&state, &replacement, false);
        raw(
            &mut state,
            "/DomainPalette cs 0 sc /DomainPalette CS 0 SC q",
        );
        let original = state.active_fill_color_space_resource.clone().unwrap();
        state.resources = PageResources::default().into();
        assert_eq!(state.fill_pixel_color(), expected);
        assert_eq!(state.stroke_pixel_color(), expected);
        raw(&mut state, "0 g 0 G Q");
        let restored = state.active_fill_color_space_resource.as_ref().unwrap();
        assert_eq!(restored.source, original.source);
        assert!(Arc::ptr_eq(&restored.object, &original.object));
        assert!(Arc::ptr_eq(&restored.source, &original.source));
        assert_eq!(state.fill_pixel_color(), expected);
        assert_eq!(state.stroke_pixel_color(), expected);
        state.handle_do_form("Nested", 8, 0, None, None);
        state.check_fatal_render_error().unwrap();
        assert_eq!(state.buf.get_pixel(5, 5), expected);
    }

    #[test]
    fn indexed_selected_source_changes_type3_identity_with_identical_target_and_components() {
        let engine = fixture();
        let mut state = state(&engine);
        let (_, replacement) = install_palette_domains(&mut state);
        raw(&mut state, "/DomainPalette cs 0 sc");
        let first = state.active_fill_color_space_resource.clone().unwrap();
        let hash = state.inherited_paint_context_fingerprint();
        let first_pixel = state.fill_pixel_color();
        state
            .resources
            .color_spaces
            .insert("DomainAlias".into(), replacement.clone());
        raw(&mut state, "/DomainPalette cs 0 sc");
        let second = state.active_fill_color_space_resource.as_ref().unwrap();
        assert_eq!(first.object, second.object);
        assert_ne!(first.source, second.source);
        assert_ne!(hash, state.inherited_paint_context_fingerprint());
        assert_ne!(first_pixel, state.fill_pixel_color());
        assert_eq!(
            state.fill_pixel_color(),
            palette_expected(&state, &replacement, true)
        );
    }

    #[test]
    fn packed_palette_selection_and_forced_vectors_use_original_domain() {
        let engine = fixture();
        let mut state = state(&engine);
        let (palette, replacement) = install_palette_domains(&mut state);
        let expected = palette_expected(&state, &replacement, false);
        let viewport = state.viewport.clone();
        let mut adapter = RenderStatePlanAdapter {
            state: &mut state,
            viewport_ref: &viewport,
            vector_ctm_base: None,
            forced_vector_color: None,
            forced_vector_fill_pixel_color: None,
            forced_vector_stroke_pixel_color: None,
            ignore_bounds: true,
        };
        adapter.dispatch_state(&GraphicsStateDescriptor::SetFillColorSpace {
            name: "DomainPalette".into(),
            object: Some(palette.clone()),
        });
        adapter.dispatch_state(&GraphicsStateDescriptor::SetStrokeColorSpace {
            name: "DomainPalette".into(),
            object: Some(palette),
        });
        adapter.dispatch_state(&GraphicsStateDescriptor::SetFillColor {
            components: vec![0.0],
            name: None,
            pattern: None,
        });
        adapter.dispatch_state(&GraphicsStateDescriptor::SetStrokeColor {
            components: vec![0.0],
            name: None,
            pattern: None,
        });
        assert_eq!(adapter.state.fill_pixel_color(), expected);
        assert_eq!(adapter.state.stroke_pixel_color(), expected);
        adapter.forced_vector_color = Some(adapter.state.gs.fill_color.clone());
        adapter.state.resources = PageResources::default().into();
        assert_eq!(adapter.effective_vector_color(WHITE), expected);
        adapter.state.check_fatal_render_error().unwrap();
    }

    #[test]
    fn indexed_uncolored_pattern_preserves_source_palette_for_fill_and_stroke_tiles() {
        let engine = fixture();
        for program in [
            "/PatternPalette cs 0 /P1 scn 0 0 10 10 re f",
            "/PatternPalette CS 0 /P1 SCN 2 w 1 1 8 8 re S",
        ] {
            let mut state = state(&engine);
            let (_, replacement) = install_palette_domains(&mut state);
            let expected = palette_expected(&state, &replacement, false);
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(1, 5), expected);
        }
    }

    #[test]
    fn full_image_decode_cache_distinguishes_source_domain_from_identical_replacement() {
        let engine=ContentEngine::open_bytes(build_test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R /Resources << /ColorSpace << /Alias /DeviceGray /DefaultGray [/ICCBased 6 0 R] >> /XObject << /Im 5 0 R >> >> >>".to_vec(),
            stream("",b""),
            stream("/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /Alias",&[64]),
            stream("/N 1 /Range [.25 .75] /Alternate [/CalGray << /WhitePoint [.9505 1 1.089] >>]",b""),
        ])).unwrap();
        let mut state = state(&engine);
        let mut image = ImageReference {
            page_number: 1,
            xobject_name: "Im".into(),
            object_number: 5,
            generation_number: 0,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: "ICCBased".into(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        let alias = PdfObject::Name("Alias".into());
        let first = state.bound_image_space(&alias).unwrap();
        let first_key = state.image_decode_cache_base_key(&image, Some(&first));
        let first_pixels = state
            .scheduled_decode_image_with_cache_key(
                &image,
                Some(&first),
                &first_key,
                "source-domain regression",
            )
            .unwrap();
        let first_window = state
            .scheduled_decode_raw_window_with_cache_key(
                &image,
                Some(&first),
                RawImageDecodeWindow {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                RawImageComponentSelection::All,
                &format!("{first_key}:window"),
                "source-domain window regression",
            )
            .unwrap();
        assert_eq!(first_pixels.pixels, first_window.pixels);
        let replacement = state
            .resources
            .color_spaces
            .get("DefaultGray")
            .unwrap()
            .clone();
        state
            .resources
            .color_spaces
            .insert("Alias".into(), replacement);
        let second = state.bound_image_space(&alias).unwrap();
        assert_eq!(first, second, "replacement graph is deliberately identical");
        image.color_space = second.0.clone();
        let second_key = state.image_decode_cache_base_key(&image, Some(&second));
        assert_ne!(
            first_key, second_key,
            "source domains must participate in cache identity"
        );
        let second_pixels = state
            .scheduled_decode_image_with_cache_key(
                &image,
                Some(&second),
                &second_key,
                "source-domain regression",
            )
            .unwrap();
        let second_window = state
            .scheduled_decode_raw_window_with_cache_key(
                &image,
                Some(&second),
                RawImageDecodeWindow {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                RawImageComponentSelection::All,
                &format!("{second_key}:window"),
                "source-domain window regression",
            )
            .unwrap();
        assert_eq!(second_pixels.pixels, second_window.pixels);
        assert_ne!(first_pixels.pixels, second_pixels.pixels);
        state.check_fatal_render_error().unwrap();
    }

    #[test]
    fn initial_device_colour_and_explicit_device_or_named_alias_select_defaults() {
        let engine = fixture();
        for program in [
            "0 0 10 10 re f",
            "0.4 g 0 0 10 10 re f",
            "/DeviceGray cs 0.4 sc 0 0 10 10 re f",
            "/Alias cs 0.4 scn 0 0 10 10 re f",
        ] {
            let mut state = state(&engine);
            assert_eq!(state.fill_pixel_color(), GREEN);
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(5, 5), GREEN);
        }
    }
    #[test]
    fn selection_is_bound_across_scope_changes_and_restored_by_q_q() {
        let engine = fixture();
        let mut state = state(&engine);
        raw(&mut state, "0.4 g q");
        state.resources = PageResources::default().into();
        assert_eq!(state.fill_pixel_color(), GREEN);
        raw(&mut state, "0.4 g");
        assert_ne!(state.fill_pixel_color(), GREEN);
        raw(&mut state, "Q");
        assert_eq!(state.fill_pixel_color(), GREEN);
        state.handle_do_form("Nested", 8, 0, None, None);
        state.check_fatal_render_error().unwrap();
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);
    }
    #[test]
    fn packed_device_dispatch_and_normalized_path_selection_retain_remapping() {
        let engine = fixture();
        let mut state = state(&engine);
        let viewport = state.viewport.clone();
        let ops = crate::content::ContentParser::parse(b"0.4 g 0 0 10 10 re f").unwrap();
        let list = build_display_list(&ops, viewport.clone(), &state.resources);
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::NativePatternPathOp { .. })));
        assert!(!list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::FillPath { .. })));
        let mut adapter = RenderStatePlanAdapter {
            state: &mut state,
            viewport_ref: &viewport,
            vector_ctm_base: None,
            forced_vector_color: None,
            forced_vector_fill_pixel_color: None,
            forced_vector_stroke_pixel_color: None,
            ignore_bounds: true,
        };
        adapter.dispatch_state(&GraphicsStateDescriptor::SetFillGray(0.4));
        adapter.dispatch_state(&GraphicsStateDescriptor::SetStrokeGray(0.2));
        assert_eq!(adapter.state.fill_pixel_color(), GREEN);
        assert_eq!(adapter.state.stroke_pixel_color(), GREEN);
        adapter.state.check_fatal_render_error().unwrap();
    }
    #[test]
    fn xobject_and_inline_samples_use_default_gray_and_stencil_masks_do_not_decode_through_it() {
        let engine = fixture();
        let mut state = state(&engine);
        raw(&mut state, "10 0 0 10 0 0 cm /Im Do");
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);
        let mut state = self::state(&engine);
        raw(&mut state, "10 0 0 10 0 0 cm");
        let params = vec![
            Operand::Name("W".into()),
            Operand::Integer(1),
            Operand::Name("H".into()),
            Operand::Integer(1),
            Operand::Name("BPC".into()),
            Operand::Integer(8),
            Operand::Name("CS".into()),
            Operand::Name("G".into()),
        ];
        state.paint_inline_image(&params, &[128]);
        state.check_fatal_render_error().unwrap();
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);
        let mut mask = self::state(&engine);
        raw(&mut mask, "1 0 0 rg 10 0 0 10 0 0 cm");
        let params = vec![
            Operand::Name("W".into()),
            Operand::Integer(1),
            Operand::Name("H".into()),
            Operand::Integer(1),
            Operand::Name("IM".into()),
            Operand::Boolean(true),
        ];
        mask.paint_inline_image(&params, &[0]);
        mask.check_fatal_render_error().unwrap();
        assert_eq!(mask.buf.get_pixel(5, 5), RED);
    }
    #[test]
    fn shading_and_group_colour_space_graphs_are_bound_to_defaults() {
        let engine = fixture();
        let mut state = state(&engine);
        let mut dict = PdfDictionary::empty();
        dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        let (bound, _) = state.shading_dict_with_resolved_color_space(dict).unwrap();
        assert_eq!(
            bound.get("ColorSpace"),
            Some(
                &engine
                    .document()
                    .reader()
                    .resolve(state.resources.color_spaces["DefaultGray"].clone())
                    .unwrap()
            )
        );
        let mut group = PdfDictionary::empty();
        group.insert("CS", PdfObject::Name("DeviceGray".into()));
        // A group blend space must be device/CIE, not the spot space used by
        // this fixture for testing paint remapping.
        let mut params = PdfDictionary::empty();
        params.insert(
            "WhitePoint",
            PdfObject::Array(vec![
                PdfObject::Real(0.9505),
                PdfObject::Integer(1),
                PdfObject::Real(1.089),
            ]),
        );
        state.resources.color_spaces.insert(
            "DefaultGray".into(),
            PdfObject::Array(vec![
                PdfObject::Name("CalGray".into()),
                PdfObject::Dictionary(params),
            ]),
        );
        let policy = transparency_group_color_space_policy(
            &group,
            &state.resources,
            engine.document().reader(),
        )
        .unwrap();
        assert!(matches!(
            policy,
            TransparencyGroupColorSpacePolicy::NonDeviceBackdrop { .. }
        ));
    }
    #[test]
    fn image_decode_identity_changes_when_the_default_graph_changes() {
        let engine = fixture();
        let mut state = state(&engine);
        let mut dict = PdfDictionary::empty();
        dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        let before = state.resolved_image_color_space_override(&dict).unwrap();
        state.resources.color_spaces.clear();
        let after = state.resolved_image_color_space_override(&dict).unwrap();
        assert_ne!(before, after);
        assert_eq!(
            after,
            ("DeviceGray".into(), PdfObject::Name("DeviceGray".into()))
        );
        // Decode cache keys already include this family and complete bound object.
    }

    #[test]
    fn packed_image_payload_cannot_shadow_intrinsic_device_names() {
        let engine = fixture();
        let mut state = state(&engine);
        let mut dict = PdfDictionary::empty();
        dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        let direct = state.resolved_image_color_space_override(&dict).unwrap();
        let payload = ResolvedInlineImageColorSpace {
            name: "DeviceGray".into(),
            object: PdfObject::Name("DeviceRGB".into()),
        };
        assert_eq!(
            state
                .resolved_image_color_space_override_from_payload(&dict, "DeviceRGB", &payload)
                .unwrap(),
            direct
        );
        state.check_fatal_render_error().unwrap();
    }

    #[test]
    fn inline_indexed_array_uses_its_default_base_in_raw_and_packed_routes() {
        let engine = fixture();
        let params = vec![
            Operand::Name("W".into()),
            Operand::Integer(1),
            Operand::Name("H".into()),
            Operand::Integer(1),
            Operand::Name("BPC".into()),
            Operand::Integer(8),
            Operand::Name("CS".into()),
            Operand::Array(vec![
                Operand::Name("I".into()),
                Operand::Name("G".into()),
                Operand::Integer(1),
                Operand::String(vec![0, 255]),
            ]),
        ];
        let mut direct = state(&engine);
        raw(&mut direct, "10 0 0 10 0 0 cm");
        direct.paint_inline_image(&params, &[1]);
        direct.check_fatal_render_error().unwrap();
        assert_eq!(direct.buf.get_pixel(5, 5), GREEN);
        let mut packed = state(&engine);
        raw(&mut packed, "10 0 0 10 0 0 cm");
        let viewport = packed.viewport.clone();
        let mut adapter = RenderStatePlanAdapter {
            state: &mut packed,
            viewport_ref: &viewport,
            vector_ctm_base: None,
            forced_vector_color: None,
            forced_vector_fill_pixel_color: None,
            forced_vector_stroke_pixel_color: None,
            ignore_bounds: true,
        };
        adapter.dispatch_inline_image(
            &InlineImageDescriptor {
                params,
                data: vec![1],
                color_space: None,
            },
            None,
        );
        adapter.state.check_fatal_render_error().unwrap();
        assert_eq!(adapter.state.buf.get_pixel(5, 5), GREEN);
    }

    #[test]
    fn encoded_indexed_palette_stream_is_shared_by_path_and_image_decoding() {
        let engine=ContentEngine::open_bytes(build_test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R /Resources << /ColorSpace << /Palette 5 0 R >> /XObject << /Im 6 0 R >> >> >>".to_vec(),
            stream("",b"/Palette cs 0.5 sc 0 0 5 10 re f q 5 0 0 10 5 0 cm /Im Do Q"),
            b"[/Indexed /DeviceRGB 1 7 0 R]".to_vec(),
            stream("/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace 5 0 R",&[1]),
            stream("/Filter /ASCIIHexDecode",b"FF0000 00FF00>"),
        ])).unwrap();
        let rendered = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .unwrap();
        assert_eq!(rendered.get_pixel(2, 5), GREEN);
        assert_eq!(rendered.get_pixel(8, 5), GREEN);
    }
    #[test]
    fn malformed_default_is_fatal_not_an_uncalibrated_device_fallback() {
        let engine = fixture();
        let mut state = state(&engine);
        state
            .resources
            .color_spaces
            .insert("DefaultGray".into(), PdfObject::Name("DeviceRGB".into()));
        state.dispatch(&ContentOperation::new("g", vec![Operand::Real(0.5)]));
        assert!(state.check_fatal_render_error().is_err());
    }

    #[test]
    fn uncolored_pattern_keeps_actual_default_base_through_empty_tile_resources() {
        let engine = fixture();
        for program in [
            "/PatternGray cs 0.4 /P1 scn 0 0 10 10 re f",
            "/PatternGray CS 0.4 /P1 SCN 2 w 1 1 8 8 re S",
        ] {
            let mut state = state(&engine);
            raw(&mut state, program);
            assert_eq!(state.buf.get_pixel(1, 5), GREEN);
        }
        let mut invalid = state(&engine);
        for op in
            crate::content::ContentParser::parse(b"/PatternGray cs 0.2 0.4 /P1 scn 0 0 10 10 re f")
                .unwrap()
        {
            invalid.dispatch(&op);
        }
        assert!(invalid.check_fatal_render_error().is_err());
    }

    #[test]
    fn type3_explicit_device_colour_uses_scope_bound_replay_instead_of_baked_geometry() {
        let engine = fixture();
        let mut state = state(&engine);
        raw(&mut state, "BT /T3 10 Tf 1 0 0 1 0 0 Tm (A) Tj ET");
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);
        let font = state.resources.fonts["T3"].clone();
        let geometry = state.cached_type3_glyph_geometry("T3", &font, "A").unwrap();
        assert!(geometry._uses_color_state);
        assert_ne!(geometry.fills[0].color, Some(GREEN));
        assert!(state.pending_text_clip.is_none());
        let mut small = self::state(&engine);
        raw(
            &mut small,
            "BT /T3 5 Tf 1 0 0 1 0 0 Tm (A) Tj ET 1 0 0 rg 0 0 10 10 re f",
        );
        assert_eq!(
            small.buf.get_pixel(8, 5),
            RED,
            "normal glyph paint must not establish a text clip"
        );
    }

    #[test]
    fn selecting_a_space_resets_its_colour_and_marks_it_explicit_in_every_path() {
        let ops = crate::content::ContentParser::parse(b"1 0 0 rg /DeviceGray cs 0 0 10 10 re f")
            .unwrap();
        let geometry = Type3PathCollector::collect("A", &ops, "initial-colour").unwrap();
        assert_eq!(geometry.fills[0].color, Some(BLACK));
        let list = build_display_list(
            &ops,
            Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            &PageResources::default(),
        );
        assert!(list.ops.iter().any(|op|matches!(op,DisplayOp::FillPath{state,..} if state.fill_color_explicit && state.fill_color==BLACK)));
        let mut gs = GraphicsState::default();
        gs.process(&ContentOperation::new(
            "cs",
            vec![Operand::Name("DeviceCMYK".into())],
        ));
        assert_eq!(gs.fill_color.components, vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn pattern_initial_colour_is_no_paint_but_preserves_clipping_stroke_and_saved_selection() {
        let engine = fixture();
        let mut state = state(&engine);
        raw(
            &mut state,
            "/PatternGray cs 0.4 /P1 scn q /PatternGray cs 0 0 10 10 re f",
        );
        assert_eq!(state.buf.get_pixel(5, 5), WHITE);
        assert!(state.gs.fill_pattern_name.is_none());
        assert!(state.active_fill_pattern_resource.is_none());
        assert_eq!(state.fill_pixel_color()[3], 0);
        raw(&mut state, "1 0 0 RG 2 w 1 1 8 8 re B");
        assert_eq!(state.buf.get_pixel(1, 5), RED);
        assert_eq!(state.buf.get_pixel(5, 5), WHITE);
        raw(&mut state, "Q 0 0 10 10 re f");
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);

        let mut clipped = self::state(&engine);
        raw(
            &mut clipped,
            "/Pattern cs 0 0 5 10 re W f 1 0 0 rg 0 0 10 10 re f",
        );
        assert_eq!(clipped.buf.get_pixel(2, 5), RED);
        assert_eq!(clipped.buf.get_pixel(8, 5), WHITE);
        let mut stroked = self::state(&engine);
        raw(&mut stroked, "/Pattern CS 1 0 0 rg 0 0 10 10 re B");
        assert_eq!(stroked.buf.get_pixel(5, 5), RED);
        assert_eq!(stroked.stroke_pixel_color()[3], 0);
        let ops = crate::content::ContentParser::parse(b"/Pattern cs 0 0 10 10 re f").unwrap();
        assert!(
            build_display_list(&ops, state.viewport.clone(), &state.resources)
                .unsupported
                .is_empty()
        );
    }

    #[test]
    fn lab_and_indexed_components_keep_their_non_unit_range_in_raw_and_packed_dispatch() {
        let engine = fixture();
        let mut state = state(&engine);
        state.resources.color_spaces.insert(
            "Palette".into(),
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".into()),
                PdfObject::Name("DeviceRGB".into()),
                PdfObject::Integer(2),
                PdfObject::String(vec![0, 0, 0, 255, 0, 0, 0, 255, 0]),
            ]),
        );
        raw(&mut state, "/Palette cs 2 sc 0 0 10 10 re f");
        assert_eq!(state.gs.fill_color.components, vec![2.0]);
        assert_eq!(state.buf.get_pixel(5, 5), GREEN);
        let mut params = PdfDictionary::empty();
        params.insert(
            "WhitePoint",
            PdfObject::Array(vec![
                PdfObject::Real(0.9505),
                PdfObject::Integer(1),
                PdfObject::Real(1.089),
            ]),
        );
        let lab = PdfObject::Array(vec![
            PdfObject::Name("Lab".into()),
            PdfObject::Dictionary(params),
        ]);
        state
            .resources
            .color_spaces
            .insert("LabColor".into(), lab.clone());
        raw(&mut state, "/LabColor cs 50 -20 30 sc");
        assert_eq!(state.gs.fill_color.components, vec![50.0, -20.0, 30.0]);
        let expected = state.fill_pixel_color();
        let viewport = state.viewport.clone();
        let mut adapter = RenderStatePlanAdapter {
            state: &mut state,
            viewport_ref: &viewport,
            vector_ctm_base: None,
            forced_vector_color: None,
            forced_vector_fill_pixel_color: None,
            forced_vector_stroke_pixel_color: None,
            ignore_bounds: true,
        };
        adapter.dispatch_state(&GraphicsStateDescriptor::SetFillColorSpace {
            name: "LabColor".into(),
            object: Some(lab),
        });
        adapter.dispatch_state(&GraphicsStateDescriptor::SetFillColor {
            components: vec![50.0, -20.0, 30.0],
            name: None,
            pattern: None,
        });
        assert_eq!(adapter.state.fill_pixel_color(), expected);
        adapter.state.check_fatal_render_error().unwrap();
    }

    #[test]
    fn default_resource_edits_invalidate_tiles_even_for_implicit_device_operators() {
        let engine = fixture();
        let resources = engine.get_page_resources(1).unwrap();
        let ops = crate::content::ContentParser::parse(b"0.4 g 0 0 10 10 re f").unwrap();
        let list = build_display_list(&ops, Viewport::new([0.0, 0.0, 10.0, 10.0], 72), &resources);
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let mut cache = RenderDocumentCache::new();
        PageRenderer::record_display_list_tile_resource_dependencies(
            &engine, 1, &resources, &list, tile, &mut cache,
        );
        let revision = RevisionId(engine.canonical_document().revision().0.wrapping_add(1));
        let invalidated =
            engine.invalidate_for_transaction(&mut cache, &["6 0 R".into()], &[], revision);
        assert!(invalidated
            .invalidation
            .invalidated_tiles
            .contains(&(1, tile)));
    }

    #[test]
    fn vector_export_reports_raster_requirement_instead_of_emitting_raw_device_colours() {
        use crate::render::vector_fallback::{
            classify_page_for_vector_output, VectorFallbackDecision,
        };
        let engine = fixture();
        let resources = engine.get_page_resources(1).unwrap();
        let ops = crate::content::ContentParser::parse(b"0.4 g 0 0 10 10 re f").unwrap();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "default colour spaces require scope-bound native rendering"
            }
        ));
        assert!(matches!(
            classify_page_for_vector_output(&ops, &PageResources::default(), 1.0),
            VectorFallbackDecision::PureVector
        ));
    }
}
