//! Source-only painted table-of-contents regressions. No build, test, PDF or
//! rendering workload was executed when these cases were added.
use super::*;

const EPS: f64 = 1e-7;

fn flow(width: f64, height: f64) -> FlowDocument {
    FlowDocument::new(PageSize::custom(width, height), Margins::all(10.0))
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

fn text_sample(command: &PageCommand, needle: &str) -> Option<(f64, f64)> {
    match command {
        PageCommand::Text { text, x, style, .. } if text.contains(needle) => Some((*x, style.size)),
        PageCommand::TextGroup { runs, .. } => runs.iter().find_map(|run| text_sample(run, needle)),
        _ => None,
    }
}

fn page_text_sample(page: &PdfPageBuilder, needle: &str) -> Option<(f64, f64)> {
    page.commands
        .iter()
        .find_map(|command| text_sample(command, needle))
}

#[test]
fn forward_outline_target_paints_then_resolves_clickable_page_value() {
    let mut flow = flow(240.0, 140.0);
    flow.set_outline(vec![PdfOutlineEntry::new("Appendix", "appendix")])
        .unwrap();
    let report = flow
        .add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.rows.len(), 1);
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::DeferredField(_))));
    flow.add_page_break();
    flow.add_anchor("appendix").unwrap();

    let resolved = fields::materialize(flow.builder()).unwrap();
    assert!(logical(&resolved.pages[0]).contains("Appendix"));
    assert!(logical(&resolved.pages[0]).contains('2'));
    assert_eq!(resolved.pages[0].links.len(), 1);
    assert_eq!(resolved.pages[0].links[0].destination, "appendix");
}

#[test]
fn page_value_is_right_aligned_inside_reported_fixed_column() {
    let mut flow = flow(240.0, 140.0);
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    let report = flow
        .add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    let resolved = fields::materialize(flow.builder()).unwrap();
    let column = report.rows[0].page_column;
    let link = &resolved.pages[0].links[0];
    assert!(link.rect[0] >= column[0] - EPS);
    assert!(link.rect[2] <= column[2] + EPS);
    assert!((link.rect[2] - column[2]).abs() < EPS);
}

#[test]
fn nested_entries_indent_without_changing_page_column() {
    let mut flow = flow(260.0, 160.0);
    flow.add_anchor("parent").unwrap();
    flow.add_anchor("child").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Parent", "parent")
        .children(vec![PdfOutlineEntry::new("Child", "child")])])
        .unwrap();
    let report = flow
        .add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.rows[0].level, 0);
    assert_eq!(report.rows[1].level, 1);
    assert_eq!(
        [report.rows[0].page_column[0], report.rows[0].page_column[2]],
        [report.rows[1].page_column[0], report.rows[1].page_column[2]]
    );
}

#[test]
fn invalid_geometry_rolls_back_rows_pages_cursor_and_field_identity() {
    let mut flow = flow(200.0, 120.0);
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    let commands = flow.builder.pages[0].commands.len();
    let pages = flow.builder.pages.len();
    let cursor = flow.cursor_y;
    let identity = flow.builder.next_field_plan_id;
    let mut style = TableOfContentsStyle::new();
    style.page_column_width = 500.0;
    assert!(flow.add_table_of_contents(&style).is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.builder.next_field_plan_id, identity);
}

#[test]
fn unresolved_target_is_retained_as_deferred_failure_not_silent_text() {
    let mut flow = flow(240.0, 140.0);
    flow.set_outline(vec![PdfOutlineEntry::new("Missing", "missing")])
        .unwrap();
    flow.add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    assert!(fields::materialize(flow.builder()).is_err());
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::DeferredField(_))));
}

#[test]
fn many_rows_paginate_as_one_rollback_capable_transaction() {
    let mut flow = flow(220.0, 80.0);
    let mut outline = Vec::new();
    for index in 0..12 {
        let anchor = format!("item-{index}");
        flow.add_anchor(anchor.clone()).unwrap();
        outline.push(PdfOutlineEntry::new(format!("Item {index}"), anchor));
    }
    flow.set_outline(outline).unwrap();
    let report = flow
        .add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(report.rows.len(), 12);
    assert!(flow.builder.pages.len() > 1);
    assert!(report
        .rows
        .windows(2)
        .all(|rows| rows[0].page <= rows[1].page));
}

#[test]
fn left_page_column_supports_rtl_title_and_left_aligned_value() {
    let mut flow = flow(260.0, 140.0);
    flow.add_anchor("rtl").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new(
        "\u{05e4}\u{05e8}\u{05e7}",
        "rtl",
    )])
    .unwrap();
    let mut style = TableOfContentsStyle::new();
    style.page_side = TableOfContentsPageSide::Left;
    style.title_align = TextAlign::Right;
    style.page_align = TextAlign::Left;
    let report = flow.add_table_of_contents(&style).unwrap();
    let resolved = fields::materialize(flow.builder()).unwrap();
    assert!((report.rows[0].page_column[0] - 10.0).abs() < EPS);
    assert!((resolved.pages[0].links[0].rect[0] - 10.0).abs() < EPS);
    assert!(logical(&resolved.pages[0]).contains("\u{05e4}\u{05e8}\u{05e7}"));
}

#[test]
fn exact_level_override_changes_typography_indent_and_leader() {
    let mut flow = flow(300.0, 180.0);
    flow.add_anchor("parent").unwrap();
    flow.add_anchor("child").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Parent", "parent")
        .children(vec![PdfOutlineEntry::new("Child", "child")])])
        .unwrap();
    let mut style = TableOfContentsStyle::new();
    style.leader = None;
    style.level_styles.push(
        TableOfContentsLevelStyle::new(1)
            .title_style(TextStyle::unicode(16.0))
            .indent(36.0)
            .leader("*"),
    );
    flow.add_table_of_contents(&style).unwrap();
    let parent = page_text_sample(&flow.builder.pages[0], "Parent").unwrap();
    let child = page_text_sample(&flow.builder.pages[0], "Child").unwrap();
    assert_eq!(parent.1, 11.0);
    assert_eq!(child.1, 16.0);
    assert!((child.0 - parent.0 - 36.0).abs() < EPS);
    assert!(page_text_sample(&flow.builder.pages[0], "*").is_some());
}

#[test]
fn duplicate_level_override_fails_before_mutating_flow() {
    let mut flow = flow(240.0, 140.0);
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    let pages = flow.builder.pages.len();
    let commands = flow.builder.pages[0].commands.len();
    let cursor = flow.cursor_y;
    let identity = flow.builder.next_field_plan_id;
    let mut style = TableOfContentsStyle::new();
    style.level_styles = vec![
        TableOfContentsLevelStyle::new(0),
        TableOfContentsLevelStyle::new(0),
    ];
    assert!(flow.add_table_of_contents(&style).is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.builder.next_field_plan_id, identity);
}

#[test]
fn keep_with_next_moves_a_fitting_pair_to_the_next_page() {
    let mut flow = flow(240.0, 100.0);
    flow.add_spacer(60.0);
    flow.add_anchor("heading").unwrap();
    flow.add_anchor("first-child").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Heading", "heading")
        .children(vec![PdfOutlineEntry::new("First child", "first-child")])])
        .unwrap();
    let style = TableOfContentsStyle::new()
        .with_level_style(TableOfContentsLevelStyle::new(0).keep_with_next(true));
    let report = flow.add_table_of_contents(&style).unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.rows[0].page, 2);
    assert_eq!(report.rows[1].page, 2);
}

#[test]
fn keep_with_previous_moves_the_preceding_row_without_a_forward_flag() {
    let mut flow = flow(240.0, 100.0);
    flow.add_spacer(60.0);
    flow.add_anchor("heading").unwrap();
    flow.add_anchor("dependent-child").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Heading", "heading").children(
        vec![PdfOutlineEntry::new("Dependent child", "dependent-child")],
    )])
    .unwrap();
    let style = TableOfContentsStyle::new()
        .with_level_style(TableOfContentsLevelStyle::new(1).keep_with_previous(true));
    let report = flow.add_table_of_contents(&style).unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.rows[0].page, 2);
    assert_eq!(report.rows[1].page, 2);
}
