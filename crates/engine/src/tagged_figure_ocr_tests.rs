// Unexecuted regressions for whole-Figure OCR migration, not visual qualification.
fn whole_figure_ocr_fixture(
    nested_actual: bool,
    split_stream: bool,
    separate_owner: bool,
) -> (Vec<u8>, LinkedStoryRequest) {
    let (input, mut request) = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let mut store = Store::new(engine.document().reader());
    let mut font = PdfDictionary::empty();
    for (k, v) in [
        ("Type", "Font"),
        ("Subtype", "Type1"),
        ("BaseFont", "Helvetica"),
        ("Encoding", "WinAnsiEncoding"),
    ] {
        font.insert(k, PdfObject::Name(v.into()));
    }
    let mut resources = page.resources.clone();
    let mut fonts = store
        .resolve(resources.get("Font").unwrap())
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    fonts.insert("OCR", PdfObject::Dictionary(font));
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut image_text = Vec::new();
    for (index, word) in ["A", "B"].iter().enumerate() {
        let mcid = index + 1;
        if separate_owner {
            image_text.extend(format!("/Figure << /MCID {mcid} >> BDC\n").into_bytes());
            image_text.extend(
                format!(
                    "q 20 0 0 20 {} 100 cm /Figure Do Q\nEMC\n/Span << /MCID {} /ActualText (READ_{word}) >> BDC\n",
                    20 + 60 * index,
                    index + 3
                )
                .into_bytes(),
            );
        } else if nested_actual {
            image_text.extend(
                format!(
                    "/Figure << /MCID {mcid} >> BDC /Span << /ActualText (READ_{word}) >> BDC\n"
                )
                .into_bytes(),
            );
        } else {
            image_text.extend(
                format!("/Figure << /MCID {mcid} /ActualText (READ_{word}) >> BDC\n").into_bytes(),
            );
        }
        if separate_owner {
            image_text.extend(
                format!(
                    "q BT /OCR 8 Tf 3 Tr 1 0 0 1 {} 106 Tm (SCAN_{word}) Tj ET Q\nEMC\n",
                    22 + 60 * index
                )
                .into_bytes(),
            );
        } else {
            image_text.extend(format!("q 20 0 0 20 {} 100 cm /Figure Do Q\nq BT /OCR 8 Tf 3 Tr 1 0 0 1 {} 106 Tm (SCAN_{word}) Tj ET Q\nEMC\n",
                20+60*index,22+60*index).into_bytes());
        }
        if nested_actual && !separate_owner {
            image_text.extend_from_slice(b"EMC\n");
        }
    }
    let mut contents = Vec::new();
    if split_stream {
        let split = image_text.windows(2).position(|p| p == b"BT").unwrap();
        contents.push(reference(stream(&mut store, image_text[..split].to_vec())));
        contents.push(reference(stream(&mut store, image_text[split..].to_vec())));
    } else {
        contents.push(reference(stream(&mut store, image_text)));
    }
    // OCR precedes the independently owned caption: source scalar rebinding
    // must run after detachment, not merely after MCID cleanup.
    contents.push(reference(page.contents[0]));
    let mut pd = store
        .dict((page.object_number, page.generation_number))
        .unwrap();
    pd.insert("Contents", PdfObject::Array(contents));
    pd.insert("Resources", PdfObject::Dictionary(resources));
    store
        .replace_dict((page.object_number, page.generation_number), pd)
        .unwrap();
    let mut separate_owners = Vec::new();
    if separate_owner {
        let parent_ref = request.source_tags.as_ref().unwrap().parent.clone();
        let parent = (parent_ref.object, parent_ref.generation);
        for (index, mcid) in [3, 4].into_iter().enumerate() {
            let mut owner = PdfDictionary::empty();
            owner.insert("Type", PdfObject::Name("StructElem".into()));
            owner.insert("S", PdfObject::Name("Span".into()));
            owner.insert("P", reference(parent));
            owner.insert(
                "ID",
                PdfObject::String(format!("ocr-owner-{index}").into_bytes()),
            );
            owner.insert(
                "Pg",
                reference((page.object_number, page.generation_number)),
            );
            owner.insert("K", PdfObject::Integer(mcid));
            separate_owners.push(store.add(PdfObject::Dictionary(owner)).unwrap());
        }
        let mut dictionary = store.dict(parent).unwrap();
        let old = kids(&dictionary);
        assert_eq!(old.len(), 4);
        dictionary.insert(
            "K",
            PdfObject::Array(vec![
                old[0].clone(),
                reference(separate_owners[0]),
                old[1].clone(),
                old[2].clone(),
                reference(separate_owners[1]),
                old[3].clone(),
            ]),
        );
        store.replace_dict(parent, dictionary).unwrap();
    }
    let raw = write_store(store).unwrap();
    let input = if separate_owner {
        rebuild_owner_trees(&raw, None).unwrap().0
    } else {
        raw
    };
    validate_parent_tree(&input).unwrap();
    request.input_sha256 = format!("{:x}", Sha256::digest(&input));
    let model = crate::advanced_editing::analyze_multi_run_text_range(&input, 1).unwrap();
    let caption = model
        .source_spans
        .iter()
        .filter(|s| s.text_render_mode == 0)
        .collect::<Vec<_>>();
    assert_eq!(
        caption.iter().map(|s| s.text.as_str()).collect::<String>(),
        "OLD"
    );
    request.frames[0].logical_range = [
        caption.first().unwrap().logical_range[0],
        caption.last().unwrap().logical_range[1],
    ];
    let invisible = model
        .source_spans
        .iter()
        .filter(|s| s.text_render_mode == 3)
        .collect::<Vec<_>>();
    let images = crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
    assert_eq!(images.len(), 2);
    assert_eq!(invisible.len(), 2);
    for ((figure, image), span) in request.figures.iter_mut().zip(images).zip(invisible) {
        figure.source = ImageFragmentSource::Occurrence {
            page: 1,
            content_stream_index: image.content_stream_index,
            occurrence_id: image.occurrence_id,
        };
        figure.ocr = Some(crate::advanced_editing::ocr_carriers::OcrCarrierSelection {
            span_ids: vec![span.span_id.clone()],
            expected_text: span.source_text.clone(),
            form_target: None,
        });
    }
    if separate_owner {
        let tags = request.source_tags.as_mut().unwrap();
        let old = tags.selected.clone();
        tags.selected = vec![
            old[0].clone(),
            TagReference {
                object: separate_owners[0].0,
                generation: separate_owners[0].1,
                key: None,
            },
            old[1].clone(),
            old[2].clone(),
            TagReference {
                object: separate_owners[1].0,
                generation: separate_owners[1].1,
                key: None,
            },
            old[3].clone(),
        ];
        for (index, figure) in request.figures.iter().enumerate() {
            tags.figures.get_mut(&figure.id).unwrap().separate_ocr_owner =
                Some(FigureOcrOwnerBinding {
                    source: TagReference {
                        object: separate_owners[index].0,
                        generation: separate_owners[index].1,
                        key: None,
                    },
                    policy: FigureOcrOwnerPolicy::MergeIntoFigure,
                    span_ids: Vec::new(),
                });
        }
    }
    (input, request)
}

fn combine_separate_ocr_owners(mut request: LinkedStoryRequest) -> LinkedStoryRequest {
    let first_id = request.figures[0].id.clone();
    let second_id = request.figures[1].id.clone();
    let second = request.figures[1]
        .ocr
        .take()
        .expect("second exact OCR selection");
    request.figures[1].ocr_unrelated = true;
    let first = request.figures[0]
        .ocr
        .as_mut()
        .expect("first exact OCR selection");
    let first_span = first.span_ids[0].clone();
    let second_span = second.span_ids[0].clone();
    first.span_ids.extend(second.span_ids);
    first.expected_text.push_str(&second.expected_text);

    let tags = request.source_tags.as_mut().expect("tagged request");
    let first_owner = tags
        .figures
        .get_mut(&first_id)
        .and_then(|binding| binding.separate_ocr_owner.take())
        .expect("first separate OCR owner");
    let second_owner = tags
        .figures
        .get_mut(&second_id)
        .and_then(|binding| binding.separate_ocr_owner.take())
        .expect("second separate OCR owner");
    tags.figures
        .get_mut(&first_id)
        .expect("first Figure binding")
        .separate_ocr_owners = vec![
        FigureOcrOwnerBinding {
            source: first_owner.source,
            policy: first_owner.policy,
            span_ids: vec![first_span],
        },
        FigureOcrOwnerBinding {
            source: second_owner.source,
            policy: second_owner.policy,
            span_ids: vec![second_span],
        },
    ];
    request
}

fn assert_whole_figure_ocr(input: &[u8], request: &LinkedStoryRequest) {
    assert_figure_owners(input, request);
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = engine.document().reader();
    for figure in &request.figures {
        assert!(figure.ocr.is_none() && !figure.ocr_unrelated);
        let ImageFragmentSource::Owned { binding } = &figure.source else {
            panic!("missing native owner")
        };
        let page = engine.document().get_page(binding.page).unwrap();
        let resources = reader
            .resolve(page.resources.get("XObject").unwrap().clone())
            .unwrap();
        let group = reader
            .resolve(
                resources
                    .as_dict()
                    .unwrap()
                    .get(&format!("WFIF{}", binding.key))
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let group = group.as_stream().unwrap().0;
        assert_eq!(group.get_bool("WFOcrGroup"), Some(true));
        let r = reader
            .resolve(group.get("Resources").unwrap().clone())
            .unwrap();
        let children = reader
            .resolve(r.as_dict().unwrap().get("XObject").unwrap().clone())
            .unwrap();
        let search = children.as_dict().unwrap().get_reference("Search").unwrap();
        let mut scopes = ContentScopes::new(&engine);
        let bytes = scopes.stream(search).unwrap();
        crate::image_fragments::validate_carrier_program(&bytes).unwrap();
        // ActualText survives inside a non-structural search Form. The one
        // page-owned Figure MCID (checked above) owns the complete invocation.
        assert!(bytes
            .windows(b"ActualText".len())
            .any(|w| w == b"ActualText"));
        let text = engine.get_page_text(binding.page).unwrap();
        let word = if figure.id == "figure-0" {
            "READ_A"
        } else {
            "READ_B"
        };
        assert_eq!(text.matches(word).count(), 1);
    }
}

#[test]
fn tagged_ocr_whole_figure_preserves_actualtext_through_growth_contraction_and_reopen() {
    for (nested, split) in [(false, false), (false, true), (true, true)] {
        let (input, mut request) = whole_figure_ocr_fixture(nested, split, false);
        let mut tail = request.paragraphs[0].clone();
        tail.id = "tail".into();
        tail.text = "continued paragraph ".repeat(120);
        tail.break_before = true;
        request
            .source_tags
            .as_mut()
            .unwrap()
            .paragraph_sources
            .insert(tail.id.clone(), None);
        request.paragraphs.push(tail);
        let (output, report) = apply_linked_story(&input, &request).unwrap();
        assert!(report.generated_pages > 0);
        let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_whole_figure_ocr(&output, &saved);
        saved.paragraphs.pop();
        saved
            .source_tags
            .as_mut()
            .unwrap()
            .paragraph_sources
            .remove("tail");
        saved.paragraphs[1].break_before = false;
        let (contracted, _) = apply_linked_story(&output, &saved).unwrap();
        let rebound = load_linked_stories(&contracted).unwrap().remove(0).request;
        assert_whole_figure_ocr(&contracted, &rebound);
        assert!(rebound
            .figures
            .iter()
            .all(|f| matches!(&f.source,ImageFragmentSource::Owned{binding} if binding.page==1)));
        let (again, _) = apply_linked_story(&contracted, &rebound).unwrap();
        assert_whole_figure_ocr(
            &again,
            &load_linked_stories(&again).unwrap().remove(0).request,
        );
    }
}

#[test]
fn tagged_ocr_group_deletion_consumes_visual_search_and_semantic_owner_together() {
    let (input, request) = whole_figure_ocr_fixture(false, true, false);
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let figure = saved.figures.remove(0);
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("missing owner")
    };
    saved
        .figure_removals
        .push(crate::linked_stories::figures::StoryFigureRemoval {
            figure_id: figure.id,
            binding,
        });
    let (deleted, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&deleted).unwrap();
    let engine = ContentEngine::open_bytes(deleted.clone()).unwrap();
    let text = (1..=engine.page_count().unwrap())
        .map(|p| engine.get_page_text(p).unwrap())
        .collect::<String>();
    assert!(!text.contains("READ_A"));
    assert_eq!(text.matches("READ_B").count(), 1);
    assert!(text.contains("First caption"));
    assert_eq!(
        sources(&deleted)
            .unwrap()
            .iter()
            .filter(|s| s.role == "Figure")
            .count(),
        1
    );
    let rebound = load_linked_stories(&deleted).unwrap().remove(0).request;
    assert_whole_figure_ocr(&deleted, &rebound);
    assert!(apply_linked_story(&deleted, &rebound).is_ok());
}

#[test]
fn tagged_ocr_does_not_authorize_wrong_visible_partial_or_unselected_owners() {
    let (input, request) = whole_figure_ocr_fixture(false, true, false);
    let mut wrong = request.clone();
    let other = wrong.figures[1].ocr.clone();
    wrong.figures[1].ocr = wrong.figures[0].ocr.clone();
    wrong.figures[0].ocr = other;
    assert!(preview_linked_story(&input, &wrong)
        .unwrap_err()
        .to_string()
        .contains("semantic owner"));
    let mut missing = request.clone();
    missing.figures[0].ocr = None;
    missing.figures[0].ocr_unrelated = true;
    assert!(preview_linked_story(&input, &missing).is_err());
    let mut overlap = request.clone();
    overlap.frames[0].logical_range = [0, 15];
    overlap.frames[0].expected_text = ["SCAN_A", "SCAN_B", "OLD"].concat();
    assert!(preview_linked_story(&input, &overlap).is_err());
    let mut duplicate = request.clone();
    let other = duplicate.figures[1].ocr.as_ref().unwrap().span_ids[0].clone();
    duplicate.figures[0]
        .ocr
        .as_mut()
        .unwrap()
        .span_ids
        .push(other);
    assert!(preview_linked_story(&input, &duplicate).is_err());
    let mut stale = request.clone();
    stale.figures[0].ocr.as_mut().unwrap().span_ids[0].push('0');
    assert!(preview_linked_story(&input, &stale).is_err());
    let mut new_owner = request.clone();
    let id = new_owner.figures[0].id.clone();
    let binding = new_owner
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&id)
        .unwrap();
    binding.source = None;
    binding.semantic_text = Some(StorySemanticText {
        alternate: Some("Replacement owner".into()),
        expansion: None,
    });
    assert!(preview_linked_story(&input, &new_owner).is_err());
    let model = crate::advanced_editing::analyze_multi_run_text_range(&input, 1).unwrap();
    let visible = model
        .source_spans
        .iter()
        .find(|s| s.text_render_mode == 0)
        .unwrap();
    let mut wrong = request;
    wrong.figures[0].ocr = Some(crate::advanced_editing::ocr_carriers::OcrCarrierSelection {
        span_ids: vec![visible.span_id.clone()],
        expected_text: visible.source_text.clone(),
        form_target: None,
    });
    assert!(preview_linked_story(&input, &wrong).is_err());
}

#[test]
fn tagged_ocr_explicitly_merges_separate_span_owners_and_consumes_the_decision() {
    for split_stream in [false, true] {
        let (input, request) = whole_figure_ocr_fixture(false, split_stream, true);
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        validate_parent_tree(&output).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_whole_figure_ocr(&output, &saved);
        assert!(saved
            .source_tags
            .as_ref()
            .unwrap()
            .figures
            .values()
            .all(|binding| binding.separate_ocr_owner.is_none()));
        let tags = sources(&output).unwrap();
        assert_eq!(
            tags.iter().filter(|source| source.role == "Figure").count(),
            2
        );
        assert_eq!(
            tags.iter().filter(|source| source.role == "Span").count(),
            0
        );
        assert_eq!(tags.iter().filter(|source| source.role == "P").count(), 2);
        let (again, _) = apply_linked_story(&output, &saved).unwrap();
        let rebound = load_linked_stories(&again).unwrap().remove(0).request;
        assert_whole_figure_ocr(&again, &rebound);
        assert!(rebound
            .source_tags
            .as_ref()
            .unwrap()
            .figures
            .values()
            .all(|binding| binding.separate_ocr_owner.is_none()));
    }
}

#[test]
fn tagged_ocr_explicitly_merges_multiple_exact_owners_into_one_figure() {
    let (input, request) = whole_figure_ocr_fixture(false, true, true);
    let request = combine_separate_ocr_owners(request);
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_figure_owners(&output, &saved);
    let page_text = ContentEngine::open_bytes(output.clone())
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert_eq!(page_text.matches("READ_A").count(), 1);
    assert_eq!(page_text.matches("READ_B").count(), 1);
    assert_eq!(
        sources(&output)
            .unwrap()
            .iter()
            .filter(|source| source.role == "Span")
            .count(),
        0
    );
    assert!(saved
        .source_tags
        .as_ref()
        .unwrap()
        .figures
        .values()
        .all(|binding| binding.separate_ocr_owner.is_none()
            && binding.separate_ocr_owners.is_empty()));
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    let rebound = load_linked_stories(&again).unwrap().remove(0).request;
    assert_figure_owners(&again, &rebound);
    let page_text = ContentEngine::open_bytes(again)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert_eq!(page_text.matches("READ_A").count(), 1);
    assert_eq!(page_text.matches("READ_B").count(), 1);
}

#[test]
fn tagged_ocr_multi_owner_requires_one_exact_disjoint_binding_per_owner() {
    let (input, request) = whole_figure_ocr_fixture(false, true, true);
    let request = combine_separate_ocr_owners(request);
    let figure_id = request.figures[0].id.clone();

    let mut mixed_spelling = request.clone();
    let binding = mixed_spelling
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap();
    binding.separate_ocr_owner = Some(binding.separate_ocr_owners[0].clone());
    assert!(preview_linked_story(&input, &mixed_spelling)
        .unwrap_err()
        .to_string()
        .contains("either separate_ocr_owner or separate_ocr_owners"));

    let mut empty = request.clone();
    empty
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .separate_ocr_owners[0]
        .span_ids
        .clear();
    assert!(preview_linked_story(&input, &empty)
        .unwrap_err()
        .to_string()
        .contains("requires nonempty exact span_ids"));

    let mut duplicate_span = request.clone();
    let binding = duplicate_span
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap();
    binding.separate_ocr_owners[1].span_ids = binding.separate_ocr_owners[0].span_ids.clone();
    assert!(preview_linked_story(&input, &duplicate_span)
        .unwrap_err()
        .to_string()
        .contains("selected and disjoint"));

    let mut stale_span = request.clone();
    stale_span
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .separate_ocr_owners[0]
        .span_ids[0]
        .push_str(":stale");
    assert!(preview_linked_story(&input, &stale_span)
        .unwrap_err()
        .to_string()
        .contains("selected and disjoint"));

    let mut duplicate_owner = request;
    let binding = duplicate_owner
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap();
    let first_owner = binding.separate_ocr_owners[0].source.clone();
    binding.separate_ocr_owners[1].source = first_owner;
    assert!(preview_linked_story(&input, &duplicate_owner)
        .unwrap_err()
        .to_string()
        .contains("unique selected content-only"));
}

#[test]
fn tagged_ocr_separate_owner_requires_exact_explicit_unambiguous_approval() {
    let (input, request) = whole_figure_ocr_fixture(false, true, true);

    let mut omitted = request.clone();
    let first_figure = omitted.figures[0].id.clone();
    omitted
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&first_figure)
        .unwrap()
        .separate_ocr_owner = None;
    assert!(preview_linked_story(&input, &omitted)
        .unwrap_err()
        .to_string()
        .contains("different or nested semantic owner"));

    let mut wrong = request.clone();
    let first_figure = wrong.figures[0].id.clone();
    let caption = wrong
        .source_tags
        .as_ref()
        .unwrap()
        .paragraph_sources
        .get(&wrong.paragraphs[0].id)
        .unwrap()
        .clone()
        .unwrap();
    wrong
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&first_figure)
        .unwrap()
        .separate_ocr_owner
        .as_mut()
        .unwrap()
        .source = caption;
    assert!(preview_linked_story(&input, &wrong)
        .unwrap_err()
        .to_string()
        .contains("unique selected content-only"));

    let mut nonselected = request.clone();
    let first_figure = nonselected.figures[0].id.clone();
    let parent = nonselected.source_tags.as_ref().unwrap().parent.clone();
    nonselected
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&first_figure)
        .unwrap()
        .separate_ocr_owner
        .as_mut()
        .unwrap()
        .source = parent;
    assert!(preview_linked_story(&input, &nonselected)
        .unwrap_err()
        .to_string()
        .contains("unique selected content-only"));

    let mut duplicate = request.clone();
    let first_figure = duplicate.figures[0].id.clone();
    let second_figure = duplicate.figures[1].id.clone();
    let first = duplicate.source_tags.as_ref().unwrap().figures[&first_figure]
        .separate_ocr_owner
        .clone();
    duplicate
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&second_figure)
        .unwrap()
        .separate_ocr_owner = first;
    assert!(preview_linked_story(&input, &duplicate)
        .unwrap_err()
        .to_string()
        .contains("unique selected content-only"));
}

#[test]
fn tagged_ocr_separate_owner_refuses_semantics_relationships_and_extra_content() {
    let (input, mut request) = whole_figure_ocr_fixture(false, true, true);
    let owner = request.source_tags.as_ref().unwrap().figures[&request.figures[0].id]
        .separate_ocr_owner
        .as_ref()
        .unwrap()
        .source
        .clone();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let mut dictionary = store.dict((owner.object, owner.generation)).unwrap();
    dictionary.insert("Alt", logical_string("independent OCR meaning"));
    store
        .replace_dict((owner.object, owner.generation), dictionary)
        .unwrap();
    let semantic = write_store(store).unwrap();
    request.input_sha256 = format!("{:x}", Sha256::digest(&semantic));
    assert!(preview_linked_story(&semantic, &request)
        .unwrap_err()
        .to_string()
        .contains("semantics/attributes"));

    let (input, mut request) = whole_figure_ocr_fixture(false, true, true);
    let tags = request.source_tags.as_ref().unwrap();
    let owner = tags.figures[&request.figures[0].id]
        .separate_ocr_owner
        .as_ref()
        .unwrap()
        .source
        .clone();
    let target = tags.figures[&request.figures[0].id]
        .source
        .as_ref()
        .unwrap()
        .clone();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let mut dictionary = store.dict((owner.object, owner.generation)).unwrap();
    dictionary.insert("Ref", reference((target.object, target.generation)));
    store
        .replace_dict((owner.object, owner.generation), dictionary)
        .unwrap();
    let relationship = write_store(store).unwrap();
    request.input_sha256 = format!("{:x}", Sha256::digest(&relationship));
    assert!(preview_linked_story(&relationship, &request)
        .unwrap_err()
        .to_string()
        .contains("semantics/attributes"));

    let (input, mut request) = whole_figure_ocr_fixture(false, true, true);
    let owner = request.source_tags.as_ref().unwrap().figures[&request.figures[0].id]
        .separate_ocr_owner
        .as_ref()
        .unwrap()
        .source
        .clone();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let mut store = Store::new(engine.document().reader());
    let mut dictionary = store.dict((owner.object, owner.generation)).unwrap();
    dictionary.insert(
        "K",
        PdfObject::Array(vec![PdfObject::Integer(3), PdfObject::Integer(5)]),
    );
    store
        .replace_dict((owner.object, owner.generation), dictionary)
        .unwrap();
    let extra = stream(
        &mut store,
        b"/Span << /MCID 5 >> BDC BT /OCR 8 Tf 0 Tr 1 0 0 1 10 90 Tm (EXTRA) Tj ET EMC".to_vec(),
    );
    let page_id = (page.object_number, page.generation_number);
    let mut page_dictionary = store.dict(page_id).unwrap();
    let mut contents = page_dictionary
        .get("Contents")
        .and_then(PdfObject::as_array)
        .map(|items| items.to_vec())
        .unwrap();
    contents.push(reference(extra));
    page_dictionary.insert("Contents", PdfObject::Array(contents));
    store.replace_dict(page_id, page_dictionary).unwrap();
    let raw = write_store(store).unwrap();
    let extra = rebuild_owner_trees(&raw, None).unwrap().0;
    request.input_sha256 = format!("{:x}", Sha256::digest(&extra));
    assert!(preview_linked_story(&extra, &request)
        .unwrap_err()
        .to_string()
        .contains("outside the approved ranges"));
}
