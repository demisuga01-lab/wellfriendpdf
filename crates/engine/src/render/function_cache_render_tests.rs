// Unexecuted source regressions for render-owned function retention.
mod retained_function_rendering {
    use super::*;
    use crate::render::function::{FunctionResources, PreparedFunction};
    use crate::render::parameter_dictionary::tests::{bytes_with_objects, numbers, reference};

    fn function(end: f64) -> PdfObject {
        let mut dict = PdfDictionary::empty();
        dict.insert("FunctionType", PdfObject::Integer(2));
        dict.insert("Domain", numbers(&[0.0, 1.0]));
        dict.insert("C0", numbers(&[0.0]));
        dict.insert("C1", numbers(&[end]));
        dict.insert("N", PdfObject::Integer(1));
        PdfObject::Dictionary(dict)
    }

    fn state<'a>(engine: &'a ContentEngine, cache: &mut RenderDocumentCache) -> RenderState<'a> {
        RenderState::new_with_document_cache(
            PixelBuffer::new_filled_with_mode(10, 10, WHITE, RenderMode::Compat),
            Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            engine.get_page_resources(1).unwrap(),
            engine,
            1,
            cache,
        )
    }

    #[test]
    fn returned_render_cache_retains_functions_on_success_and_error_handoffs() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let reader = engine.document().reader();
        let mut cache = RenderDocumentCache::new();
        let mut held = None;
        for success in [false, true, true] {
            let state = state(&engine, &mut cache);
            let graph = PreparedFunction::cached_with_resources(
                &reference(4),
                1,
                false,
                reader,
                state.function_resources(),
            )
            .unwrap();
            if let Some(previous) = &held {
                assert!(Arc::ptr_eq(previous, graph.graph()));
            }
            held = Some(Arc::clone(graph.graph()));
            drop(graph);
            if success {
                state.into_buffer_and_document_cache(&mut cache);
            } else {
                state.return_document_cache(&mut cache);
            }
            assert_eq!(cache.function_cache_metrics().entries, 1);
            assert!(cache.aggregate_resource_cache_bytes() >= cache.function_cache_metrics().bytes);
        }
        assert_eq!(cache.function_cache_metrics().hits, 2);
        assert_eq!(reader.function_cache_metrics().entries, 0);
    }

    #[test]
    fn aggregate_eviction_counts_functions_alongside_image_entries() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let mut cache = RenderDocumentCache::new();
        let graph = PreparedFunction::cached_with_resources(
            &reference(4),
            1,
            false,
            engine.document().reader(),
            FunctionResources {
                cache: Some(cache.function_cache.as_ref()),
                ..Default::default()
            },
        )
        .unwrap();
        let bytes = cache.function_cache_metrics().bytes;
        let image = Arc::new(RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0],
        });
        cache.image_xobject_cache.insert("one".into(), image);
        cache.image_xobject_cache_order.push_back("one".into());
        cache.image_xobject_cache_bytes = 1;
        assert_eq!(cache.aggregate_resource_cache_bytes(), bytes + 1);
        cache.enforce_bounded_maps(RenderResourceBudget {
            max_cache_bytes: bytes as u64,
            ..Default::default()
        });
        // The existing aggregate policy evicts the largest oldest candidate.
        assert_eq!(cache.function_cache_metrics().entries, 0);
        assert_eq!(cache.function_cache_metrics().evictions, 1);
        assert_eq!(cache.aggregate_resource_cache_bytes(), 1);
        assert_eq!(graph.evaluate(&[0.25]), vec![0.25]);
    }

    #[test]
    fn worker_policy_changes_do_not_change_other_workers_or_reader_cache() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let reader = engine.document().reader();
        let standalone = reader.function_cache_metrics();
        let mut first = blank_render_state(&engine);
        let second = blank_render_state(&engine);
        first.apply_resource_budget(RenderResourceBudget {
            max_cache_bytes: 0,
            ..Default::default()
        });
        for state in [&first, &second] {
            // Retargeting colour provenance must not erase the resource policy
            // when callers compose the option builders in a different order.
            let recolored = ShadingRenderOptions::default()
                .with_function_cache(state.function_cache.as_ref())
                .with_memory_budget(&state.temporary_scheduler.budget)
                .with_color_context(None, state.color_transform_options());
            let policy = recolored.function_resources();
            assert!(std::ptr::eq(
                policy.cache.unwrap(),
                state.function_cache.as_ref()
            ));
            assert!(Arc::ptr_eq(
                policy.memory.unwrap(),
                &state.temporary_scheduler.budget
            ));
            for _ in 0..2 {
                let graph = PreparedFunction::cached_with_resources(
                    &reference(4),
                    1,
                    false,
                    reader,
                    state.function_resources(),
                )
                .unwrap();
                assert_eq!(graph.evaluate(&[0.25]), vec![0.25]);
            }
        }
        assert_eq!(first.function_cache.lock().unwrap().metrics().entries, 0);
        assert_eq!(second.function_cache.lock().unwrap().metrics().entries, 1);
        assert_eq!(second.function_cache.lock().unwrap().metrics().hits, 1);
        assert_eq!(reader.function_cache_metrics(), standalone);
    }

    #[test]
    fn reused_render_cache_rebinds_equal_object_ids_in_another_reader() {
        let first = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let second = ContentEngine::open_bytes(bytes_with_objects(&[function(0.5)])).unwrap();
        let mut cache = RenderDocumentCache::new();
        for (engine, expected) in [(&first, 1.0), (&second, 0.5), (&first, 1.0)] {
            let state = state(engine, &mut cache);
            let graph = PreparedFunction::cached_with_resources(
                &reference(4),
                1,
                false,
                engine.document().reader(),
                state.function_resources(),
            )
            .unwrap();
            assert_eq!(graph.evaluate(&[1.0]), vec![expected]);
            drop(graph);
            state.return_document_cache(&mut cache);
        }
        assert_eq!(cache.function_cache_metrics().reader_rebinds, 2);
        assert_eq!(cache.function_cache_metrics().entries, 1);
    }

    #[test]
    fn named_paint_shading_and_soft_mask_transfer_use_render_owned_functions() {
        for retain in [false, true] {
            let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
            let mut state = blank_render_state(&engine);
            state.apply_resource_budget(RenderResourceBudget {
                max_cache_bytes: if retain { 1024 * 1024 } else { 0 },
                ..Default::default()
            });
            state.resources.color_spaces.insert(
                "Ink".into(),
                PdfObject::Array(vec![
                    PdfObject::Name("Separation".into()),
                    PdfObject::Name("Ink".into()),
                    PdfObject::Name("DeviceGray".into()),
                    reference(4),
                ]),
            );
            let ops =
                crate::content::ContentParser::parse(b"/Ink cs .5 scn 0 0 10 10 re f").unwrap();
            state.dispatch_all(&ops);
            state.check_fatal_render_error().unwrap();
            assert_eq!(state.buf.get_pixel(5, 5), [128, 128, 128, 255]);
            let mut shading = PdfDictionary::empty();
            shading.insert("ShadingType", PdfObject::Integer(2));
            shading.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
            shading.insert("Coords", numbers(&[0.0, 0.0, 10.0, 0.0]));
            shading.insert("Function", reference(4));
            state
                .resources
                .shadings
                .insert("S".into(), PdfObject::Dictionary(shading));
            state.dispatch_all(&crate::content::ContentParser::parse(b"/S sh").unwrap());
            state.check_fatal_render_error().unwrap();
            let pixel = state.buf.get_pixel(5, 5);
            assert!((pixel[0] as i32 - 140).abs() <= 1);
            let mut mask = PdfDictionary::empty();
            mask.insert("TR", reference(4));
            let lut = state.build_transfer_lut(&mask).unwrap().unwrap();
            assert_eq!((lut[0], lut[255]), (0, 255));
            let metrics = state.function_cache.lock().unwrap().metrics();
            assert_eq!(metrics.entries > 0, retain);
            assert_eq!(
                engine.document().reader().function_cache_metrics().entries,
                0
            );
        }
    }

    #[test]
    fn clearing_or_recovering_poisoned_render_cache_releases_retention() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let mut cache = RenderDocumentCache::new();
        let graph = PreparedFunction::cached_with_resources(
            &reference(4),
            1,
            false,
            engine.document().reader(),
            FunctionResources {
                cache: Some(cache.function_cache.as_ref()),
                ..Default::default()
            },
        )
        .unwrap();
        cache.clear();
        assert_eq!(cache.function_cache_metrics().entries, 0);
        assert_eq!(graph.evaluate(&[0.5]), vec![0.5]);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = cache.function_cache.lock().unwrap();
            panic!("poison optional function cache");
        }));
        assert!(!cache.function_cache_metrics().available);
        assert_eq!(cache.aggregate_resource_cache_bytes(), usize::MAX);
        cache.enforce_bounded_maps(RenderResourceBudget {
            max_cache_bytes: 0,
            ..Default::default()
        });
        assert!(cache.function_cache_metrics().available);
        assert_eq!(cache.aggregate_resource_cache_bytes(), 0);
        assert_eq!(cache.function_cache_metrics().max_bytes, 0);
    }

    #[test]
    fn scheduled_xobject_and_inline_images_do_not_bypass_render_function_policy() {
        let space = PdfObject::Array(vec![
            PdfObject::Name("Separation".into()),
            PdfObject::Name("Ink".into()),
            PdfObject::Name("DeviceGray".into()),
            reference(4),
        ]);
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Image".into()));
        dict.insert("Width", PdfObject::Integer(1));
        dict.insert("Height", PdfObject::Integer(1));
        dict.insert("BitsPerComponent", PdfObject::Integer(8));
        dict.insert("ColorSpace", space);
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[
            function(1.0),
            PdfObject::Stream {
                dict: dict.clone(),
                raw: vec![128],
            },
        ]))
        .unwrap();
        let image = ImageReference {
            page_number: 1,
            xobject_name: "Ink".into(),
            object_number: 5,
            generation_number: 0,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: "Separation".into(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        for retain in [false, true] {
            let mut state = blank_render_state(&engine);
            let budget = RenderResourceBudget {
                max_cache_bytes: if retain { 1024 * 1024 } else { 0 },
                ..Default::default()
            };
            state.apply_render_contract_identity("image-policy", budget);
            let output = state
                .scheduled_decode_image(&image, "image policy regression")
                .unwrap();
            assert_eq!(output.pixels, vec![128, 128, 128, 255]);
            let inline = state
                .scheduled_decode_inline_image_with_color_space(
                    &[128],
                    1,
                    1,
                    8,
                    "Separation",
                    &[],
                    &[],
                    None,
                    &dict,
                )
                .unwrap();
            assert_eq!(inline.pixels, output.pixels);
            assert_eq!(
                state.function_cache.lock().unwrap().metrics().entries > 0,
                retain
            );
            assert_eq!(
                engine.document().reader().function_cache_metrics().entries,
                0
            );
            state.apply_render_contract_identity(
                "strict-image-policy",
                RenderResourceBudget {
                    max_temporary_bytes: 0,
                    ..budget
                },
            );
            assert!(state
                .scheduled_decode_image(&image, "strict image policy regression")
                .is_err());
            assert!(state
                .scheduled_decode_inline_image_with_color_space(
                    &[128],
                    1,
                    1,
                    8,
                    "Separation",
                    &[],
                    &[],
                    None,
                    &dict,
                )
                .is_err());
        }
    }

    #[test]
    fn reused_cache_adopts_new_default_retention_after_a_zero_cache_contract() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[function(1.0)])).unwrap();
        let mut cache = RenderDocumentCache::new();
        let mut previous = state(&engine, &mut cache);
        previous.apply_resource_budget(RenderResourceBudget {
            max_cache_bytes: 0,
            ..Default::default()
        });
        previous.return_document_cache(&mut cache);
        assert_eq!(cache.function_cache_metrics().max_bytes, 0);
        let next = state(&engine, &mut cache);
        assert!(next.function_cache.lock().unwrap().metrics().max_bytes > 0);
        PreparedFunction::cached_with_resources(
            &reference(4),
            1,
            false,
            engine.document().reader(),
            next.function_resources(),
        )
        .unwrap();
        next.return_document_cache(&mut cache);
        assert_eq!(cache.function_cache_metrics().entries, 1);
    }
}
