//! Regression source only; no compiler, PDF workload or test was run.
use super::*;
use crate::CancelToken;

fn fixture() -> (Vec<u8>, HistorySource) {
    use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
    let mut builder = PdfBuilder::new();
    builder
        .add_page(PageSize::custom(200.0, 200.0))
        .draw_text(
            "OLD",
            10.0,
            50.0,
            &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
        )
        .unwrap();
    let bytes = builder.to_bytes().unwrap();
    let mut base = crate::linked_stories::tests::request("alpha beta".into());
    base.frames.truncate(1);
    base.input_sha256 = hash(&bytes);
    let history = text_history::new_history(&base).unwrap();
    (
        bytes,
        HistorySource::Start {
            base,
            history,
            replace_epoch: false,
        },
    )
}
fn edit(
    input: &[u8],
    source: &HistorySource,
    actor: &str,
    range: [usize; 2],
    replacement: &str,
) -> PreparedHistory {
    let current = prepare_history(input, source).unwrap();
    let paragraph = &current.result.merged.as_ref().unwrap().paragraphs[0];
    edit_history(
        input,
        source,
        &StoryHistoryEdit {
            expected_history_sha256: current.result.history_sha256.clone(),
            actor: actor.into(),
            paragraph_id: paragraph.id.clone(),
            range,
            expected_text: paragraph.text[range[0]..range[1]].into(),
            replacement: replacement.into(),
        },
    )
    .unwrap()
}
fn commit(session: &mut LinkedStorySession, source: &HistorySource) -> HistoryCheckpointReport {
    let preview = session
        .preview_history(source, &CancelToken::none())
        .unwrap();
    session
        .checkpoint_history(
            &preview.prepared.source,
            &preview.receipt,
            &CancelToken::none(),
        )
        .unwrap()
}
fn text(prepared: &PreparedHistory) -> &str {
    &prepared.result.merged.as_ref().unwrap().paragraphs[0].text
}
fn catalog_change(input: &[u8]) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = engine.document().reader();
    let root = reader.root_reference().unwrap();
    let mut catalog = engine.document().get_catalog().unwrap();
    catalog.insert("WFUnrelatedReview", PdfObject::Boolean(true));
    write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: root.0,
            generation: root.1,
            object: PdfObject::Dictionary(catalog),
        }],
    )
    .unwrap()
}
fn change_record(input: &[u8], change: impl FnOnce(&mut StoredStory)) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = engine.document().reader();
    let mut record = read_record(&engine, "contract-body").unwrap().unwrap();
    change(&mut record);
    let entries = read_story_metadata(&engine).unwrap();
    let PdfObject::Reference { number, generation } = &entries[&hash(b"contract-body")] else {
        panic!("indirect metadata expected")
    };
    let raw = bounded_metadata(&record).unwrap();
    let mut dict = PdfDictionary::empty();
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: *number,
            generation: *generation,
            object: PdfObject::Stream { dict, raw },
        }],
    )
    .unwrap()
}

#[test]
fn acknowledged_compaction_starts_a_new_epoch_and_rejects_stale_replica_history() {
    let (bytes, source) = fixture();
    let edited = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let stale_history = edited.result.history.clone();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let saved = commit(&mut session, &edited.source);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    let request = HistoryCompactionRequest {
        input_sha256: session.revision_sha256(),
        story_id: "contract-body".into(),
        expected_checkpoint_sha256: resumed.checkpoint_before.clone().unwrap(),
        expected_history_sha256: resumed.result.history_sha256.clone(),
        acknowledged_frontier: resumed.result.frontier.clone(),
        acknowledge_operation_and_undo_loss: true,
        acknowledge_prior_epoch_rejected: true,
    };
    let plan = plan_history_compaction(session.bytes(), &request).unwrap();
    assert_eq!(plan.source_operation_count, 1);
    assert_eq!(plan.acknowledged_frontier.get("a"), Some(&1));
    let mut wrong = request.clone();
    wrong.acknowledged_frontier.insert("a".into(), 0);
    assert!(plan_history_compaction(session.bytes(), &wrong).is_err());
    wrong = request.clone();
    wrong.acknowledge_operation_and_undo_loss = false;
    assert!(plan_history_compaction(session.bytes(), &wrong).is_err());

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let before = session.bytes().to_vec();
    assert!(session
        .compact_history(&request, &plan.plan_sha256, &cancelled)
        .is_err());
    assert_eq!(session.bytes(), before);

    let report = session
        .compact_history(&request, &plan.plan_sha256, &CancelToken::none())
        .unwrap();
    assert_eq!(report.generation_before, saved.generation);
    assert_eq!(report.generation_after, saved.generation + 1);
    assert_eq!(report.retired_operation_count, 1);
    assert_ne!(report.seed_before, report.seed_after);
    assert!(report.prior_epoch_rejected);
    assert!(report.exact_session_undo_available);
    assert_eq!(report.output_sha256, session.revision_sha256());

    let compacted = resume_history(session.bytes(), "contract-body").unwrap();
    assert!(compacted.result.history.operations.is_empty());
    assert!(compacted.result.frontier.is_empty());
    assert_eq!(text(&compacted), "ALPHA beta");
    assert!(join_history(session.bytes(), &compacted.source, &[stale_history]).is_err());

    assert!(session.undo().unwrap());
    let restored = resume_history(session.bytes(), "contract-body").unwrap();
    assert_eq!(restored.result.history.operations.len(), 1);
    assert_eq!(text(&restored), "ALPHA beta");
}

#[test]
fn compaction_report_does_not_promise_undo_beyond_the_session_budget() {
    let (bytes, source) = fixture();
    let edited = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let mut session = LinkedStorySession::open(bytes).unwrap();
    commit(&mut session, &edited.source);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    let request = HistoryCompactionRequest {
        input_sha256: session.revision_sha256(),
        story_id: "contract-body".into(),
        expected_checkpoint_sha256: resumed.checkpoint_before.unwrap(),
        expected_history_sha256: resumed.result.history_sha256,
        acknowledged_frontier: resumed.result.frontier,
        acknowledge_operation_and_undo_loss: true,
        acknowledge_prior_epoch_rejected: true,
    };
    let plan = plan_history_compaction(session.bytes(), &request).unwrap();

    // Exercise the same publication decision without allocating a 128 MiB PDF:
    // the report is not finalized until the bounded session accepts a preimage.
    let (_, provisional) =
        apply_history_compaction(session.bytes(), &request, &plan.plan_sha256).unwrap();
    assert!(!provisional.exact_session_undo_available);
    let published = session
        .compact_history(&request, &plan.plan_sha256, &CancelToken::none())
        .unwrap();
    assert!(published.exact_session_undo_available);
}

#[test]
fn wrapped_story_history_retains_policy_and_rejects_downgraded_metadata() {
    use crate::fonts::line_break_policy::{EmergencyWrap, LineBreakProfile};
    let (bytes, mut source) = fixture();
    let HistorySource::Start { base, history, .. } = &mut source else {
        panic!("start fixture expected")
    };
    base.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
    base.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    base.paragraphs[0].line_break.composition =
        crate::fonts::line_break_policy::LineComposition::Balanced;
    *history = text_history::new_history(base).unwrap();
    let policy = base.paragraphs[0].line_break.clone();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    commit(&mut session, &source);
    assert_eq!(session.saved_stories().unwrap()[0].schema_version, 3);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    assert_eq!(
        resumed.result.merged.as_ref().unwrap().paragraphs[0].line_break,
        policy
    );
    let next = edit(session.bytes(), &resumed.source, "a", [0, 5], "ALPHA");
    commit(&mut session, &next.source);
    assert_eq!(
        session.saved_stories().unwrap()[0].request.paragraphs[0].line_break,
        policy
    );
    let downgraded = change_record(session.bytes(), |record| record.schema_version = 1);
    assert!(load_linked_stories(&downgraded).is_err());
    assert!(resume_history(&downgraded, "contract-body").is_err());
    let downgraded = change_record(session.bytes(), |record| record.schema_version = 2);
    assert!(load_linked_stories(&downgraded).is_err());
    assert!(resume_history(&downgraded, "contract-body").is_err());
}

#[test]
fn checkpoint_reopen_remote_join_and_next_actor_event_preserve_original_seed() {
    let (bytes, source) = fixture();
    let a = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let b = edit(&bytes, &source, "b", [6, 10], "BETA");
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let first = commit(&mut session, &a.source);
    assert_eq!(first.generation, 1);
    let first_bytes = session.bytes().to_vec();
    let mut reopened = LinkedStorySession::open(first_bytes).unwrap();
    let resume = resume_history(reopened.bytes(), "contract-body").unwrap();
    assert_eq!(text(&resume), "ALPHA beta");
    let joined = join_history(reopened.bytes(), &resume.source, &[b.result.history]).unwrap();
    assert_eq!(text(&joined), "ALPHA BETA");
    assert_eq!(
        joined.result.merged.as_ref().unwrap().input_sha256,
        reopened.revision_sha256()
    );
    assert!(joined.result.merged.as_ref().unwrap().frames[0]
        .owner
        .is_some());
    let second = commit(&mut reopened, &joined.source);
    assert_eq!(second.generation, 2);
    assert!(second.same_epoch_preserved);
    assert_eq!(second.seed_sha256, first.seed_sha256);
    assert_eq!(second.output_sha256, reopened.revision_sha256());
    assert_eq!(
        second.story.output_sha256.as_deref(),
        Some(second.output_sha256.as_str())
    );
    let (saved, _, _) = bound_record(reopened.bytes(), "contract-body").unwrap();
    assert_eq!(
        saved.parent_checkpoint_sha256.as_deref(),
        Some(first.checkpoint_sha256.as_str())
    );
    let resumed = resume_history(reopened.bytes(), "contract-body").unwrap();
    let next = edit(reopened.bytes(), &resumed.source, "a", [10, 10], "!");
    assert_eq!(next.result.frontier.get("a"), Some(&2));
    assert_eq!(text(&next), "ALPHA BETA!");
    commit(&mut reopened, &next.source);
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("ALPHA BETA!"));
    assert_eq!(
        text(&resume_history(reopened.bytes(), "contract-body").unwrap()),
        "ALPHA BETA!"
    );
}

#[test]
fn checkpoint_is_one_publication_with_exact_undo_redo_of_text_and_history() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    let first = commit(&mut session, &source);
    let saved = session.bytes().to_vec();
    assert_eq!(session.history_status().undo_steps, 1);
    assert!(session.undo().unwrap());
    assert_eq!(session.bytes(), bytes);
    assert!(resume_history(session.bytes(), "contract-body").is_err());
    assert!(session.redo().unwrap());
    assert_eq!(session.bytes(), saved);
    assert_eq!(
        resume_history(session.bytes(), "contract-body")
            .unwrap()
            .checkpoint_before,
        Some(first.checkpoint_sha256)
    );
    assert!(session.preview_receipt().is_err());
}

#[test]
fn ordinary_story_edit_preserves_history_but_cannot_silently_advance_it() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let first = commit(&mut session, &source);
    let mut ordinary = session.saved_stories().unwrap().remove(0).request;
    ordinary.paragraphs[0].text = "outside causal history".into();
    session.preview(&ordinary, &CancelToken::none()).unwrap();
    session
        .checkpoint_approved(
            &ordinary,
            &session.preview_receipt().unwrap(),
            &CancelToken::none(),
        )
        .unwrap();
    let error = resume_history(session.bytes(), "contract-body")
        .unwrap_err()
        .to_string();
    assert!(error.contains("detached"), "{error}");
    let record = read_record(session.document(), "contract-body")
        .unwrap()
        .unwrap();
    assert_eq!(
        record.text_history.unwrap().history_sha256,
        first.history_sha256
    );
    assert_eq!(
        session.saved_stories().unwrap()[0].request.paragraphs[0].text,
        "outside causal history"
    );
}

#[test]
fn stale_source_changed_history_and_replaced_preview_do_not_publish() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    let preview = session
        .preview_history(&source, &CancelToken::none())
        .unwrap();
    let changed = edit(&bytes, &source, "a", [0, 5], "changed");
    assert!(session
        .checkpoint_history(&changed.source, &preview.receipt, &CancelToken::none())
        .is_err());
    assert_eq!(session.bytes(), bytes);
    session
        .preview(
            preview.prepared.result.merged.as_ref().unwrap(),
            &CancelToken::none(),
        )
        .unwrap();
    assert!(session
        .checkpoint_history(&source, &preview.receipt, &CancelToken::none())
        .is_err());
    assert_eq!(session.history_status().undo_steps, 0);
    commit(&mut session, &source);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    let mut tampered = resumed.source.clone();
    if let HistorySource::Resume {
        expected_checkpoint_sha256,
        ..
    } = &mut tampered
    {
        *expected_checkpoint_sha256 = "wrong checkpoint".into();
    }
    assert!(prepare_history(session.bytes(), &tampered).is_err());
    assert!(session
        .preview_history(&source, &CancelToken::none())
        .is_err());
}

#[test]
fn missing_events_are_retained_but_cannot_be_previewed_and_persisted_events_cannot_be_omitted() {
    let (bytes, source) = fixture();
    let a = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let next = edit(&bytes, &a.source, "a", [6, 10], "BETA");
    let mut delta = next.result.history.clone();
    delta.operations.retain(|op| op.id.sequence == 2);
    let partial = join_history(&bytes, &source, &[delta]).unwrap();
    assert_eq!(partial.result.history.operations.len(), 1);
    assert_eq!(partial.result.missing_dependencies.len(), 1);
    assert!(partial.result.merged.is_none());
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    assert!(session
        .preview_history(&partial.source, &CancelToken::none())
        .is_err());
    assert_eq!(session.bytes(), bytes);
    commit(&mut session, &next.source);
    let resume = resume_history(session.bytes(), "contract-body").unwrap();
    let mut incomplete = resume.source.clone();
    let mut empty = resume.result.history.clone();
    empty.operations.clear();
    incomplete.replace_history(empty);
    let retained = prepare_history(session.bytes(), &incomplete).unwrap();
    assert_eq!(retained.result.history, resume.result.history);
    assert_eq!(text(&retained), "ALPHA BETA");
    assert!(history_delta(
        session.bytes(),
        &resume.source,
        &BTreeMap::from([("a".into(), 2)])
    )
    .unwrap()
    .operations
    .is_empty());
}

#[test]
fn epoch_replacement_requires_explicit_decision_and_rejects_old_events() {
    let (bytes, source) = fixture();
    let old = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let first = commit(&mut session, &old.source);
    let base = session.saved_stories().unwrap().remove(0).request;
    let history = text_history::new_history(&base).unwrap();
    let mut start = HistorySource::Start {
        base,
        history,
        replace_epoch: false,
    };
    assert!(prepare_history(session.bytes(), &start).is_err());
    if let HistorySource::Start { replace_epoch, .. } = &mut start {
        *replace_epoch = true;
    }
    let second = commit(&mut session, &start);
    assert_eq!(second.generation, 2);
    assert!(!second.same_epoch_preserved);
    assert_ne!(second.seed_sha256, first.seed_sha256);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    assert!(join_history(session.bytes(), &resumed.source, &[old.result.history]).is_err());
}

#[test]
fn unrelated_revision_can_resume_but_an_old_resume_cannot_replay_automatically() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let report = commit(&mut session, &source);
    let old = resume_history(session.bytes(), "contract-body").unwrap();
    let changed = catalog_change(session.bytes());
    assert!(prepare_history(&changed, &old.source).is_err());
    let resumed = resume_history(&changed, "contract-body").unwrap();
    assert_eq!(resumed.checkpoint_before, Some(report.checkpoint_sha256));
    assert_eq!(
        resumed.result.merged.as_ref().unwrap().input_sha256,
        hash(&changed)
    );
    let mut current = LinkedStorySession::open(changed).unwrap();
    commit(&mut current, &resumed.source);
    assert_eq!(
        current
            .document()
            .document()
            .get_catalog()
            .unwrap()
            .get("WFUnrelatedReview"),
        Some(&PdfObject::Boolean(true))
    );
}

#[test]
fn malformed_saved_seed_history_and_detached_model_cannot_resume() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    commit(&mut session, &source);
    for mutation in 0..3 {
        let altered = change_record(session.bytes(), |record| match mutation {
            0 => {
                record.text_history.as_mut().unwrap().seed.paragraphs[0].text = "forged seed".into()
            }
            1 => record.text_history.as_mut().unwrap().history_sha256 = "bad hash".into(),
            _ => record.request.paragraphs[0].text = "unrecorded change".into(),
        });
        assert!(resume_history(&altered, "contract-body").is_err());
    }
}

#[test]
fn cancelled_history_checkpoint_preserves_bytes_undo_and_receipt() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    let preview = session
        .preview_history(&source, &CancelToken::none())
        .unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(session
        .checkpoint_history(&preview.prepared.source, &preview.receipt, &cancel)
        .is_err());
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_status().undo_steps, 0);
    assert_eq!(session.history_receipt.as_ref(), Some(&preview.receipt));
}

#[test]
fn page_insertion_rebinds_saved_markers_instead_of_replaying_original_page_numbers() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    commit(&mut session, &source);
    let before = resume_history(session.bytes(), "contract-body").unwrap();
    let extra_page = ContentEngine::open_bytes(bytes).unwrap();
    let moved = crate::writer::insert_authored_pages_preserving_catalog(
        session.document().document(),
        &[(extra_page.document(), None)],
        1,
    )
    .unwrap();
    let current = resume_history(&moved, "contract-body").unwrap();
    assert_eq!(current.result.merged.as_ref().unwrap().frames[0].page, 2);
    assert_eq!(current.result.history_sha256, before.result.history_sha256);
    let changed = edit(&moved, &current.source, "a", [0, 5], "ALPHA");
    let mut session = LinkedStorySession::open(moved).unwrap();
    commit(&mut session, &changed.source);
    assert!(session.document().get_page_text(1).unwrap().contains("OLD"));
    assert!(session
        .document()
        .get_page_text(2)
        .unwrap()
        .contains("ALPHA beta"));
}

#[test]
fn modified_source_program_cannot_rebind_only_because_logical_text_matches() {
    let (bytes, source) = fixture();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    commit(&mut session, &source);
    let engine = session.document();
    let reader = engine.document().reader();
    let pages = engine.document().get_pages().unwrap();
    let marker = b"/WFStoryFrame";
    let mut altered = None;
    for &(number, generation) in &pages[0].contents {
        let object = reader.get_object(number, generation).unwrap();
        let mut decoded = crate::filters::decode_stream_lossless_with_limits(
            &object,
            reader,
            &Default::default(),
        )
        .unwrap();
        if let Some(offset) = decoded
            .data
            .windows(marker.len())
            .position(|window| window == marker)
        {
            // A harmless whitespace edit retains all visible/logical text but
            // changes the exact program that the saved owner digest binds.
            decoded.data.insert(offset, b' ');
            let PdfObject::Stream { mut dict, .. } = object else {
                panic!("content stream required")
            };
            dict.remove("Filter");
            dict.remove("DecodeParms");
            dict.insert("Length", PdfObject::Integer(decoded.data.len() as i64));
            altered = Some(
                write_incremental_update(
                    reader,
                    vec![IncrementalObject {
                        number,
                        generation,
                        object: PdfObject::Stream {
                            dict,
                            raw: decoded.data,
                        },
                    }],
                )
                .unwrap(),
            );
            break;
        }
    }
    let altered = altered.expect("story content marker");
    let error = resume_history(&altered, "contract-body")
        .unwrap_err()
        .to_string();
    assert!(error.contains("saved frame content changed"), "{error}");
}

fn set_active(input: &[u8], prepared: &PreparedHistory, active: bool) -> PreparedHistory {
    let target = crate::story_text_history::StoryOperationId {
        actor: "a".into(),
        sequence: 1,
    };
    set_operation_active(
        input,
        &prepared.source,
        &StoryHistorySetActive {
            expected_history_sha256: prepared.result.history_sha256.clone(),
            actor: "a".into(),
            expected_active: !prepared.result.inactive_operations.contains(&target),
            target,
            active,
        },
    )
    .unwrap()
}

#[test]
fn selective_undo_and_redo_survive_checkpoints_without_reverting_other_replica_text() {
    let (bytes, source) = fixture();
    let a = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let b = edit(&bytes, &source, "b", [6, 10], "BETA");
    let joined = join_history(&bytes, &a.source, &[b.result.history]).unwrap();
    let mut session = LinkedStorySession::open(bytes).unwrap();
    let initial = commit(&mut session, &joined.source);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    let undone = set_active(session.bytes(), &resumed, false);
    assert_eq!(text(&undone), "alpha BETA");
    assert_eq!(session.history_status().undo_steps, 1);
    let saved = commit(&mut session, &undone.source);
    assert_eq!(saved.seed_sha256, initial.seed_sha256);
    assert!(saved.same_epoch_preserved);
    assert_eq!(session.history_status().undo_steps, 2);
    let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    let resumed = resume_history(reopened.bytes(), "contract-body").unwrap();
    assert_eq!(resumed.result.history.schema_version, 2);
    assert_eq!(resumed.result.inactive_operations.len(), 1);
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("alpha BETA"));
    let redone = set_active(reopened.bytes(), &resumed, true);
    assert_eq!(text(&redone), "ALPHA BETA");
    assert_eq!(redone.result.frontier.get("a"), Some(&3));
    commit(&mut reopened, &redone.source);
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("ALPHA BETA"));
    assert!(resume_history(reopened.bytes(), "contract-body")
        .unwrap()
        .result
        .inactive_operations
        .is_empty());
}

#[test]
fn changing_an_edits_activity_requires_a_new_preview_and_does_not_publish_the_draft() {
    let (bytes, source) = fixture();
    let changed = edit(&bytes, &source, "a", [0, 5], "ALPHA");
    let mut session = LinkedStorySession::open(bytes.clone()).unwrap();
    let review = session
        .preview_history(&changed.source, &CancelToken::none())
        .unwrap();
    let undone = set_active(&bytes, &changed, false);
    assert_eq!(text(&undone), "alpha beta");
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_status().undo_steps, 0);
    assert!(session
        .checkpoint_history(&undone.source, &review.receipt, &CancelToken::none())
        .is_err());
    assert_eq!(session.bytes(), bytes);
    commit(&mut session, &undone.source);
    let resumed = resume_history(session.bytes(), "contract-body").unwrap();
    assert_eq!(resumed.result.history.operations.len(), 2);
    assert_eq!(text(&resumed), "alpha beta");
}
