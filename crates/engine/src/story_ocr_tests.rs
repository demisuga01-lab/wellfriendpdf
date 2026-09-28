// Source-only integration regressions. Not executed in this implementation phase.
fn story_ocr_fixture() -> Vec<u8> {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let number = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    let mut resources = page.resources.clone();
    let mut fonts = match resources.get("Font").cloned() {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference { number, generation }) => reader
            .get_object(number, generation)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone(),
        _ => PdfDictionary::empty(),
    };
    fonts.insert("OCR", PdfObject::Dictionary(font));
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let raw=b"q /Span << /ActualText (SEARCH_A) /Reviewed true >> BDC BT /OCR 3 Tf 3 Tr 1 0 0 1 20 110 Tm (SCAN_A) Tj ET EMC /Span << /ActualText (SEARCH_B) >> BDC BT /OCR 3 Tf 3 Tr 1 0 0 1 80 110 Tm (SCAN_B) Tj ET EMC Q".to_vec();
    let mut stream = PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(raw.len() as i64));
    let mut contents = vec![reference(number)];
    contents.extend(page.contents.iter().map(|(n, g)| PdfObject::Reference {
        number: *n,
        generation: *g,
    }));
    let mut d = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    d.insert("Resources", PdfObject::Dictionary(resources));
    d.insert("Contents", PdfObject::Array(contents));
    write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream { dict: stream, raw },
            },
            IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(d),
            },
        ],
    )
    .unwrap()
}
fn story_ocr_request(input: &[u8]) -> LinkedStoryRequest {
    let mut r = with_figures(input);
    let model = analyze_multi_run_text_range(input, 1).unwrap();
    let caption = model
        .source_spans
        .iter()
        .filter(|s| s.text_render_mode == 0)
        .collect::<Vec<_>>();
    assert_eq!(
        caption.iter().map(|s| s.text.as_str()).collect::<String>(),
        "OLD"
    );
    r.frames[0].logical_range = [
        caption.first().unwrap().logical_range[0],
        caption.last().unwrap().logical_range[1],
    ];
    let invisible = model
        .source_spans
        .iter()
        .filter(|s| s.text_render_mode == 3)
        .collect::<Vec<_>>();
    assert_eq!(invisible.len(), r.figures.len());
    for (figure, span) in r.figures.iter_mut().zip(invisible) {
        figure.ocr = Some(crate::advanced_editing::ocr_carriers::OcrCarrierSelection {
            span_ids: vec![span.span_id.clone()],
            expected_text: span.source_text.clone(),
            form_target: None,
        });
    }
    r
}

#[test]
fn story_ocr_ranges_use_unicode_scalars_not_utf8_bytes_or_utf16_offsets() {
    let input = story_ocr_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let number = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut resources = page.resources.clone();
    let mut fonts = resources.get("Font").unwrap().as_dict().unwrap().clone();
    let mut font = fonts.get("OCR").unwrap().as_dict().unwrap().clone();
    font.insert("ToUnicode", reference(number));
    fonts.insert("OCR", PdfObject::Dictionary(font));
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let raw=b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /OCRUnicode def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <53> <D83DDE000041> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end".to_vec();
    let mut stream = PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(raw.len() as i64));
    let mut pd = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    pd.insert("Resources", PdfObject::Dictionary(resources));
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream { dict: stream, raw },
            },
            IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(pd),
            },
        ],
    )
    .unwrap();
    let request = story_ocr_request(&input);
    assert!(request.figures[0]
        .ocr
        .as_ref()
        .unwrap()
        .expected_text
        .starts_with("\u{1f600}A"));
    // Page-logical selection is governed by the two eight-scalar ActualText
    // values; the OCR carrier itself remains bound to the source-decoded
    // ToUnicode value through `source_text`.
    assert_eq!(request.frames[0].logical_range, [16, 19]);
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    assert_eq!(story_word_count(&output, "First caption"), 1);
    assert_eq!(story_word_count(&output, "SEARCH_A"), 1);
    assert_eq!(story_word_count(&output, "SEARCH_B"), 1);
}
fn story_word_count(input: &[u8], word: &str) -> usize {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    (1..=engine.page_count().unwrap())
        .map(|page| engine.get_page_text(page).unwrap().matches(word).count())
        .sum()
}

#[test]
fn story_ocr_rebases_caption_after_batch_removal_and_survives_page_insertion() {
    let input = story_ocr_fixture();
    let request = story_ocr_request(&input);
    assert_eq!(request.frames[0].logical_range, [16, 19]);
    let (output, report) = apply_linked_story(&input, &request).unwrap();
    assert_eq!(report.generated_pages, 1);
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(engine.page_count().unwrap(), 3);
    assert!(engine.get_page_text(1).unwrap().contains("SEARCH_A"));
    assert!(engine.get_page_text(2).unwrap().contains("SEARCH_B"));
    assert!(engine.get_page_text(1).unwrap().contains("First caption"));
    assert!(engine.get_page_text(2).unwrap().contains("Second caption"));
    assert!(engine.get_page_text(3).unwrap().contains("OLD"));
    assert_eq!(story_word_count(&output, "SEARCH_A"), 1);
    assert_eq!(story_word_count(&output, "SEARCH_B"), 1);
    let page_model = analyze_multi_run_text_range(&output, 1).unwrap();
    assert!(
        !page_model.logical_text.contains("SCAN_A") && !page_model.logical_text.contains("SCAN_B")
    );
    assert!(!engine
        .document()
        .get_catalog()
        .unwrap()
        .contains_key("WFStagedStoryFigures"));
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(saved
        .figures
        .iter()
        .all(|f| f.ocr.is_none() && !f.ocr_unrelated));
    saved.paragraphs[1].break_before = false;
    let (contracted, _) = apply_linked_story(&output, &saved).unwrap();
    assert_eq!(story_word_count(&contracted, "SEARCH_A"), 1);
    assert_eq!(story_word_count(&contracted, "SEARCH_B"), 1);
    let owners = crate::image_fragments::image_fragment_bindings(&contracted).unwrap();
    assert_eq!(owners.len(), 2);
    assert!(owners.iter().all(|b| b.page == 1));
}

#[test]
fn story_ocr_owned_group_deletion_removes_search_and_consumes_commands() {
    let input = story_ocr_fixture();
    let request = story_ocr_request(&input);
    let (saved, _) = apply_linked_story(&input, &request).unwrap();
    let mut request = load_linked_stories(&saved).unwrap().remove(0).request;
    let removal = remove_figure(&mut request, 0);
    let (output, _) = apply_linked_story(&saved, &request).unwrap();
    assert_eq!(story_word_count(&output, "SEARCH_A"), 0);
    assert_eq!(story_word_count(&output, "SEARCH_B"), 1);
    assert_eq!(story_word_count(&output, "First caption"), 1);
    assert!(crate::image_fragments::image_fragment_bindings(&output)
        .unwrap()
        .iter()
        .all(|b| b.key != removal.binding.key));
    let next = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(next.figure_removals.is_empty());
    assert!(next.figures.iter().all(|f| f.ocr.is_none()));
    let (again, _) = apply_linked_story(&output, &next).unwrap();
    assert_eq!(story_word_count(&again, "SEARCH_A"), 0);
    assert_eq!(story_word_count(&again, "SEARCH_B"), 1);
}

#[test]
fn story_ocr_requires_disjoint_text_and_image_ownership_on_original_revision() {
    let input = story_ocr_fixture();
    let r = story_ocr_request(&input);
    let mut duplicate = r.clone();
    duplicate.figures[1].ocr = duplicate.figures[0].ocr.clone();
    assert!(figures::validate(&input, &duplicate)
        .unwrap_err()
        .to_string()
        .contains("multiple figures"));
    let mut overlap = r.clone();
    overlap.frames[0].logical_range = [0, 6];
    overlap.frames[0].expected_text = "SCAN_A".into();
    assert!(figures::validate(&input, &overlap)
        .unwrap_err()
        .to_string()
        .contains("both to OCR"));
    let mut stale = r.clone();
    stale.figures[0].ocr.as_mut().unwrap().expected_text = "wrong".into();
    assert!(figures::validate(&input, &stale).is_err());
    let mut missing = r.clone();
    missing.figures[0].ocr = None;
    assert!(figures::validate(&input, &missing).is_err());
    let mut incompatible = r;
    incompatible.figures[0].ocr_unrelated = true;
    assert!(figures::validate(&input, &incompatible).is_err());
}

#[test]
fn story_ocr_explicit_unrelated_decision_leaves_that_carrier_on_source_page() {
    let input = story_ocr_fixture();
    let mut request = story_ocr_request(&input);
    request.figures[1].ocr = None;
    request.figures[1].ocr_unrelated = true;
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert!(engine.get_page_text(1).unwrap().contains("SEARCH_B"));
    assert!(!engine.get_page_text(2).unwrap().contains("SEARCH_B"));
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(saved.figures.iter().all(|f| !f.ocr_unrelated));
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    assert_eq!(story_word_count(&again, "SEARCH_B"), 1);
}

#[test]
fn story_ocr_receipt_undo_and_redo_preserve_exact_revisions() {
    let input = story_ocr_fixture();
    let mut r = story_ocr_request(&input);
    let mut session = LinkedStorySession::open(input.clone()).unwrap();
    let cancel = crate::cancel::CancelToken::none();
    session.preview(&r, &cancel).unwrap();
    let old = session.preview_receipt().unwrap();
    r.figures[0].ocr = None;
    r.figures[0].ocr_unrelated = true;
    assert!(session.checkpoint_approved(&r, &old, &cancel).is_err());
    assert_eq!(session.bytes(), input.as_slice());
    session.preview(&r, &cancel).unwrap();
    let approval = session.preview_receipt().unwrap();
    session.checkpoint_approved(&r, &approval, &cancel).unwrap();
    let saved = session.bytes().to_vec();
    assert_eq!(story_word_count(&saved, "SEARCH_A"), 1);
    assert_eq!(story_word_count(&saved, "SEARCH_B"), 1);
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), input.as_slice());
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), saved.as_slice());
    let mut replay = session.saved_stories().unwrap().remove(0).request;
    replay.figures[0].ocr_unrelated = true;
    assert!(figures::validate(session.bytes(), &replay).is_err());
}
