// Regression source only. No execution has been performed in this source-only phase.
#[test]
fn native_ocr_search_program_cannot_gain_visible_paint_when_reused() {
    assert!(validate_carrier_program(b"BT /F 10 Tf 3 Tr (OCR) Tj 0 Tr [120] TJ ET").is_ok());
    assert!(validate_carrier_program(
        b"/Span << /MCID null /ActualText (OCR) >> BDC BT /F 10 Tf 3 Tr (OCR) Tj ET EMC"
    )
    .is_ok());
    for program in [
        &b"BT /F 10 Tf 0 Tr (VISIBLE) Tj ET"[..],
        &b"/Image Do"[..],
        &b"0 0 10 10 re f"[..],
        &b"BI /W 1 /H 1 /CS /G /BPC 8 ID x EI"[..],
        &b"/Span << /MCID 0 >> BDC BT /F 10 Tf 3 Tr (OCR) Tj ET EMC"[..],
        &b"/Span /Named BDC BT /F 10 Tf 3 Tr (OCR) Tj ET EMC"[..],
        &b"/OC BMC BT /F 10 Tf 3 Tr (OCR) Tj ET EMC"[..],
    ] {
        assert!(validate_carrier_program(program).is_err());
    }
}

fn ocr_fixture(parts: &[&[u8]]) -> Vec<u8> {
    let (input, _, _) = fixture(false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut updates = Updates {
        next: reader.object_ids().iter().map(|r| r.0).max().unwrap(),
        objects: Vec::new(),
    };
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    let mut fonts = PdfDictionary::empty();
    fonts.insert("F", PdfObject::Dictionary(font));
    let mut resources = page.resources.clone();
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut contents = page
        .contents
        .iter()
        .copied()
        .map(reference)
        .collect::<Vec<_>>();
    for bytes in parts {
        contents.push(reference(
            updates.stream(bytes, PdfDictionary::empty()).unwrap(),
        ));
    }
    let mut d = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    d.insert("Resources", PdfObject::Dictionary(resources));
    d.insert("Contents", PdfObject::Array(contents));
    updates.put(
        (page.object_number, page.generation_number),
        PdfObject::Dictionary(d),
    );
    write_incremental_update(reader, updates.objects).unwrap()
}
fn ocr_request(input: &[u8]) -> ImageFragmentMove {
    let model = crate::advanced_editing::analyze_multi_run_text_range(input, 1).unwrap();
    let spans = model
        .source_spans
        .iter()
        .filter(|s| s.text_render_mode == 3)
        .collect::<Vec<_>>();
    let mut r = request(input, 0);
    r.ocr = Some(OcrCarrierSelection {
        span_ids: spans.iter().map(|s| s.span_id.clone()).collect(),
        expected_text: spans.iter().map(|s| s.source_text.as_str()).collect(),
        form_target: None,
    });
    r
}
fn search_program(input: &[u8], page_number: usize) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(page_number).unwrap();
    let mut budget = 0;
    let owners = owned_on_page(
        reader,
        &page,
        &page_buffers(reader, &page, &mut budget).unwrap(),
    )
    .unwrap();
    let group = reader
        .get_object(owners[0].form.0, owners[0].form.1)
        .unwrap();
    let resources = dict(reader, group.as_stream().unwrap().0.get("Resources")).unwrap();
    let children = dict(reader, resources.get("XObject")).unwrap();
    decode(
        reader,
        children.get("Search").unwrap().as_reference().unwrap(),
    )
    .unwrap()
}

#[test]
fn native_ocr_move_keeps_visible_duplicate_and_untouched_following_advance() {
    let input =
        ocr_fixture(&[b"BT /F 10 Tf 1 0 0 1 15 15 Tm 3 Tr (OLD) Tj 0 Tr (OLD) Tj (TAIL) Tj ET"]);
    let before = ContentEngine::open_bytes(input.clone())
        .unwrap()
        .collect_page_text_chunks(1)
        .unwrap();
    let tail = before.iter().find(|c| c.text == "TAIL").unwrap();
    let (output, report) = apply(&input, &ocr_request(&input));
    assert_eq!(report.preview.ocr_spans, 1);
    assert_eq!(report.preview.ocr_source_text, "OLD");
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let source = engine.collect_page_text_chunks(1).unwrap();
    assert!(!source.iter().any(|c| c.is_invisible && !c.text.is_empty()));
    assert_eq!(source.iter().filter(|c| c.text == "OLD").count(), 1);
    let after = source.iter().find(|c| c.text == "TAIL").unwrap();
    assert!((tail.x - after.x).abs() < 1e-6 && (tail.y - after.y).abs() < 1e-6);
    let target = engine.collect_page_text_chunks(2).unwrap();
    assert_eq!(
        target
            .iter()
            .filter(|c| c.text == "OLD" && c.is_invisible)
            .count(),
        1
    );
    assert!(!target.iter().any(|c| c.text == "TAIL"));
    // Original origin 15,15; image origin 10,10; destination origin 40,60;
    // both dimensions are scaled by two in the existing request fixture.
    let moved = target.iter().find(|c| c.text == "OLD").unwrap();
    assert!((moved.x - 50.0).abs() < 1e-6 && (moved.y - 70.0).abs() < 1e-6);
    assert!(search_program(&output, 2).windows(5).any(|w| w == b"(OLD)"));
}

#[test]
fn native_ocr_moves_complete_cross_stream_actualtext_owner_atomically() {
    let input=ocr_fixture(&[
        b"/Span << /ActualText (SEARCH WORDS) /Reviewed true /Unused null >> BDC BT /F 10 Tf 3 Tr 1 0 0 1 12 16 Tm (AB) Tj",
        b"(CD) Tj ET EMC",
    ]);
    let r = ocr_request(&input);
    assert_eq!(r.ocr.as_ref().unwrap().expected_text, "ABCD");
    let mut partial = r.clone();
    partial.ocr = Some(OcrCarrierSelection {
        span_ids: vec![r.ocr.as_ref().unwrap().span_ids[0].clone()],
        expected_text: "AB".into(),
        form_target: None,
    });
    assert!(preview_image_fragment_move(&input, &partial).is_err());
    let (output, report) = apply(&input, &r);
    assert_eq!(report.preview.ocr_spans, 2);
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert!(!engine.get_page_text(1).unwrap().contains("SEARCH WORDS"));
    assert!(engine.get_page_text(2).unwrap().contains("SEARCH WORDS"));
    let program = String::from_utf8(search_program(&output, 2)).unwrap();
    assert!(program.contains("/ActualText (SEARCH WORDS)"));
    assert!(program.contains("(AB) Tj") && program.contains("(CD) Tj"));
}

#[test]
fn native_ocr_repeat_move_reuses_group_and_preserves_transformed_program() {
    let input =
        ocr_fixture(&[b"q 1 0.2 0.1 1 3 4 cm BT /F 9 Tf 3 Tr 0 1 -1 0 13 14 Tm (ROTATED) Tj ET Q"]);
    let (mut output, mut report) = apply(&input, &ocr_request(&input));
    let captured = search_program(&output, 2);
    assert!(String::from_utf8_lossy(&captured).contains("0 1 -1 0 13 14 Tm"));
    for page in [1, 2, 1] {
        let r = ImageFragmentMove {
            input_sha256: hash(&output),
            source: ImageFragmentSource::Owned {
                binding: report.binding.clone(),
            },
            target_page: page,
            target_rect: [40.0, 60.0, 100.0, 100.0],
            stack: ImageFragmentStack::Foreground,
            ocr: None,
            signature_policy_override: false,
        };
        (output, report) = apply(&output, &r);
        assert_eq!(search_program(&output, page), captured);
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let total = (1..=2)
            .map(|p| {
                engine
                    .collect_page_text_chunks(p)
                    .unwrap()
                    .iter()
                    .filter(|c| c.text == "ROTATED")
                    .count()
            })
            .sum::<usize>();
        assert_eq!(total, 1);
    }
}

#[test]
fn native_ocr_selection_rejects_visible_stale_duplicate_and_unapproved_changes() {
    let input = ocr_fixture(&[b"BT /F 10 Tf 3 Tr (SAME) Tj 0 Tr (SAME) Tj ET"]);
    let r = ocr_request(&input);
    let preview = preview_image_fragment_move(&input, &r).unwrap();
    let model = crate::advanced_editing::analyze_multi_run_text_range(&input, 1).unwrap();
    let visible = model
        .source_spans
        .iter()
        .find(|s| s.text_render_mode == 0)
        .unwrap();
    let mut bad = r.clone();
    bad.ocr.as_mut().unwrap().span_ids = vec![visible.span_id.clone()];
    assert!(preview_image_fragment_move(&input, &bad).is_err());
    bad = r.clone();
    bad.ocr.as_mut().unwrap().span_ids.push("p1:s99:o99".into());
    assert!(preview_image_fragment_move(&input, &bad).is_err());
    bad = r.clone();
    bad.ocr.as_mut().unwrap().expected_text = "WRONG".into();
    assert!(preview_image_fragment_move(&input, &bad).is_err());
    bad = r.clone();
    let id = bad.ocr.as_ref().unwrap().span_ids[0].clone();
    bad.ocr.as_mut().unwrap().span_ids.push(id);
    assert!(preview_image_fragment_move(&input, &bad).is_err());
    bad = r;
    bad.ocr = None;
    assert!(apply_image_fragment_move(&input, &bad, &preview.plan_sha256).is_err());
}

#[test]
fn native_ocr_clipping_and_optional_content_are_not_silently_detached() {
    for text in [
        &b"BT /F 10 Tf 4 Tr (CLIP) Tj 3 Tr (OCR) Tj ET"[..],
        &b"/OC /Layer BDC BT /F 10 Tf 3 Tr (OCR) Tj ET EMC"[..],
    ] {
        let input = ocr_fixture(&[text]);
        assert!(preview_image_fragment_move(&input, &ocr_request(&input)).is_err());
    }
}

#[test]
fn native_ocr_vertical_removal_preserves_signed_spacing_and_ignores_tw_for_cids() {
    let input = ocr_fixture(&[
        b"BT /F 10 Tf 2 Tc 4 Tw 50 Tz 1 0 0 1 15 100 Tm 3 Tr <00010020> Tj 0 Tr <0002> Tj ET",
    ]);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut updates = Updates {
        next: reader.object_ids().iter().map(|r| r.0).max().unwrap(),
        objects: Vec::new(),
    };
    let cmap=updates.stream(b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /WFVerticalUnicode def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange 3 beginbfchar <0001> <0041> <0020> <0020> <0002> <0042> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end",PdfDictionary::empty()).unwrap();
    let mut descendant = PdfDictionary::empty();
    descendant.insert("Type", PdfObject::Name("Font".into()));
    descendant.insert("Subtype", PdfObject::Name("CIDFontType2".into()));
    descendant.insert("BaseFont", PdfObject::Name("VerticalFixture".into()));
    descendant.insert("DW", PdfObject::Integer(1000));
    descendant.insert(
        "DW2",
        PdfObject::Array(vec![PdfObject::Integer(880), PdfObject::Integer(-1000)]),
    );
    let mut system = PdfDictionary::empty();
    system.insert("Registry", PdfObject::String(b"Adobe".to_vec()));
    system.insert("Ordering", PdfObject::String(b"Identity".to_vec()));
    system.insert("Supplement", PdfObject::Integer(0));
    descendant.insert("CIDSystemInfo", PdfObject::Dictionary(system));
    let mut descriptor = PdfDictionary::empty();
    descriptor.insert("Type", PdfObject::Name("FontDescriptor".into()));
    descriptor.insert("FontName", PdfObject::Name("VerticalFixture".into()));
    descriptor.insert("FontBBox", array([0.0, -200.0, 1000.0, 1000.0]));
    for (name, value) in [
        ("Flags", 4),
        ("ItalicAngle", 0),
        ("Ascent", 1000),
        ("Descent", -200),
        ("CapHeight", 800),
        ("StemV", 80),
    ] {
        descriptor.insert(name, PdfObject::Integer(value));
    }
    descendant.insert("FontDescriptor", PdfObject::Dictionary(descriptor));
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type0".into()));
    font.insert("BaseFont", PdfObject::Name("VerticalFixture".into()));
    font.insert("Encoding", PdfObject::Name("Identity-V".into()));
    font.insert("ToUnicode", reference(cmap));
    font.insert(
        "DescendantFonts",
        PdfObject::Array(vec![PdfObject::Dictionary(descendant)]),
    );
    let mut fonts = PdfDictionary::empty();
    fonts.insert("F", PdfObject::Dictionary(font));
    let mut resources = page.resources.clone();
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut d = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    d.insert("Resources", PdfObject::Dictionary(resources));
    updates.put(
        (page.object_number, page.generation_number),
        PdfObject::Dictionary(d),
    );
    let input = write_incremental_update(reader, updates.objects).unwrap();
    let before = ContentEngine::open_bytes(input.clone())
        .unwrap()
        .collect_page_text_chunks(1)
        .unwrap();
    let tail = before.iter().find(|c| c.text == "B").unwrap();
    assert!((tail.y - 84.0).abs() < 1e-6);
    let (output, report) = apply(&input, &ocr_request(&input));
    assert_eq!(report.preview.ocr_source_text, "A ");
    let engine = ContentEngine::open_bytes(output).unwrap();
    let after = engine.collect_page_text_chunks(1).unwrap();
    let retained = after.iter().find(|c| c.text == "B").unwrap();
    assert!((retained.x - tail.x).abs() < 1e-6 && (retained.y - tail.y).abs() < 1e-6);
}
