//! Source-only deferred-field regressions. No build, test or PDF was executed.
use super::*;

fn flow(width: f64) -> FlowDocument {
    FlowDocument::new(PageSize::custom(width, 120.0), Margins::all(10.0))
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

fn field(value: BodyField, width: usize) -> BodyFieldPart {
    BodyFieldPart::field(value, BodyFieldFormat::new(width))
}

fn linked_field(value: BodyField, width: usize) -> BodyFieldPart {
    BodyFieldPart::field(value, BodyFieldFormat::new(width).link_to_anchor(true))
}

#[test]
fn forward_anchor_resolves_without_mutating_the_source_builder() {
    let mut flow = flow(220.0);
    let report = flow
        .add_field_paragraph(
            &[
                BodyFieldPart::text("See page "),
                field(BodyField::AnchorDocumentPage("target".into()), 3),
            ],
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
        )
        .unwrap();
    assert_eq!(report.line_pages, vec![1]);
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::DeferredField(_))));
    flow.add_page_break();
    let anchor = flow.add_anchor("target").unwrap();
    assert_eq!(anchor.page, 2);

    let resolved = materialize(flow.builder()).unwrap();
    assert_eq!(logical(&resolved.pages[0]), "See page 2");
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::DeferredField(_))));

    let PdfObject::Dictionary(names) = named_destinations(flow.builder(), 3).unwrap().unwrap()
    else {
        panic!()
    };
    let destinations = names.get("Dests").and_then(PdfObject::as_dict).unwrap();
    let values = destinations
        .get("Names")
        .and_then(PdfObject::as_array)
        .unwrap();
    assert_eq!(values.len(), 2);
    let destination = values[1].as_array().unwrap();
    assert_eq!(destination[0].as_reference().map(|value| value.0), Some(4));
}

#[test]
fn unresolved_forward_anchor_fails_only_at_final_resolution_and_keeps_source_plan() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[field(BodyField::AnchorDocumentPage("missing".into()), 3)],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    assert!(materialize(flow.builder()).is_err());
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::DeferredField(_))));
}

#[test]
fn capacity_overflow_refuses_instead_of_reflowing_final_document() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[field(BodyField::DocumentPages, 1)],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    for _ in 0..9 {
        flow.add_page_break();
    }
    assert_eq!(flow.builder.pages.len(), 10);
    assert!(materialize(flow.builder()).is_err());
}

#[test]
fn section_and_anchor_fields_use_target_page_label_authority() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[
            BodyFieldPart::text("Appendix "),
            field(BodyField::AnchorSectionPage("appendix".into()), 4),
        ],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.start_section(
        FlowSection::new(PageSize::custom(220.0, 120.0), Margins::all(10.0))
            .page_numbering(3, PageNumberStyle::LowerRoman),
    )
    .unwrap();
    flow.add_anchor("appendix").unwrap();
    let resolved = materialize(flow.builder()).unwrap();
    assert_eq!(logical(&resolved.pages[0]), "Appendix iii");
}

#[test]
fn final_document_and_section_fields_resolve_from_distinct_counts() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[
            BodyFieldPart::text("doc "),
            field(BodyField::DocumentPage, 2),
            BodyFieldPart::text("/"),
            field(BodyField::DocumentPages, 2),
            BodyFieldPart::text(" section "),
            field(BodyField::SectionPage, 2),
            BodyFieldPart::text("/"),
            field(BodyField::SectionPages, 2),
            BodyFieldPart::text(" last "),
            field(BodyField::SectionLastPage, 2),
        ],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_page_break();
    let resolved = materialize(flow.builder()).unwrap();
    assert_eq!(logical(&resolved.pages[0]), "doc 1/2 section 1/2 last 2");
}

#[test]
fn oversized_placeholder_rolls_back_commands_and_plan_identity() {
    let mut flow = flow(50.0);
    let plan = flow.builder.next_field_plan_id;
    let cursor = flow.cursor_y;
    assert!(flow
        .add_field_paragraph(
            &[field(BodyField::DocumentPages, 64)],
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
        )
        .is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
    assert_eq!(flow.builder.next_field_plan_id, plan);
    assert_eq!(flow.cursor_y, cursor);
}

#[test]
fn duplicate_or_invalid_anchor_names_do_not_replace_the_original() {
    let mut flow = flow(220.0);
    let first = flow.add_anchor("chapter").unwrap();
    assert!(flow.add_anchor("chapter").is_err());
    assert!(flow.add_anchor("bad\nname").is_err());
    assert_eq!(flow.builder.anchors.len(), 1);
    assert_eq!(flow.builder.anchors["chapter"].page_index + 1, first.page);
}

#[test]
fn field_materialization_is_idempotent() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[field(BodyField::DocumentPage, 3)],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    let once = materialize(flow.builder()).unwrap();
    let twice = materialize(&once).unwrap();
    assert_eq!(logical(&once.pages[0]), "1");
    assert_eq!(logical(&once.pages[0]), logical(&twice.pages[0]));
    assert_eq!(once.pages[0].commands.len(), twice.pages[0].commands.len());
}

#[test]
fn forward_anchor_field_materializes_one_exact_clickable_link() {
    let mut flow = flow(220.0);
    let report = flow
        .add_field_paragraph(
            &[
                BodyFieldPart::text("See "),
                linked_field(BodyField::AnchorDocumentPage("target".into()), 3),
            ],
            &TextStyle::unicode(10.0),
            &ParagraphStyle::new(),
        )
        .unwrap();
    assert!(report.fields[0].clickable);
    flow.add_page_break();
    flow.add_anchor("target").unwrap();

    let resolved = materialize(flow.builder()).unwrap();
    assert_eq!(resolved.pages[0].links.len(), 1);
    let link = &resolved.pages[0].links[0];
    assert_eq!(link.destination, "target");
    assert_eq!(link.name, "WFAuthoredLink-0-0");
    assert_eq!(link.contents, "Go to target");
    assert!(link.rect[2] > link.rect[0]);
    assert!(link.rect[3] > link.rect[1]);
    assert!(link.rect[0] >= 0.0 && link.rect[2] <= resolved.pages[0].size.width);
    assert!(link.rect[1] >= 0.0 && link.rect[3] <= resolved.pages[0].size.height);
}

#[test]
fn link_annotation_dictionary_has_navigation_and_nonvisual_border_contract() {
    let link = AuthoredLink {
        name: "WFAuthoredLink-7-2".into(),
        destination: "chapter-2".into(),
        rect: [10.0, 20.0, 30.0, 40.0],
        contents: "Go to chapter-2".into(),
        structure_id: None,
    };
    let dictionary = link_annotation_dict(&link, 9, Some(12)).unwrap();
    assert!(matches!(dictionary.get("Type"), Some(PdfObject::Name(name)) if name == "Annot"));
    assert!(matches!(dictionary.get("Subtype"), Some(PdfObject::Name(name)) if name == "Link"));
    assert!(matches!(dictionary.get("Dest"), Some(PdfObject::String(_))));
    assert!(matches!(dictionary.get("NM"), Some(PdfObject::String(_))));
    assert_eq!(
        dictionary.get("P").and_then(PdfObject::as_reference),
        Some((9, 0))
    );
    assert_eq!(
        dictionary
            .get("StructParent")
            .and_then(PdfObject::as_integer),
        Some(12)
    );
    assert_eq!(
        dictionary
            .get("Border")
            .and_then(PdfObject::as_array)
            .unwrap(),
        &[
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(0)
        ]
    );
    assert_eq!(
        dictionary
            .get("Rect")
            .and_then(PdfObject::as_array)
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn non_anchor_field_cannot_request_a_link_and_rolls_back() {
    let mut flow = flow(220.0);
    let cursor = flow.cursor_y;
    let plan = flow.builder.next_field_plan_id;
    let result = flow.add_field_paragraph(
        &[BodyFieldPart::field(
            BodyField::DocumentPage,
            BodyFieldFormat::new(3).link_to_anchor(true),
        )],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    );
    assert!(result.is_err());
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.builder.next_field_plan_id, plan);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert!(flow.builder.pages[0].links.is_empty());
}

#[test]
fn two_anchor_fields_on_one_line_get_distinct_hitboxes() {
    let mut flow = flow(260.0);
    flow.add_field_paragraph(
        &[
            linked_field(BodyField::AnchorDocumentPage("first".into()), 2),
            BodyFieldPart::text(" and "),
            linked_field(BodyField::AnchorDocumentPage("second".into()), 2),
        ],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_page_break();
    flow.add_anchor("first").unwrap();
    flow.add_page_break();
    flow.add_anchor("second").unwrap();

    let resolved = materialize(flow.builder()).unwrap();
    let links = &resolved.pages[0].links;
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].destination, "first");
    assert_eq!(links[1].destination, "second");
    assert_ne!(links[0].name, links[1].name);
    assert!(!rectangles_overlap(links[0].rect, links[1].rect));
}

#[test]
fn mixed_direction_anchor_field_bounds_remain_finite() {
    let mut flow = flow(260.0);
    flow.add_field_paragraph(
        &[
            BodyFieldPart::text("\u{05e2}\u{05de}\u{05d5}\u{05d3} "),
            linked_field(BodyField::AnchorDocumentPage("target".into()), 3),
            BodyFieldPart::text(" English"),
        ],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_page_break();
    flow.add_anchor("target").unwrap();
    let resolved = materialize(flow.builder()).unwrap();
    let rect = resolved.pages[0].links[0].rect;
    assert!(rect.iter().all(|value| value.is_finite()));
    assert!(rect[2] > rect[0]);
}

#[test]
fn repeated_materialization_does_not_duplicate_link_annotations() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[linked_field(
            BodyField::AnchorDocumentPage("target".into()),
            3,
        )],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_anchor("target").unwrap();
    let once = materialize(flow.builder()).unwrap();
    let twice = materialize(&once).unwrap();
    assert_eq!(once.pages[0].links, twice.pages[0].links);
    assert_eq!(twice.pages[0].links.len(), 1);
}

#[test]
fn authored_object_graph_references_link_annotation_from_owning_page() {
    let mut flow = flow(220.0);
    flow.add_field_paragraph(
        &[linked_field(
            BodyField::AnchorDocumentPage("target".into()),
            3,
        )],
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.add_anchor("target").unwrap();
    let resolved = materialize(flow.builder()).unwrap();
    let font_plan = FontBuildPlan::from_builder(&resolved).unwrap();
    let image_plan = ImageBuildPlan::from_builder(&resolved).unwrap();
    let objects = AuthoredObjects::build(&resolved, &font_plan, &image_plan).unwrap();

    let page = objects
        .objects
        .iter()
        .find(|object| object.number == 3)
        .and_then(|object| object.object.as_dict())
        .unwrap();
    let annotations = page.get("Annots").and_then(PdfObject::as_array).unwrap();
    assert_eq!(annotations.len(), 1);
    let annotation_number = annotations[0].as_reference().unwrap().0;
    let annotation = objects
        .objects
        .iter()
        .find(|object| object.number == annotation_number)
        .and_then(|object| object.object.as_dict())
        .unwrap();
    assert_eq!(
        annotation.get("Subtype").and_then(PdfObject::as_name),
        Some("Link")
    );
    assert!(matches!(annotation.get("Dest"), Some(PdfObject::String(_))));
    assert_eq!(
        annotation.get("P").and_then(PdfObject::as_reference),
        Some((3, 0))
    );
    assert!(matches!(annotation.get("NM"), Some(PdfObject::String(_))));
}

#[test]
fn destination_leaf_is_sorted_by_encoded_key_and_publishes_limits() {
    let mut flow = flow(220.0);
    flow.add_anchor("zulu").unwrap();
    flow.add_anchor("\u{0100}-chapter").unwrap();
    flow.add_anchor("alpha").unwrap();
    let PdfObject::Dictionary(names) = named_destinations(flow.builder(), 3).unwrap().unwrap()
    else {
        panic!()
    };
    let leaf = names.get("Dests").and_then(PdfObject::as_dict).unwrap();
    let entries = leaf.get("Names").and_then(PdfObject::as_array).unwrap();
    let keys = entries
        .chunks_exact(2)
        .map(|pair| pair[0].as_string().unwrap().to_vec())
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    let limits = leaf.get("Limits").and_then(PdfObject::as_array).unwrap();
    assert_eq!(limits[0].as_string().unwrap(), keys.first().unwrap());
    assert_eq!(limits[1].as_string().unwrap(), keys.last().unwrap());
}

#[test]
fn large_destination_set_builds_bounded_indirect_name_tree() {
    let mut flow = flow(220.0);
    for index in 0..65 {
        flow.add_anchor(format!("anchor-{index:03}")).unwrap();
    }
    let mut next = 100u32;
    let tree = destination_tree_objects(flow.builder(), 3, &mut next).unwrap();
    assert_eq!(next, 103);
    assert_eq!(tree.objects.len(), 3);
    let names = tree.names.as_ref().and_then(PdfObject::as_dict).unwrap();
    let root_number = names
        .get("Dests")
        .and_then(PdfObject::as_reference)
        .unwrap()
        .0;
    let root = tree
        .objects
        .iter()
        .find(|object| object.number == root_number)
        .and_then(|object| object.object.as_dict())
        .unwrap();
    let kids = root.get("Kids").and_then(PdfObject::as_array).unwrap();
    assert_eq!(kids.len(), 2);
    assert_eq!(
        root.get("Limits")
            .and_then(PdfObject::as_array)
            .unwrap()
            .len(),
        2
    );
    for child in kids {
        let number = child.as_reference().unwrap().0;
        let leaf = tree
            .objects
            .iter()
            .find(|object| object.number == number)
            .and_then(|object| object.object.as_dict())
            .unwrap();
        assert!(leaf.get("Kids").is_none());
        assert!(
            leaf.get("Names")
                .and_then(PdfObject::as_array)
                .unwrap()
                .len()
                <= 128
        );
        assert_eq!(
            leaf.get("Limits")
                .and_then(PdfObject::as_array)
                .unwrap()
                .len(),
            2
        );
    }
}

#[test]
fn fixed_capacity_value_alignment_preserves_bytes_and_exact_raw_range() {
    let left = BodyFieldFormat::new(3)
        .affixes("[", "]")
        .value_align(TextAlign::Left);
    let (visual, logical, range) = formatted_value("7", &left).unwrap();
    assert_eq!(visual, "[7  ]");
    assert_eq!(logical, "[7]");
    assert_eq!(&visual[range], "7");

    let center = BodyFieldFormat::new(4).value_align(TextAlign::Center);
    let (visual, logical, range) = formatted_value("12", &center).unwrap();
    assert_eq!(visual, " 12 ");
    assert_eq!(logical, "12");
    assert_eq!(&visual[range], "12");
}
