// Source-only regressions. These have not been executed during implementation.
mod resource_scopes {
    use super::*;

    fn stream(entries: &str, content: &str) -> Vec<u8> {
        format!(
            "<< {entries} /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .into_bytes()
    }

    fn fixture_pdf(group: bool, inner_resources: &str, inner_content: &str) -> Vec<u8> {
        let group = if group {
            "/Group << /S /Transparency /I true /CS /DeviceRGB >>"
        } else {
            ""
        };
        build_test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Outer 6 0 R /Nested 7 0 R /Shared 8 0 R >> >> >>".to_vec(),
            stream("", "/Outer Do"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            stream(
                &format!("/Subtype /Form /BBox [0 0 10 10] {group} /Resources << /Font << /F1 << /Subtype /Type1 /BaseFont /Courier >> >> /XObject << /Nested 7 0 R /Shared 9 0 R >> >>"),
                "/Nested Do",
            ),
            stream(&format!("/Subtype /Form /BBox [0 0 10 10] {inner_resources}"), inner_content),
            stream("/Subtype /Form /BBox [0 0 10 10] /Resources <<>>", "1 0 0 rg 0 0 10 10 re f"),
            stream("/Subtype /Form /BBox [0 0 10 10] /Resources <<>>", "0 0 1 rg 0 0 10 10 re f"),
            b"null".to_vec(),
            b"12 0 R".to_vec(),
            b"11 0 R".to_vec(),
        ])
    }

    fn state(engine: &ContentEngine) -> RenderState<'_> {
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let buf = PixelBuffer::new_filled_with_mode(10, 10, WHITE, RenderMode::Compat);
        RenderState::new(
            buf,
            viewport,
            engine.get_page_resources(1).unwrap(),
            engine,
            1,
        )
    }

    fn install_outer_scope(state: &mut RenderState<'_>) {
        let PdfObject::Stream { dict, .. } =
            state.engine.document().reader().get_object(6, 0).unwrap()
        else {
            panic!("fixture outer Form");
        };
        state.resources = SharedPageResources::from(Arc::new(
            PageResources::from_content_owner(&dict, state.engine.document().reader())
                .unwrap()
                .unwrap(),
        ));
    }

    fn type3_font(resources: Option<PdfObject>) -> PdfDictionary {
        let mut dict = PdfDictionary::empty();
        if let Some(resources) = resources {
            dict.insert("Resources", resources);
        }
        let mut charprocs = PdfDictionary::empty();
        charprocs.insert(
            "A",
            PdfObject::Reference {
                number: 7,
                generation: 0,
            },
        );
        dict.insert("CharProcs", PdfObject::Dictionary(charprocs));
        dict
    }

    #[test]
    fn explicit_empty_null_indirect_and_malformed_scopes_are_distinguished() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "")).unwrap();
        let reader = engine.document().reader();
        assert!(
            PageResources::from_content_owner(&PdfDictionary::empty(), reader)
                .unwrap()
                .is_none()
        );
        for null in [
            PdfObject::Null,
            PdfObject::Reference {
                number: 10,
                generation: 0,
            },
        ] {
            let owner = dict_with(&[("Resources", null)]);
            assert!(PageResources::from_content_owner(&owner, reader)
                .unwrap()
                .is_none());
        }
        let owner = dict_with(&[("Resources", PdfObject::Dictionary(PdfDictionary::empty()))]);
        let local = PageResources::from_content_owner(&owner, reader)
            .unwrap()
            .unwrap();
        let page = Arc::new(engine.get_page_resources(1).unwrap());
        assert!(content_resource_scope(Some(&local), &page).fonts.is_empty());
        for invalid in [
            PdfObject::Integer(42),
            PdfObject::Reference {
                number: 11,
                generation: 0,
            },
        ] {
            let owner = dict_with(&[("Resources", invalid)]);
            assert!(PageResources::from_content_owner(&owner, reader).is_err());
        }
    }

    #[test]
    fn direct_local_font_does_not_retain_the_page_font_reference() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "")).unwrap();
        let mut state = state(&engine);
        assert_eq!(state.resources.font_references.get("F1"), Some(&(5, 0)));
        install_outer_scope(&mut state);
        assert_eq!(
            state.resources.fonts["F1"].get_name("BaseFont"),
            Some("Courier")
        );
        assert!(!state.resources.font_references.contains_key("F1"));
        assert_eq!(
            state.page_resources.font_references.get("F1"),
            Some(&(5, 0))
        );
    }

    #[test]
    fn legacy_nested_form_uses_original_page_scope_including_inside_group() {
        for group in [false, true] {
            let engine = ContentEngine::open_bytes(fixture_pdf(group, "", "/Shared Do")).unwrap();
            let mut state = state(&engine);
            let before = page_resources_fingerprint(&state.resources);
            state.handle_do_form("Outer", 6, 0, None, None);
            state.check_fatal_render_error().unwrap();
            assert_eq!(
                state.buf.get_pixel(5, 5),
                RED,
                "must use page /Shared, not blue caller /Shared"
            );
            assert_eq!(page_resources_fingerprint(&state.resources), before);
        }
    }

    #[test]
    fn explicit_empty_nested_form_cannot_borrow_page_or_caller_xobjects() {
        for group in [false, true] {
            let engine =
                ContentEngine::open_bytes(fixture_pdf(group, "/Resources <<>>", "/Shared Do"))
                    .unwrap();
            let mut state = state(&engine);
            state.handle_do_form("Outer", 6, 0, None, None);
            assert!(state.check_fatal_render_error().is_err());
            assert_eq!(state.buf.get_pixel(5, 5), WHITE);
        }
    }

    #[test]
    fn form_cache_is_page_scoped_not_caller_scoped_and_rejects_bad_resources() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "/Shared Do")).unwrap();
        let mut state = state(&engine);
        let page = state
            .cached_form_xobject_program("Nested", 7, 0, None, None)
            .unwrap();
        install_outer_scope(&mut state);
        let nested = state
            .cached_form_xobject_program("Nested", 7, 0, None, None)
            .unwrap();
        assert!(Arc::ptr_eq(&page, &nested));
        for invalid in ["/Resources 42", "/Resources 11 0 R"] {
            let engine = ContentEngine::open_bytes(fixture_pdf(false, invalid, "")).unwrap();
            let mut state = self::state(&engine);
            assert!(state
                .cached_form_xobject_program("Nested", 7, 0, None, None)
                .is_none());
            assert!(state.check_fatal_render_error().is_err());
            // Reuse of a failed program cache entry must still report failure,
            // including when a later render state starts without an error.
            state.fatal_render_error = None;
            assert!(state
                .cached_form_xobject_program("Nested", 7, 0, None, None)
                .is_none());
            assert!(state.check_fatal_render_error().is_err());
        }
    }

    #[test]
    fn annotation_appearance_missing_resources_uses_page_not_active_form() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "/Shared Do")).unwrap();
        let mut state = state(&engine);
        install_outer_scope(&mut state);
        let PdfObject::Stream { dict, raw } = engine.document().reader().get_object(7, 0).unwrap()
        else {
            panic!("fixture appearance");
        };
        let annot = PdfObject::Reference {
            number: 20,
            generation: 0,
        };
        let program = state
            .cached_annotation_appearance_program(
                "N",
                &annot,
                0,
                &dict,
                raw,
                [0.0, 0.0, 10.0, 10.0],
            )
            .unwrap();
        state.render_form_program_content_stream("N", &program);
        state.check_fatal_render_error().unwrap();
        assert_eq!(state.buf.get_pixel(5, 5), RED);
        assert_eq!(state.resources.xobjects.get("Shared"), Some(&(9, 0)));
    }

    #[test]
    fn inherited_selected_font_remains_bound_when_empty_scope_is_entered() {
        for group in [false, true] {
            let engine =
                ContentEngine::open_bytes(fixture_pdf(group, "/Resources <<>>", "BT (A) Tj ET"))
                    .unwrap();
            let mut state = state(&engine);
            let font = state.resources.fonts["F1"].clone();
            state.set_active_text_font("F1", 4.0, Some(font.clone()));
            let mut reference = self::state(&engine);
            reference.set_active_text_font("F1", 4.0, Some(font));
            reference.handle_do_form("Nested", 7, 0, None, None);
            reference.check_fatal_render_error().unwrap();
            // No local Tf: inherit the page's selected font object through the
            // (possibly offscreen) outer Form, despite its shadowing /F1.
            state.handle_do_form("Outer", 6, 0, None, None);
            state.check_fatal_render_error().unwrap();
            assert_eq!(
                state
                    .current_text_font_resource("F1")
                    .0
                    .unwrap()
                    .get_name("BaseFont"),
                Some("Helvetica")
            );
            assert!((0..10).any(|y| (0..10).any(|x| state.buf.get_pixel(x, y) != WHITE)));
            for y in 0..10 {
                for x in 0..10 {
                    assert_eq!(state.buf.get_pixel(x, y), reference.buf.get_pixel(x, y));
                }
            }
        }
    }

    #[test]
    fn type3_resources_follow_charproc_then_font_then_original_page() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "/Resources <<>>", "")).unwrap();
        let mut state = state(&engine);
        install_outer_scope(&mut state);
        let font = type3_font(Some(PdfObject::Integer(42)));
        let proc = state.collect_type3_charproc(&font, "A").unwrap();
        // An explicit empty glyph scope wins, even if lower-priority font scope
        // is malformed. No unrelated scope is evaluated or merged.
        assert!(state
            .type3_charproc_resources(&font, &proc)
            .unwrap()
            .fonts
            .is_empty());
        let legacy_proc = Type3CharProc {
            resources: None,
            ..proc
        };
        assert!(state.type3_charproc_resources(&font, &legacy_proc).is_err());
        let legacy_font = type3_font(None);
        let resources = state
            .type3_charproc_resources(&legacy_font, &legacy_proc)
            .unwrap();
        assert_eq!(
            resources.fonts["F1"].get_name("BaseFont"),
            Some("Helvetica")
        );
        let empty_font = type3_font(Some(PdfObject::Dictionary(PdfDictionary::empty())));
        assert!(state
            .type3_charproc_resources(&empty_font, &legacy_proc)
            .unwrap()
            .fonts
            .is_empty());
    }

    #[test]
    fn type3_charproc_replay_missing_scope_uses_page_xobject_and_restores_caller() {
        let engine =
            ContentEngine::open_bytes(fixture_pdf(false, "", "500 0 d0 /Shared Do")).unwrap();
        let mut state = state(&engine);
        install_outer_scope(&mut state);
        let font = type3_font(None);
        let proc = state.collect_type3_charproc(&font, "A").unwrap();
        assert!(state.render_type3_charproc_full_with_ctm(&font, &proc, Transform2D::identity()));
        assert_eq!(state.buf.get_pixel(5, 5), RED);
        assert_eq!(state.resources.xobjects.get("Shared"), Some(&(9, 0)));
    }

    #[test]
    fn smask_group_cannot_borrow_xobjects_from_explicitly_empty_scope() {
        let engine = ContentEngine::open_bytes(fixture_pdf(
            false,
            "/Resources <<>> /Group << /S /Transparency /I true /CS /DeviceRGB >>",
            "/Shared Do",
        ))
        .unwrap();
        let mut state = state(&engine);
        let smask = dict_with(&[
            ("S", PdfObject::Name("Alpha".into())),
            (
                "G",
                PdfObject::Reference {
                    number: 7,
                    generation: 0,
                },
            ),
        ]);
        state.apply_smask(smask, None);
        assert!(state.check_fatal_render_error().is_err());
    }

    #[test]
    fn tiling_pattern_has_a_complete_required_scope_and_legacy_children_use_page() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "/Shared Do")).unwrap();
        for (resources, content, expected) in [
            (None, "/Shared Do", None),
            (Some(PdfDictionary::empty()), "/Shared Do", None),
            (
                Some(dict_with(&[(
                    "XObject",
                    PdfObject::Dictionary(dict_with(&[(
                        "Shared",
                        PdfObject::Reference {
                            number: 9,
                            generation: 0,
                        },
                    )])),
                )])),
                "/Shared Do",
                Some(BLUE),
            ),
            (
                Some(dict_with(&[(
                    "XObject",
                    PdfObject::Dictionary(dict_with(&[
                        (
                            "Nested",
                            PdfObject::Reference {
                                number: 7,
                                generation: 0,
                            },
                        ),
                        (
                            "Shared",
                            PdfObject::Reference {
                                number: 9,
                                generation: 0,
                            },
                        ),
                    ])),
                )])),
                "/Nested Do",
                Some(RED),
            ),
        ] {
            let mut state = state(&engine);
            let mut dict = dict_with(&[
                ("PatternType", PdfObject::Integer(1)),
                ("PaintType", PdfObject::Integer(1)),
                ("TilingType", PdfObject::Integer(1)),
                ("XStep", PdfObject::Integer(10)),
                ("YStep", PdfObject::Integer(10)),
                (
                    "BBox",
                    PdfObject::Array(vec![
                        PdfObject::Integer(0),
                        PdfObject::Integer(0),
                        PdfObject::Integer(10),
                        PdfObject::Integer(10),
                    ]),
                ),
            ]);
            if let Some(resources) = resources {
                dict.insert("Resources", PdfObject::Dictionary(resources));
            }
            let pattern = PdfObject::Stream {
                dict,
                raw: content.as_bytes().to_vec(),
            };
            state.path.rect(0.0, 0.0, 10.0, 10.0);
            state.paint_tiling_pattern_fill(FillRule::NonZero, &pattern);
            if let Some(color) = expected {
                state.check_fatal_render_error().unwrap();
                assert_eq!(state.buf.get_pixel(5, 5), color);
            } else {
                assert!(state.check_fatal_render_error().is_err());
            }
        }
    }

    #[test]
    fn appearance_invalid_scope_stays_invalid_on_negative_cache_hit() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "/Resources 42", "")).unwrap();
        let mut state = state(&engine);
        let PdfObject::Stream { dict, raw } = engine.document().reader().get_object(7, 0).unwrap()
        else {
            panic!("fixture appearance");
        };
        for _ in 0..2 {
            state.fatal_render_error = None;
            assert!(state
                .cached_annotation_appearance_program(
                    "N",
                    &PdfObject::Null,
                    0,
                    &dict,
                    raw.clone(),
                    [0.0, 0.0, 10.0, 10.0]
                )
                .is_none());
            assert!(state.check_fatal_render_error().is_err());
        }
    }

    #[test]
    fn type3_raster_identity_includes_resource_ownership_and_inherited_paint() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "/Shared Do")).unwrap();
        let mut state = state(&engine);
        let font = type3_font(None);
        let proc = state.collect_type3_charproc(&font, "A").unwrap();
        let key = state
            .type3_rendered_program_cache_key("T3", &font, "A", &proc)
            .unwrap();
        let mut other_page = (*state.page_resources).clone();
        other_page.xobjects.insert("Shared".into(), (9, 0));
        state.page_resources = Arc::new(other_page);
        assert_ne!(
            key,
            state
                .type3_rendered_program_cache_key("T3", &font, "A", &proc)
                .unwrap()
        );
        let key = state
            .type3_rendered_program_cache_key("T3", &font, "A", &proc)
            .unwrap();
        let new_font = dict_with(&[
            ("Subtype", PdfObject::Name("Type1".into())),
            ("BaseFont", PdfObject::Name("Courier".into())),
        ]);
        state.set_active_text_font("Inherited", 4.0, Some(new_font));
        assert_ne!(
            key,
            state
                .type3_rendered_program_cache_key("T3", &font, "A", &proc)
                .unwrap()
        );
        let key = state
            .type3_rendered_program_cache_key("T3", &font, "A", &proc)
            .unwrap();
        state.gs.line_width = 7.0;
        assert_ne!(
            key,
            state
                .type3_rendered_program_cache_key("T3", &font, "A", &proc)
                .unwrap()
        );
    }

    #[test]
    fn raw_tf_selects_local_font_instead_of_retaining_same_named_inherited_font() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "")).unwrap();
        let mut state = state(&engine);
        let font = state.resources.fonts["F1"].clone();
        state.set_active_text_font("F1", 4.0, Some(font));
        install_outer_scope(&mut state);
        state.dispatch(&ContentOperation::new(
            "Tf",
            vec![Operand::Name("F1".into()), Operand::Real(4.0)],
        ));
        state.check_fatal_render_error().unwrap();
        assert_eq!(
            state
                .current_text_font_resource("F1")
                .0
                .unwrap()
                .get_name("BaseFont"),
            Some("Courier")
        );
    }

    #[test]
    fn font_key_cache_validates_dictionary_instead_of_reusing_its_address() {
        let engine = ContentEngine::open_bytes(fixture_pdf(false, "", "")).unwrap();
        let mut state = state(&engine);
        let first = state.current_text_font_resource("F1");
        state
            .resources
            .fonts
            .get_mut("F1")
            .unwrap()
            .insert("BaseFont", PdfObject::Name("Courier".into()));
        let second = state.current_text_font_resource("F1");
        assert_eq!(second.0.unwrap().get_name("BaseFont"), Some("Courier"));
        assert_ne!(first.1, second.1);
    }
}
