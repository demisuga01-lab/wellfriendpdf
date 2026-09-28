//! Source regressions only; none was executed during implementation.
use super::*;
use crate::PdfDictionary;

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}
fn fixture(content: &[u8], collision: bool) -> (Vec<u8>, u32) {
    let mut builder = crate::PdfBuilder::new();
    builder.add_page(crate::AuthorPageSize::custom(200.0, 200.0));
    let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let base = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name("Courier".into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    let mut gs = PdfDictionary::empty();
    gs.insert("Type", PdfObject::Name("ExtGState".into()));
    gs.insert("Font", reference(base + 2)); // array and size may both be indirect
    gs.insert("ca", PdfObject::Real(0.5));
    let mut resources = PdfDictionary::empty();
    let mut states = PdfDictionary::empty();
    states.insert("G", reference(base + 1));
    resources.insert("ExtGState", PdfObject::Dictionary(states));
    if collision {
        let mut other = font.clone();
        other.insert("BaseFont", PdfObject::Name("Helvetica".into()));
        let mut fonts = PdfDictionary::empty();
        fonts.insert(format!("WFExtFont{base}_0"), PdfObject::Dictionary(other));
        resources.insert("Font", PdfObject::Dictionary(fonts));
    }
    let mut dictionary = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    dictionary.insert("Resources", PdfObject::Dictionary(resources));
    dictionary.insert("Contents", reference(base + 4));
    let mut stream = PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(content.len() as i64));
    let objects = vec![
        (base, PdfObject::Dictionary(font)),
        (base + 1, PdfObject::Dictionary(gs)),
        (
            base + 2,
            PdfObject::Array(vec![reference(base), reference(base + 3)]),
        ),
        (base + 3, PdfObject::Integer(10)),
        (
            base + 4,
            PdfObject::Stream {
                dict: stream,
                raw: content.to_vec(),
            },
        ),
    ];
    let mut updates = objects
        .into_iter()
        .map(|(number, object)| IncrementalObject {
            number,
            generation: 0,
            object,
        })
        .collect::<Vec<_>>();
    updates.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(dictionary),
    });
    (write_incremental_update(reader, updates).unwrap(), base)
}
fn parsed(input: &[u8]) -> (ContentEngine, PageResources) {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let resources = PageResources::from_dict(&page.resources, engine.document().reader());
    (engine, resources)
}
fn gs_font(resources: &PageResources) -> &str {
    resources.ext_g_states["G"]
        .get("Font")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_name()
        .unwrap()
}

#[test]
fn ext_only_font_normalization_resolves_indirect_array_size_and_name_collisions() {
    let (input, base) = fixture(b"BT /G gs (AB) Tj ET", true);
    let (engine, resources) = parsed(&input);
    let name = gs_font(&resources);
    assert_ne!(name, format!("WFExtFont{base}_0"));
    assert_eq!(resources.font_references[name], (base, 0));
    assert_eq!(resources.fonts[name].get_name("BaseFont"), Some("Courier"));
    assert_eq!(
        resources.ext_g_states["G"]
            .get("Font")
            .unwrap()
            .as_array()
            .unwrap()[1]
            .as_number(),
        Some(10.0)
    );
    let source = engine.document().reader().get_object(base + 1, 0).unwrap();
    assert_eq!(
        source
            .as_dict()
            .unwrap()
            .get("Font")
            .unwrap()
            .as_reference(),
        Some((base + 2, 0))
    );
    let page = engine.document().get_page(1).unwrap();
    assert_eq!(
        gs_font(&PageResources::from_dict(
            &page.resources,
            engine.document().reader()
        )),
        name
    );
}

#[test]
fn existing_font_alias_choice_is_sorted_and_does_not_create_more_aliases() {
    let (input, base) = fixture(b"", false);
    let (engine, _) = parsed(&input);
    let mut resources = engine.document().get_page(1).unwrap().resources;
    let mut fonts = PdfDictionary::empty();
    fonts.insert("Z", reference(base));
    fonts.insert("A", reference(base));
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let parsed = PageResources::from_dict(&resources, engine.document().reader());
    assert_eq!(gs_font(&parsed), "A");
    assert_eq!(parsed.fonts.len(), 2);
    assert!(!crate::ext_gstate_fonts::materialize(
        &parsed,
        &mut resources,
        engine.document().reader()
    )
    .unwrap());
}

#[test]
fn extraction_and_source_analysis_see_text_selected_only_by_gs() {
    let (input, _) = fixture(b"BT /G gs 1 0 0 1 20 150 Tm (AA) Tj (BB) Tj ET", false);
    let (engine, resources) = parsed(&input);
    let chunks = engine.collect_page_text_chunks(1).unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
        "AABB"
    );
    assert!((chunks[1].x - 32.0).abs() < 1e-7);
    let model = analyze_multi_run_text_range(&input, 1).unwrap();
    assert_eq!(model.logical_text, "AABB");
    assert!(model
        .source_spans
        .iter()
        .all(|s| s.font_resource == gs_font(&resources)));
}

#[test]
fn same_width_gs_only_edit_keeps_source_operands_and_reopens() {
    let (input, _) = fixture(b"BT /G gs 1 0 0 1 20 150 Tm (AA) Tj (BB) Tj ET", false);
    let (output, _) = apply_same_width_patch(&input, 1, "AA", "CC", &Default::default()).unwrap();
    let model = analyze_multi_run_text_range(&output, 1).unwrap();
    assert_eq!(model.logical_text, "CCBB");
    assert!(output.starts_with(&input));
}

#[test]
fn generated_inline_gs_font_edit_persists_alias_and_restores_following_source() {
    let (input, base) = fixture(b"BT /G gs 1 0 0 1 20 150 Tm 7 Tr (AA) Tj (BB) Tj ET", true);
    let (_, resources) = parsed(&input);
    let alias = gs_font(&resources).to_owned();
    let request = MultiRunTextRangeRequest {
        page: 1,
        logical_start: 0,
        logical_end: 2,
        replacement_text: "XYZ".into(),
        mode: AdvancedTextMode::ParagraphReflowVertical,
        style_policy: MultiRunStylePolicy::InheritLeading,
        options: AdvancedTextEditOptions {
            region: [10.0, 10.0, 180.0, 180.0],
            font_size: 10.0,
            ..Default::default()
        },
        final_lines: None,
    };
    let (output, _) = edit_multi_run_text_range(
        &input,
        &request,
        crate::render::get_fallback_font("Helvetica"),
    )
    .unwrap();
    let (engine, resources) = parsed(&output);
    let page = engine.document().get_page(1).unwrap();
    let fonts = engine
        .document()
        .reader()
        .resolve(page.resources.get("Font").unwrap().clone())
        .unwrap();
    assert_eq!(
        fonts.as_dict().unwrap().get(&alias).unwrap().as_reference(),
        Some((base, 0))
    );
    let following = engine
        .collect_page_text_chunks(1)
        .unwrap()
        .into_iter()
        .find(|c| c.text == "BB")
        .unwrap();
    assert!((following.x - 32.0).abs() < 1e-7);
    assert!((following.y - 150.0).abs() < 1e-7);
    assert_eq!(following.font_name, alias);
    assert_eq!(
        resources.fonts[&alias].get_name("BaseFont"),
        Some("Courier")
    );
    let model = analyze_multi_run_text_range(&output, 1).unwrap();
    assert!(model.logical_text.contains("XYZ") && model.logical_text.ends_with("BB"));
    assert!(output.starts_with(&input));
}

#[test]
fn output_alias_materialization_is_idempotent_and_never_overwrites_a_font() {
    let (input, base) = fixture(b"", false);
    let (engine, parsed) = parsed(&input);
    let reader = engine.document().reader();
    let mut resources = engine.document().get_page(1).unwrap().resources;
    assert!(crate::ext_gstate_fonts::materialize(&parsed, &mut resources, reader).unwrap());
    assert!(!crate::ext_gstate_fonts::materialize(&parsed, &mut resources, reader).unwrap());
    let name = gs_font(&parsed);
    resources
        .get_mut("Font")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .insert(name, reference(base + 1));
    assert!(crate::ext_gstate_fonts::materialize(&parsed, &mut resources, reader).is_err());
}

#[test]
fn invisible_gs_carrier_capture_keeps_original_font_and_removes_only_selected_source() {
    let (input, _) = fixture(b"BT /G gs 3 Tr 1 0 0 1 20 150 Tm (AA) Tj (BB) Tj ET", false);
    let (engine, _) = parsed(&input);
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let model = analyze_multi_run_text_range(&input, 1).unwrap();
    let buffers = page
        .contents
        .iter()
        .map(|&(number, generation)| {
            decode_stream_lossless_with_limits(
                &reader.get_object(number, generation).unwrap(),
                reader,
                &DecodeLimits::default(),
            )
            .unwrap()
            .data
        })
        .collect::<Vec<_>>();
    let captured = ocr_carriers::capture(
        &input,
        &engine,
        &page,
        &buffers,
        &ocr_carriers::OcrCarrierSelection {
            span_ids: vec![model.source_spans[0].span_id.clone()],
            expected_text: "AA".into(),
            form_target: None,
        },
        false,
    )
    .unwrap();
    assert_eq!(captured.logical_ranges, vec![[0, 2]]);
    assert!(String::from_utf8_lossy(&captured.program).contains("/G gs"));
    let replace = |raw: Vec<u8>| {
        let (number, generation) = page.contents[0];
        let mut dict = PdfDictionary::empty();
        dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        write_incremental_update(
            reader,
            vec![IncrementalObject {
                number,
                generation,
                object: PdfObject::Stream { dict, raw },
            }],
        )
        .unwrap()
    };
    let carrier = ContentEngine::open_bytes(replace(captured.program)).unwrap();
    let chunks = carrier.collect_page_text_chunks(1).unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
        "AA"
    );
    assert!((chunks[0].x - 20.0).abs() < 1e-7);
    let remaining =
        ocr_carriers::apply_patches(&buffers[0], captured.source_edits[&0].clone()).unwrap();
    let source = ContentEngine::open_bytes(replace(remaining)).unwrap();
    let chunks = source.collect_page_text_chunks(1).unwrap();
    assert_eq!(
        chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
        "BB"
    );
    assert!((chunks[0].x - 32.0).abs() < 1e-7);
}
