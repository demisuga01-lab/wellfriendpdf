//! Source-only regressions. No build, test or PDF execution is authorized yet.
use super::*;
use crate::linked_stories::{apply_linked_story, hash, load_linked_stories, preview_linked_story};
include!("tagged_figure_ocr_tests.rs");

fn stream(store: &mut Store<'_>, bytes: Vec<u8>) -> ObjectRef {
    let mut dict = PdfDictionary::empty();
    dict.insert("Length", PdfObject::Integer(bytes.len() as i64));
    store.add(PdfObject::Stream { dict, raw: bytes }).unwrap()
}
fn fixture() -> (Vec<u8>, LinkedStoryRequest) {
    let input = crate::linked_stories::figure_tests::fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let pages = engine.document().get_pages().unwrap();
    let mut scopes = ContentScopes::new(&engine);
    let mut store = Store::new(engine.document().reader());
    for page in &pages {
        let mut contents = Vec::new();
        let mut text = b"/P << /MCID 0 >> BDC\n".to_vec();
        text.extend(scopes.stream(page.contents[0]).unwrap());
        text.extend_from_slice(b"\nEMC\n");
        contents.push(reference(stream(&mut store, text)));
        if page.page_number == 1 {
            let images=b"/Figure << /MCID 1 >> BDC q 20 0 0 20 20 100 cm /Figure Do Q EMC\n/Figure << /MCID 2 >> BDC q 20 0 0 20 80 100 cm /Figure Do Q EMC".to_vec();
            contents.push(reference(stream(&mut store, images)));
        }
        let id = (page.object_number, page.generation_number);
        let mut dict = store.dict(id).unwrap();
        dict.insert("Contents", PdfObject::Array(contents));
        store.replace_dict(id, dict).unwrap();
    }
    let root = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let parent = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let mut owners = Vec::new();
    for (id, role, page, mcid) in [
        ("fig1", "Figure", 0, 1),
        ("cap1", "P", 0, 0),
        ("fig2", "Figure", 0, 2),
        ("cap2", "P", 1, 0),
    ] {
        let mut dict = PdfDictionary::empty();
        dict.insert("Type", PdfObject::Name("StructElem".into()));
        dict.insert("S", PdfObject::Name(role.into()));
        dict.insert("P", reference(parent));
        dict.insert("ID", PdfObject::String(id.as_bytes().to_vec()));
        dict.insert(
            "Pg",
            reference((pages[page].object_number, pages[page].generation_number)),
        );
        dict.insert("K", PdfObject::Integer(mcid));
        if role == "Figure" {
            dict.insert("Alt", logical_string(&format!("Original {id}")));
            let mut a = PdfDictionary::empty();
            a.insert("O", PdfObject::Name("Layout".into()));
            a.insert("BBox", PdfObject::Array(vec![PdfObject::Integer(0); 4]));
            a.insert("Placement", PdfObject::Name("Block".into()));
            dict.insert("A", PdfObject::Dictionary(a));
        }
        owners.push(store.add(PdfObject::Dictionary(dict)).unwrap());
    }
    let mut parent_dict = PdfDictionary::empty();
    parent_dict.insert("Type", PdfObject::Name("StructElem".into()));
    parent_dict.insert("S", PdfObject::Name("Document".into()));
    parent_dict.insert("P", reference(root));
    parent_dict.insert(
        "K",
        PdfObject::Array(owners.iter().copied().map(reference).collect()),
    );
    store.replace_dict(parent, parent_dict).unwrap();
    let mut root_dict = PdfDictionary::empty();
    root_dict.insert("Type", PdfObject::Name("StructTreeRoot".into()));
    root_dict.insert("K", reference(parent));
    store.replace_dict(root, root_dict).unwrap();
    let catalog_id = store.reader.root_reference().unwrap();
    let mut catalog = store.dict(catalog_id).unwrap();
    catalog.insert("StructTreeRoot", reference(root));
    let mut info = PdfDictionary::empty();
    info.insert("Marked", PdfObject::Boolean(true));
    catalog.insert("MarkInfo", PdfObject::Dictionary(info));
    store.replace_dict(catalog_id, catalog).unwrap();
    let (input, _) = rebuild_owner_trees(&write_store(store).unwrap(), None).unwrap();
    let mut request = crate::linked_stories::figure_tests::with_figures(&input);
    let mut second = request.frames[0].clone();
    second.id = "frame-2".into();
    second.page = 2;
    request.frames.push(second);
    let tag = |id: ObjectRef| TagReference {
        object: id.0,
        generation: id.1,
        key: None,
    };
    request.source_tags = Some(StoryTagging {
        parent: tag(parent),
        selected: owners.iter().copied().map(tag).collect(),
        insert_at: Some(0),
        paragraph_sources: BTreeMap::from([
            (request.paragraphs[0].id.clone(), Some(tag(owners[1]))),
            (request.paragraphs[1].id.clone(), Some(tag(owners[3]))),
        ]),
        new_roles: BTreeMap::new(),
        semantic_text: BTreeMap::new(),
        figures: BTreeMap::from([
            (
                request.figures[0].id.clone(),
                FigureTagBinding {
                    source: Some(tag(owners[0])),
                    semantic_text: None,
                    split_reused_form_semantics: false,
                    outbound_ref_split: None,
                    inbound_ref_split: None,
                    preserve_semantic_subtree: false,
                    delete_semantic_subtree: false,
                    clone_semantic_subtree_for_reused_form: false,
                    separate_ocr_owner: None,
                    separate_ocr_owners: Vec::new(),
                },
            ),
            (
                request.figures[1].id.clone(),
                FigureTagBinding {
                    source: Some(tag(owners[2])),
                    semantic_text: None,
                    split_reused_form_semantics: false,
                    outbound_ref_split: None,
                    inbound_ref_split: None,
                    preserve_semantic_subtree: false,
                    delete_semantic_subtree: false,
                    clone_semantic_subtree_for_reused_form: false,
                    separate_ocr_owner: None,
                    separate_ocr_owners: Vec::new(),
                },
            ),
        ]),
    });
    (input, request)
}

fn semantic_subtree_fixture() -> (Vec<u8>, LinkedStoryRequest) {
    let (input, mut request) = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let figure_id = request.figures[0].id.clone();
    let figure = request.source_tags.as_ref().unwrap().figures[&figure_id]
        .source
        .as_ref()
        .unwrap();
    let figure = (figure.object, figure.generation);

    let mut leaf = PdfDictionary::empty();
    leaf.insert("Type", PdfObject::Name("StructElem".into()));
    leaf.insert("S", PdfObject::Name("Span".into()));
    leaf.insert("ID", PdfObject::String(b"figure-semantic-leaf".to_vec()));
    leaf.insert("E", logical_string("Preserved expansion"));
    leaf.insert("K", PdfObject::Null);
    let leaf = store.add(PdfObject::Dictionary(leaf)).unwrap();

    let mut group = PdfDictionary::empty();
    group.insert("Type", PdfObject::Name("StructElem".into()));
    group.insert("S", PdfObject::Name("Div".into()));
    group.insert("P", reference(figure));
    group.insert("ID", PdfObject::String(b"figure-semantic-group".to_vec()));
    group.insert("Lang", PdfObject::String(b"en-GB".to_vec()));
    group.insert("Alt", logical_string("Preserved semantic group"));
    group.insert("K", reference(leaf));
    let group = store.add(PdfObject::Dictionary(group)).unwrap();
    let mut leaf_dictionary = store.dict(leaf).unwrap();
    leaf_dictionary.insert("P", reference(group));
    store.replace_dict(leaf, leaf_dictionary).unwrap();

    let mut figure_dictionary = store.dict(figure).unwrap();
    let content = figure_dictionary.get("K").cloned().unwrap();
    figure_dictionary.insert("K", PdfObject::Array(vec![content, reference(group)]));
    store.replace_dict(figure, figure_dictionary).unwrap();
    let (input, _) = rebuild_owner_trees(&write_store(store).unwrap(), None).unwrap();
    request.input_sha256 = hash(&input);
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .preserve_semantic_subtree = true;
    (input, request)
}

fn nested_fixture_with_ocr_owners(
    repeat: bool,
    with_ocr: bool,
    separate_owner_count: usize,
) -> (Vec<u8>, LinkedStoryRequest, ObjectRef, ObjectRef) {
    assert!(separate_owner_count == 0 || with_ocr);
    assert!(separate_owner_count <= 8);
    let input = crate::linked_stories::tests::two_page_input();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let pages = engine.document().get_pages().unwrap();
    let page = &pages[0];
    let mut scopes = ContentScopes::new(&engine);
    let mut store = Store::new(engine.document().reader());

    let mut image = PdfDictionary::empty();
    image.insert("Type", PdfObject::Name("XObject".into()));
    image.insert("Subtype", PdfObject::Name("Image".into()));
    image.insert("Width", PdfObject::Integer(1));
    image.insert("Height", PdfObject::Integer(1));
    image.insert("BitsPerComponent", PdfObject::Integer(8));
    image.insert("ColorSpace", PdfObject::Name("DeviceRGB".into()));
    image.insert("Length", PdfObject::Integer(3));
    let image = store
        .add(PdfObject::Stream {
            dict: image,
            raw: vec![32, 112, 224],
        })
        .unwrap();

    let mut leaf_xobjects = PdfDictionary::empty();
    leaf_xobjects.insert("Im", reference(image));
    let mut leaf_resources = PdfDictionary::empty();
    leaf_resources.insert("XObject", PdfObject::Dictionary(leaf_xobjects));
    if with_ocr {
        let mut font = PdfDictionary::empty();
        font.insert("Type", PdfObject::Name("Font".into()));
        font.insert("Subtype", PdfObject::Name("Type1".into()));
        font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
        font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
        let mut fonts = PdfDictionary::empty();
        fonts.insert("FOcr", PdfObject::Dictionary(font));
        leaf_resources.insert("Font", PdfObject::Dictionary(fonts));
    }
    let leaf_bytes = if separate_owner_count > 0 {
        let mut bytes = b"/Figure << /MCID 0 >> BDC q 40 0 0 30 0 0 cm /Im Do Q EMC".to_vec();
        for index in 0..separate_owner_count {
            let text = if index == 0 {
                "NESTED OCR".to_owned()
            } else {
                format!("NESTED OCR {}", index + 1)
            };
            bytes.extend(
                format!(
                    " /Span << /MCID {} >> BDC BT /FOcr 8 Tf 3 Tr 1 0 0 1 2 {} Tm ({text}) Tj ET EMC",
                    index + 1,
                    2 + index * 9
                )
                .into_bytes(),
            );
        }
        bytes
    } else if with_ocr {
        b"/Figure << /MCID 0 >> BDC q 40 0 0 30 0 0 cm /Im Do Q BT /FOcr 8 Tf 3 Tr 1 0 0 1 2 2 Tm (NESTED OCR) Tj ET EMC".to_vec()
    } else {
        b"/Figure << /MCID 0 >> BDC q 40 0 0 30 0 0 cm /Im Do Q EMC".to_vec()
    };
    let mut leaf = PdfDictionary::empty();
    leaf.insert("Type", PdfObject::Name("XObject".into()));
    leaf.insert("Subtype", PdfObject::Name("Form".into()));
    leaf.insert("FormType", PdfObject::Integer(1));
    leaf.insert(
        "BBox",
        PdfObject::Array(
            [0.0, 0.0, 40.0, 30.0]
                .into_iter()
                .map(PdfObject::Real)
                .collect(),
        ),
    );
    leaf.insert("Resources", PdfObject::Dictionary(leaf_resources));
    leaf.insert("Length", PdfObject::Integer(leaf_bytes.len() as i64));
    let leaf = store
        .add(PdfObject::Stream {
            dict: leaf,
            raw: leaf_bytes,
        })
        .unwrap();

    let mut outer_xobjects = PdfDictionary::empty();
    outer_xobjects.insert("Leaf", reference(leaf));
    let mut outer_resources = PdfDictionary::empty();
    outer_resources.insert("XObject", PdfObject::Dictionary(outer_xobjects));
    let outer_bytes = b"q /Leaf Do Q".to_vec();
    let mut outer = PdfDictionary::empty();
    outer.insert("Type", PdfObject::Name("XObject".into()));
    outer.insert("Subtype", PdfObject::Name("Form".into()));
    outer.insert("FormType", PdfObject::Integer(1));
    outer.insert(
        "BBox",
        PdfObject::Array(
            [0.0, 0.0, 40.0, 30.0]
                .into_iter()
                .map(PdfObject::Real)
                .collect(),
        ),
    );
    outer.insert("Resources", PdfObject::Dictionary(outer_resources));
    outer.insert("Length", PdfObject::Integer(outer_bytes.len() as i64));
    let outer = store
        .add(PdfObject::Stream {
            dict: outer,
            raw: outer_bytes,
        })
        .unwrap();

    let mut page_xobjects = page
        .resources
        .get("XObject")
        .and_then(PdfObject::as_dict)
        .cloned()
        .unwrap_or_else(PdfDictionary::empty);
    page_xobjects.insert("Outer", reference(outer));
    let mut page_resources = page.resources.clone();
    page_resources.insert("XObject", PdfObject::Dictionary(page_xobjects));
    let page_bytes = if repeat {
        b"q 1 0 0 1 30 100 cm /Outer Do Q q 1 0 0 1 100 100 cm /Outer Do Q".to_vec()
    } else {
        b"q 1 0 0 1 30 100 cm /Outer Do Q".to_vec()
    };
    let page_paint = stream(&mut store, page_bytes);
    let mut text = b"/P << /MCID 0 >> BDC\n".to_vec();
    text.extend(scopes.stream(page.contents[0]).unwrap());
    text.extend_from_slice(b"\nEMC\n");
    let text = stream(&mut store, text);
    let page_id = (page.object_number, page.generation_number);
    let mut page_dictionary = store.dict(page_id).unwrap();
    page_dictionary.insert("Resources", PdfObject::Dictionary(page_resources));
    page_dictionary.insert(
        "Contents",
        PdfObject::Array(vec![reference(text), reference(page_paint)]),
    );
    store.replace_dict(page_id, page_dictionary).unwrap();

    let root = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let parent = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let mut figure = PdfDictionary::empty();
    figure.insert("Type", PdfObject::Name("StructElem".into()));
    figure.insert("S", PdfObject::Name("Figure".into()));
    figure.insert("P", reference(parent));
    figure.insert("ID", PdfObject::String(b"nested-figure-owner".to_vec()));
    figure.insert("Alt", logical_string("Nested source Figure"));
    let mut mcr = PdfDictionary::empty();
    mcr.insert("Type", PdfObject::Name("MCR".into()));
    mcr.insert("Pg", reference(page_id));
    mcr.insert("Stm", reference(leaf));
    mcr.insert("MCID", PdfObject::Integer(0));
    figure.insert("K", PdfObject::Dictionary(mcr));
    let figure_owner = store.add(PdfObject::Dictionary(figure)).unwrap();

    let mut ocr_owners = Vec::new();
    for index in 0..separate_owner_count {
        let mut owner = PdfDictionary::empty();
        owner.insert("Type", PdfObject::Name("StructElem".into()));
        owner.insert("S", PdfObject::Name("Span".into()));
        owner.insert("P", reference(parent));
        owner.insert(
            "ID",
            PdfObject::String(format!("nested-ocr-owner-{index}").into_bytes()),
        );
        let mut mcr = PdfDictionary::empty();
        mcr.insert("Type", PdfObject::Name("MCR".into()));
        mcr.insert("Pg", reference(page_id));
        mcr.insert("Stm", reference(leaf));
        mcr.insert("MCID", PdfObject::Integer((index + 1) as i64));
        owner.insert("K", PdfObject::Dictionary(mcr));
        ocr_owners.push(store.add(PdfObject::Dictionary(owner)).unwrap());
    }

    let mut caption = PdfDictionary::empty();
    caption.insert("Type", PdfObject::Name("StructElem".into()));
    caption.insert("S", PdfObject::Name("P".into()));
    caption.insert("P", reference(parent));
    caption.insert("ID", PdfObject::String(b"nested-caption-owner".to_vec()));
    caption.insert("Pg", reference(page_id));
    caption.insert("K", PdfObject::Integer(0));
    let caption_owner = store.add(PdfObject::Dictionary(caption)).unwrap();

    let mut parent_dictionary = PdfDictionary::empty();
    parent_dictionary.insert("Type", PdfObject::Name("StructElem".into()));
    parent_dictionary.insert("S", PdfObject::Name("Document".into()));
    parent_dictionary.insert("P", reference(root));
    let mut children = vec![reference(figure_owner)];
    for &owner in &ocr_owners {
        children.push(reference(owner));
    }
    children.push(reference(caption_owner));
    parent_dictionary.insert("K", PdfObject::Array(children));
    store.replace_dict(parent, parent_dictionary).unwrap();
    let mut root_dictionary = PdfDictionary::empty();
    root_dictionary.insert("Type", PdfObject::Name("StructTreeRoot".into()));
    root_dictionary.insert("K", reference(parent));
    store.replace_dict(root, root_dictionary).unwrap();
    let catalog_id = store.reader.root_reference().unwrap();
    let mut catalog = store.dict(catalog_id).unwrap();
    catalog.insert("StructTreeRoot", reference(root));
    let mut mark_info = PdfDictionary::empty();
    mark_info.insert("Marked", PdfObject::Boolean(true));
    catalog.insert("MarkInfo", PdfObject::Dictionary(mark_info));
    store.replace_dict(catalog_id, catalog).unwrap();
    let (input, _) = rebuild_owner_trees(&write_store(store).unwrap(), None).unwrap();

    let occurrence = crate::universal_editing::universal_image_occurrences_v2(&input, &[1])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(occurrence.invocation_path.len(), 2);
    let mut request = crate::linked_stories::tests::request("Nested caption".into());
    request.input_sha256 = hash(&input);
    request.frames.truncate(1);
    request.frames[0].rect = [10.0, 10.0, 190.0, 190.0];
    request
        .figures
        .push(crate::linked_stories::figures::StoryFigure {
            id: "nested-figure".into(),
            caption_paragraph: request.paragraphs[0].id.clone(),
            source: ImageFragmentSource::Occurrence {
                page: 1,
                content_stream_index: occurrence.content_stream_index,
                occurrence_id: occurrence.occurrence_id,
            },
            ocr: None,
            ocr_unrelated: false,
            width: 100.0,
            height: 50.0,
            gap: 5.0,
            alignment: crate::linked_stories::figures::FigureAlignment::Center,
            stack: crate::image_fragments::ImageFragmentStack::Foreground,
        });
    let tag = |owner: ObjectRef| TagReference {
        object: owner.0,
        generation: owner.1,
        key: None,
    };
    let mut selected = vec![tag(figure_owner)];
    for &owner in &ocr_owners {
        selected.push(tag(owner));
    }
    selected.push(tag(caption_owner));
    request.source_tags = Some(StoryTagging {
        parent: tag(parent),
        selected,
        insert_at: Some(0),
        paragraph_sources: BTreeMap::from([(
            request.paragraphs[0].id.clone(),
            Some(tag(caption_owner)),
        )]),
        new_roles: BTreeMap::new(),
        semantic_text: BTreeMap::new(),
        figures: BTreeMap::from([(
            "nested-figure".into(),
            FigureTagBinding {
                source: Some(tag(figure_owner)),
                semantic_text: None,
                split_reused_form_semantics: false,
                outbound_ref_split: None,
                inbound_ref_split: None,
                preserve_semantic_subtree: false,
                delete_semantic_subtree: false,
                clone_semantic_subtree_for_reused_form: false,
                separate_ocr_owner: (separate_owner_count == 1).then(|| FigureOcrOwnerBinding {
                    source: tag(ocr_owners[0]),
                    policy: FigureOcrOwnerPolicy::MergeIntoFigure,
                    span_ids: Vec::new(),
                }),
                separate_ocr_owners: Vec::new(),
            },
        )]),
    });
    if with_ocr {
        let inventory = crate::advanced_editing::form_text::analyze_form_text(&input, 1).unwrap();
        let form = inventory
            .occurrences
            .into_iter()
            .find(|candidate| {
                candidate.target.content_stream_index == occurrence.content_stream_index
                    && candidate.target.invocation_path == occurrence.invocation_path
            })
            .unwrap();
        let spans = form
            .text
            .source_spans
            .iter()
            .filter(|span| span.text_render_mode == 3)
            .collect::<Vec<_>>();
        request.figures[0].ocr = Some(crate::advanced_editing::ocr_carriers::OcrCarrierSelection {
            span_ids: spans.iter().map(|span| span.span_id.clone()).collect(),
            expected_text: spans.iter().map(|span| span.source_text.as_str()).collect(),
            form_target: Some(form.target),
        });
        if separate_owner_count > 1 {
            request
                .source_tags
                .as_mut()
                .unwrap()
                .figures
                .get_mut("nested-figure")
                .unwrap()
                .separate_ocr_owners = ocr_owners
                .iter()
                .zip(spans)
                .map(|(&owner, span)| FigureOcrOwnerBinding {
                    source: tag(owner),
                    policy: FigureOcrOwnerPolicy::MergeIntoFigure,
                    span_ids: vec![span.span_id.clone()],
                })
                .collect();
        }
    }
    (input, request, leaf, image)
}

fn nested_fixture_with_ocr(
    repeat: bool,
    with_ocr: bool,
    separate_owner: bool,
) -> (Vec<u8>, LinkedStoryRequest, ObjectRef, ObjectRef) {
    nested_fixture_with_ocr_owners(repeat, with_ocr, if separate_owner { 1 } else { 0 })
}

fn nested_fixture(repeat: bool) -> (Vec<u8>, LinkedStoryRequest, ObjectRef, ObjectRef) {
    nested_fixture_with_ocr(repeat, false, false)
}

fn reused_form_semantic_subtree_fixture() -> (Vec<u8>, LinkedStoryRequest) {
    let (input, mut request, _, _) = nested_fixture(true);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let figure = request.source_tags.as_ref().unwrap().figures["nested-figure"]
        .source
        .as_ref()
        .unwrap();
    let figure = (figure.object, figure.generation);

    let mut leaf = PdfDictionary::empty();
    leaf.insert("Type", PdfObject::Name("StructElem".into()));
    leaf.insert("S", PdfObject::Name("Span".into()));
    leaf.insert("ID", PdfObject::String(b"nested-semantic-leaf".to_vec()));
    leaf.insert("E", logical_string("Nested preserved expansion"));
    leaf.insert("K", PdfObject::Null);
    let leaf = store.add(PdfObject::Dictionary(leaf)).unwrap();

    let mut group = PdfDictionary::empty();
    group.insert("Type", PdfObject::Name("StructElem".into()));
    group.insert("S", PdfObject::Name("Div".into()));
    group.insert("P", reference(figure));
    group.insert("ID", PdfObject::String(b"nested-semantic-group".to_vec()));
    group.insert("Alt", logical_string("Nested preserved group"));
    group.insert("K", reference(leaf));
    let group = store.add(PdfObject::Dictionary(group)).unwrap();
    let mut leaf_dictionary = store.dict(leaf).unwrap();
    leaf_dictionary.insert("P", reference(group));
    store.replace_dict(leaf, leaf_dictionary).unwrap();

    let mut figure_dictionary = store.dict(figure).unwrap();
    let content = figure_dictionary.get("K").cloned().unwrap();
    figure_dictionary.insert("K", PdfObject::Array(vec![content, reference(group)]));
    store.replace_dict(figure, figure_dictionary).unwrap();
    let (input, _) = rebuild_owner_trees(&write_store(store).unwrap(), None).unwrap();
    request.input_sha256 = hash(&input);
    let binding = request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap();
    binding.preserve_semantic_subtree = true;
    binding.split_reused_form_semantics = true;
    binding.clone_semantic_subtree_for_reused_form = true;
    (input, request)
}

fn nested_relationship_fixture_with_container(
    policy: Option<FigureOutboundReferenceSplit>,
    indirect: bool,
) -> (Vec<u8>, LinkedStoryRequest) {
    let (input, mut request, _, _) = nested_fixture(true);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let paragraph = request.paragraphs[0].id.clone();
    let (figure, caption) = {
        let tags = request.source_tags.as_mut().unwrap();
        let figure = tags.figures["nested-figure"].source.clone().unwrap();
        let caption = tags.paragraph_sources[&paragraph].clone().unwrap();
        let binding = tags.figures.get_mut("nested-figure").unwrap();
        binding.split_reused_form_semantics = true;
        binding.outbound_ref_split = policy;
        (figure, caption)
    };
    let mut dictionary = store.dict((figure.object, figure.generation)).unwrap();
    let target = reference((caption.object, caption.generation));
    let relationship = if indirect {
        reference(store.add(PdfObject::Array(vec![target])).unwrap())
    } else {
        target
    };
    dictionary.insert("Ref", relationship);
    store
        .replace_dict((figure.object, figure.generation), dictionary)
        .unwrap();
    let input = write_store(store).unwrap();
    let input = rebuild_owner_trees(&input, None).unwrap().0;
    request.input_sha256 = hash(&input);
    (input, request)
}

fn nested_relationship_fixture(
    policy: Option<FigureOutboundReferenceSplit>,
) -> (Vec<u8>, LinkedStoryRequest) {
    nested_relationship_fixture_with_container(policy, false)
}

fn nested_incoming_relationship_fixture_with_container(
    policy: Option<FigureIncomingReferenceSplit>,
    indirect: bool,
) -> (Vec<u8>, LinkedStoryRequest) {
    let (input, mut request, _, _) = nested_fixture(true);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let paragraph = request.paragraphs[0].id.clone();
    let (figure, caption) = {
        let tags = request.source_tags.as_mut().unwrap();
        let figure = tags.figures["nested-figure"].source.clone().unwrap();
        let caption = tags.paragraph_sources[&paragraph].clone().unwrap();
        let binding = tags.figures.get_mut("nested-figure").unwrap();
        binding.split_reused_form_semantics = true;
        binding.inbound_ref_split = policy;
        (figure, caption)
    };
    let mut dictionary = store.dict((caption.object, caption.generation)).unwrap();
    let target = reference((figure.object, figure.generation));
    let relationship = if indirect {
        reference(store.add(PdfObject::Array(vec![target])).unwrap())
    } else {
        target
    };
    dictionary.insert("Ref", relationship.clone());
    store
        .replace_dict((caption.object, caption.generation), dictionary)
        .unwrap();
    if indirect {
        let parent = request.source_tags.as_ref().unwrap().parent.clone();
        let parent = (parent.object, parent.generation);
        let mut bystander = PdfDictionary::empty();
        bystander.insert("Type", PdfObject::Name("StructElem".into()));
        bystander.insert("S", PdfObject::Name("Span".into()));
        bystander.insert("P", reference(parent));
        bystander.insert(
            "ID",
            PdfObject::String(b"shared-indirect-ref-bystander".to_vec()),
        );
        bystander.insert("K", PdfObject::Null);
        bystander.insert("Ref", relationship);
        let bystander = store.add(PdfObject::Dictionary(bystander)).unwrap();
        let mut parent_dictionary = store.dict(parent).unwrap();
        let mut children = kids(&parent_dictionary);
        children.push(reference(bystander));
        parent_dictionary.insert("K", PdfObject::Array(children));
        store.replace_dict(parent, parent_dictionary).unwrap();
    }
    let input = write_store(store).unwrap();
    let input = rebuild_owner_trees(&input, None).unwrap().0;
    request.input_sha256 = hash(&input);
    (input, request)
}

fn nested_incoming_relationship_fixture(
    policy: Option<FigureIncomingReferenceSplit>,
) -> (Vec<u8>, LinkedStoryRequest) {
    nested_incoming_relationship_fixture_with_container(policy, false)
}

fn relationship_targets(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
) -> Vec<ObjectRef> {
    let graph = ref_graph(store, value, index).unwrap();
    let mut targets = Vec::new();
    ref_graph_targets(&graph, &mut targets);
    targets
}

fn nest_indirect_relationship(input: Vec<u8>, owner: ObjectRef) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let outer = store.dict(owner).unwrap().get_reference("Ref").unwrap();
    let target = match store.get(outer).unwrap() {
        PdfObject::Array(mut values) if values.len() == 1 => values.remove(0),
        _ => panic!("fixture /Ref owner is not a single-target indirect array"),
    };
    let inner = store.add(PdfObject::Array(vec![target])).unwrap();
    store.updates.insert(
        outer,
        PdfObject::Array(vec![PdfObject::Array(vec![reference(inner)])]),
    );
    write_store(store).unwrap()
}

fn assert_nested_relationship(
    store: &Store<'_>,
    index: &StructureIndex,
    value: &PdfObject,
) -> Vec<ObjectRef> {
    let graph = ref_graph(store, value, index).unwrap();
    assert!(matches!(
        &graph,
        RefGraph::Indirect { children, .. }
            if matches!(children.as_slice(), [RefGraph::Direct(direct)]
                if matches!(direct.as_slice(), [RefGraph::Indirect { .. }]))
    ));
    let mut targets = Vec::new();
    ref_graph_targets(&graph, &mut targets);
    targets
}

fn assert_figure_owners(input: &[u8], request: &LinkedStoryRequest) {
    validate_parent_tree(input).unwrap();
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let pages = engine.document().get_pages().unwrap();
    let ids = pages
        .iter()
        .map(|p| (p.object_number, p.generation_number))
        .collect();
    let lookup = Lookup::new(&store, root, &index, &ids).unwrap();
    let tags = request.source_tags.as_ref().unwrap();
    for figure in &request.figures {
        let node = resolve_tag(
            root,
            &index,
            &lookup,
            tags.figures[&figure.id].source.as_ref().unwrap(),
        )
        .unwrap();
        let dict = store.dict(node).unwrap();
        assert_eq!(dict.get_name("S"), Some("Figure"));
        let ImageFragmentSource::Owned { binding } = &figure.source else {
            panic!("not rebound")
        };
        let page = &pages[binding.page - 1];
        let content = kids(&dict);
        let mcr = content
            .iter()
            .filter_map(PdfObject::as_dict)
            .filter(|dictionary| dictionary.get_name("Type") == Some("MCR"))
            .collect::<Vec<_>>();
        assert_eq!(mcr.len(), 1);
        let mcr = mcr[0];
        assert_eq!(
            mcr.get_reference("Pg"),
            Some((page.object_number, page.generation_number))
        );
        assert_eq!(
            index.marked[&(page.object_number, page.generation_number)]
                [&(mcr.get_integer("MCID").unwrap() as usize)],
            node
        );
        let resolver = attributes::Resolver::new(&store).unwrap();
        let attrs = resolver
            .effective(&store, &dict, &mut attributes::Budget::default())
            .unwrap();
        let layout = attrs
            .iter()
            .filter_map(PdfObject::as_dict)
            .find(|d| d.get_name("O") == Some("Layout"))
            .unwrap();
        assert_eq!(
            layout
                .get_array("BBox")
                .unwrap()
                .iter()
                .map(|v| v.as_number().unwrap())
                .collect::<Vec<_>>(),
            binding.rect.to_vec()
        );
        assert_eq!(layout.get_name("Placement"), Some("Block"));
    }
}

#[test]
fn tagged_figures_preserve_descriptions_and_flow_through_canonical_insertion_and_reopen() {
    let (input, mut request) = fixture();
    let mut tail = request.paragraphs[0].clone();
    tail.id = "new-tail".into();
    tail.text = "additional paragraph ".repeat(120);
    tail.break_before = true;
    request
        .source_tags
        .as_mut()
        .unwrap()
        .paragraph_sources
        .insert(tail.id.clone(), None);
    request.paragraphs.push(tail);
    let (output, preview) = apply_linked_story(&input, &request).unwrap();
    assert!(preview.generated_pages > 0);
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_figure_owners(&output, &saved);
    let descriptions = sources(&output)
        .unwrap()
        .into_iter()
        .filter(|s| s.role == "Figure")
        .map(|s| s.alternate.unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        descriptions,
        BTreeSet::from(["Original fig1".into(), "Original fig2".into()])
    );
    saved.paragraphs.pop();
    saved
        .source_tags
        .as_mut()
        .unwrap()
        .paragraph_sources
        .remove("new-tail");
    saved.paragraphs[1].break_before = false;
    let (contracted, _) = apply_linked_story(&output, &saved).unwrap();
    let rebound = load_linked_stories(&contracted).unwrap().remove(0).request;
    assert_figure_owners(&contracted, &rebound);
    assert!(rebound
        .figures
        .iter()
        .all(|f| matches!(&f.source,ImageFragmentSource::Owned{binding} if binding.page==1)));
    let (again, _) = apply_linked_story(&contracted, &rebound).unwrap();
    assert_figure_owners(
        &again,
        &load_linked_stories(&again).unwrap().remove(0).request,
    );
}

#[test]
fn tagged_figure_preserves_a_contentless_semantic_subtree_without_flattening() {
    let (input, request) = semantic_subtree_fixture();
    let figure_id = request.figures[0].id.clone();
    let mut omitted = request.clone();
    omitted
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .preserve_semantic_subtree = false;
    assert!(preview_linked_story(&input, &omitted).is_err());

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(saved.source_tags.as_ref().unwrap().figures[&figure_id].preserve_semantic_subtree);
    assert_figure_owners(&output, &saved);

    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let figure = resolve_tag(
        root,
        &index,
        &lookup,
        saved.source_tags.as_ref().unwrap().figures[&figure_id]
            .source
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let find_id = |id: &[u8]| {
        index
            .nodes
            .iter()
            .copied()
            .find(|&node| {
                store
                    .dict(node)
                    .ok()
                    .and_then(|dictionary| dictionary.get("ID").cloned())
                    .and_then(|value| value.as_string().map(ToOwned::to_owned))
                    .as_deref()
                    == Some(id)
            })
            .unwrap()
    };
    let group = find_id(b"figure-semantic-group");
    let leaf = find_id(b"figure-semantic-leaf");
    let figure_children = kids(&store.dict(figure).unwrap());
    assert_eq!(figure_children.len(), 2);
    assert!(figure_children.iter().any(|value| {
        value
            .as_dict()
            .is_some_and(|mcr| mcr.get_name("Type") == Some("MCR"))
    }));
    assert!(figure_children
        .iter()
        .any(|value| value.as_reference() == Some(group)));
    let group_dictionary = store.dict(group).unwrap();
    assert_eq!(group_dictionary.get_reference("P"), Some(figure));
    assert_eq!(group_dictionary.get_reference("K"), Some(leaf));
    assert_eq!(
        group_dictionary.get("Lang").and_then(PdfObject::as_string),
        Some(b"en-GB".as_slice())
    );
    assert_eq!(
        semantic_string(&store, &group_dictionary, "Alt")
            .unwrap()
            .as_deref(),
        Some("Preserved semantic group")
    );
    let leaf_dictionary = store.dict(leaf).unwrap();
    assert_eq!(leaf_dictionary.get_reference("P"), Some(group));
    assert_eq!(
        semantic_string(&store, &leaf_dictionary, "E")
            .unwrap()
            .as_deref(),
        Some("Preserved expansion")
    );

    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    let rebound = load_linked_stories(&again).unwrap().remove(0).request;
    assert!(rebound.source_tags.as_ref().unwrap().figures[&figure_id].preserve_semantic_subtree);

    let mut removal = saved;
    let figure = removal.figures.remove(0);
    let removed_id = figure.id.clone();
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("saved Figure source is not owned")
    };
    removal
        .figure_removals
        .push(crate::linked_stories::figures::StoryFigureRemoval {
            figure_id: figure.id,
            binding,
        });
    assert!(preview_linked_story(&output, &removal)
        .unwrap_err()
        .to_string()
        .contains("complete-subtree approval"));
    removal
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&removed_id)
        .unwrap()
        .delete_semantic_subtree = true;
    let (deleted, _) = apply_linked_story(&output, &removal).unwrap();
    validate_parent_tree(&deleted).unwrap();
    let saved_after = load_linked_stories(&deleted).unwrap().remove(0).request;
    assert!(!saved_after
        .figures
        .iter()
        .any(|figure| figure.id == removed_id));
    assert!(saved_after.figure_removals.is_empty());
    assert!(!saved_after
        .source_tags
        .as_ref()
        .unwrap()
        .figures
        .contains_key(&removed_id));
    let engine = ContentEngine::open_bytes(deleted.clone()).unwrap();
    let (store, _, index) = index_document(&engine, None).unwrap();
    for id in [
        b"figure-semantic-group".as_slice(),
        b"figure-semantic-leaf".as_slice(),
    ] {
        assert!(!index.nodes.iter().copied().any(|node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(id)
        }));
    }
    let (again, _) = apply_linked_story(&deleted, &saved_after).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn tagged_figure_rebinds_page_bound_semantic_descendants_to_the_destination_page() {
    let (input, mut request) = semantic_subtree_fixture();
    let figure_id = request.figures[0].id.clone();
    let destination_page = preview_linked_story(&input, &request)
        .unwrap()
        .frames
        .iter()
        .find(|frame| {
            frame
                .figures
                .iter()
                .any(|figure| figure.figure_id == figure_id)
        })
        .unwrap()
        .frame
        .page;
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let page = engine
        .document()
        .get_pages()
        .unwrap()
        .into_iter()
        .find(|page| page.page_number != destination_page)
        .unwrap();
    let old_page = (page.object_number, page.generation_number);
    let group = index
        .nodes
        .iter()
        .copied()
        .find(|&node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"figure-semantic-group")
        })
        .unwrap();
    let mut dictionary = store.dict(group).unwrap();
    dictionary.insert(
        "Pg",
        reference((page.object_number, page.generation_number)),
    );
    store.replace_dict(group, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let figure = resolve_tag(
        root,
        &index,
        &lookup,
        saved.source_tags.as_ref().unwrap().figures[&figure_id]
            .source
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let group = index
        .nodes
        .iter()
        .copied()
        .find(|&node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"figure-semantic-group")
        })
        .unwrap();
    let figure_page = kids(&store.dict(figure).unwrap())
        .into_iter()
        .find_map(|value| {
            value
                .as_dict()
                .and_then(|dictionary| dictionary.get_reference("Pg"))
        })
        .unwrap();
    assert_eq!(
        store.dict(group).unwrap().get_reference("Pg"),
        Some(figure_page)
    );
    assert_ne!(figure_page, old_page);
}

#[test]
fn tagged_figure_refuses_a_malformed_semantic_descendant_page_binding() {
    let (input, mut request) = semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let group = index
        .nodes
        .iter()
        .copied()
        .find(|&node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"figure-semantic-group")
        })
        .unwrap();
    let mut dictionary = store.dict(group).unwrap();
    dictionary.insert("Pg", PdfObject::Integer(1));
    store.replace_dict(group, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    let error = preview_linked_story(&input, &request).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("structure Pg must be an indirect page reference"),
        "{error}"
    );
}

#[test]
fn tagged_figure_subtree_deletion_refuses_an_external_descendant_relationship() {
    let (input, request) = semantic_subtree_fixture();
    let (input, _) = apply_linked_story(&input, &request).unwrap();
    let mut request = load_linked_stories(&input).unwrap().remove(0).request;
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let find_id = |id: &[u8]| {
        index
            .nodes
            .iter()
            .copied()
            .find(|&node| {
                store
                    .dict(node)
                    .ok()
                    .and_then(|dictionary| dictionary.get("ID").cloned())
                    .and_then(|value| value.as_string().map(ToOwned::to_owned))
                    .as_deref()
                    == Some(id)
            })
            .unwrap()
    };
    let group = find_id(b"figure-semantic-group");
    let caption = find_id(b"cap2");
    let mut dictionary = store.dict(caption).unwrap();
    dictionary.insert("Ref", reference(group));
    store.replace_dict(caption, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    let figure = request.figures.remove(0);
    let figure_id = figure.id.clone();
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("source Figure is not owned")
    };
    request
        .figure_removals
        .push(crate::linked_stories::figures::StoryFigureRemoval {
            figure_id: figure.id,
            binding,
        });
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .delete_semantic_subtree = true;

    let error = preview_linked_story(&input, &request).unwrap_err();
    assert!(
        error.to_string().contains("removed structure owner"),
        "{error}"
    );
}

#[test]
fn tagged_figure_refuses_a_descendant_shared_through_an_external_k_entry() {
    let (input, mut request) = semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let find_id = |id: &[u8]| {
        index
            .nodes
            .iter()
            .copied()
            .find(|&node| {
                store
                    .dict(node)
                    .ok()
                    .and_then(|dictionary| dictionary.get("ID").cloned())
                    .and_then(|value| value.as_string().map(ToOwned::to_owned))
                    .as_deref()
                    == Some(id)
            })
            .unwrap()
    };
    let group = find_id(b"figure-semantic-group");
    let caption = find_id(b"cap2");
    let mut dictionary = store.dict(caption).unwrap();
    let mut content = kids(&dictionary);
    content.push(reference(group));
    dictionary.insert("K", PdfObject::Array(content));
    store.replace_dict(caption, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    let error = preview_linked_story(&input, &request).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cyclic/shared structure element requires an explicit ownership decision"),
        "{error}"
    );
}

#[test]
fn unique_nested_form_figure_moves_its_mcr_to_the_generated_page_owner() {
    let (input, request, leaf, _) = nested_fixture(false);
    let before = ContentEngine::open_bytes(input.clone()).unwrap();
    let preview = preview_linked_story(&input, &request).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let after = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(
        before
            .document()
            .reader()
            .get_object(leaf.0, leaf.1)
            .unwrap(),
        after
            .document()
            .reader()
            .get_object(leaf.0, leaf.1)
            .unwrap()
    );
    assert!(after.document().reader().object_ids().iter().all(|object| {
        after
            .document()
            .reader()
            .get_object(object.0, object.1)
            .ok()
            .and_then(|value| dictionary(&value).cloned())
            .is_none_or(|dictionary| !dictionary.contains_key("WFNestedImageSource"))
    }));
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_figure_owners(&output, &saved);
    assert!(sources(&output).unwrap().iter().any(|source| {
        source.role == "Figure" && source.alternate.as_deref() == Some("Nested source Figure")
    }));
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn unique_nested_form_figure_moves_its_exact_invisible_ocr_with_the_image() {
    let (input, request, _, _) = nested_fixture_with_ocr(false, true, false);
    assert!(request.figures[0]
        .ocr
        .as_ref()
        .and_then(|ocr| ocr.form_target.as_ref())
        .is_some());
    let preview = preview_linked_story(&input, &request).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    assert!(ContentEngine::open_bytes(output.clone())
        .unwrap()
        .get_page_text(1)
        .unwrap()
        .contains("NESTED OCR"));
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(matches!(
        &saved.figures[0].source,
        ImageFragmentSource::Owned { .. }
    ));
    assert!(saved.figures[0].ocr.is_none());
    assert_figure_owners(&output, &saved);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn unique_nested_form_figure_consumes_one_explicit_separate_ocr_owner() {
    let (input, request, _, _) = nested_fixture_with_ocr(false, true, true);
    let preview = preview_linked_story(&input, &request).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(
        engine
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        1
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(matches!(
        &saved.figures[0].source,
        ImageFragmentSource::Owned { .. }
    ));
    assert!(saved.figures[0].ocr.is_none());
    assert!(saved.source_tags.as_ref().unwrap().figures["nested-figure"]
        .separate_ocr_owner
        .is_none());
    assert_eq!(
        sources(&output)
            .unwrap()
            .iter()
            .filter(|source| source.role == "Span")
            .count(),
        0
    );
    assert_figure_owners(&output, &saved);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(again)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        1
    );
}

#[test]
fn unique_nested_form_figure_consumes_multiple_exact_ocr_owners() {
    let (input, request, _, _) = nested_fixture_with_ocr_owners(false, true, 2);
    let preview = preview_linked_story(&input, &request).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        2
    );
    assert_eq!(
        sources(&output)
            .unwrap()
            .iter()
            .filter(|source| source.role == "Span")
            .count(),
        0
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let binding = &saved.source_tags.as_ref().unwrap().figures["nested-figure"];
    assert!(binding.separate_ocr_owner.is_none() && binding.separate_ocr_owners.is_empty());
    assert_figure_owners(&output, &saved);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(again)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        2
    );
}

#[test]
fn reused_nested_form_preserves_a_residual_separate_ocr_owner() {
    let (input, mut request, _, _) = nested_fixture_with_ocr(true, true, true);
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap()
        .split_reused_form_semantics = true;
    let preview = preview_linked_story(&input, &request).unwrap();
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    let tags = sources(&output).unwrap();
    assert_eq!(
        tags.iter().filter(|source| source.role == "Figure").count(),
        2
    );
    assert_eq!(
        tags.iter().filter(|source| source.role == "Span").count(),
        1
    );
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        2
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.source_tags.as_ref().unwrap().insert_at, Some(2));
    assert!(
        !saved.source_tags.as_ref().unwrap().figures["nested-figure"].split_reused_form_semantics
    );
    assert!(saved.source_tags.as_ref().unwrap().figures["nested-figure"]
        .separate_ocr_owner
        .is_none());
    assert_figure_owners(&output, &saved);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(again)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        2
    );
}

#[test]
fn reused_nested_form_preserves_ordered_residual_multi_owner_ocr() {
    let (input, mut request, _, _) = nested_fixture_with_ocr_owners(true, true, 2);
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap()
        .split_reused_form_semantics = true;
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let tags = sources(&output).unwrap();
    assert_eq!(
        tags.iter().filter(|source| source.role == "Figure").count(),
        2
    );
    assert_eq!(
        tags.iter().filter(|source| source.role == "Span").count(),
        2
    );
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        4
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.source_tags.as_ref().unwrap().insert_at, Some(3));
    let binding = &saved.source_tags.as_ref().unwrap().figures["nested-figure"];
    assert!(!binding.split_reused_form_semantics);
    assert!(binding.separate_ocr_owner.is_none() && binding.separate_ocr_owners.is_empty());
    assert_figure_owners(&output, &saved);
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(again)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .matches("NESTED OCR")
            .count(),
        4
    );
}

#[test]
fn nested_form_ocr_target_must_match_the_selected_image_invocation() {
    let (input, mut request, _, _) = nested_fixture_with_ocr(false, true, false);
    request.figures[0]
        .ocr
        .as_mut()
        .unwrap()
        .form_target
        .as_mut()
        .unwrap()
        .invocation_path
        .pop();
    assert!(preview_linked_story(&input, &request).is_err());
}

#[test]
fn reused_nested_tagged_form_requires_an_explicit_semantic_split_decision() {
    let (input, request, _, _) = nested_fixture(true);
    assert!(preview_linked_story(&input, &request).is_err());
}

#[test]
fn reused_nested_tagged_form_splits_residual_semantics_when_explicitly_approved() {
    let (input, mut request, leaf, image) = nested_fixture(true);
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap()
        .split_reused_form_semantics = true;

    let preview = preview_linked_story(&input, &request).unwrap();
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        1
    );
    validate_parent_tree(&output).unwrap();

    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved.source_tags.as_ref().unwrap();
    assert!(!tags.figures["nested-figure"].split_reused_form_semantics);
    assert_eq!(tags.insert_at, Some(1));
    assert_figure_owners(&output, &saved);

    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let pages = engine.document().get_pages().unwrap();
    let page_ids = pages
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
    let moved = resolve_tag(
        root,
        &index,
        &lookup,
        tags.figures["nested-figure"].source.as_ref().unwrap(),
    )
    .unwrap();
    let parent_kids = kids(&store.dict(parent).unwrap());
    assert_eq!(parent_kids[1].as_reference(), Some(moved));
    let residual = parent_kids[0].as_reference().unwrap();
    let residual_dict = store.dict(residual).unwrap();
    assert_ne!(residual, moved);
    assert!(figure_role(&store, root, residual).unwrap());
    assert_eq!(
        semantic_string(&store, &residual_dict, "Alt")
            .unwrap()
            .as_deref(),
        Some("Nested source Figure")
    );
    assert!(!residual_dict.contains_key("ID"));
    assert!(!residual_dict.contains_key("WFStoryTagKey"));
    assert!(!residual_dict.contains_key("WFStoryID"));
    let residual_mcr = kids(&residual_dict).remove(0).as_dict().unwrap().clone();
    assert_eq!(residual_mcr.get_reference("Stm"), Some(leaf));
    assert_eq!(residual_mcr.get_integer("MCID"), Some(0));

    let moved_dict = store.dict(moved).unwrap();
    let moved_mcr = kids(&moved_dict).remove(0).as_dict().unwrap().clone();
    assert!(moved_mcr.get_reference("Pg").is_some());
    assert!(moved_mcr.get_reference("Stm").is_none());
    assert_eq!(
        crate::universal_editing::universal_image_occurrences_v2(
            &output,
            &pages
                .iter()
                .map(|page| page.page_number)
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .iter()
        .filter(|occurrence| {
            occurrence.object_number == Some(image.0) && occurrence.generation == Some(image.1)
        })
        .count(),
        2
    );
    assert!(store.reader.object_ids().iter().all(|object| {
        store
            .reader
            .get_object(object.0, object.1)
            .ok()
            .and_then(|value| dictionary(&value).cloned())
            .is_none_or(|dictionary| !dictionary.contains_key("WFNestedImageSource"))
    }));

    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn reused_nested_form_clones_an_approved_contentless_semantic_subtree() {
    let (input, request) = reused_form_semantic_subtree_fixture();
    let mut missing_approval = request.clone();
    missing_approval
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap()
        .clone_semantic_subtree_for_reused_form = false;
    assert!(preview_linked_story(&input, &missing_approval)
        .unwrap_err()
        .to_string()
        .contains("explicit clone approval"));

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let binding = &saved.source_tags.as_ref().unwrap().figures["nested-figure"];
    assert!(binding.preserve_semantic_subtree);
    assert!(!binding.split_reused_form_semantics);
    assert!(!binding.clone_semantic_subtree_for_reused_form);

    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let parent = resolve_tag(
        root,
        &index,
        &lookup,
        &saved.source_tags.as_ref().unwrap().parent,
    )
    .unwrap();
    let moved = resolve_tag(root, &index, &lookup, binding.source.as_ref().unwrap()).unwrap();
    let parent_children = kids(&store.dict(parent).unwrap());
    let residual = parent_children[0].as_reference().unwrap();
    assert_ne!(residual, moved);

    let original_group = index
        .nodes
        .iter()
        .copied()
        .find(|&node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"nested-semantic-group")
        })
        .unwrap();
    assert!(kids(&store.dict(moved).unwrap())
        .iter()
        .any(|value| value.as_reference() == Some(original_group)));

    let residual_group = kids(&store.dict(residual).unwrap())
        .into_iter()
        .filter_map(|value| value.as_reference())
        .find(|&node| store.dict(node).unwrap().get_name("S") == Some("Div"))
        .unwrap();
    assert_ne!(residual_group, original_group);
    let residual_group_dictionary = store.dict(residual_group).unwrap();
    assert_eq!(residual_group_dictionary.get_reference("P"), Some(residual));
    assert!(!residual_group_dictionary.contains_key("ID"));
    assert!(!residual_group_dictionary.contains_key("Pg"));
    assert_eq!(
        semantic_string(&store, &residual_group_dictionary, "Alt")
            .unwrap()
            .as_deref(),
        Some("Nested preserved group")
    );
    let residual_leaf = residual_group_dictionary.get_reference("K").unwrap();
    let residual_leaf_dictionary = store.dict(residual_leaf).unwrap();
    assert_eq!(
        residual_leaf_dictionary.get_reference("P"),
        Some(residual_group)
    );
    assert!(!residual_leaf_dictionary.contains_key("ID"));
    assert!(!residual_leaf_dictionary.contains_key("Pg"));
    assert_eq!(
        semantic_string(&store, &residual_leaf_dictionary, "E")
            .unwrap()
            .as_deref(),
        Some("Nested preserved expansion")
    );

    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn reused_nested_form_subtree_clone_rewrites_internal_descendant_relationships() {
    let (input, mut request) = reused_form_semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let find_id = |id: &[u8]| {
        index
            .nodes
            .iter()
            .copied()
            .find(|&node| {
                store
                    .dict(node)
                    .ok()
                    .and_then(|dictionary| dictionary.get("ID").cloned())
                    .and_then(|value| value.as_string().map(ToOwned::to_owned))
                    .as_deref()
                    == Some(id)
            })
            .unwrap()
    };
    let group = find_id(b"nested-semantic-group");
    let leaf = find_id(b"nested-semantic-leaf");
    let mut dictionary = store.dict(group).unwrap();
    dictionary.insert("Ref", reference(leaf));
    store.replace_dict(group, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let parent = resolve_tag(
        root,
        &index,
        &lookup,
        &saved.source_tags.as_ref().unwrap().parent,
    )
    .unwrap();
    let moved = resolve_tag(
        root,
        &index,
        &lookup,
        saved.source_tags.as_ref().unwrap().figures["nested-figure"]
            .source
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let residual = kids(&store.dict(parent).unwrap())
        .into_iter()
        .filter_map(|value| value.as_reference())
        .find(|node| *node != moved && figure_role(&store, root, *node).unwrap())
        .unwrap();
    let residual_group = kids(&store.dict(residual).unwrap())
        .into_iter()
        .filter_map(|value| value.as_reference())
        .find(|node| store.dict(*node).unwrap().get_name("S") == Some("Div"))
        .unwrap();
    let residual_leaf = store
        .dict(residual_group)
        .unwrap()
        .get_reference("K")
        .unwrap();
    assert_eq!(
        store.dict(residual_group).unwrap().get_reference("Ref"),
        Some(residual_leaf)
    );
    let original_group = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"nested-semantic-group")
        })
        .unwrap();
    let original_leaf = store
        .dict(original_group)
        .unwrap()
        .get_reference("K")
        .unwrap();
    assert_eq!(
        store.dict(original_group).unwrap().get_reference("Ref"),
        Some(original_leaf)
    );
    assert_ne!(residual_leaf, original_leaf);
}

#[test]
fn reused_nested_form_subtree_clone_refuses_external_descendant_relationships() {
    let (input, mut request) = reused_form_semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let group = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"nested-semantic-group")
        })
        .unwrap();
    let caption = request.source_tags.as_ref().unwrap().paragraph_sources
        [&request.paragraphs[0].id]
        .as_ref()
        .unwrap();
    let mut dictionary = store.dict(group).unwrap();
    dictionary.insert("Ref", reference((caption.object, caption.generation)));
    store.replace_dict(group, dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);

    assert!(preview_linked_story(&input, &request)
        .unwrap_err()
        .to_string()
        .contains("outbound /Ref relationships requiring an explicit split policy"));
}

#[test]
fn reused_nested_form_subtree_applies_external_relationship_policies_to_descendants() {
    let (input, mut request) = reused_form_semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let group = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"nested-semantic-group")
        })
        .unwrap();
    let caption_reference = request.source_tags.as_ref().unwrap().paragraph_sources
        [&request.paragraphs[0].id]
        .as_ref()
        .unwrap()
        .clone();
    let caption = (caption_reference.object, caption_reference.generation);
    let mut group_dictionary = store.dict(group).unwrap();
    group_dictionary.insert("Ref", reference(caption));
    store.replace_dict(group, group_dictionary).unwrap();
    let mut caption_dictionary = store.dict(caption).unwrap();
    caption_dictionary.insert("Ref", reference(group));
    store.replace_dict(caption, caption_dictionary).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = hash(&input);
    let binding = request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut("nested-figure")
        .unwrap();
    binding.outbound_ref_split = Some(FigureOutboundReferenceSplit::CopyToBoth);
    binding.inbound_ref_split = Some(FigureIncomingReferenceSplit::ReferenceBoth);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let tags = saved.source_tags.as_ref().unwrap();
    let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
    let moved = resolve_tag(
        root,
        &index,
        &lookup,
        tags.figures["nested-figure"].source.as_ref().unwrap(),
    )
    .unwrap();
    let caption = resolve_tag(
        root,
        &index,
        &lookup,
        tags.paragraph_sources[&saved.paragraphs[0].id]
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let residual = kids(&store.dict(parent).unwrap())
        .into_iter()
        .filter_map(|value| value.as_reference())
        .find(|node| *node != moved && figure_role(&store, root, *node).unwrap())
        .unwrap();
    let residual_group = kids(&store.dict(residual).unwrap())
        .into_iter()
        .filter_map(|value| value.as_reference())
        .find(|node| store.dict(*node).unwrap().get_name("S") == Some("Div"))
        .unwrap();
    let original_group = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"nested-semantic-group")
        })
        .unwrap();
    assert_eq!(
        store.dict(original_group).unwrap().get_reference("Ref"),
        Some(caption)
    );
    assert_eq!(
        store.dict(residual_group).unwrap().get_reference("Ref"),
        Some(caption)
    );
    assert_eq!(
        store
            .dict(caption)
            .unwrap()
            .get("Ref")
            .and_then(PdfObject::as_array)
            .unwrap()
            .iter()
            .filter_map(PdfObject::as_reference)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([original_group, residual_group])
    );
}

#[test]
fn coordinated_figure_split_maps_cross_tree_relationships_to_peer_clones() {
    let (input, request) = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let tags = request.source_tags.as_ref().unwrap();
    let first = tags.figures[&request.figures[0].id]
        .source
        .as_ref()
        .unwrap();
    let second = tags.figures[&request.figures[1].id]
        .source
        .as_ref()
        .unwrap();
    let first = (first.object, first.generation);
    let second = (second.object, second.generation);
    let mut first_dictionary = store.dict(first).unwrap();
    first_dictionary.insert("Ref", reference(second));
    store.replace_dict(first, first_dictionary).unwrap();
    let mut second_dictionary = store.dict(second).unwrap();
    second_dictionary.insert("Ref", reference(first));
    store.replace_dict(second, second_dictionary).unwrap();
    let first_clone_dictionary = store.dict(first).unwrap();
    let first_clone = store
        .add(PdfObject::Dictionary(first_clone_dictionary))
        .unwrap();
    let second_clone_dictionary = store.dict(second).unwrap();
    let second_clone = store
        .add(PdfObject::Dictionary(second_clone_dictionary))
        .unwrap();
    let first_domain = BTreeMap::from([(first, first_clone)]);
    let second_domain = BTreeMap::from([(second, second_clone)]);
    let coordinated = BTreeMap::from([(first, first_clone), (second, second_clone)]);

    split_semantic_relationships(&mut store, &index, &first_domain, &coordinated, None).unwrap();
    split_semantic_relationships(&mut store, &index, &second_domain, &coordinated, None).unwrap();

    assert_eq!(
        store.dict(first).unwrap().get_reference("Ref"),
        Some(second)
    );
    assert_eq!(
        store.dict(second).unwrap().get_reference("Ref"),
        Some(first)
    );
    assert_eq!(
        store.dict(first_clone).unwrap().get_reference("Ref"),
        Some(second_clone)
    );
    assert_eq!(
        store.dict(second_clone).unwrap().get_reference("Ref"),
        Some(first_clone)
    );
}

#[test]
fn reused_figure_outbound_relationship_split_requires_and_honors_explicit_policy() {
    let (input, request) = nested_relationship_fixture(None);
    assert!(preview_linked_story(&input, &request).is_err());

    for (policy, selected_keeps, residual_keeps) in [
        (FigureOutboundReferenceSplit::MoveWithSelected, true, false),
        (
            FigureOutboundReferenceSplit::RetainWithResidual,
            false,
            true,
        ),
        (FigureOutboundReferenceSplit::CopyToBoth, true, true),
    ] {
        let (input, request) = nested_relationship_fixture(Some(policy));
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        validate_parent_tree(&output).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        let tags = saved.source_tags.as_ref().unwrap();
        assert!(tags.figures["nested-figure"].outbound_ref_split.is_none());

        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let (store, root, index) = index_document(&engine, None).unwrap();
        let page_ids = engine
            .document()
            .get_pages()
            .unwrap()
            .iter()
            .map(|page| (page.object_number, page.generation_number))
            .collect::<BTreeSet<_>>();
        let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
        let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
        let moved = resolve_tag(
            root,
            &index,
            &lookup,
            tags.figures["nested-figure"].source.as_ref().unwrap(),
        )
        .unwrap();
        let children = kids(&store.dict(parent).unwrap());
        let residual = children[0].as_reference().unwrap();
        assert_eq!(
            store.dict(moved).unwrap().contains_key("Ref"),
            selected_keeps
        );
        assert_eq!(
            store.dict(residual).unwrap().contains_key("Ref"),
            residual_keeps
        );

        let (again, _) = apply_linked_story(&output, &saved).unwrap();
        validate_parent_tree(&again).unwrap();
    }
}

#[test]
fn reused_figure_outbound_relationship_preserves_indirect_array_containers() {
    for (policy, selected_keeps, residual_keeps) in [
        (FigureOutboundReferenceSplit::MoveWithSelected, true, false),
        (
            FigureOutboundReferenceSplit::RetainWithResidual,
            false,
            true,
        ),
        (FigureOutboundReferenceSplit::CopyToBoth, true, true),
    ] {
        let (input, request) = nested_relationship_fixture_with_container(Some(policy), true);
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        validate_parent_tree(&output).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        let tags = saved.source_tags.as_ref().unwrap();
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let (store, root, index) = index_document(&engine, None).unwrap();
        let page_ids = engine
            .document()
            .get_pages()
            .unwrap()
            .iter()
            .map(|page| (page.object_number, page.generation_number))
            .collect::<BTreeSet<_>>();
        let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
        let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
        let moved = resolve_tag(
            root,
            &index,
            &lookup,
            tags.figures["nested-figure"].source.as_ref().unwrap(),
        )
        .unwrap();
        let caption = resolve_tag(
            root,
            &index,
            &lookup,
            tags.paragraph_sources[&saved.paragraphs[0].id]
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        let residual = kids(&store.dict(parent).unwrap())[0]
            .as_reference()
            .unwrap();
        for (node, keeps) in [(moved, selected_keeps), (residual, residual_keeps)] {
            let dictionary = store.dict(node).unwrap();
            assert_eq!(dictionary.contains_key("Ref"), keeps);
            if keeps {
                let value = dictionary.get("Ref").unwrap();
                assert!(matches!(
                    ref_graph(&store, value, &index).unwrap(),
                    RefGraph::Indirect { .. }
                ));
                assert_eq!(relationship_targets(&store, &index, value), vec![caption]);
            }
        }
        let (again, _) = apply_linked_story(&output, &saved).unwrap();
        validate_parent_tree(&again).unwrap();
    }
}

#[test]
fn reused_figure_incoming_relationship_split_requires_and_honors_explicit_policy() {
    let (input, request) = nested_incoming_relationship_fixture(None);
    assert!(preview_linked_story(&input, &request).is_err());

    for policy in [
        FigureIncomingReferenceSplit::FollowSelected,
        FigureIncomingReferenceSplit::RetargetResidual,
        FigureIncomingReferenceSplit::ReferenceBoth,
    ] {
        let (input, request) = nested_incoming_relationship_fixture(Some(policy));
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        validate_parent_tree(&output).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        let tags = saved.source_tags.as_ref().unwrap();
        assert!(tags.figures["nested-figure"].inbound_ref_split.is_none());

        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let (store, root, index) = index_document(&engine, None).unwrap();
        let page_ids = engine
            .document()
            .get_pages()
            .unwrap()
            .iter()
            .map(|page| (page.object_number, page.generation_number))
            .collect::<BTreeSet<_>>();
        let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
        let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
        let moved = resolve_tag(
            root,
            &index,
            &lookup,
            tags.figures["nested-figure"].source.as_ref().unwrap(),
        )
        .unwrap();
        let caption = resolve_tag(
            root,
            &index,
            &lookup,
            tags.paragraph_sources[&saved.paragraphs[0].id]
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        let children = kids(&store.dict(parent).unwrap());
        let residual = children[0].as_reference().unwrap();
        let relationship = store.dict(caption).unwrap().get("Ref").cloned().unwrap();
        let expected = match policy {
            FigureIncomingReferenceSplit::FollowSelected => reference(moved),
            FigureIncomingReferenceSplit::RetargetResidual => reference(residual),
            FigureIncomingReferenceSplit::ReferenceBoth => {
                PdfObject::Array(vec![reference(moved), reference(residual)])
            }
        };
        assert_eq!(relationship, expected);

        let (again, _) = apply_linked_story(&output, &saved).unwrap();
        validate_parent_tree(&again).unwrap();
    }
}

#[test]
fn reused_figure_incoming_relationship_rewrites_indirect_arrays_copy_on_write() {
    for policy in [
        FigureIncomingReferenceSplit::FollowSelected,
        FigureIncomingReferenceSplit::RetargetResidual,
        FigureIncomingReferenceSplit::ReferenceBoth,
    ] {
        let (input, request) =
            nested_incoming_relationship_fixture_with_container(Some(policy), true);
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        validate_parent_tree(&output).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        let tags = saved.source_tags.as_ref().unwrap();
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let (store, root, index) = index_document(&engine, None).unwrap();
        let page_ids = engine
            .document()
            .get_pages()
            .unwrap()
            .iter()
            .map(|page| (page.object_number, page.generation_number))
            .collect::<BTreeSet<_>>();
        let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
        let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
        let moved = resolve_tag(
            root,
            &index,
            &lookup,
            tags.figures["nested-figure"].source.as_ref().unwrap(),
        )
        .unwrap();
        let caption = resolve_tag(
            root,
            &index,
            &lookup,
            tags.paragraph_sources[&saved.paragraphs[0].id]
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        let residual = kids(&store.dict(parent).unwrap())[0]
            .as_reference()
            .unwrap();
        let bystander = index
            .nodes
            .iter()
            .copied()
            .find(|&node| {
                store
                    .dict(node)
                    .ok()
                    .and_then(|dictionary| dictionary.get("ID").cloned())
                    .and_then(|value| value.as_string().map(ToOwned::to_owned))
                    .as_deref()
                    == Some(b"shared-indirect-ref-bystander".as_slice())
            })
            .unwrap();
        let caption_value = store.dict(caption).unwrap().get("Ref").cloned().unwrap();
        let bystander_value = store.dict(bystander).unwrap().get("Ref").cloned().unwrap();
        assert!(matches!(
            ref_graph(&store, &caption_value, &index).unwrap(),
            RefGraph::Indirect { .. }
        ));
        assert!(matches!(
            ref_graph(&store, &bystander_value, &index).unwrap(),
            RefGraph::Indirect { .. }
        ));
        assert_ne!(caption_value.as_reference(), bystander_value.as_reference());
        let expected = match policy {
            FigureIncomingReferenceSplit::FollowSelected => vec![moved],
            FigureIncomingReferenceSplit::RetargetResidual => vec![residual],
            FigureIncomingReferenceSplit::ReferenceBoth => vec![moved, residual],
        };
        assert_eq!(
            relationship_targets(&store, &index, &caption_value),
            expected
        );
        assert_eq!(
            relationship_targets(&store, &index, &bystander_value),
            expected
        );
        let (again, _) = apply_linked_story(&output, &saved).unwrap();
        validate_parent_tree(&again).unwrap();
    }
}

#[test]
fn reused_figure_outbound_relationship_preserves_nested_container_topology() {
    let (input, mut request) = nested_relationship_fixture_with_container(
        Some(FigureOutboundReferenceSplit::CopyToBoth),
        true,
    );
    let source = request.source_tags.as_ref().unwrap().figures["nested-figure"]
        .source
        .as_ref()
        .unwrap();
    let input = nest_indirect_relationship(input, (source.object, source.generation));
    request.input_sha256 = hash(&input);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved.source_tags.as_ref().unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
    let moved = resolve_tag(
        root,
        &index,
        &lookup,
        tags.figures["nested-figure"].source.as_ref().unwrap(),
    )
    .unwrap();
    let caption = resolve_tag(
        root,
        &index,
        &lookup,
        tags.paragraph_sources[&saved.paragraphs[0].id]
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let residual = kids(&store.dict(parent).unwrap())[0]
        .as_reference()
        .unwrap();
    let selected_ref = store.dict(moved).unwrap().get("Ref").cloned().unwrap();
    let residual_ref = store.dict(residual).unwrap().get("Ref").cloned().unwrap();
    assert_ne!(selected_ref.as_reference(), residual_ref.as_reference());
    assert_eq!(
        assert_nested_relationship(&store, &index, &selected_ref),
        vec![caption]
    );
    assert_eq!(
        assert_nested_relationship(&store, &index, &residual_ref),
        vec![caption]
    );

    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn reused_figure_incoming_relationship_rewrites_nested_shared_containers_copy_on_write() {
    let (input, mut request) = nested_incoming_relationship_fixture_with_container(
        Some(FigureIncomingReferenceSplit::ReferenceBoth),
        true,
    );
    let paragraph = request.paragraphs[0].id.clone();
    let caption = request.source_tags.as_ref().unwrap().paragraph_sources[&paragraph]
        .as_ref()
        .unwrap();
    let input = nest_indirect_relationship(input, (caption.object, caption.generation));
    request.input_sha256 = hash(&input);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved.source_tags.as_ref().unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
    let moved = resolve_tag(
        root,
        &index,
        &lookup,
        tags.figures["nested-figure"].source.as_ref().unwrap(),
    )
    .unwrap();
    let caption = resolve_tag(
        root,
        &index,
        &lookup,
        tags.paragraph_sources[&saved.paragraphs[0].id]
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let residual = kids(&store.dict(parent).unwrap())[0]
        .as_reference()
        .unwrap();
    let bystander = index
        .nodes
        .iter()
        .copied()
        .find(|&node| {
            store
                .dict(node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"shared-indirect-ref-bystander".as_slice())
        })
        .unwrap();
    let caption_ref = store.dict(caption).unwrap().get("Ref").cloned().unwrap();
    let bystander_ref = store.dict(bystander).unwrap().get("Ref").cloned().unwrap();
    assert_ne!(caption_ref.as_reference(), bystander_ref.as_reference());
    for value in [&caption_ref, &bystander_ref] {
        assert_eq!(
            assert_nested_relationship(&store, &index, value),
            vec![moved, residual]
        );
    }

    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}

#[test]
fn reused_figure_relationship_split_preserves_unrelated_malformed_metadata() {
    let (input, mut request) =
        nested_incoming_relationship_fixture(Some(FigureIncomingReferenceSplit::ReferenceBoth));
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let parent = request.source_tags.as_ref().unwrap().parent.clone();
    let parent = (parent.object, parent.generation);
    let mut unrelated = PdfDictionary::empty();
    unrelated.insert("Type", PdfObject::Name("StructElem".into()));
    unrelated.insert("S", PdfObject::Name("Span".into()));
    unrelated.insert("P", reference(parent));
    unrelated.insert("ID", PdfObject::String(b"unrelated-malformed-ref".to_vec()));
    unrelated.insert("K", PdfObject::Null);
    unrelated.insert("Ref", PdfObject::Integer(7));
    let unrelated = store.add(PdfObject::Dictionary(unrelated)).unwrap();
    let mut parent_dictionary = store.dict(parent).unwrap();
    let mut children = kids(&parent_dictionary);
    children.push(reference(unrelated));
    parent_dictionary.insert("K", PdfObject::Array(children));
    store.replace_dict(parent, parent_dictionary).unwrap();
    let input = write_store(store).unwrap();
    // This fixture adds a real structure element with an /ID.  Keep the
    // fixture itself valid so the operation under test reaches the unrelated
    // malformed /Ref value instead of failing earlier on a stale /IDTree.
    let input = rebuild_owner_trees(&input, None).unwrap().0;
    request.input_sha256 = hash(&input);

    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, _, index) = index_document(&engine, None).unwrap();
    let unrelated = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"unrelated-malformed-ref".as_slice())
        })
        .unwrap();
    assert_eq!(store.dict(unrelated).unwrap().get_integer("Ref"), Some(7));
}

#[test]
fn reused_figure_indirect_relationship_containers_refuse_mixed_values_and_cycles() {
    let (input, mut request) = nested_relationship_fixture_with_container(
        Some(FigureOutboundReferenceSplit::CopyToBoth),
        true,
    );
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let figure = request.source_tags.as_ref().unwrap().figures["nested-figure"]
        .source
        .as_ref()
        .unwrap();
    let container = store
        .dict((figure.object, figure.generation))
        .unwrap()
        .get_reference("Ref")
        .unwrap();
    let mut values = match store.get(container).unwrap() {
        PdfObject::Array(values) => values,
        _ => panic!("fixture /Ref container is not an array"),
    };
    values.push(PdfObject::Integer(7));
    store.updates.insert(container, PdfObject::Array(values));
    let mixed = write_store(store).unwrap();
    request.input_sha256 = hash(&mixed);
    assert!(preview_linked_story(&mixed, &request)
        .unwrap_err()
        .to_string()
        .contains("malformed container"));

    let (input, mut request) = nested_incoming_relationship_fixture_with_container(
        Some(FigureIncomingReferenceSplit::ReferenceBoth),
        true,
    );
    let engine = ContentEngine::open_bytes(input).unwrap();
    let mut store = Store::new(engine.document().reader());
    let paragraph = request.paragraphs[0].id.clone();
    let caption = request.source_tags.as_ref().unwrap().paragraph_sources[&paragraph]
        .as_ref()
        .unwrap();
    let container = store
        .dict((caption.object, caption.generation))
        .unwrap()
        .get_reference("Ref")
        .unwrap();
    store
        .updates
        .insert(container, PdfObject::Array(vec![reference(container)]));
    let cyclic = write_store(store).unwrap();
    request.input_sha256 = hash(&cyclic);
    assert!(preview_linked_story(&cyclic, &request)
        .unwrap_err()
        .to_string()
        .contains("cyclic Figure /Ref container graph"));
}

#[test]
fn tagged_cross_story_transfer_moves_the_existing_figure_leaf_and_rebuilds_ownership() {
    let (input, all) = fixture();
    let source_paragraph = all.paragraphs[0].id.clone();
    let target_paragraph = all.paragraphs[1].id.clone();
    let source_figure = all.figures[0].id.clone();

    let mut source = all.clone();
    source.paragraphs.truncate(1);
    source.frames.truncate(1);
    source.figures.truncate(1);
    {
        let tags = source.source_tags.as_mut().unwrap();
        tags.selected.truncate(2);
        tags.paragraph_sources
            .retain(|paragraph, _| paragraph == &source_paragraph);
        tags.figures.retain(|figure, _| figure == &source_figure);
        tags.insert_at = Some(0);
    }
    let (with_source, _) = apply_linked_story(&input, &source).unwrap();

    let mut target = all;
    target.story_id = "tagged-target-story".into();
    target.input_sha256 = crate::linked_stories::hash(&with_source);
    target.paragraphs = vec![target.paragraphs[1].clone()];
    target.frames = vec![target.frames[1].clone()];
    target.figures.clear();
    {
        let tags = target.source_tags.as_mut().unwrap();
        let caption = tags.paragraph_sources[&target_paragraph].clone().unwrap();
        tags.selected = vec![caption.clone()];
        tags.insert_at = Some(3);
        tags.paragraph_sources = BTreeMap::from([(target_paragraph.clone(), Some(caption))]);
        tags.new_roles.clear();
        tags.semantic_text.clear();
        tags.figures.clear();
    }
    let (both_saved, _) = apply_linked_story(&with_source, &target).unwrap();
    let stories = load_linked_stories(&both_saved).unwrap();
    let source = stories
        .iter()
        .find(|story| story.request.story_id == "contract-body")
        .unwrap()
        .request
        .clone();
    let figure = source
        .figures
        .iter()
        .find(|figure| figure.id == source_figure)
        .unwrap();
    let ImageFragmentSource::Owned { binding } = &figure.source else {
        panic!("saved tagged Figure is not native-owned")
    };
    let transfer = crate::linked_stories::figures::StoryFigureTransferRequest {
        input_sha256: crate::linked_stories::hash(&both_saved),
        source_story_id: source.story_id.clone(),
        source_figure_id: source_figure.clone(),
        source_binding: binding.clone(),
        target_story_id: "tagged-target-story".into(),
        target_figure_id: "moved-tagged-figure".into(),
        target_caption_paragraph: target_paragraph.clone(),
        signature_policy_override: false,
    };
    let preview =
        crate::linked_stories::figures::preview_story_figure_transfer(&both_saved, &transfer)
            .unwrap();
    let (output, report) = crate::linked_stories::figures::apply_story_figure_transfer(
        &both_saved,
        &transfer,
        &preview.plan_sha256,
    )
    .unwrap();
    assert_eq!(report.tagged_structure_owner_transferred, Some(true));
    assert_eq!(report.parent_tree_verified, Some(true));
    assert_eq!(report.physical_owner_count, 1);
    validate_parent_tree(&output).unwrap();

    let stories = load_linked_stories(&output).unwrap();
    let source = stories
        .iter()
        .find(|story| story.request.story_id == "contract-body")
        .unwrap();
    let target = stories
        .iter()
        .find(|story| story.request.story_id == "tagged-target-story")
        .unwrap();
    assert!(!source
        .request
        .figures
        .iter()
        .any(|figure| figure.id == source_figure));
    assert!(!source
        .request
        .source_tags
        .as_ref()
        .unwrap()
        .figures
        .contains_key(&source_figure));
    assert!(target
        .request
        .source_tags
        .as_ref()
        .unwrap()
        .figures
        .contains_key("moved-tagged-figure"));
    assert_figure_owners(&output, &target.request);
    assert!(sources(&output).unwrap().iter().any(|source| {
        source.role == "Figure" && source.alternate.as_deref() == Some("Original fig1")
    }));
}

#[test]
fn tagged_cross_story_transfer_preserves_a_contentless_figure_subtree() {
    let (input, mut all) = semantic_subtree_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let leaf = index
        .nodes
        .iter()
        .copied()
        .find(|node| {
            store
                .dict(*node)
                .ok()
                .and_then(|dictionary| dictionary.get("ID").cloned())
                .and_then(|value| value.as_string().map(ToOwned::to_owned))
                .as_deref()
                == Some(b"figure-semantic-leaf")
        })
        .unwrap();
    let pages = engine.document().get_pages().unwrap();
    let source_page = &pages[0];
    let mut leaf_dictionary = store.dict(leaf).unwrap();
    leaf_dictionary.insert(
        "Pg",
        reference((source_page.object_number, source_page.generation_number)),
    );
    store.replace_dict(leaf, leaf_dictionary).unwrap();
    let input = write_store(store).unwrap();
    all.input_sha256 = hash(&input);
    let source_paragraph = all.paragraphs[0].id.clone();
    let target_paragraph = all.paragraphs[1].id.clone();
    let source_figure = all.figures[0].id.clone();

    let mut source = all.clone();
    source.paragraphs.truncate(1);
    source.frames.truncate(1);
    source.figures.truncate(1);
    {
        let tags = source.source_tags.as_mut().unwrap();
        tags.selected.truncate(2);
        tags.paragraph_sources
            .retain(|paragraph, _| paragraph == &source_paragraph);
        tags.figures.retain(|figure, _| figure == &source_figure);
        tags.insert_at = Some(0);
    }
    let (with_source, _) = apply_linked_story(&input, &source).unwrap();

    let mut target = all;
    target.story_id = "tagged-subtree-target-story".into();
    target.input_sha256 = crate::linked_stories::hash(&with_source);
    target.paragraphs = vec![target.paragraphs[1].clone()];
    target.frames = vec![target.frames[1].clone()];
    target.figures.clear();
    {
        let tags = target.source_tags.as_mut().unwrap();
        let caption = tags.paragraph_sources[&target_paragraph].clone().unwrap();
        tags.selected = vec![caption.clone()];
        tags.insert_at = Some(3);
        tags.paragraph_sources = BTreeMap::from([(target_paragraph.clone(), Some(caption))]);
        tags.new_roles.clear();
        tags.semantic_text.clear();
        tags.figures.clear();
    }
    let (both_saved, _) = apply_linked_story(&with_source, &target).unwrap();
    let stories = load_linked_stories(&both_saved).unwrap();
    let source = stories
        .iter()
        .find(|story| story.request.story_id == "contract-body")
        .unwrap()
        .request
        .clone();
    let figure = source
        .figures
        .iter()
        .find(|figure| figure.id == source_figure)
        .unwrap();
    let ImageFragmentSource::Owned { binding } = &figure.source else {
        panic!("saved tagged Figure is not native-owned")
    };
    let transfer = crate::linked_stories::figures::StoryFigureTransferRequest {
        input_sha256: crate::linked_stories::hash(&both_saved),
        source_story_id: source.story_id.clone(),
        source_figure_id: source_figure.clone(),
        source_binding: binding.clone(),
        target_story_id: "tagged-subtree-target-story".into(),
        target_figure_id: "moved-tagged-subtree".into(),
        target_caption_paragraph: target_paragraph.clone(),
        signature_policy_override: false,
    };
    let preview =
        crate::linked_stories::figures::preview_story_figure_transfer(&both_saved, &transfer)
            .unwrap();
    let (output, report) = crate::linked_stories::figures::apply_story_figure_transfer(
        &both_saved,
        &transfer,
        &preview.plan_sha256,
    )
    .unwrap();
    assert_eq!(report.tagged_structure_owner_transferred, Some(true));
    validate_parent_tree(&output).unwrap();

    let stories = load_linked_stories(&output).unwrap();
    let target = stories
        .iter()
        .find(|story| story.request.story_id == "tagged-subtree-target-story")
        .unwrap();
    let binding = &target.request.source_tags.as_ref().unwrap().figures["moved-tagged-subtree"];
    assert!(binding.preserve_semantic_subtree);
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, root, index) = index_document(&engine, None).unwrap();
    let page_ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|page| (page.object_number, page.generation_number))
        .collect::<BTreeSet<_>>();
    let lookup = Lookup::new(&store, root, &index, &page_ids).unwrap();
    let moved = resolve_tag(root, &index, &lookup, binding.source.as_ref().unwrap()).unwrap();
    let descendants = validate_semantic_subtree(&store, &index, moved).unwrap();
    assert_eq!(descendants.len(), 2);
    assert!(descendants.iter().any(|node| {
        store
            .dict(*node)
            .ok()
            .and_then(|dictionary| dictionary.get("ID").cloned())
            .and_then(|value| value.as_string().map(ToOwned::to_owned))
            .as_deref()
            == Some(b"figure-semantic-leaf")
    }));
    let moved_page = kids(&store.dict(moved).unwrap())
        .into_iter()
        .find_map(|value| {
            value
                .as_dict()
                .and_then(|dictionary| dictionary.get_reference("Pg"))
        })
        .unwrap();
    assert!(descendants.iter().all(|node| {
        store.dict(*node).unwrap().get("Pg").is_none_or(|page| {
            matches!(page, PdfObject::Null) || page.as_reference() == Some(moved_page)
        })
    }));
}

#[test]
fn deleting_tagged_figures_consumes_both_paint_and_structure_owners() {
    let (input, request) = fixture();
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    for figure in saved.figures.drain(..) {
        let ImageFragmentSource::Owned { binding } = figure.source else {
            panic!("not rebound")
        };
        saved
            .figure_removals
            .push(crate::linked_stories::figures::StoryFigureRemoval {
                figure_id: figure.id,
                binding,
            });
    }
    let (deleted, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&deleted).unwrap();
    assert!(crate::image_fragments::image_fragment_bindings(&deleted)
        .unwrap()
        .is_empty());
    assert!(sources(&deleted)
        .unwrap()
        .iter()
        .all(|s| s.role != "Figure"));
    let saved = load_linked_stories(&deleted).unwrap().remove(0).request;
    assert!(saved.source_tags.as_ref().unwrap().figures.is_empty());
    assert!(saved.figure_removals.is_empty());
    assert!(apply_linked_story(&deleted, &saved).is_ok());
}
#[test]
fn mismatched_figure_caption_or_image_ownership_is_rejected_before_mutation() {
    let (input, request) = fixture();
    let mut bad = request.clone();
    let other = bad.source_tags.as_ref().unwrap().figures[&bad.figures[1].id]
        .source
        .clone();
    bad.source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&bad.figures[0].id)
        .unwrap()
        .source = other;
    assert!(preview_linked_story(&input, &bad).is_err());
    let mut bad = request.clone();
    let id = bad.paragraphs[0].id.clone();
    let source = bad.source_tags.as_ref().unwrap().figures[&bad.figures[0].id]
        .source
        .clone();
    bad.source_tags
        .as_mut()
        .unwrap()
        .paragraph_sources
        .insert(id, source);
    assert!(preview_linked_story(&input, &bad).is_err());
    let mut bad = request;
    bad.figures[0].source = bad.figures[1].source.clone();
    assert!(preview_linked_story(&input, &bad).is_err());
}
#[test]
fn figure_alternate_review_updates_meaning_without_turning_it_into_caption_text() {
    let (input, mut request) = fixture();
    let id = request.figures[0].id.clone();
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&id)
        .unwrap()
        .semantic_text = Some(StorySemanticText {
        alternate: Some("Reviewed diagram description".into()),
        expansion: None,
    });
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_figure_owners(&output, &saved);
    let tags = sources(&output).unwrap();
    assert!(tags
        .iter()
        .any(|s| s.role == "Figure"
            && s.alternate.as_deref() == Some("Reviewed diagram description")));
    assert!(tags
        .iter()
        .filter(|s| s.role == "P")
        .all(|s| s.alternate.is_none()));
}

#[test]
fn untagged_source_image_requires_review_before_creating_a_new_figure_owner() {
    let (input, mut request) = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, root, index) = index_document(&engine, None).unwrap();
    let pages = engine.document().get_pages().unwrap();
    let ids = pages
        .iter()
        .map(|p| (p.object_number, p.generation_number))
        .collect();
    let lookup = Lookup::new(&store, root, &index, &ids).unwrap();
    let tags = request.source_tags.as_mut().unwrap();
    let figure_id = request.figures[0].id.clone();
    let source = tags.figures[&figure_id].source.clone().unwrap();
    let removed = resolve_tag(root, &index, &lookup, &source).unwrap();
    let parent = resolve_tag(root, &index, &lookup, &tags.parent).unwrap();
    let mut d = store.dict(parent).unwrap();
    d.insert(
        "K",
        PdfObject::Array(
            kids(&d)
                .into_iter()
                .filter(|v| v.as_reference() != Some(removed))
                .collect(),
        ),
    );
    store.replace_dict(parent, d).unwrap();
    let marks = page_marks(&engine, &pages[0]).unwrap();
    let mut patches = StreamPatches::new();
    for mark in marks.marks.iter().filter(|m| m.mcid == Some(1)) {
        patches
            .entry(mark.stream)
            .or_default()
            .push((mark.start, mark.end, Vec::new()));
        let (stream, start, end) = mark.closing.unwrap();
        patches
            .entry(stream)
            .or_default()
            .push((start, end, Vec::new()));
    }
    stage_stream_patches(&mut store, &mut BTreeSet::new(), patches).unwrap();
    let (input, _) = rebuild_owner_trees(&write_store(store).unwrap(), None).unwrap();
    request.input_sha256 = format!("{:x}", Sha256::digest(&input));
    let images = crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
    assert_eq!(images.len(), 2);
    for (f, image) in request.figures.iter_mut().zip(images) {
        f.source = ImageFragmentSource::Occurrence {
            page: 1,
            content_stream_index: image.content_stream_index,
            occurrence_id: image.occurrence_id,
        };
    }
    let tags = request.source_tags.as_mut().unwrap();
    tags.selected.retain(|r| r != &source);
    tags.figures.get_mut(&figure_id).unwrap().source = None;
    assert!(preview_linked_story(&input, &request).is_err());
    request
        .source_tags
        .as_mut()
        .unwrap()
        .figures
        .get_mut(&figure_id)
        .unwrap()
        .semantic_text = Some(StorySemanticText {
        alternate: Some("Newly reviewed figure".into()),
        expansion: None,
    });
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.source_tags.as_ref().unwrap().figures.len(), 2);
    assert!(sources(&output)
        .unwrap()
        .iter()
        .any(|s| s.role == "Figure" && s.alternate.as_deref() == Some("Newly reviewed figure")));
}

#[test]
fn structure_merge_does_not_silently_orphan_an_image_when_its_caption_is_deleted() {
    use crate::story_structure_merge::{
        merge_story_structure, StoryStructureBranch, StoryStructureMergeRequest,
    };
    let (_, request) = fixture();
    let mut branch = request.clone();
    branch.paragraphs.remove(0);
    let result = merge_story_structure(&StoryStructureMergeRequest {
        base: request.clone(),
        branches: vec![StoryStructureBranch {
            branch_id: "delete-caption".into(),
            base_story_sha256: crate::story_merge::story_fingerprint(&request).unwrap(),
            proposed: branch,
        }],
    })
    .unwrap();
    assert!(result.merged.is_none());
    assert!(result
        .conflicts
        .iter()
        .any(|c| c.path == format!("figures/{}/caption", request.figures[0].id)));
}

#[test]
fn deleting_a_figure_referenced_by_a_surviving_owner_requires_explicit_migration() {
    let (input, request) = fixture();
    let (input, _) = apply_linked_story(&input, &request).unwrap();
    let mut request = load_linked_stories(&input).unwrap().remove(0).request;
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, root, index) = index_document(&engine, None).unwrap();
    let ids = engine
        .document()
        .get_pages()
        .unwrap()
        .iter()
        .map(|p| (p.object_number, p.generation_number))
        .collect();
    let lookup = Lookup::new(&store, root, &index, &ids).unwrap();
    let tags = request.source_tags.as_ref().unwrap();
    let figure = resolve_tag(
        root,
        &index,
        &lookup,
        tags.figures[&request.figures[0].id]
            .source
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let caption = resolve_tag(
        root,
        &index,
        &lookup,
        tags.paragraph_sources[&request.paragraphs[0].id]
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let mut d = store.dict(caption).unwrap();
    d.insert("Ref", PdfObject::Array(vec![reference(figure)]));
    store.replace_dict(caption, d).unwrap();
    let input = write_store(store).unwrap();
    request.input_sha256 = format!("{:x}", Sha256::digest(&input));
    let figure = request.figures.remove(0);
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("not rebound")
    };
    request
        .figure_removals
        .push(crate::linked_stories::figures::StoryFigureRemoval {
            figure_id: figure.id,
            binding,
        });
    let error = preview_linked_story(&input, &request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("removed structure owner"));
}

#[test]
fn named_actualtext_on_an_image_cannot_disappear_during_capsule_capture() {
    let (input, mut request) = fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let mut store = Store::new(engine.document().reader());
    let marks = page_marks(&engine, &page).unwrap();
    let mark = marks.marks.iter().find(|m| m.mcid == Some(1)).unwrap();
    let mut patches = StreamPatches::new();
    patches.entry(mark.stream).or_default().push((
        mark.start,
        mark.end,
        b"/Figure /LogicalImage BDC".to_vec(),
    ));
    stage_stream_patches(&mut store, &mut BTreeSet::new(), patches).unwrap();
    let mut properties = PdfDictionary::empty();
    let mut logical = PdfDictionary::empty();
    logical.insert("MCID", PdfObject::Integer(1));
    logical.insert("ActualText", logical_string("Existing logical image text"));
    properties.insert("LogicalImage", PdfObject::Dictionary(logical));
    let mut resources = page.resources.clone();
    resources.insert("Properties", PdfObject::Dictionary(properties));
    let id = (page.object_number, page.generation_number);
    let mut d = store.dict(id).unwrap();
    d.insert("Resources", PdfObject::Dictionary(resources));
    store.replace_dict(id, d).unwrap();
    let input = write_store(store).unwrap();
    let reopened = ContentEngine::open_bytes(input.clone()).unwrap();
    let marks = page_marks(&reopened, &reopened.document().get_page(1).unwrap()).unwrap();
    assert!(marks
        .marks
        .iter()
        .any(|m| m.mcid == Some(1) && m.actual_text));
    request.input_sha256 = format!("{:x}", Sha256::digest(&input));
    let images = crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
    assert_eq!(images.len(), 2);
    for (figure, image) in request.figures.iter_mut().zip(images) {
        figure.source = ImageFragmentSource::Occurrence {
            page: 1,
            content_stream_index: image.content_stream_index,
            occurrence_id: image.occurrence_id,
        };
    }
    assert!(validate_selection(&input, &request).is_err());
}
