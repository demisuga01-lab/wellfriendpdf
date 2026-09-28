//! Unexecuted story/table integration specifications; no pixel comparisons.
use super::tests::{request, two_page_input};
use super::*;
use crate::fonts::line_break_policy::{EmergencyWrap, LineComposition};

#[test]
fn balanced_composition_requires_schema_three_and_changes_request_authority() {
    let mut story = request("one two three".into());
    let previous = value_hash(&story).unwrap();
    story.paragraphs[0].line_break.composition = LineComposition::Balanced;
    assert_ne!(value_hash(&story).unwrap(), previous);
    assert_eq!(required_story_schema(&story), 3);
    assert!(!supported_story_schema(1, &story));
    assert!(!supported_story_schema(2, &story));
    assert!(supported_story_schema(3, &story));
    assert!(supported_story_schema(4, &story));
    assert!(supported_story_schema(5, &story));
    assert!(supported_story_schema(6, &story));
    assert!(supported_story_schema(7, &story));
    assert!(supported_story_schema(8, &story));
    assert!(!supported_story_schema(9, &story));
}

#[test]
fn story_modes_share_the_compositor_without_losing_logical_text() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let mut story = request("one two three four five six seven eight".into());
        story.writing_mode = mode;
        story.paragraphs[0].line_break.composition = LineComposition::Balanced;
        story.paragraphs[0].orphans = 1;
        story.paragraphs[0].widows = 1;
        for frame in &mut story.frames {
            frame.rect = if mode.is_vertical() {
                [10.0, 10.0, 190.0, 80.0]
            } else {
                [10.0, 10.0, 80.0, 190.0]
            };
        }
        let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
        let lines = preview
            .frames
            .iter()
            .flat_map(|f| &f.lines)
            .collect::<Vec<_>>();
        assert!(lines.len() > 1);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<String>(),
            story.paragraphs[0].text
        );
        assert!(lines.iter().all(|l| l.bidi.is_some()));
    }
}

#[test]
fn changing_composition_invalidates_approval_and_survives_two_saved_revisions() {
    let input = two_page_input();
    let mut story = request("one two three four five six seven eight".into());
    story.input_sha256 = hash(&input);
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input.clone()).unwrap();
    session.preview(&story, &cancel).unwrap();
    let old = session.preview_receipt().unwrap();
    story.paragraphs[0].line_break.composition = LineComposition::Balanced;
    assert!(session.checkpoint_approved(&story, &old, &cancel).is_err());
    assert_eq!(session.bytes(), input.as_slice());
    session.preview(&story, &cancel).unwrap();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&story, &receipt, &cancel)
        .unwrap();
    let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    let saved = reopened.saved_stories().unwrap().remove(0);
    assert_eq!(saved.schema_version, 3);
    assert_eq!(
        saved.request.paragraphs[0].line_break.composition,
        LineComposition::Balanced
    );
    let mut next = saved.request;
    next.paragraphs[0].text.push_str(" nine ten");
    reopened.preview(&next, &cancel).unwrap();
    let receipt = reopened.preview_receipt().unwrap();
    reopened
        .checkpoint_approved(&next, &receipt, &cancel)
        .unwrap();
    assert_eq!(
        reopened.saved_stories().unwrap()[0].request.paragraphs[0].text,
        next.paragraphs[0].text
    );
}

#[test]
fn balanced_table_cells_use_the_shared_policy_and_reopen_with_it() {
    let input = tables::tests::fixture(false);
    let mut story = tables::tests::request(&input, false);
    story.paragraphs[2].text = "one two three four five six seven eight".into();
    story.paragraphs[2].line_break.composition = LineComposition::Balanced;
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input).unwrap();
    session.preview(&story, &cancel).unwrap();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&story, &receipt, &cancel)
        .unwrap();
    let reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    let saved = reopened.saved_stories().unwrap().remove(0);
    assert_eq!(saved.schema_version, 3);
    assert_eq!(
        saved.request.paragraphs[2].line_break.composition,
        LineComposition::Balanced
    );
    assert_eq!(saved.request.paragraphs[2].text, story.paragraphs[2].text);
    assert!(saved.request.table_layout.is_some());
}

#[test]
fn balanced_protected_words_can_skip_an_inadequate_frame() {
    let mut story = request("AAAAAAAAAAAA".into());
    story.paragraphs[0].line_break.composition = LineComposition::Balanced;
    story.paragraphs[0].line_break.emergency = EmergencyWrap::PreserveWords;
    story.paragraphs[0].orphans = 1;
    story.paragraphs[0].widows = 1;
    story.allow_page_creation = false;
    story.frames[0].rect = [10.0, 10.0, 23.0, 190.0];
    story.frames[1].rect = [10.0, 10.0, 190.0, 190.0];
    let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
    assert!(preview.frames[0].lines.is_empty());
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
