//! Unexecuted native story regressions. No rendered-PDF evidence is implied.
use super::tests::{request, two_page_input};
use super::*;
use crate::fonts::line_break_policy::{EmergencyWrap, LineBreakProfile};

#[test]
fn omitted_policy_keeps_old_serialization_but_nondefault_changes_review_hash() {
    let original = request("Text".into());
    let value = serde_json::to_value(&original).unwrap();
    assert!(value["paragraphs"][0].get("line_break").is_none());
    let reopened: LinkedStoryRequest = serde_json::from_value(value).unwrap();
    assert!(reopened.paragraphs[0].line_break.is_default());
    assert_eq!(
        value_hash(&original).unwrap(),
        value_hash(&reopened).unwrap()
    );
    let mut changed = original.clone();
    changed.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    assert_ne!(
        value_hash(&original).unwrap(),
        value_hash(&changed).unwrap()
    );
    let value = serde_json::to_value(&changed).unwrap();
    assert_eq!(
        value["paragraphs"][0]["line_break"]["emergency"],
        "preserve_words"
    );
}

#[test]
fn horizontal_and_both_vertical_story_modes_use_the_requested_wrap_policy() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let mut story = request("AAAAAAAAAAAA".into());
        story.writing_mode = mode;
        story.paragraphs[0].orphans = 1;
        story.paragraphs[0].widows = 1;
        for frame in &mut story.frames {
            frame.rect = if mode.is_vertical() {
                [10.0, 10.0, 190.0, 23.0]
            } else {
                [10.0, 10.0, 23.0, 190.0]
            };
        }
        let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
        assert_eq!(
            preview
                .frames
                .iter()
                .flat_map(|frame| &frame.lines)
                .map(|line| line.text.as_str())
                .collect::<String>(),
            story.paragraphs[0].text
        );
        story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
        assert!(
            layout(&story, &story.fonts, &[0], vec![]).is_err(),
            "{mode:?}"
        );
        story.paragraphs[0].line_break.emergency = EmergencyWrap::BreakWord;
        story.paragraphs[0].line_break.prohibit_start = "A".into();
        assert!(
            layout(&story, &story.fonts, &[0], vec![]).is_err(),
            "{mode:?}"
        );
    }
}

#[test]
fn protected_words_skip_narrow_frames_in_horizontal_and_vertical_layout() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let mut story = request("AAAAAAAAAAAA".into());
        story.writing_mode = mode;
        story.allow_page_creation = false;
        story.paragraphs[0].orphans = 1;
        story.paragraphs[0].widows = 1;
        story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
        story.frames[0].rect = if mode.is_vertical() {
            [10.0, 10.0, 190.0, 23.0]
        } else {
            [10.0, 10.0, 23.0, 190.0]
        };
        story.frames[1].rect = [10.0, 10.0, 190.0, 190.0];
        let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
        assert!(preview.frames[0].lines.is_empty(), "{mode:?}");
        assert_eq!(
            preview.frames[1]
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<String>(),
            story.paragraphs[0].text
        );
        assert_eq!(preview.generated_pages, 0);
    }
}

#[test]
fn protected_word_overflow_keeps_fitting_prefix_in_earlier_frame() {
    let mut story = request("AA AA AAAAAAAAAAAA".into());
    story.allow_page_creation = false;
    story.paragraphs[0].orphans = 1;
    story.paragraphs[0].widows = 1;
    story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    story.frames[0].rect = [10.0, 10.0, 50.0, 190.0];
    story.frames[1].rect = [10.0, 10.0, 190.0, 190.0];
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    assert!(!preview.frames[0].lines.is_empty());
    assert_eq!(
        preview.frames[1]
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<String>(),
        "AAAAAAAAAAAA"
    );
    assert_eq!(
        preview
            .frames
            .iter()
            .flat_map(|f| &f.lines)
            .map(|l| l.text.as_str())
            .collect::<String>(),
        story.paragraphs[0].text
    );
}

#[test]
fn width_blocked_keep_chain_moves_together_to_later_approved_frame() {
    let mut story = request("Hi".into());
    story.allow_page_creation = false;
    story.paragraphs[0].orphans = 1;
    story.paragraphs[0].widows = 1;
    let mut body = story.paragraphs[0].clone();
    body.id = "body".into();
    body.text = "AAAAAAAAAAAA".into();
    body.line_break.emergency = EmergencyWrap::PreserveWords;
    story.paragraphs[0].keep_with_next = true;
    story.paragraphs.push(body);
    story.frames[0].rect = [10.0, 10.0, 30.0, 190.0];
    story.frames[1].rect = [10.0, 10.0, 190.0, 190.0];
    let preview = layout(&story, &story.fonts, &[0, 0], vec![]).unwrap();
    assert!(preview.frames[0].lines.is_empty());
    for p in &story.paragraphs {
        assert!(preview.frames[1].paragraph_ids.contains(&p.id));
    }
}

#[test]
fn actual_widow_destination_can_skip_an_intervening_overwide_frame() {
    let mut story = request("AA AA AA AA AA AA".into());
    story.allow_page_creation = false;
    story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    story.paragraphs[0].orphans = 2;
    story.paragraphs[0].widows = 2;
    story.frames[0].rect = [10.0, 10.0, 33.0, 55.0];
    story.frames[1].rect = [10.0, 10.0, 190.0, 190.0];
    let mut third = story.frames[1].clone();
    third.id = "third".into();
    third.page = 3;
    third.rect = [10.0, 10.0, 33.0, 190.0];
    story.frames.push(third);
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    assert!(preview.frames[0].lines.len() >= 2);
    assert!(preview.frames[1].lines.is_empty());
    assert!(preview.frames[2].lines.len() >= 2);
    assert_eq!(
        preview
            .frames
            .iter()
            .flat_map(|f| &f.lines)
            .map(|l| l.text.as_str())
            .collect::<String>(),
        story.paragraphs[0].text
    );
}

#[test]
fn repeated_identical_continuations_do_not_make_an_unbreakable_word_fit() {
    let mut story = request("AAAAAAAAAAAA".into());
    story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    story.allow_page_creation = true;
    story.max_new_pages = 100;
    for frame in &mut story.frames {
        frame.rect = [10.0, 10.0, 23.0, 190.0];
    }
    let error = layout(&story, &story.fonts, &[0], vec![]).unwrap_err();
    assert!(error
        .to_string()
        .contains("unbreakable text exceeds frame width"));
}

#[test]
fn wrapping_changes_invalidate_receipts_and_survive_save_reopen_edit() {
    let input = two_page_input();
    let mut story = request("Short reviewed text.".into());
    story.input_sha256 = hash(&input);
    story.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
    story.paragraphs[0].line_break.prohibit_start = ")".into();
    let cancel = crate::cancel::CancelToken::new();
    let mut session = LinkedStorySession::open(input.clone()).unwrap();
    session.preview(&story, &cancel).unwrap();
    let old = session.preview_receipt().unwrap();
    story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    assert!(session.checkpoint_approved(&story, &old, &cancel).is_err());
    assert_eq!(session.bytes(), input.as_slice());
    session.preview(&story, &cancel).unwrap();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&story, &receipt, &cancel)
        .unwrap();
    let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    assert_eq!(reopened.saved_stories().unwrap()[0].schema_version, 2);
    let mut saved = reopened.saved_stories().unwrap().remove(0).request;
    assert_eq!(
        saved.paragraphs[0].line_break,
        story.paragraphs[0].line_break
    );
    saved.paragraphs[0].text = "Changed after reopening.".into();
    reopened.preview(&saved, &cancel).unwrap();
    let receipt = reopened.preview_receipt().unwrap();
    reopened
        .checkpoint_approved(&saved, &receipt, &cancel)
        .unwrap();
    assert_eq!(
        reopened.saved_stories().unwrap()[0].request.paragraphs[0].line_break,
        story.paragraphs[0].line_break
    );
}

#[test]
fn table_cells_share_paragraph_policy_and_preserve_owned_grid_workflow() {
    let input = tables::tests::fixture(false);
    let mut story = tables::tests::request(&input, false);
    story.paragraphs[2].text = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into();
    let cancel = crate::cancel::CancelToken::new();
    let mut session = LinkedStorySession::open(input).unwrap();
    session.preview(&story, &cancel).unwrap();
    story.paragraphs[2].line_break.emergency = EmergencyWrap::PreserveWords;
    assert!(session.preview(&story, &cancel).is_err());
    story.paragraphs[2].text = "Short words".into();
    session.preview(&story, &cancel).unwrap();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&story, &receipt, &cancel)
        .unwrap();
    let saved = session.saved_stories().unwrap().remove(0).request;
    assert_eq!(
        saved.paragraphs[2].line_break.emergency,
        EmergencyWrap::PreserveWords
    );
    assert!(saved.table_layout.is_some());
}

#[test]
fn nondefault_wrapping_requires_the_new_saved_metadata_schema() {
    let mut story = request("Text".into());
    assert_eq!(required_story_schema(&story), 1);
    assert!(supported_story_schema(1, &story));
    story.paragraphs[0].line_break.profile = LineBreakProfile::JapaneseStrict;
    assert_eq!(required_story_schema(&story), 2);
    assert!(!supported_story_schema(1, &story));
    assert!(supported_story_schema(2, &story));
    assert!(!supported_story_schema(0, &story));
    assert!(supported_story_schema(3, &story));
    assert!(supported_story_schema(4, &story));
    assert!(supported_story_schema(5, &story));
    assert!(supported_story_schema(6, &story));
    assert!(supported_story_schema(7, &story));
    assert!(supported_story_schema(8, &story));
    assert!(!supported_story_schema(9, &story));
}

#[test]
fn tabbed_story_uses_exact_fields_and_requires_saved_schema_six() {
    let mut story = request("A\t12.5\tZ".into());
    story.frames[0].rect = [10.0, 10.0, 210.0, 190.0];
    story.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![
            crate::fonts::tab_stops::TabStop {
                position: 100.0,
                alignment: crate::fonts::tab_stops::TabAlignment::Decimal,
                decimal: '.',
                decimal_token: None,
                leader: crate::fonts::tab_stops::TabLeader::None,
                bar: false,
            },
            crate::fonts::tab_stops::TabStop {
                position: 180.0,
                alignment: crate::fonts::tab_stops::TabAlignment::Right,
                decimal: '.',
                decimal_token: None,
                leader: crate::fonts::tab_stops::TabLeader::None,
                bar: false,
            },
        ],
        default_interval: 36.0,
    };
    assert_eq!(required_story_schema(&story), 6);
    assert!(!supported_story_schema(5, &story));
    assert!(supported_story_schema(6, &story));
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    let line = &preview.frames[0].lines[0];
    assert_eq!(line.text, story.paragraphs[0].text);
    assert_eq!(line.tab_segments.len(), 3);
    assert_eq!(line.tab_segments[0].range, [0, 1]);
    assert_eq!(line.tab_segments[1].range, [2, 6]);
    assert_eq!(line.tab_segments[2].range, [7, 8]);
    assert!(line
        .tab_segments
        .windows(2)
        .all(|pair| { pair[0].origin + pair[0].width <= pair[1].origin + 1e-7 }));
}

#[test]
fn decorated_story_tabs_require_schema_seven_and_bind_artifact_geometry() {
    let mut story = request("A\tB".into());
    story.frames[0].rect = [10.0, 10.0, 210.0, 190.0];
    story.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![crate::fonts::tab_stops::TabStop {
            position: 100.0,
            alignment: crate::fonts::tab_stops::TabAlignment::Left,
            decimal: '.',
            decimal_token: None,
            leader: crate::fonts::tab_stops::TabLeader::Dashes,
            bar: true,
        }],
        default_interval: 36.0,
    };
    assert_eq!(required_story_schema(&story), 7);
    assert!(!supported_story_schema(6, &story));
    assert!(supported_story_schema(7, &story));
    assert!(supported_story_schema(8, &story));
    assert!(!supported_story_schema(9, &story));
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    let line = &preview.frames[0].lines[0];
    assert_eq!(line.tab_segments.len(), 2);
    assert_eq!(
        line.tab_decorations,
        vec![
            crate::fonts::tab_stops::PositionedTabDecoration::Leader {
                from: line.tab_segments[0].width,
                to: 100.0,
                leader: crate::fonts::tab_stops::TabLeader::Dashes,
            },
            crate::fonts::tab_stops::PositionedTabDecoration::Bar { position: 100.0 },
        ]
    );
}

#[test]
fn multi_character_decimal_story_requires_schema_eight() {
    let mut story = request("A\t12::50".into());
    story.frames[0].rect = [10.0, 10.0, 210.0, 190.0];
    story.paragraphs[0].tab_stops = crate::fonts::tab_stops::TabStops {
        stops: vec![crate::fonts::tab_stops::TabStop {
            position: 100.0,
            alignment: crate::fonts::tab_stops::TabAlignment::Decimal,
            decimal: '.',
            decimal_token: Some("::".into()),
            leader: crate::fonts::tab_stops::TabLeader::None,
            bar: false,
        }],
        default_interval: 36.0,
    };
    assert_eq!(required_story_schema(&story), 8);
    assert!(!supported_story_schema(7, &story));
    assert!(supported_story_schema(8, &story));
    assert!(!supported_story_schema(9, &story));
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    assert_eq!(preview.frames[0].lines[0].tab_segments.len(), 2);
}
