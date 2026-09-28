//! Source-only authored-index regressions. No build, test, PDF or rendering
//! workload was executed when these cases were added.
use super::*;

const EPS: f64 = 1e-7;

fn flow() -> FlowDocument {
    FlowDocument::new(PageSize::custom(300.0, 180.0), Margins::all(10.0))
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

fn text_x(command: &PageCommand, needle: &str) -> Option<f64> {
    match command {
        PageCommand::Text { text, x, .. } if text.contains(needle) => Some(*x),
        PageCommand::TextGroup { runs, .. } => runs.iter().find_map(|run| text_x(run, needle)),
        _ => None,
    }
}

fn page_text_x(page: &PdfPageBuilder, needle: &str) -> Option<f64> {
    page.commands
        .iter()
        .find_map(|command| text_x(command, needle))
}

#[test]
fn unicode_sort_and_same_page_dedup_produce_clickable_deferred_values() {
    let mut flow = flow();
    flow.add_anchor("z-top").unwrap();
    flow.add_spacer(8.0);
    flow.add_anchor("z-lower").unwrap();
    flow.add_page_break();
    flow.add_anchor("alpha").unwrap();
    let entries = vec![
        PdfIndexEntry::new("Zulu").anchors(["z-lower", "z-top"]),
        PdfIndexEntry::new("alpha").anchor("alpha"),
    ];
    let report = flow
        .add_document_index(&entries, &DocumentIndexStyle::new())
        .unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.rows[0].term, "alpha");
    assert_eq!(report.rows[0].source_pages, vec![2]);
    assert_eq!(report.rows[1].term, "Zulu");
    assert_eq!(report.rows[1].source_pages, vec![1]);
    assert_eq!(report.rows[1].anchors, vec!["z-top"]);

    let resolved = fields::materialize(flow.builder()).unwrap();
    assert_eq!(
        resolved
            .pages
            .iter()
            .map(|page| page.links.len())
            .sum::<usize>(),
        2
    );
    let text = resolved.pages.iter().map(logical).collect::<String>();
    assert!(text.contains("alpha"));
    assert!(text.contains("Zulu"));
}

#[test]
fn explicit_sort_keys_override_display_term_order() {
    let mut flow = flow();
    flow.add_anchor("one").unwrap();
    flow.add_anchor("two").unwrap();
    let entries = vec![
        PdfIndexEntry::new("Displayed first alphabetically")
            .sort_key("z")
            .anchor("one"),
        PdfIndexEntry::new("Displayed second alphabetically")
            .sort_key("a")
            .anchor("two"),
    ];
    let mut style = DocumentIndexStyle::new();
    style.sort = DocumentIndexSort::ExplicitKeys;
    let report = flow.add_document_index(&entries, &style).unwrap();
    assert_eq!(report.rows[0].term, "Displayed second alphabetically");
    assert_eq!(report.rows[1].term, "Displayed first alphabetically");
}

#[test]
fn hierarchy_indents_children_and_supports_textual_cross_references() {
    let mut flow = flow();
    flow.add_anchor("child").unwrap();
    let entries = vec![PdfIndexEntry::new("Parent")
        .see_also("Related")
        .children(vec![PdfIndexEntry::new("Child").anchor("child")])];
    let report = flow
        .add_document_index(&entries, &DocumentIndexStyle::new())
        .unwrap();
    assert_eq!(
        report.rows.iter().map(|row| row.level).collect::<Vec<_>>(),
        vec![0, 1]
    );
    let resolved = fields::materialize(flow.builder()).unwrap();
    let parent_x = page_text_x(&resolved.pages[0], "Parent").unwrap();
    let child_x = page_text_x(&resolved.pages[0], "Child").unwrap();
    assert!((child_x - parent_x - 14.0).abs() < EPS);
    assert!(logical(&resolved.pages[0]).contains("see also Related"));
}

#[test]
fn unresolved_occurrence_rejects_without_page_or_field_mutation() {
    let mut flow = flow();
    let pages = flow.builder.pages.len();
    let commands = flow.builder.pages[0].commands.len();
    let cursor = flow.cursor_y;
    let identity = flow.builder.next_field_plan_id;
    let entries = vec![PdfIndexEntry::new("Missing").anchor("missing")];
    assert!(flow
        .add_document_index(&entries, &DocumentIndexStyle::new())
        .is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.builder.next_field_plan_id, identity);
}

#[test]
fn consecutive_occurrences_compress_to_a_range_with_clickable_endpoints() {
    let mut flow = flow();
    let mut anchors = Vec::new();
    for page in 1..=4 {
        let anchor = format!("range-{page}");
        flow.add_anchor(anchor.clone()).unwrap();
        anchors.push(anchor);
        if page < 4 {
            flow.add_page_break();
        }
    }
    let entries = vec![PdfIndexEntry::new("Range").anchors(anchors)];
    let report = flow
        .add_document_index(&entries, &DocumentIndexStyle::new())
        .unwrap();
    assert_eq!(report.rows[0].source_pages, vec![1, 2, 3, 4]);
    let resolved = fields::materialize(flow.builder()).unwrap();
    assert_eq!(
        resolved
            .pages
            .iter()
            .map(|page| page.links.len())
            .sum::<usize>(),
        2
    );
    let text = resolved.pages.iter().map(logical).collect::<String>();
    assert!(text.contains("1\u{2013}4"));
}

#[test]
fn section_number_ranges_do_not_cross_a_numbering_reset() {
    let first = FlowSection::new(PageSize::custom(300.0, 180.0), Margins::all(10.0))
        .page_numbering(1, PageNumberStyle::Decimal);
    let mut flow = FlowDocument::from_section(first).unwrap();
    flow.add_anchor("first-section").unwrap();
    flow.start_section(
        FlowSection::new(PageSize::custom(300.0, 180.0), Margins::all(10.0))
            .page_numbering(1, PageNumberStyle::Decimal),
    )
    .unwrap();
    flow.add_anchor("second-section").unwrap();
    let entries = vec![PdfIndexEntry::new("Reset").anchors(["first-section", "second-section"])];
    let mut style = DocumentIndexStyle::new();
    style.section_page_numbers = true;
    style.minimum_range_pages = 2;
    flow.add_document_index(&entries, &style).unwrap();
    let resolved = fields::materialize(flow.builder()).unwrap();
    let text = resolved.pages.iter().map(logical).collect::<String>();
    assert!(!text.contains('\u{2013}'));
    assert_eq!(
        resolved
            .pages
            .iter()
            .map(|page| page.links.len())
            .sum::<usize>(),
        2
    );
}
