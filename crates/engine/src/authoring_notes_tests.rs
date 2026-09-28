//! Source-only note regressions. No build, test or PDF workload was executed.
use super::*;

fn flow(height: f64) -> FlowDocument {
    FlowDocument::new(PageSize::custom(220.0, height), Margins::all(10.0))
}

fn marker(text: &str, value: char) -> Range<usize> {
    let start = text.find(value).unwrap();
    start..start + value.len_utf8()
}

fn logical(page: &PdfPageBuilder) -> String {
    page.commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::TextGroup { logical_text, .. } => Some(logical_text.as_str()),
            PageCommand::LogicalBreak { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn note_reservation_is_private_and_following_flow_respects_it() {
    let mut flow = flow(100.0);
    let text = "Body¹";
    let note = FlowFootnote::new(
        marker(text, '¹'),
        "A page-owned source note.",
        TextStyle::unicode(8.0),
    );
    let report = flow
        .add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[note],
        )
        .unwrap();
    assert_eq!(report.body_line_pages, vec![1]);
    assert_eq!(report.fragments.len(), 1);
    assert_eq!(logical(&flow.builder.pages[0]), text);
    assert!(flow.builder.pages[0].footnote_reserved_height > 0.0);
    assert!(flow.cursor_y >= flow.current_bottom());

    while flow.current_page == 0 {
        flow.add_paragraph(
            "following content",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
        )
        .unwrap();
    }
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .all(|command| match command {
            PageCommand::Text { text, .. } => !text.contains("page-owned source note"),
            _ => true,
        }));
    let painted = materialize(flow.builder()).unwrap();
    assert!(logical(&painted.pages[0]).contains("¹ A page-owned source note."));
    assert!(painted.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::Path { .. })));
}

#[test]
fn long_note_fragments_contiguously_and_retains_one_reference_page() {
    let mut flow = flow(80.0);
    let text = "Claim¹";
    let body = (0..30)
        .map(|index| format!("line {index:02}\n"))
        .collect::<String>();
    let display = format!("¹ {body}");
    let report = flow
        .add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[FlowFootnote::new(
                marker(text, '¹'),
                body,
                TextStyle::unicode(8.0),
            )],
        )
        .unwrap();
    assert!(report.added_pages > 0);
    assert!(report.fragments.len() > 1);
    let reference_page = report.fragments[0].reference_page;
    let mut end = 0usize;
    let mut reconstructed = String::new();
    for (index, fragment) in report.fragments.iter().enumerate() {
        assert_eq!(fragment.note, 0);
        assert_eq!(fragment.reference_page, reference_page);
        assert_eq!(fragment.display_utf8_range[0], end);
        reconstructed.push_str(
            display
                .get(fragment.display_utf8_range[0]..fragment.display_utf8_range[1])
                .unwrap(),
        );
        end = fragment.display_utf8_range[1];
        assert_eq!(fragment.continued_from_previous, index > 0);
        assert_eq!(fragment.continues, index + 1 < report.fragments.len());
    }
    assert_eq!(end, display.len());
    assert_eq!(reconstructed, display);
    let painted = materialize(flow.builder()).unwrap();
    assert_eq!(
        painted.pages.iter().map(logical).collect::<String>(),
        format!("{text}{display}")
    );
}

#[test]
fn multiple_paragraphs_use_distinct_retained_note_identities() {
    let mut flow = flow(100.0);
    for (text, body) in [("One¹", "first"), ("Two²", "second")] {
        let symbol = text.chars().last().unwrap();
        flow.add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[FlowFootnote::new(
                marker(text, symbol),
                body,
                TextStyle::unicode(8.0),
            )],
        )
        .unwrap();
    }
    assert_eq!(flow.builder.next_footnote_id, 2);
    let painted = materialize(flow.builder()).unwrap();
    let all = painted.pages.iter().map(logical).collect::<String>();
    assert!(all.contains("¹ first"));
    assert!(all.contains("² second"));
}

#[test]
fn invalid_cross_line_or_overlapping_references_roll_back_every_note_state() {
    let mut flow = flow(100.0);
    let text = "¹ first\n² second";
    let pages = flow.builder.pages.len();
    let commands = flow.builder.pages[0].commands.len();
    let cursor = flow.cursor_y;
    let identity = flow.builder.next_footnote_id;
    let cross = vec![FlowFootnote::new(
        0..text.find('²').unwrap() + '²'.len_utf8(),
        "bad",
        TextStyle::unicode(8.0),
    )];
    assert!(flow
        .add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &cross,
        )
        .is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert!(flow.builder.pages[0].footnotes.is_empty());
    assert_eq!(flow.builder.pages[0].footnote_reserved_height, 0.0);
    assert_eq!(flow.builder.next_footnote_id, identity);
    assert_eq!(flow.cursor_y, cursor);

    let first = marker(text, '¹');
    let overlap = vec![
        FlowFootnote::new(first.clone(), "one", TextStyle::unicode(8.0)),
        FlowFootnote::new(first, "two", TextStyle::unicode(8.0)),
    ];
    assert!(flow
        .add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &overlap,
        )
        .is_err());
    assert!(flow.builder.pages[0].footnotes.is_empty());
    assert_eq!(flow.builder.next_footnote_id, identity);
}

#[test]
fn table_after_note_uses_the_reserved_bottom_instead_of_overpainting() {
    let mut flow = flow(100.0);
    let text = "Body¹";
    flow.add_paragraph_with_footnotes(
        text,
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
        &[FlowFootnote::new(
            marker(text, '¹'),
            "note line one\nnote line two",
            TextStyle::unicode(8.0),
        )],
    )
    .unwrap();
    let reserved = flow.current_bottom();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row(["table body"]);
    let report = flow.add_table_with_report(&table).unwrap();
    for fragment in &report.fragments {
        if fragment.page == 1 {
            assert!(fragment.top - fragment.height >= reserved - EPS);
        }
    }
}

#[test]
fn note_too_tall_for_any_fresh_page_fails_without_allocating_forever() {
    let mut flow = flow(40.0);
    let text = "Body¹";
    let before = flow.builder.pages.len();
    let result = flow.add_paragraph_with_footnotes(
        text,
        &TextStyle::unicode(18.0),
        &ParagraphStyle::new(),
        &[FlowFootnote::new(
            marker(text, '¹'),
            "note",
            TextStyle::unicode(18.0),
        )],
    );
    assert!(result.is_err());
    assert_eq!(flow.builder.pages.len(), before);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert!(flow.builder.pages[0].footnotes.is_empty());
}

#[test]
fn note_materialization_is_idempotent_and_does_not_mutate_source_builder() {
    let mut flow = flow(100.0);
    let text = "Body¹";
    flow.add_paragraph_with_footnotes(
        text,
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
        &[FlowFootnote::new(
            marker(text, '¹'),
            "note",
            TextStyle::unicode(8.0),
        )],
    )
    .unwrap();
    let source_commands = flow.builder.pages[0].commands.len();
    let once = materialize(flow.builder()).unwrap();
    let twice = materialize(&once).unwrap();
    assert_eq!(logical(&once.pages[0]), logical(&twice.pages[0]));
    assert_eq!(once.pages[0].commands.len(), twice.pages[0].commands.len());
    assert_eq!(flow.builder.pages[0].commands.len(), source_commands);
}

#[test]
fn endnotes_start_on_requested_parity_and_report_their_page_ranges() {
    let mut flow = flow(80.0);
    flow.add_paragraph("Body", &TextStyle::unicode(10.0), &ParagraphStyle::new())
        .unwrap();
    let notes = vec![
        FlowEndnote::new("1.", "first endnote", TextStyle::unicode(8.0)),
        FlowEndnote::new(
            "2.",
            "second endnote\nwith another line",
            TextStyle::unicode(8.0),
        ),
    ];
    let report = flow
        .add_endnotes(&notes, FlowPageBreak::NextOddPage)
        .unwrap();
    assert_eq!(report.added_pages, 2);
    assert_eq!(flow.builder.pages.len(), 3);
    assert!(flow.builder.pages[1].suppress_section_master);
    assert_eq!(report.items.len(), 2);
    assert!(report.items.iter().all(|item| item.first_page >= 3));
    assert!(logical(&flow.builder.pages[2]).contains("1. first endnote"));
    assert!(logical(&flow.builder.pages[2]).contains("2. second endnote"));
}

#[test]
fn empty_endnote_collection_is_a_true_noop() {
    let mut flow = flow(80.0);
    let pages = flow.builder.pages.len();
    let cursor = flow.cursor_y;
    let report = flow.add_endnotes(&[], FlowPageBreak::NextEvenPage).unwrap();
    assert!(report.items.is_empty());
    assert_eq!(report.added_pages, 0);
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.cursor_y, cursor);
}

#[test]
fn invalid_endnote_collection_validates_before_page_allocation() {
    let mut flow = flow(80.0);
    let pages = flow.builder.pages.len();
    let invalid = vec![
        FlowEndnote::new("1.", "valid", TextStyle::unicode(8.0)),
        FlowEndnote::new("bad\nlabel", "invalid", TextStyle::unicode(8.0)),
    ];
    assert!(flow
        .add_endnotes(&invalid, FlowPageBreak::NextEvenPage)
        .is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn long_endnote_flows_through_multiple_pages_without_losing_text() {
    let mut flow = flow(60.0);
    let body = (0..30)
        .map(|index| format!("endnote line {index:02}\n"))
        .collect::<String>();
    let display = format!("1. {body}");
    let report = flow
        .add_endnotes(
            &[FlowEndnote::new("1.", body, TextStyle::unicode(8.0))],
            FlowPageBreak::NextPage,
        )
        .unwrap();
    assert!(report.items[0].last_page > report.items[0].first_page);
    assert_eq!(
        flow.builder.pages.iter().map(logical).collect::<String>(),
        display
    );
}

#[test]
fn automatic_markers_preserve_original_offsets_and_feed_exact_reference_ranges() {
    let mut flow = flow(120.0);
    let source = "Alpha and beta.";
    let notes = vec![
        NumberedFootnote::new(5, "first", TextStyle::unicode(8.0)),
        NumberedFootnote::new(9, "second", TextStyle::unicode(8.0)),
    ];
    let report = flow
        .add_numbered_footnoted_paragraph(
            source,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &notes,
        )
        .unwrap();
    assert_eq!(report.enriched_text, "Alpha[1] and[2] beta.");
    assert_eq!(report.markers.len(), 2);
    assert_eq!(report.markers[0].original_utf8_offset, 5);
    assert_eq!(report.markers[0].enriched_utf8_range, [5, 8]);
    assert_eq!(report.markers[1].original_utf8_offset, 9);
    assert_eq!(report.markers[1].enriched_utf8_range, [12, 15]);
    assert_eq!(
        report.layout.fragments[0].reference_utf8_range,
        report.markers[0].enriched_utf8_range
    );
    assert_eq!(
        report
            .layout
            .fragments
            .iter()
            .find(|fragment| fragment.note == 1)
            .unwrap()
            .reference_utf8_range,
        report.markers[1].enriched_utf8_range
    );
}

#[test]
fn automatic_markers_at_the_same_source_boundary_remain_ordered_and_disjoint() {
    let mut flow = flow(120.0);
    let report = flow
        .add_numbered_footnoted_paragraph(
            "AB",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[
                NumberedFootnote::new(1, "first", TextStyle::unicode(8.0)),
                NumberedFootnote::new(1, "second", TextStyle::unicode(8.0)),
            ],
        )
        .unwrap();
    assert_eq!(report.enriched_text, "A[1][2]B");
    assert_eq!(report.markers[0].enriched_utf8_range, [1, 4]);
    assert_eq!(report.markers[1].enriched_utf8_range, [4, 7]);
}

#[test]
fn section_numbering_restarts_while_document_numbering_continues() {
    let first = FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
        .footnote_numbering(FootnoteNumbering::default().start(4));
    let mut flow = FlowDocument::from_section(first).unwrap();
    let a = flow
        .add_numbered_footnoted_paragraph(
            "A",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "a", TextStyle::unicode(8.0))],
        )
        .unwrap();
    assert_eq!(a.markers[0].number, 4);
    flow.start_section(
        FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
            .footnote_numbering(FootnoteNumbering::default().start(9)),
    )
    .unwrap();
    let b = flow
        .add_numbered_footnoted_paragraph(
            "B",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "b", TextStyle::unicode(8.0))],
        )
        .unwrap();
    assert_eq!(b.markers[0].number, 9);

    let document = FootnoteNumbering::new(NoteNumberScope::Document, NoteNumberStyle::LowerRoman)
        .start(3)
        .affixes("(", ")");
    let mut continuous = FlowDocument::from_section(
        FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
            .footnote_numbering(document.clone()),
    )
    .unwrap();
    let first = continuous
        .add_numbered_footnoted_paragraph(
            "A",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "a", TextStyle::unicode(8.0))],
        )
        .unwrap();
    continuous
        .start_section(
            FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
                .footnote_numbering(document),
        )
        .unwrap();
    let second = continuous
        .add_numbered_footnoted_paragraph(
            "B",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "b", TextStyle::unicode(8.0))],
        )
        .unwrap();
    assert_eq!(
        (first.markers[0].number, first.markers[0].label.as_str()),
        (3, "(iii)")
    );
    assert_eq!(
        (second.markers[0].number, second.markers[0].label.as_str()),
        (4, "(iv)")
    );
}

#[test]
fn failed_automatic_paragraph_does_not_consume_a_number() {
    let mut flow = flow(120.0);
    assert!(flow
        .add_numbered_footnoted_paragraph(
            "🚀",
            &TextStyle::standard(StandardFont::Helvetica, 10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(0, "bad", TextStyle::unicode(8.0))],
        )
        .is_err());
    assert!(!flow.section_footnote_started);
    let report = flow
        .add_numbered_footnoted_paragraph(
            "A",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "good", TextStyle::unicode(8.0))],
        )
        .unwrap();
    assert_eq!(report.markers[0].number, 1);
}

#[test]
fn invalid_number_style_or_offset_fails_before_publishing_markers() {
    let section = FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
        .footnote_numbering(
            FootnoteNumbering::new(NoteNumberScope::Section, NoteNumberStyle::UpperRoman)
                .start(4000),
        );
    let mut flow = FlowDocument::from_section(section).unwrap();
    assert!(flow
        .add_numbered_footnoted_paragraph(
            "A",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(1, "note", TextStyle::unicode(8.0))],
        )
        .is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
    assert!(flow.builder.pages[0].footnotes.is_empty());

    assert!(flow
        .add_numbered_footnoted_paragraph(
            "é",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[NumberedFootnote::new(
                1,
                "inside UTF-8",
                TextStyle::unicode(8.0)
            )],
        )
        .is_err());
}
