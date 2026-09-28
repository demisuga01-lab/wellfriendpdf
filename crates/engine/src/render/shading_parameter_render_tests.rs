// Source assertions only: no compiler, test runner or renderer was executed.
mod shading_parameter_rendering {
    use super::*;
    use crate::render::parameter_dictionary::tests::{bytes_with_objects, numbers, reference};

    fn extra() -> Vec<PdfObject> {
        vec![
            PdfObject::Integer(2),
            numbers(&[0.0, 0.0, 10.0, 0.0]),
            numbers(&[0.0, 1.0]),
            numbers(&[0.0]),
            numbers(&[1.0]),
            PdfObject::Integer(1),
            numbers(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        ]
    }
    fn shading() -> PdfDictionary {
        let mut f = PdfDictionary::empty();
        for (key, number) in [
            ("FunctionType", 4),
            ("Domain", 6),
            ("C0", 7),
            ("C1", 8),
            ("N", 9),
        ] {
            f.insert(key, reference(number));
        }
        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", reference(4));
        shading.insert("Coords", reference(5));
        shading.insert("Domain", reference(6));
        shading.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        shading.insert("Function", PdfObject::Dictionary(f));
        shading
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
    fn execute(state: &mut RenderState<'_>, program: &[u8], packed: bool) {
        let ops = crate::content::ContentParser::parse(program).unwrap();
        if packed {
            let viewport = state.viewport.clone();
            let list = crate::render::display_list::build_display_list(
                &ops,
                viewport.clone(),
                &state.resources,
            );
            assert!(list.unsupported.is_empty(), "{:?}", list.unsupported);
            let plan = crate::render::plan::PackedDisplayList::compile_with_resources(
                list,
                Some(&state.resources),
            );
            let mut adapter = RenderStatePlanAdapter {
                state: &mut *state,
                viewport_ref: &viewport,
                vector_ctm_base: None,
                forced_vector_color: None,
                forced_vector_fill_pixel_color: None,
                forced_vector_stroke_pixel_color: None,
                ignore_bounds: false,
            };
            plan.execute_plan(&mut adapter).unwrap();
        } else {
            for op in &ops {
                state.dispatch(op);
            }
        }
        state.check_fatal_render_error().unwrap();
    }

    #[test]
    fn raw_and_compiled_shading_resolve_the_same_parameter_graph() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&extra())).unwrap();
        for packed in [false, true] {
            let mut state = state(&engine);
            state
                .resources
                .shadings
                .insert("S".into(), PdfObject::Dictionary(shading()));
            execute(&mut state, b"/S sh", packed);
            let pixel = state.buf.get_pixel(5, 5);
            assert!((pixel[0] as i32 - 140).abs() <= 1);
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
        }
    }

    #[test]
    fn raw_and_compiled_shadings_preserve_cubic_sampled_functions() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[])).unwrap();
        for packed in [false, true] {
            for kind in [1, 2] {
                let mut function = PdfDictionary::empty();
                function.insert("FunctionType", PdfObject::Integer(0));
                function.insert("BitsPerSample", PdfObject::Integer(8));
                function.insert("Order", PdfObject::Integer(3));
                function.insert("Range", numbers(&[0.0, 1.0]));
                let mut shading = PdfDictionary::empty();
                shading.insert("ShadingType", PdfObject::Integer(kind));
                shading.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
                let raw = if kind == 1 {
                    function.insert("Size", PdfObject::Array(vec![PdfObject::Integer(4); 2]));
                    function.insert("Domain", numbers(&[0.0, 3.0, 0.0, 3.0]));
                    shading.insert("Domain", numbers(&[0.0, 3.0, 0.0, 3.0]));
                    shading.insert("Matrix", numbers(&[3.0, 0.0, 0.0, 3.0, 0.0, 0.0]));
                    let mut samples = vec![0; 16];
                    samples[5] = 255;
                    samples
                } else {
                    function.insert("Size", PdfObject::Array(vec![PdfObject::Integer(4)]));
                    function.insert("Domain", numbers(&[0.0, 3.0]));
                    shading.insert("Domain", numbers(&[0.0, 3.0]));
                    shading.insert("Coords", numbers(&[0.5, 0.0, 10.5, 0.0]));
                    vec![0, 0, 255, 0]
                };
                shading.insert(
                    "Function",
                    PdfObject::Stream {
                        dict: function,
                        raw,
                    },
                );
                let mut state = state(&engine);
                state
                    .resources
                    .shadings
                    .insert("Cubic".into(), PdfObject::Dictionary(shading));
                execute(&mut state, b"/Cubic sh", packed);
                let (pixel, expected) = if kind == 1 {
                    (state.buf.get_pixel(4, 5), 81)
                } else {
                    (state.buf.get_pixel(5, 5), 143)
                };
                assert!(
                    (pixel[0] as i32 - expected).abs() <= 1,
                    "kind={kind} packed={packed} pixel={pixel:?}"
                );
                assert_eq!(pixel[0], pixel[1]);
                assert_eq!(pixel[1], pixel[2]);
                assert_eq!(pixel[3], 255);
            }
        }
    }

    #[test]
    fn raw_and_compiled_analytic_shadings_use_prepared_calculator_arrays() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[])).unwrap();
        for packed in [false, true] {
            for kind in [1, 2, 3] {
                let mut function = PdfDictionary::empty();
                function.insert("FunctionType", PdfObject::Integer(4));
                function.insert(
                    "Domain",
                    numbers(&[0.0, 1.0].repeat(if kind == 1 { 2 } else { 1 })),
                );
                function.insert("Range", numbers(&[0.0, 1.0]));
                let raw = if kind == 1 {
                    b"{ pop pop .8 }".as_slice()
                } else {
                    b"{ pop .8 }".as_slice()
                };
                let mut shading = PdfDictionary::empty();
                shading.insert("ShadingType", PdfObject::Integer(kind));
                shading.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
                shading.insert(
                    "Function",
                    PdfObject::Array(vec![PdfObject::Stream {
                        dict: function,
                        raw: raw.to_vec(),
                    }]),
                );
                match kind {
                    1 => {
                        shading.insert("Domain", numbers(&[0.0, 10.0, 0.0, 10.0]));
                    }
                    2 => {
                        shading.insert("Coords", numbers(&[0.0, 0.0, 10.0, 0.0]));
                    }
                    _ => {
                        shading.insert("Coords", numbers(&[5.0, 5.0, 0.0, 5.0, 5.0, 10.0]));
                    }
                }
                let mut state = state(&engine);
                state
                    .resources
                    .shadings
                    .insert("Prepared".into(), PdfObject::Dictionary(shading));
                execute(&mut state, b"/Prepared sh", packed);
                let pixel = state.buf.get_pixel(5, 5);
                assert_eq!(pixel, [204, 204, 204, 255], "kind={kind} packed={packed}");
            }
        }
    }

    #[test]
    fn raw_and_compiled_shading_preserve_typed_calculator_results() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&[])).unwrap();
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(4));
        function.insert("Domain", numbers(&[0.0, 1.0]));
        function.insert("Range", numbers(&[0.0, 1.0]));
        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", PdfObject::Integer(2));
        shading.insert("Coords", numbers(&[0.0, 0.0, 10.0, 0.0]));
        shading.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        shading.insert("Function", PdfObject::Stream {
            dict: function,
            raw: b"{ pop -6.5 round -6 eq -1 -1 bitshift 2147483647 eq and { .75 } { .25 } ifelse }".to_vec(),
        });
        for packed in [false, true] {
            let mut state = state(&engine);
            state
                .resources
                .shadings
                .insert("Typed".into(), PdfObject::Dictionary(shading.clone()));
            execute(&mut state, b"/Typed sh", packed);
            assert_eq!(state.buf.get_pixel(5, 5), [191, 191, 191, 255]);
        }
    }

    #[test]
    fn indirect_shading_pattern_type_and_matrix_reach_fill_and_stroke() {
        let engine = ContentEngine::open_bytes(bytes_with_objects(&extra())).unwrap();
        for packed in [false, true] {
            for program in [
                b"/Pattern cs /P scn 0 0 10 10 re f".as_slice(),
                b"/Pattern CS /P SCN 2 w 1 5 m 9 5 l S".as_slice(),
            ] {
                let mut state = state(&engine);
                let mut pattern = PdfDictionary::empty();
                pattern.insert("PatternType", reference(4));
                pattern.insert("Matrix", reference(10));
                pattern.insert("Shading", PdfObject::Dictionary(shading()));
                state
                    .resources
                    .patterns
                    .insert("P".into(), PdfObject::Dictionary(pattern));
                execute(&mut state, program, packed);
                let pixel = state.buf.get_pixel(5, 5);
                assert!((pixel[0] as i32 - 140).abs() <= 1);
            }
        }
    }

    #[test]
    fn indirect_mesh_type_is_resolved_before_stream_decode_and_dispatch() {
        let mut objects = extra();
        objects.extend([
            PdfObject::Integer(5),
            PdfObject::Integer(8),
            numbers(&[0.0, 10.0, 0.0, 10.0, 0.0, 1.0]),
            PdfObject::Null,
            numbers(&[2.0, 2.0, 8.0, 8.0]),
        ]);
        let mut dict = PdfDictionary::empty();
        dict.insert("ShadingType", reference(11));
        dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        dict.insert("BitsPerCoordinate", reference(12));
        dict.insert("BitsPerComponent", reference(12));
        dict.insert("VerticesPerRow", reference(4));
        dict.insert("Decode", reference(13));
        dict.insert("Function", reference(14));
        dict.insert("BBox", reference(15));
        objects.push(PdfObject::Stream {
            dict,
            raw: vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 255, 255, 0],
        });
        objects.push(reference(16));
        let engine = ContentEngine::open_bytes(bytes_with_objects(&objects)).unwrap();
        for packed in [false, true] {
            let mut state = state(&engine);
            state.resources.shadings.insert("S".into(), reference(17));
            execute(&mut state, b"/S sh", packed);
            assert_eq!(state.buf.get_pixel(5, 5), [0, 0, 0, 255]);
            assert_eq!(state.buf.get_pixel(0, 0), WHITE);
        }
    }
}
