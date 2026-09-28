//! Unexecuted source regressions for physical-page story boundaries.
use super::tests::request;
use super::*;

fn line_pages(preview: &LinkedStoryPreview) -> Vec<(usize, String)> {
    preview
        .frames
        .iter()
        .flat_map(|frame| {
            frame
                .lines
                .iter()
                .map(|line| (frame.frame.page, line.text.clone()))
        })
        .collect()
}

#[test]
fn form_feed_skips_remaining_columns_on_the_same_physical_page() {
    let mut story = request("A\u{000c}B".into());
    story.paragraphs[0].orphans = 1;
    story.paragraphs[0].widows = 1;
    let mut second_column = story.frames[0].clone();
    second_column.id = "same-page-second-column".into();
    second_column.rect = [210.0, 10.0, 390.0, 70.0];
    story.frames.insert(1, second_column);
    let preview = layout(&story, &story.fonts, &[0], Vec::new()).unwrap();
    assert_eq!(
        line_pages(&preview),
        vec![(1, "A\u{000c}".into()), (2, "B".into())]
    );
    assert!(preview.frames[1].lines.is_empty());
    assert_eq!(
        preview.page_breaks,
        vec![StoryPageBreakReceipt {
            paragraph_id: "p1".into(),
            byte_offset: 1,
            source: StoryPageBreakSource::FormFeed,
            policy: StoryPageBreakBefore::NextPage,
            from_page: 1,
            to_page: 2,
        }]
    );
}

#[test]
fn explicit_page_policy_uses_approved_even_page_and_creates_odd_page() {
    let mut even = request("Even".into());
    even.paragraphs[0].page_break_before = StoryPageBreakBefore::NextEvenPage;
    let preview = layout(&even, &even.fonts, &[0], Vec::new()).unwrap();
    assert_eq!(line_pages(&preview), vec![(2, "Even".into())]);
    assert_eq!(preview.generated_pages, 0);

    let mut odd = request("Odd".into());
    odd.paragraphs[0].page_break_before = StoryPageBreakBefore::NextOddPage;
    let preview = layout(&odd, &odd.fonts, &[0], Vec::new()).unwrap();
    assert_eq!(line_pages(&preview), vec![(3, "Odd".into())]);
    assert_eq!(preview.generated_pages, 1);
    assert_eq!(preview.page_breaks[0].to_page, 3);
}

#[test]
fn trailing_and_repeated_form_feeds_materialize_bounded_blank_pages() {
    let mut trailing = request("A\u{000c}".into());
    trailing.paragraphs[0].orphans = 1;
    trailing.paragraphs[0].widows = 1;
    let preview = layout(&trailing, &trailing.fonts, &[0], Vec::new()).unwrap();
    assert_eq!(line_pages(&preview), vec![(1, "A\u{000c}".into())]);
    assert_eq!(preview.page_breaks[0].to_page, 2);
    assert!(preview.frames.iter().any(|frame| frame.frame.page == 2));

    let mut repeated = request("A\u{000c}B\u{000c}C".into());
    repeated.paragraphs[0].orphans = 1;
    repeated.paragraphs[0].widows = 1;
    repeated.max_new_pages = 0;
    assert!(layout(&repeated, &repeated.fonts, &[0], Vec::new()).is_err());
    repeated.max_new_pages = 1;
    let preview = layout(&repeated, &repeated.fonts, &[0], Vec::new()).unwrap();
    assert_eq!(
        line_pages(&preview),
        vec![
            (1, "A\u{000c}".into()),
            (2, "B\u{000c}".into()),
            (3, "C".into()),
        ]
    );
    assert_eq!(preview.page_breaks.len(), 2);
}

#[test]
fn physical_page_rules_are_versioned_and_conflicts_fail_closed() {
    let mut story = request("A\u{000c}B".into());
    assert_eq!(required_story_schema(&story), 4);
    story.paragraphs[0].keep_together = true;
    assert!(validate_paragraphs(&story).is_err());
    story.paragraphs[0].keep_together = false;
    story.mode = StoryMode::PreserveLayout;
    story.frames.truncate(1);
    assert!(validate_paragraphs(&story).is_err());

    let mut table = crate::linked_stories::tables::tests::request(
        &crate::linked_stories::tables::tests::fixture(false),
        false,
    );
    table.paragraphs[0].text.push('\u{000c}');
    assert!(crate::linked_stories::tables::validate_topology(&table).is_err());
}

#[test]
fn vertical_stories_use_the_same_physical_page_boundaries() {
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let mut story = request("A\u{000c}B".into());
        story.writing_mode = mode;
        story.paragraphs[0].orphans = 1;
        story.paragraphs[0].widows = 1;
        for frame in &mut story.frames {
            frame.rect = [10.0, 10.0, 190.0, 190.0];
        }
        let preview = layout(&story, &story.fonts, &[0], Vec::new()).unwrap();
        assert_eq!(
            line_pages(&preview),
            vec![(1, "A\u{000c}".into()), (2, "B".into())]
        );
        assert_eq!(preview.page_breaks.len(), 1);
    }
}
