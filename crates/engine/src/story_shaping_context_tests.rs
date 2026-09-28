//! Unexecuted integration source. These cases do not compare independent pixels.
use super::tests::{request, two_page_input};
use super::*;

fn arabic_font() -> ApprovedFontAsset {
    ApprovedFontAsset {
        lookup_name: "Approved Arabic".into(),
        bytes: crate::render::get_fallback_font("Symbol").unwrap().to_vec(),
    }
}
fn story(text: String) -> LinkedStoryRequest {
    let mut request = request(text);
    request.fonts = vec![arabic_font()];
    request.paragraphs[0].preferred_font = "Approved Arabic".into();
    request.paragraphs[0].rtl = true;
    request.paragraphs[0].orphans = 1;
    request.paragraphs[0].widows = 1;
    request
}

#[test]
fn horizontal_and_vertical_preview_lines_retain_serializable_parent_context() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let mut request = story("ب".repeat(16));
        request.writing_mode = mode;
        for frame in &mut request.frames {
            frame.rect = if mode.is_vertical() {
                [10.0, 10.0, 190.0, 28.0]
            } else {
                [10.0, 10.0, 28.0, 190.0]
            };
        }
        let prepared = crate::fonts::shaper::ParagraphBidi::new(
            &request.paragraphs[0].text,
            ShapeOptions {
                direction: Some(TextDirection::RightToLeft),
            },
        )
        .unwrap();
        let preview = layout(&request, &request.fonts, &[0], vec![]).unwrap();
        let lines = preview
            .frames
            .iter()
            .flat_map(|f| &f.lines)
            .collect::<Vec<_>>();
        assert!(lines.len() > 1);
        let mut offset = 0;
        for line in lines {
            let expected = prepared.line(offset..offset + line.text.len()).unwrap();
            assert_eq!(line.bidi.as_ref(), Some(&expected));
            let serialized = serde_json::to_value(line).unwrap();
            let restored: StoryPaintLine = serde_json::from_value(serialized).unwrap();
            assert_eq!(restored.bidi, line.bidi);
            offset += line.text.len();
        }
        assert_eq!(offset, request.paragraphs[0].text.len());
    }
}

#[test]
fn native_story_checkpoint_reopen_and_next_edit_recompute_joining_context() {
    let input = two_page_input();
    let mut request = story("ب".repeat(24));
    request.input_sha256 = hash(&input);
    for frame in &mut request.frames {
        frame.rect = [10.0, 10.0, 38.0, 70.0];
    }
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input).unwrap();
    let preview = session.preview(&request, &cancel).unwrap();
    assert!(preview.frames.iter().flat_map(|f| &f.lines).any(|l| l
        .bidi
        .as_ref()
        .is_some_and(|b| !b.context.before.is_empty())));
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&request, &receipt, &cancel)
        .unwrap();
    let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    let mut saved = reopened.saved_stories().unwrap().remove(0).request;
    assert_eq!(saved.paragraphs[0].text, request.paragraphs[0].text);
    saved.paragraphs[0].text.push('ب');
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

#[test]
fn wrapped_table_cells_measure_and_emit_with_their_original_paragraph_context() {
    let input = tables::tests::fixture(false);
    let mut request = tables::tests::request(&input, false);
    request.fonts = vec![arabic_font()];
    request.allow_font_substitution = false;
    for paragraph in &mut request.paragraphs {
        paragraph.preferred_font = "Approved Arabic".into();
    }
    request.paragraphs[2].text = "ب".repeat(120);
    request.paragraphs[2].rtl = true;
    let id = request.paragraphs[2].id.clone();
    let mut session = LinkedStorySession::open(input).unwrap();
    let preview = session
        .preview(&request, &crate::CancelToken::none())
        .unwrap();
    let lines = preview
        .frames
        .iter()
        .flat_map(|f| f.lines.iter().zip(&f.paragraph_ids))
        .filter(|(_, p)| *p == &id)
        .map(|(line, _)| line)
        .collect::<Vec<_>>();
    assert!(lines.len() > 1);
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<String>(),
        request.paragraphs[2].text
    );
    assert!(lines[1]
        .bidi
        .as_ref()
        .is_some_and(|b| !b.context.before.is_empty()));
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&request, &receipt, &crate::CancelToken::none())
        .unwrap();
    assert_eq!(
        session.saved_stories().unwrap()[0].request.paragraphs[2].text,
        request.paragraphs[2].text
    );
}

#[test]
fn marked_grapheme_wrap_retains_joining_synopsis_after_checkpoint_and_reopen() {
    use crate::fonts::{line_layout::measure_run, shaper::ParagraphBidi, TextShaper};
    let input = two_page_input();
    let mut request = story(format!("ب{}ب", "\u{064e}".repeat(8)));
    request.input_sha256 = hash(&input);
    request.paragraphs[0].font_size = 6.0;
    request.paragraphs[0].line_height = 8.0;
    let text = &request.paragraphs[0].text;
    let prepared = ParagraphBidi::new(
        text,
        ShapeOptions {
            direction: Some(TextDirection::RightToLeft),
        },
    )
    .unwrap();
    let split = text.len() - 2;
    let mut width = 0.0f64;
    for range in [0..split, split..text.len()] {
        let bidi = prepared.line(range.clone()).unwrap();
        let run = TextShaper::shape_resolved(
            &request.fonts[0].bytes,
            &text[range],
            &bidi,
            &Default::default(),
        )
        .unwrap();
        width = width.max(
            measure_run(&request.fonts[0].bytes, &run, 6.0)
                .unwrap()
                .width(),
        );
    }
    for frame in &mut request.frames {
        frame.rect = [10.0, 10.0, 10.0 + width + 0.1, 190.0];
    }
    let cancel = crate::CancelToken::none();
    let mut session = LinkedStorySession::open(input).unwrap();
    let preview = session.preview(&request, &cancel).unwrap();
    let lines = preview
        .frames
        .iter()
        .flat_map(|f| &f.lines)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, request.paragraphs[0].text[..split]);
    assert_eq!(lines[1].text, "ب");
    assert_eq!(
        lines[1].bidi.as_ref().unwrap().context.joining_before,
        Some('ب')
    );
    let expected = lines[1].bidi.clone();
    let receipt = session.preview_receipt().unwrap();
    session
        .checkpoint_approved(&request, &receipt, &cancel)
        .unwrap();
    let mut reopened = LinkedStorySession::open(session.bytes().to_vec()).unwrap();
    let saved = reopened.saved_stories().unwrap().remove(0).request;
    let next = reopened.preview(&saved, &cancel).unwrap();
    let lines = next
        .frames
        .iter()
        .flat_map(|f| &f.lines)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].bidi, expected);
    let receipt = reopened.preview_receipt().unwrap();
    reopened
        .checkpoint_approved(&saved, &receipt, &cancel)
        .unwrap();
    assert_eq!(
        reopened.saved_stories().unwrap()[0].request.paragraphs[0].text,
        saved.paragraphs[0].text
    );
}
