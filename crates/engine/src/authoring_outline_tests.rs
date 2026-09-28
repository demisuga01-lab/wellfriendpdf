//! Source-only document-outline regressions. No build, test or PDF workload was
//! executed when these cases were added.
use super::*;

fn flow() -> FlowDocument {
    FlowDocument::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
}

fn dictionary(objects: &[OutputObject], number: u32) -> &PdfDictionary {
    objects
        .iter()
        .find(|object| object.number == number)
        .and_then(|object| object.object.as_dict())
        .unwrap()
}

#[test]
fn outline_accepts_forward_anchor_and_resolves_at_final_build() {
    let mut flow = flow();
    flow.set_outline(vec![PdfOutlineEntry::new("Appendix", "appendix")])
        .unwrap();
    flow.add_page_break();
    flow.add_anchor("appendix").unwrap();
    let mut next = 100;
    let built = build(flow.builder(), &mut next).unwrap();
    assert_eq!(built.root, Some(100));
    assert_eq!(built.objects.len(), 2);
    assert_eq!(next, 102);
    let item = dictionary(&built.objects, 101);
    assert!(matches!(item.get("Dest"), Some(PdfObject::String(_))));
    assert_eq!(
        item.get("Parent").and_then(PdfObject::as_reference),
        Some((100, 0))
    );
}

#[test]
fn hierarchy_has_exact_parent_sibling_and_visible_count_links() {
    let mut flow = flow();
    for anchor in ["a", "b", "c", "d", "e"] {
        flow.add_anchor(anchor).unwrap();
    }
    let tree = PdfOutlineEntry::new("A", "a").children(vec![
        PdfOutlineEntry::new("B", "b").children(vec![PdfOutlineEntry::new("D", "d")]),
        PdfOutlineEntry::new("C", "c")
            .open(false)
            .children(vec![PdfOutlineEntry::new("E", "e")]),
    ]);
    flow.set_outline(vec![tree]).unwrap();
    let mut next = 20;
    let built = build(flow.builder(), &mut next).unwrap();
    let root = dictionary(&built.objects, 20);
    assert_eq!(root.get_integer("Count"), Some(4));
    let a = dictionary(&built.objects, 21);
    let b = dictionary(&built.objects, 22);
    let d = dictionary(&built.objects, 23);
    let c = dictionary(&built.objects, 24);
    let e = dictionary(&built.objects, 25);
    assert_eq!(a.get_integer("Count"), Some(3));
    assert_eq!(b.get_integer("Count"), Some(1));
    assert_eq!(c.get_integer("Count"), Some(-1));
    assert_eq!(
        b.get("Next").and_then(PdfObject::as_reference),
        Some((24, 0))
    );
    assert_eq!(
        c.get("Prev").and_then(PdfObject::as_reference),
        Some((22, 0))
    );
    assert_eq!(
        d.get("Parent").and_then(PdfObject::as_reference),
        Some((22, 0))
    );
    assert_eq!(
        e.get("Parent").and_then(PdfObject::as_reference),
        Some((24, 0))
    );
}

#[test]
fn closed_top_level_item_hides_its_descendants_from_root_count() {
    let mut flow = flow();
    flow.add_anchor("root").unwrap();
    flow.add_anchor("child").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Root", "root")
        .open(false)
        .children(vec![PdfOutlineEntry::new("Child", "child")])])
        .unwrap();
    let mut next = 10;
    let built = build(flow.builder(), &mut next).unwrap();
    assert_eq!(dictionary(&built.objects, 10).get_integer("Count"), Some(1));
    assert_eq!(
        dictionary(&built.objects, 11).get_integer("Count"),
        Some(-1)
    );
}

#[test]
fn unresolved_outline_target_fails_without_mutating_builder() {
    let mut flow = flow();
    flow.set_outline(vec![PdfOutlineEntry::new("Missing", "missing")])
        .unwrap();
    let mut next = 50;
    assert!(build(flow.builder(), &mut next).is_err());
    assert_eq!(next, 50);
    assert_eq!(flow.builder.outline.len(), 1);
}

#[test]
fn invalid_replacement_is_atomic_and_keeps_previous_outline() {
    let mut builder = PdfBuilder::new();
    builder
        .set_outline(vec![PdfOutlineEntry::new("Valid", "valid")])
        .unwrap();
    assert!(builder
        .set_outline(vec![PdfOutlineEntry::new("bad\ntitle", "target")])
        .is_err());
    assert_eq!(builder.outline[0].title, "Valid");
}

#[test]
fn authored_catalog_references_outline_root_and_requests_outline_panel() {
    let mut flow = flow();
    flow.add_anchor("start").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Start", "start")])
        .unwrap();
    let font_plan = FontBuildPlan::from_builder(flow.builder()).unwrap();
    let image_plan = ImageBuildPlan::from_builder(flow.builder()).unwrap();
    let authored = AuthoredObjects::build(flow.builder(), &font_plan, &image_plan).unwrap();
    let catalog = dictionary(&authored.objects, authored.catalog_number);
    let outline = catalog
        .get("Outlines")
        .and_then(PdfObject::as_reference)
        .unwrap()
        .0;
    assert_eq!(catalog.get_name("PageMode"), Some("UseOutlines"));
    assert_eq!(
        dictionary(&authored.objects, outline).get_name("Type"),
        Some("Outlines")
    );
}

#[test]
fn outlined_headings_capture_visible_text_anchor_and_hierarchy_atomically() {
    let mut flow = flow();
    flow.add_outlined_heading("Chapter One", 1, "chapter-1")
        .unwrap();
    flow.add_outlined_heading("Section A", 2, "section-a")
        .unwrap();
    flow.add_outlined_heading("Section B", 2, "section-b")
        .unwrap();
    flow.add_outlined_heading("Chapter Two", 1, "chapter-2")
        .unwrap();

    assert_eq!(flow.builder.outline_item_count, 4);
    assert_eq!(flow.builder.outline.len(), 2);
    assert_eq!(flow.builder.outline[0].children.len(), 2);
    assert_eq!(flow.builder.outline[0].children[0].anchor, "section-a");
    assert_eq!(flow.builder.outline[1].anchor, "chapter-2");
    assert_eq!(flow.builder.anchors.len(), 4);
    assert_eq!(flow.outline_heading_path, vec!["chapter-2"]);
    assert!(!flow.builder.pages[0].commands.is_empty());
}

#[test]
fn outlined_heading_rejects_skipped_level_without_mutation() {
    let mut flow = flow();
    let cursor = flow.cursor_y;
    assert!(flow.add_outlined_heading("Skipped", 2, "skipped").is_err());
    assert_eq!(flow.cursor_y, cursor);
    assert!(flow.builder.outline.is_empty());
    assert!(flow.builder.anchors.is_empty());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn heading_layout_failure_rolls_back_outline_anchor_path_and_page() {
    let mut flow = flow();
    let cursor = flow.cursor_y;
    assert!(flow
        .add_outlined_heading("\u{6f22}\u{5b57}", 1, "unsupported-standard14")
        .is_err());
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.builder.outline_item_count, 0);
    assert!(flow.builder.outline.is_empty());
    assert!(flow.builder.anchors.is_empty());
    assert!(flow.outline_heading_path.is_empty());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn replacing_outline_resets_automatic_heading_stack() {
    let mut flow = flow();
    flow.add_outlined_heading("Original", 1, "original")
        .unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Manual", "manual")])
        .unwrap();
    assert!(flow.outline_heading_path.is_empty());
    assert!(flow
        .add_outlined_heading("Cannot attach", 2, "child")
        .is_err());
    assert_eq!(flow.builder.outline.len(), 1);
    assert_eq!(flow.builder.outline[0].anchor, "manual");
}

#[test]
fn outlined_heading_anchor_follows_its_first_line_to_the_next_page() {
    let mut flow = flow();
    flow.add_spacer(90.0);
    assert_eq!(flow.builder.pages.len(), 1);
    let anchor = flow
        .add_outlined_heading("Moved heading", 1, "moved")
        .unwrap();
    assert_eq!(anchor.page, 2);
    assert_eq!(flow.builder.anchors["moved"].page_index, 1);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert!(!flow.builder.pages[1].commands.is_empty());
}
