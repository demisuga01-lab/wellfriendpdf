//! Source-only front-matter regressions. No build, test, PDF or rendering
//! workload was executed when these cases were added.
use super::*;

fn body_section() -> FlowSection {
    FlowSection::new(
        PageSize::custom(260.0, 160.0),
        Margins {
            left: 20.0,
            right: 36.0,
            top: 12.0,
            bottom: 12.0,
        },
    )
    .mirrored_margins(true)
}

fn front_section(height: f64) -> FlowSection {
    FlowSection::new(PageSize::custom(240.0, height), Margins::all(10.0))
        .page_numbering(1, PageNumberStyle::LowerRoman)
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
fn one_page_toc_adds_suppressed_parity_page_and_shifts_body_authorities() {
    let mut flow = FlowDocument::from_section(body_section()).unwrap();
    flow.add_paragraph(
        "Body page one",
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_anchor("first").unwrap();
    flow.add_page_break();
    flow.add_paragraph(
        "Body page two",
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_anchor("second").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Second", "second")])
        .unwrap();
    let body_margins = flow
        .builder
        .pages
        .iter()
        .map(|page| page.margins)
        .collect::<Vec<_>>();

    let report = flow
        .prepend_table_of_contents(front_section(120.0), &TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.content_pages, 1);
    assert_eq!(report.inserted_pages, 2);
    assert_eq!(report.parity_blank_page, Some(2));
    assert_eq!(report.body_start_page, 3);
    assert!(flow.builder.pages[1].suppress_section_master);
    assert_eq!(flow.builder.anchors["first"].page_index, 2);
    assert_eq!(flow.builder.anchors["second"].page_index, 3);
    assert_eq!(flow.builder.anchors["second"].section_index, 1);
    assert_eq!(flow.current_page, 3);
    assert_eq!(flow.current_section, 1);
    assert_eq!(flow.builder.pages[2].margins, body_margins[0]);
    assert_eq!(flow.builder.pages[3].margins, body_margins[1]);
    assert_eq!(flow.builder.pages[0].section_index, Some(0));
    assert_eq!(flow.builder.pages[2].section_index, Some(1));

    let resolved = fields::materialize(flow.builder()).unwrap();
    assert!(logical(&resolved.pages[0]).contains("Second"));
    assert!(logical(&resolved.pages[0]).contains('4'));
    assert_eq!(resolved.pages[0].links.len(), 1);
    assert!(logical(&resolved.pages[2]).contains("Body page one"));
}

#[test]
fn even_multi_page_toc_needs_no_parity_blank() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 120.0), Margins::all(10.0));
    let mut outline = Vec::new();
    for index in 0..8 {
        let anchor = format!("entry-{index}");
        flow.add_anchor(anchor.clone()).unwrap();
        outline.push(PdfOutlineEntry::new(format!("Entry {index}"), anchor));
    }
    flow.set_outline(outline).unwrap();
    let report = flow
        .prepend_table_of_contents(front_section(60.0), &TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.content_pages % 2, 0);
    assert_eq!(report.inserted_pages, report.content_pages);
    assert_eq!(report.parity_blank_page, None);
    assert_eq!(report.table_of_contents.rows.len(), 8);
}

#[test]
fn insertion_shifts_retained_footnote_reference_pages() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 160.0), Margins::all(10.0));
    let text = "Claim\u{00b9}";
    let marker = text.find('\u{00b9}').unwrap();
    flow.add_paragraph_with_footnotes(
        text,
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
        &[FlowFootnote::new(
            marker..marker + '\u{00b9}'.len_utf8(),
            "Source note",
            TextStyle::unicode(8.0),
        )],
    )
    .unwrap();
    flow.add_anchor("claim").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Claim", "claim")])
        .unwrap();
    let report = flow
        .prepend_table_of_contents(front_section(120.0), &TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.inserted_pages, 2);
    assert_eq!(notes::reference_pages(&flow.builder.pages[2]), vec![3]);
    assert!(notes::materialize(flow.builder()).is_ok());
}

#[test]
fn failed_staging_leaves_body_pages_sections_anchors_cursor_and_ids_unchanged() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 120.0), Margins::all(10.0));
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    let pages = flow.builder.pages.clone();
    let sections = flow.builder.sections.clone();
    let anchor_page = flow.builder.anchors["target"].page_index;
    let cursor = flow.cursor_y;
    let current_page = flow.current_page;
    let current_section = flow.current_section;
    let next_field = flow.builder.next_field_plan_id;
    let mut style = TableOfContentsStyle::new();
    style.page_column_width = 10_000.0;
    assert!(flow
        .prepend_table_of_contents(front_section(120.0), &style)
        .is_err());
    assert_eq!(flow.builder.pages.len(), pages.len());
    assert_eq!(flow.builder.sections, sections);
    assert_eq!(flow.builder.anchors["target"].page_index, anchor_page);
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.current_page, current_page);
    assert_eq!(flow.current_section, current_section);
    assert_eq!(flow.builder.next_field_plan_id, next_field);
}

#[test]
fn arbitrary_multi_section_blocks_merge_front_anchors_and_resources() {
    let mut flow = FlowDocument::from_section(body_section()).unwrap();
    flow.add_anchor("body").unwrap();
    let image_count = flow.builder.images.len();
    let report = flow
        .prepend_front_matter(front_section(120.0), |front| {
            front.add_anchor("front-title")?;
            front.add_paragraph(
                "Title page",
                &TextStyle::unicode(18.0),
                &ParagraphStyle::new().align(TextAlign::Center),
            )?;
            let image = front.builder_mut().add_rgb_image(1, 1, vec![20, 40, 60])?;
            front.add_image(image, 12.0, 12.0)?;
            front.start_section(
                FlowSection::new(PageSize::custom(240.0, 120.0), Margins::all(10.0))
                    .page_numbering(1, PageNumberStyle::LowerRoman),
            )?;
            front.add_anchor("front-second")?;
            front.add_paragraph(
                "Copyright",
                &TextStyle::unicode(10.0),
                &ParagraphStyle::new(),
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(report.content_pages, 2);
    assert_eq!(report.inserted_pages, 2);
    assert_eq!(report.inserted_sections, 2);
    assert_eq!(report.parity_blank_page, None);
    assert_eq!(report.body_start_page, 3);
    assert_eq!(report.anchor_names, vec!["front-second", "front-title"]);
    assert_eq!(flow.builder.images.len(), image_count + 1);
    assert_eq!(flow.builder.anchors["front-title"].page_index, 0);
    assert_eq!(flow.builder.anchors["front-second"].section_index, 1);
    assert_eq!(flow.builder.anchors["body"].page_index, 2);
    assert_eq!(flow.builder.anchors["body"].section_index, 2);
    assert_eq!(flow.current_page, 2);
    assert_eq!(flow.current_section, 2);
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::Image { .. })));
}

#[test]
fn arbitrary_front_matter_callback_error_is_fully_isolated() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 120.0), Margins::all(10.0));
    flow.add_anchor("body").unwrap();
    let pages = flow.builder.pages.len();
    let images = flow.builder.images.len();
    let sections = flow.builder.sections.len();
    let result = flow.prepend_front_matter(front_section(120.0), |front| {
        front.add_anchor("temporary")?;
        front.builder_mut().add_rgb_image(1, 1, vec![0, 0, 0])?;
        Err(WellfriendError::invalid_input("intentional staged failure"))
    });
    assert!(result.is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.images.len(), images);
    assert_eq!(flow.builder.sections.len(), sections);
    assert!(!flow.builder.anchors.contains_key("temporary"));
    assert_eq!(flow.builder.anchors["body"].page_index, 0);
}

#[test]
fn conflicting_front_anchor_rejects_before_body_mutation() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 120.0), Margins::all(10.0));
    flow.add_anchor("shared").unwrap();
    let cursor = flow.cursor_y;
    let result = flow.prepend_front_matter(front_section(120.0), |front| {
        front.add_anchor("shared")?;
        front.add_paragraph(
            "conflict",
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
        )?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert_eq!(flow.builder.anchors["shared"].page_index, 0);
    assert_eq!(flow.cursor_y, cursor);
}

#[test]
fn document_scoped_front_footnotes_refuse_body_renumbering() {
    let mut flow = FlowDocument::new(PageSize::custom(240.0, 140.0), Margins::all(10.0));
    let document_notes =
        FlowSection::new(PageSize::custom(240.0, 140.0), Margins::all(10.0)).footnote_numbering(
            FootnoteNumbering::new(NoteNumberScope::Document, NoteNumberStyle::Decimal),
        );
    let result = flow.prepend_front_matter(document_notes, |front| {
        let text = "Front\u{00b9}";
        let marker = text.find('\u{00b9}').unwrap();
        front.add_paragraph_with_footnotes(
            text,
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
            &[FlowFootnote::new(
                marker..marker + '\u{00b9}'.len_utf8(),
                "front note",
                TextStyle::unicode(8.0),
            )],
        )?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert!(flow.builder.pages[0].footnotes.is_empty());
}
