//! Regression source only. No PDF/test execution in the source-only phase.
use super::*;
use crate::CancelToken;

fn reference(id: Ref) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}
fn grown() -> (Vec<u8>, LinkedStoryRequest) {
    let input = super::super::tests::two_page_input();
    let mut request = super::super::tests::request("Growing paragraph content. ".repeat(55));
    request.input_sha256 = hash(&input);
    request.frames.truncate(1); // keep the second original page unrelated
    request.max_new_pages = 64;
    request.prune_empty_pages = true;
    let (output, report) = apply_linked_story(&input, &request).unwrap();
    assert!(report.generated_pages > 2);
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    (output, saved)
}

fn update_page(
    input: &[u8],
    page: usize,
    work: impl FnOnce(&mut PdfDictionary, &mut Vec<IncrementalObject>, u32),
) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let p = engine.document().get_page(page).unwrap();
    let mut dict = page_dictionary(&engine, page).unwrap();
    let next = engine
        .document()
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap()
        + 1;
    let mut updates = Vec::new();
    work(&mut dict, &mut updates, next);
    updates.push(IncrementalObject {
        number: p.object_number,
        generation: p.generation_number,
        object: PdfObject::Dictionary(dict),
    });
    write_incremental_update(engine.document().reader(), updates).unwrap()
}
fn update_catalog(input: &[u8], work: impl FnOnce(&mut PdfDictionary)) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let id = engine.document().reader().root_reference().unwrap();
    let mut dict = engine.document().get_catalog().unwrap();
    work(&mut dict);
    write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: id.0,
            generation: id.1,
            object: PdfObject::Dictionary(dict),
        }],
    )
    .unwrap()
}
fn shorter(input: &[u8]) -> LinkedStoryRequest {
    let mut request = load_linked_stories(input).unwrap().remove(0).request;
    request.paragraphs[0].text = "Short".into();
    request.prune_empty_pages = true;
    request
}

#[test]
fn universal_pruning_report_does_not_claim_the_original_byte_prefix_survives() {
    use crate::universal_editing::{
        plan_universal_edit_v2, UniversalEditOperationV2, UniversalEditRequestV2,
    };
    let (input, _) = grown();
    let story = shorter(&input);
    let mut request = UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::LinkedStory { request: story },
        policy: Default::default(),
    };
    let pruned = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(pruned.preview["generated_pages"], 0);
    assert!(!pruned.preview["page_pruning"]["removed_pages"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        pruned.signature_impact["route_specific_impact"]["original_prefix_preserved"],
        false
    );
    let UniversalEditOperationV2::LinkedStory { request: story } = &mut request.operation else {
        unreachable!()
    };
    story.prune_empty_pages = false;
    let retained = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(retained.preview["generated_pages"], 0);
    assert!(retained.preview["page_pruning"]["removed_pages"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        retained.signature_impact["route_specific_impact"]["original_prefix_preserved"],
        true
    );
}

#[test]
fn contraction_prunes_only_owned_continuations_and_reopens_regrows_with_exact_history() {
    let (grown, request) = grown();
    let original_last = ContentEngine::open_bytes(grown.clone())
        .unwrap()
        .page_count()
        .unwrap();
    let mut request = request;
    request.paragraphs[0].text = "Short".into();
    let mut session = LinkedStorySession::open(grown.clone()).unwrap();
    let preview = session.preview(&request, &CancelToken::none()).unwrap();
    assert_eq!(
        preview.page_pruning.removed_pages,
        (2..original_last).collect::<Vec<_>>()
    );
    assert_eq!(preview.page_pruning.output_page_count, 2);
    assert_eq!(preview.frames.len(), 1);
    assert!(preview
        .full_page_invalidations
        .iter()
        .any(|(page, _)| *page == 2));
    let receipt = session.preview_receipt().unwrap();
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert!(session
        .checkpoint_approved(&request, &receipt, &cancelled)
        .is_err());
    assert_eq!(session.bytes(), grown);
    let mut altered = request.clone();
    altered.prune_empty_pages = false;
    assert!(session
        .checkpoint_approved(&altered, &receipt, &CancelToken::none())
        .is_err());
    let report = session
        .checkpoint_approved(&request, &receipt, &CancelToken::none())
        .unwrap();
    assert_eq!(
        report.page_pruning.removed_pages,
        preview.page_pruning.removed_pages
    );
    let short = session.bytes().to_vec();
    let reopened = ContentEngine::open_bytes(short.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 2);
    assert!(reopened.get_page_text(1).unwrap().contains("Short"));
    assert!(reopened.get_page_text(2).unwrap().contains("OLD"));
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), grown);
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), short);
    let mut refill = load_linked_stories(&short).unwrap().remove(0).request;
    assert_eq!(refill.frames.len(), 1);
    refill.paragraphs[0].text = "Again a growing paragraph. ".repeat(45);
    let (again, report) = apply_linked_story(&short, &refill).unwrap();
    assert!(report.generated_pages > 0);
    let (short_again, _) = apply_linked_story(&again, &shorter(&again)).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(short_again)
            .unwrap()
            .page_count()
            .unwrap(),
        2
    );
}

#[test]
fn table_contraction_removes_unused_grid_frames_without_removing_its_source_page() {
    let input = super::super::tables::tests::fixture(false);
    let request = super::super::tables::tests::request(&input, true);
    let (grown, report) = apply_linked_story(&input, &request).unwrap();
    assert!(report.generated_pages > 1);
    let mut saved = load_linked_stories(&grown).unwrap().remove(0).request;
    saved.paragraphs[2].text = "Smaller".into();
    saved.prune_empty_pages = true;
    let (output, report) = apply_linked_story(&grown, &saved).unwrap();
    assert_eq!(report.page_pruning.output_page_count, 1);
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        1
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(saved.frames.len(), 1);
    assert!(saved.table_layout.is_some());
    assert!(apply_linked_story(&output, &saved).is_ok());
}

#[test]
fn deliberate_empty_frame_breaks_are_not_treated_as_unused_overflow() {
    let (grown, _) = grown();
    let mut request = shorter(&grown);
    for index in 1..=2 {
        let mut empty = request.paragraphs[0].clone();
        empty.id = format!("blank-{index}");
        empty.text.clear();
        empty.break_before = true;
        request.paragraphs.push(empty);
    }
    let preview = preview_linked_story(&grown, &request).unwrap();
    for page in [2, 3] {
        assert!(!preview.page_pruning.removed_pages.contains(&page));
        assert!(preview
            .page_pruning
            .retained_pages
            .iter()
            .any(|p| p.page == page && p.reason == "explicit_frame_break"));
    }
    let (output, _) = apply_linked_story(&grown, &request).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        4
    );
    let rebound = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(rebound.frames.len(), 3);
    let (again, _) = apply_linked_story(&output, &rebound).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(again)
            .unwrap()
            .page_count()
            .unwrap(),
        4
    );
}

#[test]
fn trailing_form_feed_and_parity_receipts_protect_intentional_blank_pages() {
    let (grown, _) = grown();
    let mut request = shorter(&grown);
    request.paragraphs[0].text = "Short\u{000c}".into();
    request.paragraphs[0].orphans = 1;
    request.paragraphs[0].widows = 1;
    let preview = preview_linked_story(&grown, &request).unwrap();
    let target = preview.page_breaks[0].to_page;
    assert!(!preview.page_pruning.removed_pages.contains(&target));
    assert!(preview
        .page_pruning
        .retained_pages
        .iter()
        .any(|page| page.page == target && page.reason == "explicit_frame_break"));

    let mut parity = shorter(&grown);
    parity.paragraphs[0].page_break_before = StoryPageBreakBefore::NextOddPage;
    let preview = preview_linked_story(&grown, &parity).unwrap();
    let target = preview.page_breaks[0].to_page;
    assert_eq!(target % 2, 1);
    for page in 1..=target {
        assert!(!preview.page_pruning.removed_pages.contains(&page));
    }
}

#[test]
fn original_pages_unmarked_pages_unrelated_paint_and_annotations_are_retained() {
    let (grown, _) = grown();
    for kind in ["marker", "paint", "annotation", "feature"] {
        let input = update_page(&grown, 2, |page, updates, n| match kind {
            "marker" => {
                page.remove(MARKER);
            }
            "paint" => {
                let raw = b"q 0 0 10 10 re f Q".to_vec();
                let mut d = PdfDictionary::empty();
                d.insert("Length", PdfObject::Integer(raw.len() as i64));
                updates.push(IncrementalObject {
                    number: n,
                    generation: 0,
                    object: PdfObject::Stream { dict: d, raw },
                });
                let mut contents = match page.get("Contents").unwrap().clone() {
                    PdfObject::Array(a) => a,
                    v => vec![v],
                };
                contents.push(reference((n, 0)));
                page.insert("Contents", PdfObject::Array(contents));
            }
            "annotation" => {
                let mut d = PdfDictionary::empty();
                d.insert("Type", PdfObject::Name("Annot".into()));
                d.insert("Subtype", PdfObject::Name("Text".into()));
                d.insert(
                    "Rect",
                    PdfObject::Array([0, 0, 10, 10].into_iter().map(PdfObject::Integer).collect()),
                );
                updates.push(IncrementalObject {
                    number: n,
                    generation: 0,
                    object: PdfObject::Dictionary(d),
                });
                page.insert("Annots", PdfObject::Array(vec![reference((n, 0))]));
            }
            _ => {
                page.insert("Dur", PdfObject::Real(2.0));
            }
        });
        let request = shorter(&input);
        let preview = preview_linked_story(&input, &request).unwrap();
        assert!(!preview.page_pruning.removed_pages.contains(&1));
        assert!(!preview.page_pruning.removed_pages.contains(&2));
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        assert_eq!(
            ContentEngine::open_bytes(output.clone())
                .unwrap()
                .page_count()
                .unwrap(),
            3
        );
        assert_eq!(
            load_linked_stories(&output).unwrap()[0]
                .request
                .frames
                .len(),
            2
        );
    }
}

#[test]
fn explicitly_moved_annotation_leaves_the_pruned_page_atomically() {
    let (grown, _) = grown();
    let engine = ContentEngine::open_bytes(grown.clone()).unwrap();
    let page = engine.document().get_page(2).unwrap();
    let source = (page.object_number, page.generation_number);
    let input = update_page(&grown, 2, |page, updates, n| {
        let mut a = PdfDictionary::empty();
        a.insert("Type", PdfObject::Name("Annot".into()));
        a.insert("Subtype", PdfObject::Name("Text".into()));
        a.insert("NM", PdfObject::String(b"MoveMe".to_vec()));
        a.insert("P", reference(source));
        a.insert(
            "Rect",
            PdfObject::Array(
                [10, 10, 20, 20]
                    .into_iter()
                    .map(PdfObject::Integer)
                    .collect(),
            ),
        );
        updates.push(IncrementalObject {
            number: n,
            generation: 0,
            object: PdfObject::Dictionary(a),
        });
        page.insert("Annots", PdfObject::Array(vec![reference((n, 0))]));
    });
    let mut request = shorter(&input);
    let source = crate::story_anchors::annotation_anchor_sources(&input)
        .unwrap()
        .remove(0);
    request
        .annotation_anchors
        .push(crate::story_anchors::StoryAnnotationAnchor {
            annotation_id: source.annotation_id,
            geometry_sha256: source.geometry_sha256,
            paragraph_id: request.paragraphs[0].id.clone(),
            offset: [0.0, 0.0],
            group: source.group,
            rename_conflicting_names: false,
        });
    let preview = preview_linked_story(&input, &request).unwrap();
    assert!(preview.page_pruning.removed_pages.contains(&2));
    assert_eq!(preview.anchor_moves[0].source_page, 2);
    assert_eq!(preview.anchor_moves[0].target_page, 1);
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(output.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        2
    );
    let sources = crate::story_anchors::annotation_anchor_sources(&output).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].page, 1);
    let rebound = load_linked_stories(&output).unwrap().remove(0).request;
    assert_eq!(rebound.annotation_anchors.len(), 1);
    assert!(apply_linked_story(&output, &rebound).is_ok());
}

#[test]
fn incoming_destinations_and_scripts_protect_pages_and_labels_keep_original_numbers() {
    let (grown, _) = grown();
    let engine = ContentEngine::open_bytes(grown.clone()).unwrap();
    let p = engine.document().get_page(2).unwrap();
    let target = (p.object_number, p.generation_number);
    let with_dest = update_catalog(&grown, |catalog| {
        catalog.insert(
            "OpenAction",
            PdfObject::Array(vec![reference(target), PdfObject::Name("Fit".into())]),
        );
    });
    let preview = preview_linked_story(&with_dest, &shorter(&with_dest)).unwrap();
    assert!(preview
        .page_pruning
        .retained_pages
        .iter()
        .any(|p| p.page == 2 && p.reason == "live_incoming_page_reference"));
    let (output, _) = apply_linked_story(&with_dest, &shorter(&with_dest)).unwrap();
    assert_eq!(
        ContentEngine::open_bytes(output)
            .unwrap()
            .page_count()
            .unwrap(),
        3
    );
    let script = update_catalog(&grown, |catalog| {
        let mut action = PdfDictionary::empty();
        action.insert("S", PdfObject::Name("JavaScript".into()));
        action.insert("JS", PdfObject::String(b"this.pageNum=1".to_vec()));
        catalog.insert("OpenAction", PdfObject::Dictionary(action));
    });
    assert!(preview_linked_story(&script, &shorter(&script))
        .unwrap()
        .page_pruning
        .removed_pages
        .is_empty());
    let labels = update_catalog(&grown, |catalog| {
        let mut label = PdfDictionary::empty();
        label.insert("S", PdfObject::Name("D".into()));
        label.insert("St", PdfObject::Integer(10));
        label.insert("P", PdfObject::String(b"Sec-".to_vec()));
        let mut tree = PdfDictionary::empty();
        tree.insert(
            "Nums",
            PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Dictionary(label)]),
        );
        catalog.insert("PageLabels", PdfObject::Dictionary(tree));
    });
    let (output, _) = apply_linked_story(&labels, &shorter(&labels)).unwrap();
    let output = ContentEngine::open_bytes(output).unwrap();
    let catalog = output.document().get_catalog().unwrap();
    let nums = catalog
        .get("PageLabels")
        .unwrap()
        .as_dict()
        .unwrap()
        .get_array("Nums")
        .unwrap();
    assert_eq!(nums[1].as_dict().unwrap().get_integer("St"), Some(10));
    assert_eq!(nums[2].as_integer(), Some(1));
    assert_eq!(
        nums[3].as_dict().unwrap().get_integer("St"),
        Some(9 + engine.page_count().unwrap() as i64)
    );
    assert_eq!(
        nums[3].as_dict().unwrap().get("P").unwrap().as_string(),
        Some(b"Sec-".as_slice())
    );
}
