//! Unexecuted integration source; metadata/shape checks are not pixel evidence.
use super::tests::{request, two_page_input};
use super::*;
use crate::fonts::line_break_policy::LineComposition;

#[test]
fn story_measurement_and_paint_lines_agree_on_every_forced_separator() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        for composition in [LineComposition::Greedy, LineComposition::Balanced] {
            for separator in [
                "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
            ] {
                let mut story = request(format!("A{separator}B"));
                story.writing_mode = mode;
                story.paragraphs[0].line_break.composition = composition;
                story.paragraphs[0].orphans = 1;
                story.paragraphs[0].widows = 1;
                for frame in &mut story.frames {
                    frame.rect = [10.0, 10.0, 190.0, 190.0];
                }
                let preview = layout(&story, &story.fonts, &[0], vec![]).unwrap();
                let lines = preview
                    .frames
                    .iter()
                    .flat_map(|f| &f.lines)
                    .collect::<Vec<_>>();
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0].text, format!("A{separator}"));
                assert_eq!(lines[1].text, "B");
                assert!(lines
                    .iter()
                    .all(|l| l.bidi.as_ref().unwrap().levels.len() == 1));
                assert_eq!(
                    lines.iter().map(|l| l.text.as_str()).collect::<String>(),
                    story.paragraphs[0].text
                );
            }
        }
    }
}

#[test]
fn vt_and_ff_story_checkpoint_reopen_and_edit_preserve_logical_request_text() {
    for separator in ["\u{000b}", "\u{000c}"] {
        let input = two_page_input();
        let mut story = request(format!("A{separator}B"));
        story.input_sha256 = hash(&input);
        story.paragraphs[0].line_break.composition = LineComposition::Balanced;
        let cancel = crate::CancelToken::none();
        let mut session = LinkedStorySession::open(input).unwrap();
        session.preview(&story, &cancel).unwrap();
        let receipt = session.preview_receipt().unwrap();
        session
            .checkpoint_approved(&story, &receipt, &cancel)
            .unwrap();
        let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
        let mut saved = reopened.saved_stories().unwrap().remove(0).request;
        assert_eq!(saved.paragraphs[0].text, story.paragraphs[0].text);
        saved.paragraphs[0].text.push('C');
        reopened.preview(&saved, &cancel).unwrap();
        let receipt = reopened.preview_receipt().unwrap();
        reopened
            .checkpoint_approved(&saved, &receipt, &cancel)
            .unwrap();
        assert_eq!(
            reopened.saved_stories().unwrap()[0].request.paragraphs[0].text,
            saved.paragraphs[0].text
        );
    }
}

#[test]
fn table_cells_reject_form_feed_until_row_level_page_semantics_can_own_it() {
    let input = tables::tests::fixture(false);
    let mut story = tables::tests::request(&input, false);
    story.paragraphs[2].text = "A\u{000b}B\u{000c}C".into();
    story.paragraphs[2].line_break.composition = LineComposition::Balanced;
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input).unwrap();
    assert!(session.preview(&story, &cancel).is_err());
}

#[test]
fn blank_lines_survive_real_story_content_reopen_and_do_not_pollute_font_resolution() {
    let input = two_page_input();
    let cancel = crate::CancelToken::none();
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let mut story = request("\nA\n\r\nB\n\u{2028}".into());
        story.input_sha256 = hash(&input);
        story.writing_mode = mode;
        story.paragraphs[0].orphans = 1;
        story.paragraphs[0].widows = 1;
        for frame in &mut story.frames {
            frame.rect = [10.0, 10.0, 190.0, 190.0];
        }
        let mut session = LinkedStorySession::open(input.clone()).unwrap();
        session.preview(&story, &cancel).unwrap();
        let receipt = session.preview_receipt().unwrap();
        session
            .checkpoint_approved(&story, &receipt, &cancel)
            .unwrap();
        for _ in 0..2 {
            let reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
            let saved = reopened.saved_stories().unwrap().remove(0).request;
            let mut text = String::new();
            for page in 1..=reopened.document().page_count().unwrap() {
                text.extend(
                    reopened
                        .document()
                        .collect_page_text_chunks(page)
                        .unwrap()
                        .into_iter()
                        .map(|c| c.text),
                );
            }
            assert_eq!(text, story.paragraphs[0].text);
            let fonts = resolve_font_pool(reopened.bytes(), &saved).unwrap();
            // Existing unrelated subset fonts may lack 'A'. What must never
            // enter this pool is the editor's private logical carrier subset.
            for page in 1..=reopened.document().page_count().unwrap() {
                let resources = reopened.document().get_page_resources(page).unwrap();
                for dict in resources
                    .fonts
                    .values()
                    .filter(|dict| crate::advanced_editing::story_carriers::is_font(dict))
                {
                    let program = crate::fonts::provider::embedded_program(
                        reopened.document().document().reader(),
                        dict,
                    )
                    .unwrap();
                    assert!(!fonts.iter().any(|f| f.bytes == program));
                }
            }
            session = reopened;
            session.preview(&saved, &cancel).unwrap();
            let receipt = session.preview_receipt().unwrap();
            session
                .checkpoint_approved(&saved, &receipt, &cancel)
                .unwrap();
        }
    }
}

#[test]
fn blank_table_cell_lines_are_present_in_saved_content_not_only_metadata() {
    let input = tables::tests::fixture(false);
    let mut story = tables::tests::request(&input, false);
    story.paragraphs[2].text = "\nB\r\n\u{2028}".into();
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input).unwrap();
    session.preview(&story, &cancel).unwrap();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&story, &receipt, &cancel)
        .unwrap();
    let mut carried = String::new();
    for page in 1..=session.document().page_count().unwrap() {
        let resources = session.document().get_page_resources(page).unwrap();
        for item in session
            .document()
            .collect_page_scoped_text_chunks(page)
            .unwrap()
        {
            if resources
                .fonts
                .get(&item.chunk.font_name)
                .is_some_and(crate::advanced_editing::story_carriers::is_font)
            {
                assert_eq!(item.chunk.width, 0.0);
                assert!(item.chunk.is_actual_text);
                carried.push_str(&item.chunk.text);
            }
        }
    }
    assert_eq!(carried, "\n\u{2028}");
}
