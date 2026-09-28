//! Regression source only; these cases have not been executed in the source-only phase.
use super::*;
use crate::image_fragments::{ImageFragmentSource, ImageFragmentStack};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{PdfDictionary, PdfObject};
include!("story_ocr_tests.rs");

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}
pub(crate) fn fixture() -> Vec<u8> {
    let input = tests::two_page_input();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut image = PdfDictionary::empty();
    for (k, v) in [
        ("Type", "XObject"),
        ("Subtype", "Image"),
        ("ColorSpace", "DeviceRGB"),
    ] {
        image.insert(k, PdfObject::Name(v.into()));
    }
    for (k, v) in [
        ("Width", 1),
        ("Height", 1),
        ("BitsPerComponent", 8),
        ("Length", 3),
    ] {
        image.insert(k, PdfObject::Integer(v));
    }
    let raw = b"q 20 0 0 20 20 100 cm /Figure Do Q q 20 0 0 20 80 100 cm /Figure Do Q".to_vec();
    let mut stream = PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(raw.len() as i64));
    let mut resources = page.resources.clone();
    let mut xo = PdfDictionary::empty();
    xo.insert("Figure", reference(n));
    resources.insert("XObject", PdfObject::Dictionary(xo));
    let mut pd = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    pd.insert("Resources", PdfObject::Dictionary(resources));
    let mut contents = page
        .contents
        .iter()
        .map(|r| PdfObject::Reference {
            number: r.0,
            generation: r.1,
        })
        .collect::<Vec<_>>();
    contents.push(reference(n + 1));
    pd.insert("Contents", PdfObject::Array(contents));
    write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: n,
                generation: 0,
                object: PdfObject::Stream {
                    dict: image,
                    raw: vec![40, 80, 160],
                },
            },
            IncrementalObject {
                number: n + 1,
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
    .unwrap()
}

fn nested_fixture() -> (Vec<u8>, Vec<(u32, u16)>) {
    let input = tests::two_page_input();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;

    let mut image = PdfDictionary::empty();
    image.insert("Type", PdfObject::Name("XObject".into()));
    image.insert("Subtype", PdfObject::Name("Image".into()));
    image.insert("ColorSpace", PdfObject::Name("DeviceRGB".into()));
    image.insert("Width", PdfObject::Integer(1));
    image.insert("Height", PdfObject::Integer(1));
    image.insert("BitsPerComponent", PdfObject::Integer(8));
    image.insert("Length", PdfObject::Integer(3));

    let mut leaf_xobjects = PdfDictionary::empty();
    leaf_xobjects.insert("Im", reference(n));
    let mut leaf_resources = PdfDictionary::empty();
    leaf_resources.insert("XObject", PdfObject::Dictionary(leaf_xobjects));
    let leaf_bytes = b"q 40 0 0 30 0 0 cm /Im Do Q".to_vec();
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

    let mut outer_xobjects = PdfDictionary::empty();
    outer_xobjects.insert("Leaf", reference(n + 1));
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

    let page_bytes = b"q 1 0 0 1 20 100 cm /Outer Do Q q 1 0 0 1 100 100 cm /Outer Do Q".to_vec();
    let mut page_stream = PdfDictionary::empty();
    page_stream.insert("Length", PdfObject::Integer(page_bytes.len() as i64));
    let mut resources = page.resources.clone();
    let mut page_xobjects = resources
        .get("XObject")
        .and_then(PdfObject::as_dict)
        .cloned()
        .unwrap_or_else(PdfDictionary::empty);
    page_xobjects.insert("Outer", reference(n + 2));
    resources.insert("XObject", PdfObject::Dictionary(page_xobjects));
    let mut page_dictionary = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    page_dictionary.insert("Resources", PdfObject::Dictionary(resources));
    let mut contents = page
        .contents
        .iter()
        .map(|owner| PdfObject::Reference {
            number: owner.0,
            generation: owner.1,
        })
        .collect::<Vec<_>>();
    contents.push(reference(n + 3));
    page_dictionary.insert("Contents", PdfObject::Array(contents));

    (
        write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: n,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: image,
                        raw: vec![64, 128, 220],
                    },
                },
                IncrementalObject {
                    number: n + 1,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: leaf,
                        raw: leaf_bytes,
                    },
                },
                IncrementalObject {
                    number: n + 2,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: outer,
                        raw: outer_bytes,
                    },
                },
                IncrementalObject {
                    number: n + 3,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: page_stream,
                        raw: page_bytes,
                    },
                },
                IncrementalObject {
                    number: page.object_number,
                    generation: page.generation_number,
                    object: PdfObject::Dictionary(page_dictionary),
                },
            ],
        )
        .unwrap(),
        vec![(n, 0), (n + 1, 0), (n + 2, 0), (n + 3, 0)],
    )
}
pub(crate) fn with_figures(input: &[u8]) -> LinkedStoryRequest {
    let mut request = tests::request("First caption".into());
    request.input_sha256 = hash(input);
    request.frames.truncate(1);
    request.frames[0].rect = [10.0, 10.0, 190.0, 190.0];
    let mut second = request.paragraphs[0].clone();
    second.id = "caption-2".into();
    second.text = "Second caption".into();
    second.break_before = true;
    request.paragraphs.push(second);
    let images = crate::universal_editing::universal_image_occurrences_v2(input, &[1]).unwrap();
    assert_eq!(images.len(), 2);
    for (index, image) in images.iter().enumerate() {
        request.figures.push(figures::StoryFigure {
            id: format!("figure-{index}"),
            caption_paragraph: request.paragraphs[index].id.clone(),
            source: ImageFragmentSource::Occurrence {
                page: 1,
                content_stream_index: image.content_stream_index,
                occurrence_id: image.occurrence_id.clone(),
            },
            ocr: None,
            ocr_unrelated: false,
            width: 100.0,
            height: 40.0,
            gap: 5.0,
            alignment: figures::FigureAlignment::Center,
            stack: ImageFragmentStack::Foreground,
        });
    }
    request
}
#[test]
fn figure_and_caption_choose_a_larger_frame_without_being_split() {
    let input = fixture();
    let mut request = with_figures(&input);
    request.figures.truncate(1);
    request.paragraphs.truncate(1);
    request.frames[0].rect = [10.0, 10.0, 190.0, 40.0];
    let mut second = request.frames[0].clone();
    second.id = "larger".into();
    second.page = 2;
    second.rect = [10.0, 10.0, 190.0, 190.0];
    request.frames.push(second);
    let preview = preview_linked_story(&input, &request).unwrap();
    assert!(preview.frames[0].figures.is_empty());
    assert!(preview.frames[0].lines.is_empty());
    let target = &preview.frames[1];
    assert_eq!(target.figures.len(), 1);
    assert_eq!(target.figures[0].rect, [50.0, 150.0, 150.0, 190.0]);
    assert!(target
        .lines
        .iter()
        .all(|l| l.x >= 50.0 && l.x + l.width <= 150.0 + 1e-7 && l.baseline < 145.0));
}
#[test]
fn batch_images_survive_canonical_page_insertion_save_reopen_and_backward_flow() {
    let input = fixture();
    let request = with_figures(&input);
    let (output, report) = apply_linked_story(&input, &request).unwrap();
    assert_eq!(report.generated_pages, 1);
    let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 3);
    assert!(reopened.get_page_text(3).unwrap().contains("OLD"));
    let first = crate::image_fragments::image_fragment_bindings(&output).unwrap();
    assert_eq!(first.len(), 2);
    let independent = crate::image_fragments::ImageFragmentMove {
        input_sha256: hash(&output),
        source: ImageFragmentSource::Owned {
            binding: first[0].clone(),
        },
        target_page: first[0].page,
        target_rect: first[0].rect,
        stack: ImageFragmentStack::Foreground,
        ocr: None,
        signature_policy_override: false,
    };
    assert!(crate::image_fragments::preview_image_fragment_move(&output, &independent).is_err());
    assert_eq!(
        first.iter().map(|b| b.page).collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2])
    );
    assert!(!reopened
        .document()
        .get_catalog()
        .unwrap()
        .contains_key("WFStagedStoryFigures"));
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    saved.paragraphs[1].break_before = false;
    saved.paragraphs[0].text = "Short".into();
    let (contracted, report) = apply_linked_story(&output, &saved).unwrap();
    assert_eq!(report.generated_pages, 0);
    let owners = crate::image_fragments::image_fragment_bindings(&contracted).unwrap();
    assert_eq!(owners.len(), 2);
    assert!(owners.iter().all(|b| b.page == 1));
    assert_eq!(
        owners.iter().map(|b| &b.key).collect::<BTreeSet<_>>(),
        first.iter().map(|b| &b.key).collect::<BTreeSet<_>>()
    );
    let again = load_linked_stories(&contracted).unwrap().remove(0).request;
    let (repeated, _) = apply_linked_story(&contracted, &again).unwrap();
    assert_eq!(
        crate::image_fragments::image_fragment_bindings(&repeated)
            .unwrap()
            .len(),
        2
    );
    assert!(load_linked_stories(&repeated).is_ok());
}

#[test]
fn nested_form_story_batch_merges_shared_prefixes_and_preserves_original_objects() {
    let (input, original_objects) = nested_fixture();
    let request = with_figures(&input);
    let before = ContentEngine::open_bytes(input.clone()).unwrap();
    let (output, report) = apply_linked_story(&input, &request).unwrap();
    let after = ContentEngine::open_bytes(output.clone()).unwrap();

    assert_eq!(
        report
            .frames
            .iter()
            .map(|frame| frame.figures.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(
        crate::image_fragments::image_fragment_bindings(&output)
            .unwrap()
            .len(),
        2
    );
    let pages = (1..=after.page_count().unwrap()).collect::<Vec<_>>();
    assert_eq!(
        crate::universal_editing::universal_image_occurrences_v2(&output, &pages)
            .unwrap()
            .into_iter()
            .filter(|occurrence| occurrence.object_number == Some(original_objects[0].0))
            .count(),
        2
    );
    for object in original_objects.iter().copied() {
        assert_eq!(
            before
                .document()
                .reader()
                .get_object(object.0, object.1)
                .unwrap(),
            after
                .document()
                .reader()
                .get_object(object.0, object.1)
                .unwrap()
        );
    }
    assert!(!after
        .document()
        .get_page(1)
        .unwrap()
        .contents
        .contains(original_objects.last().unwrap()));
    assert!(!after
        .document()
        .get_catalog()
        .unwrap()
        .contains_key("WFStagedStoryFigures"));
    assert_eq!(load_linked_stories(&output).unwrap().len(), 1);
}
#[test]
fn pinned_artwork_moves_the_image_and_caption_together() {
    let input = fixture();
    let mut r = with_figures(&input);
    r.figures.truncate(1);
    r.paragraphs.truncate(1);
    r.frames[0].exclusions.push([30.0, 140.0, 170.0, 195.0]);
    let p = preview_linked_story(&input, &r).unwrap();
    assert_eq!(p.frames[0].figures[0].rect[3], 140.0);
    assert!(p.frames[0].lines.iter().all(|l| l.baseline < 95.0));
}
#[test]
fn empty_caption_is_a_real_reserved_image_block_and_cannot_be_silently_dropped() {
    let input = fixture();
    let mut r = with_figures(&input);
    r.figures.truncate(1);
    r.paragraphs.truncate(1);
    r.paragraphs[0].text.clear();
    let (output, p) = apply_linked_story(&input, &r).unwrap();
    assert_eq!(p.frames[0].figures.len(), 1);
    assert!(p.frames[0].lines.is_empty());
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    saved.figures.clear();
    assert!(preview_linked_story(&output, &saved).is_err());
}
#[test]
fn incremental_preview_retains_and_invalidates_figure_reservations() {
    let input = fixture();
    let mut r = with_figures(&input);
    r.paragraphs[1].break_before = false;
    let mut session = LinkedStorySession::open(input).unwrap();
    let cancel = crate::cancel::CancelToken::none();
    session.preview(&r, &cancel).unwrap();
    let old = session.preview_receipt().unwrap();
    r.paragraphs[1].text = "Second caption edited".into();
    let preview = session.preview(&r, &cancel).unwrap();
    assert_eq!(
        preview
            .frames
            .iter()
            .map(|f| f.figures.len())
            .sum::<usize>(),
        2
    );
    assert!(!session.dirty_regions().is_empty());
    assert!(session.checkpoint_approved(&r, &old, &cancel).is_err());
    let receipt = session.preview_receipt().unwrap();
    session.checkpoint_approved(&r, &receipt, &cancel).unwrap();
    assert_eq!(
        crate::image_fragments::image_fragment_bindings(session.bytes())
            .unwrap()
            .len(),
        2
    );
}

fn remove_figure(request: &mut LinkedStoryRequest, index: usize) -> figures::StoryFigureRemoval {
    let figure = request.figures.remove(index);
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("expected a saved native figure");
    };
    let removal = figures::StoryFigureRemoval {
        figure_id: figure.id,
        binding,
    };
    request.figure_removals.push(removal.clone());
    removal
}

fn detach_figure(request: &mut LinkedStoryRequest, index: usize) -> figures::StoryFigureDetachment {
    let figure = request.figures.remove(index);
    let ImageFragmentSource::Owned { binding } = figure.source else {
        panic!("expected a saved native figure");
    };
    let detachment = figures::StoryFigureDetachment {
        figure_id: figure.id,
        binding,
    };
    request.figure_detachments.push(detachment.clone());
    detachment
}

#[test]
fn explicit_detachment_preserves_paint_releases_story_ownership_and_enables_later_transfer() {
    let input = fixture();
    let mut request = with_figures(&input);
    request.paragraphs[1].break_before = false;
    let (saved, _) = apply_linked_story(&input, &request).unwrap();
    let before = crate::image_fragments::image_fragment_bindings(&saved).unwrap();
    let mut request = load_linked_stories(&saved).unwrap().remove(0).request;
    let detachment = detach_figure(&mut request, 0);

    let preview = preview_linked_story(&saved, &request).unwrap();
    assert_eq!(preview.figure_detachments.len(), 1);
    assert!(preview.figure_removals.is_empty());
    let (output, _) = apply_linked_story(&saved, &request).unwrap();
    let after = crate::image_fragments::image_fragment_bindings(&output).unwrap();
    assert_eq!(after.len(), 2);
    assert!(after.contains(&detachment.binding));
    assert!(before.contains(&detachment.binding));

    let reloaded = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(reloaded.figure_detachments.is_empty());
    assert_eq!(reloaded.figures.len(), 1);
    let independent = crate::image_fragments::ImageFragmentMove {
        input_sha256: hash(&output),
        source: ImageFragmentSource::Owned {
            binding: detachment.binding.clone(),
        },
        target_page: detachment.binding.page,
        target_rect: detachment.binding.rect,
        stack: ImageFragmentStack::Foreground,
        ocr: None,
        signature_policy_override: false,
    };
    assert!(crate::image_fragments::preview_image_fragment_move(&output, &independent).is_ok());

    let mut destination = tests::request("Transferred caption".into());
    destination.story_id = "destination-story".into();
    destination.input_sha256 = hash(&output);
    destination.figures.push(figures::StoryFigure {
        id: "transferred-figure".into(),
        caption_paragraph: destination.paragraphs[0].id.clone(),
        source: ImageFragmentSource::Owned {
            binding: detachment.binding,
        },
        ocr: None,
        ocr_unrelated: false,
        width: 100.0,
        height: 40.0,
        gap: 5.0,
        alignment: figures::FigureAlignment::Center,
        stack: ImageFragmentStack::Foreground,
    });
    assert!(figures::validate(&output, &destination).is_ok());
}

#[test]
fn cross_story_transfer_publishes_only_after_both_saved_story_postconditions() {
    let input = fixture();
    let mut source = with_figures(&input);
    source.paragraphs[1].break_before = false;
    let (with_source, _) = apply_linked_story(&input, &source).unwrap();

    let mut target = tests::request("Target caption".into());
    target.story_id = "target-story".into();
    target.input_sha256 = hash(&with_source);
    target.frames.remove(0);
    target.frames[0].logical_range = [0, 3];
    target.frames[0].expected_text = "OLD".into();
    let (both_saved, _) = apply_linked_story(&with_source, &target).unwrap();

    let source = load_linked_stories(&both_saved)
        .unwrap()
        .into_iter()
        .find(|saved| saved.request.story_id == "contract-body")
        .unwrap()
        .request;
    let figure = &source.figures[0];
    let ImageFragmentSource::Owned { binding } = &figure.source else {
        panic!("expected owned source Figure");
    };
    let request = figures::StoryFigureTransferRequest {
        input_sha256: hash(&both_saved),
        source_story_id: source.story_id.clone(),
        source_figure_id: figure.id.clone(),
        source_binding: binding.clone(),
        target_story_id: "target-story".into(),
        target_figure_id: "transferred".into(),
        target_caption_paragraph: "p1".into(),
        signature_policy_override: false,
    };
    let preview = figures::preview_story_figure_transfer(&both_saved, &request).unwrap();
    assert_eq!(preview.source.story_id, "contract-body");
    assert_eq!(preview.target.story_id, "target-story");
    let universal = crate::universal_editing::plan_universal_edit_v2(
        &both_saved,
        &crate::universal_editing::UniversalEditRequestV2 {
            operation: crate::universal_editing::UniversalEditOperationV2::StoryFigureTransfer {
                request: request.clone(),
            },
            policy: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        universal.preview["plan_sha256"].as_str(),
        Some(preview.plan_sha256.as_str())
    );
    assert_eq!(
        universal.state,
        crate::universal_editing::UniversalPlanStateV2::ApprovalRequired
    );
    assert!(figures::apply_story_figure_transfer(&both_saved, &request, "stale").is_err());
    let (output, report) =
        figures::apply_story_figure_transfer(&both_saved, &request, &preview.plan_sha256).unwrap();
    assert!(report.source_owner_released);
    assert!(report.target_owner_attached);
    assert_eq!(report.physical_owner_count, 1);
    assert_eq!(
        report.output_page_count,
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .page_count()
            .unwrap()
    );
    let stories = load_linked_stories(&output).unwrap();
    let source = stories
        .iter()
        .find(|saved| saved.request.story_id == "contract-body")
        .unwrap();
    let target = stories
        .iter()
        .find(|saved| saved.request.story_id == "target-story")
        .unwrap();
    assert!(!source.request.figures.iter().any(|f| f.id == figure.id));
    assert!(target.request.figures.iter().any(|f| f.id == "transferred"));
}

#[test]
fn explicit_figure_removal_preserves_the_other_shared_image_and_clears_commands() {
    let input = fixture();
    let mut request = with_figures(&input);
    request.paragraphs[1].break_before = false;
    let (saved, _) = apply_linked_story(&input, &request).unwrap();
    let mut request = load_linked_stories(&saved).unwrap().remove(0).request;
    let removed = remove_figure(&mut request, 0);
    let preview = preview_linked_story(&saved, &request).unwrap();
    assert_eq!(preview.figure_removals.len(), 1);
    assert_eq!(preview.figure_removals[0].binding, removed.binding);
    assert!(preview
        .full_page_invalidations
        .iter()
        .any(|(p, _)| *p == removed.binding.page));
    let (output, _) = apply_linked_story(&saved, &request).unwrap();
    let owners = crate::image_fragments::image_fragment_bindings(&output).unwrap();
    assert_eq!(owners.len(), 1);
    assert_ne!(owners[0].key, removed.binding.key);
    let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
    assert!(reopened.get_page_text(1).unwrap().contains("First caption"));
    assert!(reopened.get_page_text(2).unwrap().contains("OLD"));
    assert!(!reopened
        .document()
        .get_catalog()
        .unwrap()
        .contains_key("WFStagedStoryFigures"));
    let reloaded = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(reloaded.figure_removals.is_empty());
    assert_eq!(reloaded.figures.len(), 1);
    let (again, _) = apply_linked_story(&output, &reloaded).unwrap();
    assert_eq!(
        crate::image_fragments::image_fragment_bindings(&again)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn deleting_all_figures_is_a_real_batch_without_any_placement_entries() {
    let input = fixture();
    let (saved, _) = apply_linked_story(&input, &with_figures(&input)).unwrap();
    let mut request = load_linked_stories(&saved).unwrap().remove(0).request;
    remove_figure(&mut request, 0);
    remove_figure(&mut request, 0);
    let (output, preview) = apply_linked_story(&saved, &request).unwrap();
    assert_eq!(preview.figure_removals.len(), 2);
    assert!(preview.frames.iter().all(|f| f.figures.is_empty()));
    assert!(crate::image_fragments::image_fragment_bindings(&output)
        .unwrap()
        .is_empty());
    let request = load_linked_stories(&output).unwrap().remove(0).request;
    assert!(
        request.figures.is_empty()
            && request.figure_removals.is_empty()
            && request.figure_detachments.is_empty()
    );
    let (again, _) = apply_linked_story(&output, &request).unwrap();
    assert!(load_linked_stories(&again).is_ok());
}

#[test]
fn figure_deletion_rejects_stale_duplicate_retained_and_foreign_owners() {
    let input = fixture();
    let (saved, _) = apply_linked_story(&input, &with_figures(&input)).unwrap();
    let request = load_linked_stories(&saved).unwrap().remove(0).request;
    let mut base = request.clone();
    let removal = remove_figure(&mut base, 0);
    let mut stale = base.clone();
    stale.figure_removals[0].binding.rect[0] += 1.0;
    assert!(figures::validate(&saved, &stale)
        .unwrap_err()
        .to_string()
        .contains("binding is stale"));
    let mut duplicate = base.clone();
    duplicate.figure_removals.push(removal.clone());
    assert!(figures::validate(&saved, &duplicate).is_err());
    let mut retained = request.clone();
    retained.figure_removals.push(removal);
    assert!(figures::validate(&saved, &retained).is_err());
    let mut foreign = base;
    foreign.story_id = "other-story".into();
    assert!(figures::validate(&saved, &foreign)
        .unwrap_err()
        .to_string()
        .contains("another saved story"));
}

#[test]
fn figure_deletion_requires_fresh_approval_and_undo_restores_exact_bytes() {
    let input = fixture();
    let (saved, _) = apply_linked_story(&input, &with_figures(&input)).unwrap();
    let mut request = load_linked_stories(&saved).unwrap().remove(0).request;
    let mut session = LinkedStorySession::open(saved.clone()).unwrap();
    let cancel = crate::cancel::CancelToken::none();
    session.preview(&request, &cancel).unwrap();
    let stale = session.preview_receipt().unwrap();
    remove_figure(&mut request, 0);
    assert!(session
        .checkpoint_approved(&request, &stale, &cancel)
        .is_err());
    assert_eq!(session.bytes(), saved.as_slice());
    session.preview(&request, &cancel).unwrap();
    let approved = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&request, &approved, &cancel)
        .unwrap();
    let deleted = session.bytes().to_vec();
    assert_eq!(
        crate::image_fragments::image_fragment_bindings(&deleted)
            .unwrap()
            .len(),
        1
    );
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), saved.as_slice());
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), deleted.as_slice());
    assert!(session.saved_stories().unwrap()[0]
        .request
        .figure_removals
        .is_empty());
}
