//! Regression source, not executed during the source-only implementation phase.
use super::*;
use crate::{linked_stories::hash, ContentEngine};

fn fixture() -> (Vec<u8>, LinkedStoryRequest) {
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
    let session = LinkedStorySession::open(bytes.clone()).unwrap();
    let request = serde_json::from_value(json!({
        "story_id":"native-session", "input_sha256":session.revision_sha256(),
        "frames":[{"id":"body","page":1,"logical_range":[0,3],"expected_text":"OLD","rect":[10,10,190,180]}],
        "paragraphs":[{"id":"p","text":"New wording","preferred_font":"Approved","font_size":12,"line_height":14}],
        "fonts":[{"lookup_name":"Approved","bytes":crate::render::get_fallback_font("Helvetica").unwrap()}]
    })).unwrap();
    (bytes, request)
}
fn command(session: &mut LinkedStorySession, value: serde_json::Value) -> serde_json::Value {
    let bytes = execute_json(
        session,
        &serde_json::to_vec(&value).unwrap(),
        &CancelToken::none(),
    )
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn password_open_materializes_an_explicit_unencrypted_working_revision() {
    let (plain, _) = fixture();
    let engine = ContentEngine::open_bytes(plain).unwrap();
    let encrypted = crate::utilities::encrypt_pdf(
        &engine,
        &crate::EncryptParams {
            user_password: crate::crypto::secret_bytes(b"reader-secret".to_vec()),
            owner_password: crate::crypto::secret_bytes(b"owner-secret".to_vec()),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(open_with_password(&encrypted, b"wrong", &CancelToken::none()).is_err());
    assert!(
        open_with_password(&encrypted, b"reader-secret", &CancelToken::none())
            .err()
            .unwrap()
            .to_string()
            .contains("owner password")
    );
    let mut session =
        open_with_password(&encrypted, b"owner-secret", &CancelToken::none()).unwrap();
    let status = command(&mut session, json!({"op":"status"}));
    assert_eq!(status["source_security"]["source_was_encrypted"], true);
    assert_eq!(status["source_security"]["working_copy_decrypted"], true);
    assert_eq!(status["source_security"]["password_retained"], false);
    assert_eq!(status["source_security"]["authenticated_as_owner"], true);
    assert_eq!(status["source_security"]["permissions"], -1);
    assert_eq!(status["source_security"]["modification_permitted"], true);
    assert_eq!(
        status["source_security"]["source_input_sha256"],
        hash(&encrypted)
    );
    assert_eq!(status["revision_sha256"], session.revision_sha256());
    assert_ne!(status["revision_sha256"], hash(&encrypted));
    assert!(!session.document().is_encrypted());
    ContentEngine::open_bytes(session.bytes().to_vec()).unwrap();

    let restricted = crate::utilities::encrypt_pdf(
        &engine,
        &crate::EncryptParams {
            user_password: crate::crypto::secret_bytes(b"restricted-reader".to_vec()),
            owner_password: crate::crypto::secret_bytes(b"restricted-owner".to_vec()),
            permissions: !(1 << 3),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        open_with_password(&restricted, b"restricted-reader", &CancelToken::none())
            .err()
            .unwrap()
            .to_string()
            .contains("owner password")
    );
    let mut restricted_owner =
        open_with_password(&restricted, b"restricted-owner", &CancelToken::none()).unwrap();
    let restricted_status = command(&mut restricted_owner, json!({"op":"status"}));
    assert_eq!(
        restricted_status["source_security"]["permissions"],
        !(1 << 3)
    );
    assert_eq!(
        restricted_status["source_security"]["modification_permitted"],
        true
    );

    let legacy_empty_user = crate::utilities::encrypt_pdf(
        &engine,
        &crate::EncryptParams {
            user_password: crate::crypto::secret_bytes(Vec::new()),
            owner_password: crate::crypto::secret_bytes(b"legacy-owner".to_vec()),
            permissions: !(1 << 3),
            algorithm: crate::EncryptAlgorithm::Aes128,
            ..Default::default()
        },
    )
    .unwrap();
    let mut legacy_owner =
        open_with_password(&legacy_empty_user, b"legacy-owner", &CancelToken::none()).unwrap();
    let legacy_owner_status = command(&mut legacy_owner, json!({"op":"status"}));
    assert_eq!(
        legacy_owner_status["source_security"]["authenticated_as_owner"],
        true
    );
    assert_eq!(
        legacy_owner_status["source_security"]["modification_permitted"],
        true
    );
    assert!(open(&legacy_empty_user, &CancelToken::none())
        .err()
        .unwrap()
        .to_string()
        .contains("owner password"));
}

#[test]
fn static_font_command_prepares_actual_instance_and_story_survives_checkpoint_reopen() {
    let (pdf, mut story) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let source = crate::fonts::font_instance::tests::source();
    let request = crate::fonts::font_instance::tests::request(&source);
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":source,"request":request}),
    );
    assert_eq!(prepared["report"]["coordinates"]["TEST"], 0.5);
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);
    story.fonts = vec![serde_json::from_value(prepared["asset"].clone()).unwrap()];
    story.paragraphs[0].text = "AB A".into();
    let preview = command(&mut session, json!({"op":"preview","request":story}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":story,"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut reopened, json!({"op":"saved_stories"}));
    let mut again = saved[0]["request"].clone();
    again["paragraphs"][0]["text"] = json!("BA B");
    let preview = command(&mut reopened, json!({"op":"preview","request":again}));
    command(
        &mut reopened,
        json!({"op":"checkpoint","request":again,"receipt":preview["receipt"]}),
    );
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("BA B"));
}

#[test]
fn static_font_command_rejects_stale_selection_without_changing_existing_preview() {
    let (pdf, story) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let preview = command(&mut session, json!({"op":"preview","request":story}));
    let source = crate::fonts::font_instance::tests::source();
    let mut request = crate::fonts::font_instance::tests::request(&source);
    request.selection.source_sha256.clear();
    let bad = serde_json::to_vec(&json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":source,"request":request})).unwrap();
    assert!(execute_json(&mut session, &bad, &CancelToken::none()).is_err());
    assert_eq!(session.bytes(), pdf);
    assert_eq!(
        serde_json::to_value(session.preview_receipt().unwrap()).unwrap(),
        preview["receipt"]
    );
}

#[test]
fn cff2_instance_command_keeps_native_source_editing_through_checkpoint_and_reopen() {
    use crate::fonts::cff2_program::tests as cff;
    let (pdf, mut story) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["font_instance_protocol_version"],
        4
    );
    let font = cff::sfnt(
        cff::fixture(
            &[vec![], cff::variable_glyph(None), vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            Some(cff::store(&[[0, 16384, 16384]], &[vec![0]])),
            &[],
        ),
        true,
    );
    let request = crate::fonts::font_instance::tests::request(&font);
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":font,"request":request}),
    );
    assert_eq!(prepared["report"]["output_outline_format"], "cff1");
    assert_eq!(prepared["report"]["cff2"]["glyphs"], 4);
    assert_eq!(session.bytes(), pdf);
    story.fonts = vec![serde_json::from_value(prepared["asset"].clone()).unwrap()];
    story.paragraphs[0].text = "A A".into();
    let preview = command(&mut session, json!({"op":"preview","request":story}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":story,"receipt":preview["receipt"]}),
    );
    let mut again = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut again, json!({"op":"saved_stories"}));
    let mut draft = saved[0]["request"].clone();
    draft["paragraphs"][0]["text"] = json!("AA A");
    let preview = command(&mut again, json!({"op":"preview","request":draft}));
    command(
        &mut again,
        json!({"op":"checkpoint","request":draft,"receipt":preview["receipt"]}),
    );
    assert!(again.document().get_page_text(1).unwrap().contains("AA A"));
}

#[test]
fn selected_collection_asset_survives_native_story_checkpoint_reopen_and_reedit() {
    let (pdf, mut request) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let font = crate::render::get_fallback_font("Helvetica").unwrap();
    let collection = crate::fonts::font_asset::tests::collection(&[font], 0x00010000, false);
    let catalog = command(
        &mut session,
        json!({"op":"inspect_font","bytes":collection}),
    );
    assert_eq!(catalog["faces"][0]["face_index"], 0);
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font","bytes":collection,
        "lookup_name":"Approved","selection":{"source_sha256":catalog["source_sha256"],"face_index":0}}),
    );
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);
    request.fonts = vec![serde_json::from_value(prepared["asset"].clone()).unwrap()];
    let preview = command(&mut session, json!({"op":"preview","request":request}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":request,"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut reopened, json!({"op":"saved_stories"}));
    let mut again = saved[0]["request"].clone();
    again["paragraphs"][0]["text"] = json!("Changed after reopening");
    // Saved story assets are native standalone bytes, not a dropped collection
    // index or dependence on the original local file.
    let restored: LinkedStoryRequest = serde_json::from_value(again.clone()).unwrap();
    assert!(restored.fonts.iter().all(|f| !f.bytes.starts_with(b"ttcf")));
    let preview = command(&mut reopened, json!({"op":"preview","request":again}));
    command(
        &mut reopened,
        json!({"op":"checkpoint","request":again,"receipt":preview["receipt"]}),
    );
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("Changed after reopening"));
}

#[test]
fn contour_normalization_decision_is_atomic_and_saved_font_can_be_edited_again() {
    let (pdf, mut story) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let font = crate::fonts::cff2_program::instance::normalized_test_font(true);
    let mut request = crate::fonts::font_instance::tests::request(&font);
    request.coordinates.clear();
    request.cff2_contours = Some(crate::fonts::font_instance::Cff2ContourNormalization {
        tolerance_font_units: 0.001,
        allow_hint_loss: false,
    });
    let rejected=serde_json::to_vec(&json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":font,"request":request})).unwrap();
    assert!(execute_json(&mut session, &rejected, &CancelToken::none()).is_err());
    assert_eq!(session.bytes(), pdf);
    request.cff2_contours.as_mut().unwrap().allow_hint_loss = true;
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":font,"request":request}),
    );
    assert_eq!(
        prepared["report"]["cff2"]["contour_normalization"]["dehinted_glyphs"],
        json!([1])
    );
    assert_eq!(session.bytes(), pdf);
    story.fonts = vec![serde_json::from_value(prepared["asset"].clone()).unwrap()];
    story.paragraphs[0].text = "A A".into();
    let preview = command(&mut session, json!({"op":"preview","request":story}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":story,"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut reopened, json!({"op":"saved_stories"}));
    let mut again = saved[0]["request"].clone();
    again["paragraphs"][0]["text"] = json!("AA A");
    let preview = command(&mut reopened, json!({"op":"preview","request":again}));
    command(
        &mut reopened,
        json!({"op":"checkpoint","request":again,"receipt":preview["receipt"]}),
    );
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("AA A"));
}

#[test]
fn compatible_hinted_font_normalizes_without_loss_consent_and_survives_story_reopen() {
    let (pdf, mut story) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let font = crate::fonts::cff2_program::instance::compatible_normalization_test_font();
    let mut request = crate::fonts::font_instance::tests::request(&font);
    request.coordinates.clear();
    request.cff2_contours = Some(crate::fonts::font_instance::Cff2ContourNormalization {
        tolerance_font_units: 0.001,
        allow_hint_loss: false,
    });
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font_instance","lookup_name":"Approved","bytes":font,"request":request}),
    );
    let cff = &prepared["report"]["cff2"];
    assert_eq!(cff["stem_hints"], 1);
    assert_eq!(cff["masks"], 1);
    assert_eq!(
        cff["contour_normalization"]["preserved_hint_glyphs"],
        json!([1])
    );
    assert_eq!(
        cff["contour_normalization"]["exact_linear_preserved_glyphs"],
        json!([1])
    );
    assert_eq!(cff["contour_normalization"]["broadphase_pairs"], 0);
    assert_eq!(cff["contour_normalization"]["dehinted_glyphs"], json!([]));
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);
    story.fonts = vec![serde_json::from_value(prepared["asset"].clone()).unwrap()];
    story.paragraphs[0].text = "A A".into();
    let preview = command(&mut session, json!({"op":"preview","request":story}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":story,"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut reopened, json!({"op":"saved_stories"}));
    let mut again = saved[0]["request"].clone();
    again["paragraphs"][0]["text"] = json!("AA A");
    let preview = command(&mut reopened, json!({"op":"preview","request":again}));
    command(
        &mut reopened,
        json!({"op":"checkpoint","request":again,"receipt":preview["receipt"]}),
    );
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("AA A"));
}

#[test]
fn default_cff2_preparation_checks_contours_and_is_atomic_before_story_use() {
    let (pdf, _) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let overlapping = crate::fonts::cff2_program::instance::normalized_test_font(true);
    let mut rejected = crate::fonts::font_instance::tests::request(&overlapping);
    rejected.coordinates.clear();
    rejected.cff2_contours = None;
    let payload=serde_json::to_vec(&json!({"op":"prepare_font_instance","lookup_name":"Checked","bytes":overlapping,"request":rejected})).unwrap();
    let error = execute_json(&mut session, &payload, &CancelToken::none())
        .err()
        .unwrap();
    assert!(error
        .to_string()
        .contains("enable cff2_contours normalization"));
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);

    let compatible = crate::fonts::cff2_program::instance::compatible_normalization_test_font();
    let mut accepted = crate::fonts::font_instance::tests::request(&compatible);
    accepted.coordinates.clear();
    accepted.cff2_contours = None;
    let prepared = command(
        &mut session,
        json!({"op":"prepare_font_instance","lookup_name":"Checked","bytes":compatible,"request":accepted}),
    );
    let cff = &prepared["report"]["cff2"];
    assert_eq!(cff["contour_overlaps_checked"], true);
    assert_eq!(cff["contour_overlaps_removed"], false);
    assert_eq!(cff["contour_normalization"], serde_json::Value::Null);
    assert_eq!(
        cff["preserved_contour_check"]["exact_linear_preserved_glyphs"],
        json!([1])
    );
    assert_eq!(
        cff["preserved_contour_check"]["preserved_hint_glyphs"],
        json!([1])
    );
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);
}

#[test]
fn font_command_budget_stale_selection_and_cancellation_do_not_mutate_session() {
    let (pdf, _) = fixture();
    let mut session = open(&pdf, &CancelToken::none()).unwrap();
    let source = crate::render::get_fallback_font("Helvetica").unwrap();
    let catalog = command(&mut session, json!({"op":"inspect_font","bytes":source}));
    let bad = serde_json::to_vec(
        &json!({"op":"prepare_font","bytes":source,"lookup_name":"Selected",
        "selection":{"face_index":0,"source_sha256":"wrong"}}),
    )
    .unwrap();
    assert!(execute_json(&mut session, &bad, &CancelToken::none()).is_err());
    let unknown_axes = serde_json::to_vec(
        &json!({"op":"prepare_font","bytes":source,"lookup_name":"Selected",
        "selection":{"face_index":0,"source_sha256":catalog["source_sha256"],"axes":{"wght":700}}}),
    )
    .unwrap();
    assert!(execute_json(&mut session, &unknown_axes, &CancelToken::none()).is_err());
    assert!(execute(
        &mut session,
        StorySessionCommand::InspectFont {
            bytes: vec![0; 4 * 1024 * 1024 + 1]
        },
        &CancelToken::none()
    )
    .is_err());
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(execute(
        &mut session,
        StorySessionCommand::InspectFont {
            bytes: source.to_vec()
        },
        &cancel
    )
    .is_err());
    assert_eq!(session.bytes(), pdf);
    assert_eq!(session.history_status().undo_steps, 0);
}

#[test]
fn reviewed_structure_uses_native_preview_checkpoint_and_reopen_without_early_publication() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["line_break_policy_version"],
        2
    );
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["story_pagination_policy_version"],
        1
    );
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["line_shaping_context_version"],
        3
    );
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["tab_stop_layout_version"],
        3
    );
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["saved_story_schema_max"],
        8
    );
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["history_seed_schema_max"],
        6
    );
    let base_hash = crate::story_merge::story_fingerprint(&base).unwrap();
    let mut left = base.clone();
    let mut right = base.clone();
    left.paragraphs[0].text = "Left wording".into();
    right.paragraphs[0].text = "Right wording".into();
    let request = json!({"base":base,"branches":[
        {"branch_id":"left","base_story_sha256":base_hash,"proposed":left},
        {"branch_id":"right","base_story_sha256":base_hash,"proposed":right}
    ]});
    let review = command(
        &mut session,
        json!({"op":"review_structure","request":request}),
    );
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_status().undo_steps, 0);
    let mut paragraphs = review["candidate"]["paragraphs"].clone();
    paragraphs[0]["text"] = json!("Resolved wording");
    let frame_geometry = review["candidate"]["frames"].as_array().unwrap().iter().map(|frame|
        json!({"frame_id":frame["id"],"rect":frame["rect"],"exclusions":frame["exclusions"]})
    ).collect::<Vec<_>>();
    let acknowledgments = review["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|conflict| conflict["conflict_id"].clone())
        .collect::<Vec<_>>();
    assert!(!acknowledgments.is_empty());
    let resolved = command(
        &mut session,
        json!({"op":"resolve_structure","request":request,"resolution":{
            "expected_review_sha256":review["review_sha256"],"acknowledged_conflicts":acknowledgments,
            "paragraphs":paragraphs,"frame_geometry":frame_geometry
        }}),
    );
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_status().undo_steps, 0);
    let preview = command(
        &mut session,
        json!({"op":"preview","request":resolved["merged"]}),
    );
    command(
        &mut session,
        json!({"op":"checkpoint","request":resolved["merged"],"receipt":preview["receipt"]}),
    );
    let output = session.bytes().to_vec();
    let reopened = open(&output, &CancelToken::none()).unwrap();
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("Resolved wording"));
    assert!(!reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("OLD"));
    let stale = serde_json::to_vec(&json!({"op":"review_structure","request":request})).unwrap();
    assert!(execute_json(&mut session, &stale, &CancelToken::none()).is_err());
    assert_eq!(session.bytes(), output);
}

#[test]
fn paragraph_wrap_policy_round_trips_through_native_json_checkpoint() {
    let (bytes, mut request) = fixture();
    request.paragraphs[0].line_break = serde_json::from_value(json!({
        "profile":"japanese_strict", "emergency":"preserve_words",
        "prohibit_start":")", "prohibit_end":"("
    }))
    .unwrap();
    let policy = serde_json::to_value(&request.paragraphs[0].line_break).unwrap();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let preview = command(&mut session, json!({"op":"preview","request":request}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":request,"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let saved = command(&mut reopened, json!({"op":"saved_stories"}));
    assert_eq!(saved[0]["schema_version"], 2);
    assert_eq!(saved[0]["request"]["paragraphs"][0]["line_break"], policy);
}

#[test]
fn session_protocol_receipt_checkpoint_reopen_and_exact_history() {
    let (bytes, request) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let preview = command(&mut session, json!({"op":"preview","request":request}));
    assert_eq!(session.bytes(), bytes);
    assert!(preview["receipt"]["request_sha256"].is_string());
    let mut changed = request.clone();
    changed.paragraphs[0].text = "Unapproved".into();
    let unapproved = json!({"op":"checkpoint","request":changed,"receipt":preview["receipt"]});
    assert!(execute_json(
        &mut session,
        &serde_json::to_vec(&unapproved).unwrap(),
        &CancelToken::none()
    )
    .is_err());
    assert_eq!(session.bytes(), bytes);
    let report = command(
        &mut session,
        json!({"op":"checkpoint","request":request,"receipt":preview["receipt"]}),
    );
    assert_eq!(report["output_sha256"], session.revision_sha256());
    let output = session.bytes().to_vec();
    assert!(session
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("New wording"));
    assert!(!session.document().get_page_text(1).unwrap().contains("OLD"));
    assert!(session.preview_receipt().is_err());
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["history"]["undo_steps"],
        1
    );
    assert_eq!(command(&mut session, json!({"op":"undo"})), true);
    assert_eq!(session.bytes(), bytes);
    assert_eq!(command(&mut session, json!({"op":"redo"})), true);
    assert_eq!(session.bytes(), output);
    let mut reopened = open(&output, &CancelToken::none()).unwrap();
    assert_eq!(
        command(&mut reopened, json!({"op":"saved_stories"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn causal_history_projects_a_draft_then_uses_native_preview_checkpoint_and_rejects_stale_epoch() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let initial = command(&mut session, json!({"op":"text_history_new","base":base}));
    let projected = command(
        &mut session,
        json!({"op":"text_history_edit","base":base,"history":initial["history"],
        "edit":{"actor":"replica-a","paragraph_id":"p","range":[0,3],"expected_text":"New","replacement":"Merged","expected_history_sha256":initial["history_sha256"]}}),
    );
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_status().undo_steps, 0);
    let draft = projected["merged"].clone();
    assert_eq!(draft["paragraphs"][0]["text"], "Merged wording");
    let preview = command(&mut session, json!({"op":"preview","request":draft}));
    command(
        &mut session,
        json!({"op":"checkpoint","request":draft,"receipt":preview["receipt"]}),
    );
    assert!(session
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("Merged wording"));
    let saved = session.bytes().to_vec();
    let stale = json!({"op":"text_history_merge","base":base,"histories":[projected["history"]]});
    assert!(execute_json(
        &mut session,
        &serde_json::to_vec(&stale).unwrap(),
        &CancelToken::none()
    )
    .is_err());
    assert_eq!(session.bytes(), saved);
}

#[test]
fn session_protocol_rejects_bad_commands_and_cancellation_without_mutation() {
    let (bytes, request) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    for invalid in [
        b"{\"op\":\"undo\",\"op\":\"redo\"}".as_slice(),
        b"{\"op\":\"status\",\"ignore_policy\":true}",
        b"{\"op\":\"source_model\",\"page\":1,\"ignore_policy\":true}",
        b"{\"op\":\"unknown\"}",
        b"{\"op\":\"checkpoint\"}",
        b"{\"op\":\"page_geometry\",\"page\":1,\"dpi\":0}",
        b"\xff",
    ] {
        assert!(execute_json(&mut session, invalid, &CancelToken::none()).is_err());
        assert_eq!(session.bytes(), bytes);
    }
    let preview = command(&mut session, json!({"op":"preview","request":request}));
    let receipt = session.preview_receipt().unwrap();
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let checkpoint = json!({"op":"checkpoint","request":request,"receipt":preview["receipt"]});
    assert!(execute_json(
        &mut session,
        &serde_json::to_vec(&checkpoint).unwrap(),
        &cancelled
    )
    .is_err());
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.preview_receipt().unwrap(), receipt);
    assert!(open(&bytes, &cancelled).is_err());
    assert!(render_page_png(&session, 1, 72, &cancelled).is_err());
    assert_eq!(session.history_status().undo_steps, 0);
}

#[test]
fn saved_history_protocol_reopens_and_checkpoints_with_the_same_causal_epoch() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let initial = command(&mut session, json!({"op":"text_history_new","base":base}));
    let source = json!({"kind":"start","base":base,"history":initial["history"]});
    let prepared = command(
        &mut session,
        json!({"op":"history_prepare","source":source}),
    );
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":prepared["source"]}),
    );
    assert_eq!(session.bytes(), bytes);
    let report = command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    assert_eq!(report["generation"], 1);
    assert_eq!(report["output_sha256"], session.revision_sha256());
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        resumed["result"]["history"]["base_story_sha256"],
        initial["history"]["base_story_sha256"]
    );
    let edited = command(
        &mut reopened,
        json!({"op":"history_edit","source":resumed["source"],"edit":{
        "actor":"replica-a","paragraph_id":"p","range":[0,3],"expected_text":"New","replacement":"Changed",
        "expected_history_sha256":resumed["result"]["history_sha256"]}}),
    );
    let delta = command(
        &mut reopened,
        json!({"op":"history_delta","source":edited["source"],"peer":{}}),
    );
    let joined = command(
        &mut reopened,
        json!({"op":"history_join","source":resumed["source"],"histories":[delta]}),
    );
    assert_eq!(
        joined["result"]["merged"]["paragraphs"][0]["text"],
        "Changed wording"
    );
    let preview = command(
        &mut reopened,
        json!({"op":"history_preview","source":joined["source"]}),
    );
    let next = command(
        &mut reopened,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    assert_eq!(next["generation"], 2);
    assert_eq!(next["same_epoch_preserved"], true);
    assert_eq!(next["seed_sha256"], report["seed_sha256"]);
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("Changed wording"));
}

#[test]
fn causal_paragraph_style_survives_native_checkpoint_reopen_and_reedit() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let styled = command(
        &mut session,
        json!({"op":"text_history_style","base":base,"history":empty["history"],"edit":{
        "expected_history_sha256":empty["history_sha256"],
        "expected_style_conflicts_sha256":empty["style_conflicts_sha256"],
        "actor":"designer","paragraph_id":"p","expected":{"rgb":[0.0,0.0,0.0]},
        "replacement":{"rgb":[0.2,0.3,0.4]}}}),
    );
    assert_eq!(styled["history"]["schema_version"], 3);
    assert_eq!(
        styled["merged"]["paragraphs"][0]["rgb"],
        json!([0.2, 0.3, 0.4])
    );
    let source = json!({"kind":"start","base":base,"history":styled["history"]});
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":source}),
    );
    command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );

    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        resumed["result"]["merged"]["paragraphs"][0]["rgb"],
        json!([0.2, 0.3, 0.4])
    );
    let restyled = command(
        &mut reopened,
        json!({"op":"history_style","source":resumed["source"],"edit":{
        "expected_history_sha256":resumed["result"]["history_sha256"],
        "expected_style_conflicts_sha256":resumed["result"]["style_conflicts_sha256"],
        "actor":"designer","paragraph_id":"p","expected":{"rgb":[0.2,0.3,0.4]},
        "replacement":{"rgb":[0.6,0.1,0.2]}}}),
    );
    let preview = command(
        &mut reopened,
        json!({"op":"history_preview","source":restyled["source"]}),
    );
    command(
        &mut reopened,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    let final_state = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        final_state["result"]["merged"]["paragraphs"][0]["rgb"],
        json!([0.6, 0.1, 0.2])
    );
}

#[test]
fn causal_paragraph_structure_survives_native_checkpoint_reopen_move_and_delete() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    assert_eq!(
        command(&mut session, json!({"op":"status"}))
            ["history_paragraph_structure_protocol_version"],
        1
    );
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let mut inserted = serde_json::to_value(&base).unwrap()["paragraphs"][0].clone();
    inserted["id"] = json!("p2");
    inserted["text"] = json!("Second paragraph");
    let added = command(
        &mut session,
        json!({"op":"text_history_structure","base":base,"history":empty["history"],"edit":{
            "expected_history_sha256":empty["history_sha256"],
            "expected_structure_conflicts_sha256":empty["structure_conflicts_sha256"],
            "actor":"editor","paragraph_id":"p2","expected_absent":true,"expected":{},
            "replacement":{"present":true,"position":{"after":"p"},"inserted_paragraph":inserted}}}),
    );
    assert_eq!(added["history"]["schema_version"], 4);
    assert_eq!(added["merged"]["paragraphs"].as_array().unwrap().len(), 2);
    let source = json!({"kind":"start","base":base,"history":added["history"]});
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":source}),
    );
    command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );

    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(resumed["result"]["merged"]["paragraphs"][1]["id"], "p2");
    let moved = command(
        &mut reopened,
        json!({"op":"history_structure","source":resumed["source"],"edit":{
            "expected_history_sha256":resumed["result"]["history_sha256"],
            "expected_structure_conflicts_sha256":resumed["result"]["structure_conflicts_sha256"],
            "actor":"editor","paragraph_id":"p2","expected_absent":false,
            "expected":{"position":{"after":"p"}},"replacement":{"position":{"after":null}}}}),
    );
    assert_eq!(moved["result"]["merged"]["paragraphs"][0]["id"], "p2");
    let deleted = command(
        &mut reopened,
        json!({"op":"history_structure","source":moved["source"],"edit":{
            "expected_history_sha256":moved["result"]["history_sha256"],
            "expected_structure_conflicts_sha256":moved["result"]["structure_conflicts_sha256"],
            "actor":"editor","paragraph_id":"p","expected_absent":false,
            "expected":{"present":true},"replacement":{"present":false}}}),
    );
    let preview = command(
        &mut reopened,
        json!({"op":"history_preview","source":deleted["source"]}),
    );
    command(
        &mut reopened,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    let final_state = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        final_state["result"]["merged"]["paragraphs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(final_state["result"]["merged"]["paragraphs"][0]["id"], "p2");
}

#[test]
fn causal_inline_style_protocol_uses_atom_targets_and_does_not_publish_pdf_bytes() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    assert_eq!(
        command(&mut session, json!({"op":"status"}))["history_inline_style_protocol_version"],
        1
    );
    let before = session.bytes().to_vec();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let marked = command(
        &mut session,
        json!({"op":"text_history_inline_style","base":base,"history":empty["history"],"edit":{
            "expected_history_sha256":empty["history_sha256"],
            "expected_inline_conflicts_sha256":empty["inline_conflicts_sha256"],
            "actor":"designer","paragraph_id":"p","range":[0,3],"expected_text":"New",
            "replacement":{"rgb":[0.8,0.1,0.2]}}}),
    );
    assert_eq!(marked["history"]["schema_version"], 5);
    assert_eq!(
        marked["inline_style_runs"][0]["logical_range"],
        json!([0, 3])
    );
    assert_eq!(
        marked["inline_style_runs"][0]["style"]["rgb"],
        json!([0.8, 0.1, 0.2])
    );
    assert_eq!(session.bytes(), before);
    assert_eq!(session.history_status().undo_steps, 0);
}

#[test]
fn durable_inline_history_paints_checkpoints_reopens_and_retains_atom_targets() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let before = session.bytes().to_vec();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let source = json!({
        "kind":"start", "base":base, "history":empty["history"], "replace_epoch":false
    });
    let marked = command(
        &mut session,
        json!({"op":"history_inline_style","source":source,"edit":{
            "expected_history_sha256":empty["history_sha256"],
            "expected_inline_conflicts_sha256":empty["inline_conflicts_sha256"],
            "actor":"designer","paragraph_id":"p","range":[0,3],"expected_text":"New",
            "replacement":{"font_size":15.0,"rgb":[0.8,0.1,0.2]}}}),
    );
    assert_eq!(marked["result"]["history"]["schema_version"], 5);
    assert_eq!(
        marked["result"]["inline_style_runs"][0]["style"]["font_size"],
        15.0
    );
    let preview = command(
        &mut session,
        json!({"op":"history_preview", "source":marked["source"]}),
    );
    assert_eq!(session.bytes(), before);
    let report = command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    assert_eq!(report["generation"], 1);
    assert_ne!(session.bytes(), before);
    let saved = crate::linked_stories::load_linked_stories(session.bytes())
        .unwrap()
        .remove(0)
        .request;
    assert!(saved.frames[0]
        .owner
        .as_ref()
        .unwrap()
        .paint_sha256
        .is_some());
    assert!(saved.frames[0]
        .owner
        .as_ref()
        .unwrap()
        .shape_sha256
        .is_some());
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        resumed["result"]["inline_style_runs"][0]["logical_range"],
        json!([0, 3])
    );
    assert_eq!(
        resumed["result"]["merged"]["paragraphs"][0]["inline_styles"][0]["rgb"],
        json!([0.8, 0.1, 0.2])
    );
    assert_eq!(reopened.history_status().undo_steps, 0);
}

#[test]
fn saved_history_compaction_requires_an_exact_plan_and_starts_a_new_epoch() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let edited = command(
        &mut session,
        json!({"op":"text_history_edit","base":base,"history":empty["history"],"edit":{
        "expected_history_sha256":empty["history_sha256"],"actor":"replica-a","paragraph_id":"p",
        "range":[0,3],"expected_text":"New","replacement":"Changed"}}),
    );
    let source = json!({"kind":"start","base":base,"history":edited["history"]});
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":source}),
    );
    command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );

    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    let request = json!({
        "input_sha256": reopened.revision_sha256(),
        "story_id": "native-session",
        "expected_checkpoint_sha256": resumed["checkpoint_before"],
        "expected_history_sha256": resumed["result"]["history_sha256"],
        "acknowledged_frontier": resumed["result"]["frontier"],
        "acknowledge_operation_and_undo_loss": true,
        "acknowledge_prior_epoch_rejected": true
    });
    let plan = command(
        &mut reopened,
        json!({"op":"history_compaction_plan","request":request}),
    );
    assert_eq!(plan["source_operation_count"], 1);
    let before = reopened.bytes().to_vec();
    let wrong = serde_json::to_vec(&json!({
        "op":"history_compaction_apply","request":request,"approved_plan_sha256":"0".repeat(64)
    }))
    .unwrap();
    assert!(execute_json(&mut reopened, &wrong, &CancelToken::none()).is_err());
    assert_eq!(reopened.bytes(), before);

    let report = command(
        &mut reopened,
        json!({"op":"history_compaction_apply","request":request,
        "approved_plan_sha256":plan["plan_sha256"]}),
    );
    assert_eq!(report["generation_before"], 1);
    assert_eq!(report["generation_after"], 2);
    assert_eq!(report["retired_operation_count"], 1);
    assert_eq!(report["prior_epoch_rejected"], true);
    assert_eq!(report["exact_session_undo_available"], true);
    assert_eq!(report["output_sha256"], reopened.revision_sha256());
    let compacted = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert!(compacted["result"]["history"]["operations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        compacted["result"]["merged"]["paragraphs"][0]["text"],
        "Changed wording"
    );
    assert_eq!(command(&mut reopened, json!({"op":"undo"})), true);
    let restored = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    assert_eq!(
        restored["result"]["history"]["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn selective_activity_commands_preserve_native_preview_authority_across_reopen() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let edited = command(
        &mut session,
        json!({"op":"text_history_edit","base":base,"history":empty["history"],"edit":{
        "expected_history_sha256":empty["history_sha256"],"actor":"a","paragraph_id":"p","range":[0,3],"expected_text":"New","replacement":"Changed"}}),
    );
    let undone = command(
        &mut session,
        json!({"op":"text_history_set_active","base":base,"history":edited["history"],"change":{
        "expected_history_sha256":edited["history_sha256"],"actor":"a","target":{"actor":"a","sequence":1},"expected_active":true,"active":false}}),
    );
    assert_eq!(undone["merged"]["paragraphs"][0]["text"], "New wording");
    assert_eq!(session.bytes(), bytes);
    let source = json!({"kind":"start","base":base,"history":undone["history"]});
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":source}),
    );
    command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    let redone = command(
        &mut reopened,
        json!({"op":"history_set_active","source":resumed["source"],"change":{
        "expected_history_sha256":resumed["result"]["history_sha256"],"actor":"a","target":{"actor":"a","sequence":1},"expected_active":false,"active":true}}),
    );
    assert_eq!(
        redone["result"]["merged"]["paragraphs"][0]["text"],
        "Changed wording"
    );
    let review = command(
        &mut reopened,
        json!({"op":"history_preview","source":redone["source"]}),
    );
    command(
        &mut reopened,
        json!({"op":"history_checkpoint","source":review["prepared"]["source"],"receipt":review["receipt"]}),
    );
    assert!(reopened
        .document()
        .get_page_text(1)
        .unwrap()
        .contains("Changed wording"));
}

#[test]
fn grouped_activity_commands_are_atomic_across_native_checkpoint_and_reopen() {
    let (bytes, base) = fixture();
    let mut session = open(&bytes, &CancelToken::none()).unwrap();
    let empty = command(&mut session, json!({"op":"text_history_new","base":base}));
    let first = command(
        &mut session,
        json!({"op":"text_history_edit","base":base,"history":empty["history"],"edit":{
        "expected_history_sha256":empty["history_sha256"],"actor":"a","paragraph_id":"p","range":[0,3],"expected_text":"New","replacement":"Fresh"}}),
    );
    let second = command(
        &mut session,
        json!({"op":"text_history_edit","base":base,"history":first["history"],"edit":{
        "expected_history_sha256":first["history_sha256"],"actor":"a","paragraph_id":"p","range":[6,13],"expected_text":"wording","replacement":"text"}}),
    );
    let targets = json!([
        {"actor":"a","sequence":2},
        {"actor":"a","sequence":1}
    ]);
    let undone = command(
        &mut session,
        json!({"op":"text_history_set_many_active","base":base,"history":second["history"],"change":{
        "expected_history_sha256":second["history_sha256"],"actor":"a","targets":targets,"expected_active":true,"active":false}}),
    );
    assert_eq!(undone["merged"]["paragraphs"][0]["text"], "New wording");
    assert_eq!(undone["inactive_operations"].as_array().unwrap().len(), 2);
    assert_eq!(session.bytes(), bytes);
    let source = json!({"kind":"start","base":base,"history":undone["history"]});
    let preview = command(
        &mut session,
        json!({"op":"history_preview","source":source}),
    );
    command(
        &mut session,
        json!({"op":"history_checkpoint","source":preview["prepared"]["source"],"receipt":preview["receipt"]}),
    );
    let mut reopened = open(session.bytes(), &CancelToken::none()).unwrap();
    let resumed = command(
        &mut reopened,
        json!({"op":"history_resume","story_id":"native-session"}),
    );
    let redone = command(
        &mut reopened,
        json!({"op":"history_set_many_active","source":resumed["source"],"change":{
        "expected_history_sha256":resumed["result"]["history_sha256"],"actor":"a","targets":targets,"expected_active":false,"active":true}}),
    );
    assert_eq!(
        redone["result"]["merged"]["paragraphs"][0]["text"],
        "Fresh text"
    );
    assert_eq!(redone["result"]["frontier"]["a"], 6);
}
