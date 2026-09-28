//! Source-only tagged-authoring regressions. No build, test, PDF or rendering
//! workload was executed when these cases were added.
use super::*;

fn flow(width: f64, height: f64) -> FlowDocument {
    FlowDocument::new(PageSize::custom(width, height), Margins::all(10.0))
}

#[test]
fn toc_registers_semantic_container_rows_and_balanced_page_scopes() {
    let mut flow = flow(260.0, 140.0);
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    flow.add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    assert_eq!(flow.builder.structures.len(), 2);
    assert_eq!(flow.builder.structures[0].role, Role::Toc);
    assert_eq!(flow.builder.structures[1].role, Role::Toci);
    assert_eq!(
        flow.builder.structures[1].parent,
        Some(flow.builder.structures[0].id)
    );
    let row = flow.builder.structures[1].id;
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::BeginStructure(id) if *id == row)));
    assert!(flow.builder.pages[0]
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::EndStructure(id) if *id == row)));
}

#[test]
fn classic_serialization_contains_struct_tree_parent_tree_and_mcids() {
    let mut flow = flow(260.0, 140.0);
    flow.builder.writer_mode = WriterMode::ClassicXref;
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    flow.add_table_of_contents(&TableOfContentsStyle::new())
        .unwrap();
    let bytes = flow.builder.to_bytes().unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    for token in [
        "/StructTreeRoot",
        "/ParentTree",
        "/StructParents 0",
        "/MarkInfo",
        "/TOC",
        "/TOCI",
        "/MCID 0",
        "/OBJR",
        "/StructParent 1",
    ] {
        assert!(pdf.contains(token), "missing {token}");
    }
}

#[test]
fn wrapped_index_row_reuses_one_structure_element_across_page_mcids() {
    let mut flow = flow(150.0, 60.0);
    flow.add_anchor("occurrence").unwrap();
    let term = (0..80).map(|_| "long ").collect::<String>();
    let report = flow
        .add_document_index(
            &[PdfIndexEntry::new(term).anchor("occurrence")],
            &DocumentIndexStyle::new(),
        )
        .unwrap();
    assert!(report.rows[0].output_pages.len() > 1);
    let row = flow
        .builder
        .structures
        .iter()
        .find(|element| element.role == Role::Paragraph)
        .unwrap()
        .id;
    let pages = flow
        .builder
        .pages
        .iter()
        .filter(|page| {
            page.commands
                .iter()
                .any(|command| matches!(command, PageCommand::BeginStructure(id) if *id == row))
        })
        .count();
    assert!(pages > 1);
}

#[test]
fn failed_tagged_toc_rolls_back_structure_registry_and_identity() {
    let mut flow = flow(200.0, 120.0);
    flow.add_anchor("target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Target", "target")])
        .unwrap();
    let structures = flow.builder.structures.len();
    let identity = flow.builder.next_structure_id;
    let mut style = TableOfContentsStyle::new();
    style.page_column_width = 10_000.0;
    assert!(flow.add_table_of_contents(&style).is_err());
    assert_eq!(flow.builder.structures.len(), structures);
    assert_eq!(flow.builder.next_structure_id, identity);
}

#[test]
fn prepended_front_matter_structure_roots_precede_existing_body_roots() {
    let mut flow = flow(260.0, 180.0);
    flow.add_anchor("body-target").unwrap();
    flow.set_outline(vec![PdfOutlineEntry::new("Body", "body-target")])
        .unwrap();
    flow.add_document_index(
        &[PdfIndexEntry::new("Body").anchor("body-target")],
        &DocumentIndexStyle::new(),
    )
    .unwrap();
    flow.prepend_table_of_contents(
        FlowSection::new(flow.page_size, flow.margins),
        &TableOfContentsStyle::new(),
    )
    .unwrap();

    let roots = flow
        .builder
        .structures
        .iter()
        .filter(|element| element.parent.is_none())
        .map(|element| element.role)
        .collect::<Vec<_>>();
    assert_eq!(roots, vec![Role::Toc, Role::Index]);
}

#[test]
fn large_parent_tree_uses_bounded_indirect_number_tree_nodes() {
    let entries = (0..130)
        .map(|key| (key, reference(1_000 + u32::try_from(key).unwrap())))
        .collect::<Vec<_>>();
    let mut next = 20;
    let objects = build_number_tree(10, &entries, &mut next).unwrap();
    let root = objects.iter().find(|object| object.number == 10).unwrap();
    let PdfObject::Dictionary(root) = &root.object else {
        panic!("number-tree root is not a dictionary");
    };
    let kids = root.get("Kids").and_then(PdfObject::as_array).unwrap();
    assert_eq!(kids.len(), 3);
    for child in kids {
        let number = child.as_reference().unwrap().0;
        let leaf = objects
            .iter()
            .find(|object| object.number == number)
            .unwrap();
        let PdfObject::Dictionary(leaf) = &leaf.object else {
            panic!("number-tree leaf is not a dictionary");
        };
        assert!(
            leaf.get("Nums")
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
fn ordinary_paragraph_retains_one_structure_identity_across_pages() {
    let mut flow = flow(140.0, 60.0);
    let text = (0..80).map(|_| "word ").collect::<String>();
    flow.add_paragraph(&text, &TextStyle::unicode(10.0), &ParagraphStyle::new())
        .unwrap();
    let paragraph = flow
        .builder
        .structures
        .iter()
        .find(|element| element.role == Role::Paragraph)
        .unwrap()
        .id;
    let pages = flow
        .builder
        .pages
        .iter()
        .filter(|page| {
            page.commands.iter().any(
                |command| matches!(command, PageCommand::BeginStructure(id) if *id == paragraph),
            )
        })
        .count();
    assert!(pages > 1);
}

#[test]
fn authored_heading_levels_use_standard_structure_roles() {
    let mut flow = flow(300.0, 300.0);
    flow.add_heading("First", 1).unwrap();
    flow.add_heading("Sixth", 6).unwrap();
    flow.add_heading("Generic", 7).unwrap();
    let roles = flow
        .builder
        .structures
        .iter()
        .map(|element| element.role)
        .collect::<Vec<_>>();
    assert_eq!(roles, vec![Role::Heading1, Role::Heading6, Role::Heading]);
}

#[test]
fn authored_list_registers_list_items_labels_and_nonempty_bodies() {
    let mut flow = flow(300.0, 300.0);
    flow.add_list(
        ["alpha", ""],
        true,
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    let roles = flow
        .builder
        .structures
        .iter()
        .map(|element| element.role)
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        vec![
            Role::List,
            Role::ListItem,
            Role::ListLabel,
            Role::ListBody,
            Role::ListItem,
            Role::ListLabel,
        ]
    );
    assert_eq!(
        flow.builder.structures[1].parent,
        Some(flow.builder.structures[0].id)
    );
    assert_eq!(
        flow.builder.structures[2].parent,
        Some(flow.builder.structures[1].id)
    );
    assert_eq!(
        flow.builder.structures[3].parent,
        Some(flow.builder.structures[1].id)
    );
}

#[test]
fn meaningful_figures_require_alt_text_while_legacy_images_are_artifacts() {
    let mut flow = flow(300.0, 300.0);
    let image = flow
        .builder_mut()
        .add_rgb_image(1, 1, vec![0, 0, 0])
        .unwrap();
    flow.add_image(image, 20.0, 20.0).unwrap();
    assert!(flow.builder.structures.is_empty());
    assert!(matches!(
        flow.builder.pages[0].commands[0],
        PageCommand::BeginArtifact
    ));

    flow.add_figure(image, 20.0, 20.0, "Black square").unwrap();
    assert_eq!(flow.builder.structures.len(), 1);
    assert_eq!(flow.builder.structures[0].role, Role::Figure);
    let structure_count = flow.builder.structures.len();
    let command_count = flow.builder.pages[0].commands.len();
    assert!(flow.add_figure(image, 20.0, 20.0, "").is_err());
    assert_eq!(flow.builder.structures.len(), structure_count);
    assert_eq!(flow.builder.pages[0].commands.len(), command_count);

    flow.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&flow.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/Figure"));
    assert!(pdf.contains("/Alt"));
    assert!(pdf.contains("/Artifact BMC"));
}

#[test]
fn flowed_tables_register_rows_and_cells_while_repeated_headers_are_artifacts() {
    let mut flow = flow(180.0, 80.0);
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .caption("Semantic table")
        .summary("A two-column table with explicit row and column headers")
        .row_split_policy(TableRowSplitPolicy::Lines {
            min_fragment_lines: 1,
            min_final_lines: 1,
        });
    table.set_header(["Header", ""]);
    let long = (0..40).map(|_| "body line\n").collect::<String>();
    table.add_row([TableCell::text(long).row_header(), TableCell::text("")]);
    let report = flow.add_table_with_report(&table).unwrap();
    let caption = report.caption.expect("caption receipt");
    assert_eq!(caption.page, 1);
    assert!(caption.height > 0.0);

    let roles = flow
        .builder
        .structures
        .iter()
        .map(|element| element.role)
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        vec![
            Role::Table,
            Role::Caption,
            Role::TableHead,
            Role::TableBody,
            Role::TableRow,
            Role::TableHeader,
            Role::TableHeader,
            Role::TableRow,
            Role::TableHeader,
            Role::TableData,
        ]
    );
    let header_ids = [flow.builder.structures[5].id, flow.builder.structures[6].id];
    for page in flow.builder.pages.iter().skip(1) {
        assert!(!page.commands.iter().any(|command| {
            matches!(command, PageCommand::BeginStructure(id) if header_ids.contains(id))
        }));
    }
    let body = flow.builder.structures[8].id;
    assert!(
        flow.builder
            .pages
            .iter()
            .filter(|page| page
                .commands
                .iter()
                .any(|command| matches!(command, PageCommand::BeginStructure(id) if *id == body)))
            .count()
            > 1
    );
    flow.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&flow.builder.to_bytes().unwrap()).into_owned();
    for token in [
        "/Table",
        "/Caption",
        "/THead",
        "/TBody",
        "/TR",
        "/TH",
        "/TD",
        "/Scope /Column",
        "/Scope /Row",
        "/Headers",
        "/ID (WFTH-",
        "/Summary",
    ] {
        assert!(pdf.contains(token), "missing {token}");
    }
}

#[test]
fn spanned_cells_publish_colspan_and_union_every_covered_header() {
    let mut flow = flow(260.0, 140.0);
    let mut table = TableBuilder::new(vec![
        TableColumn::new(70.0),
        TableColumn::new(70.0),
        TableColumn::new(70.0),
    ]);
    table.set_header([
        TableCell::text("Header 1 and 2").column_span(2),
        TableCell::text("Header 3"),
    ]);
    table.push_row(TableRow::new(vec![
        TableCell::text("Row header").row_header(),
        TableCell::text("Data across 2 and 3").column_span(2),
    ]));
    flow.add_table_with_report(&table).unwrap();

    let elements = &flow.builder.structures;
    assert_eq!(elements[4].role, Role::TableHeader);
    assert_eq!(elements[4].table_column_span, Some(2));
    assert_eq!(elements[5].role, Role::TableHeader);
    assert_eq!(elements[7].role, Role::TableHeader);
    assert_eq!(elements[8].role, Role::TableData);
    assert_eq!(elements[8].table_column_span, Some(2));
    assert_eq!(
        elements[8].table_headers,
        vec![elements[4].id, elements[5].id, elements[7].id]
    );

    flow.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&flow.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.matches("/ColSpan 2").count() >= 2);
    assert!(pdf.contains("/Headers"));
}

#[test]
fn typed_cell_structure_owners_are_bounded_paired_and_unique() {
    let mut builder = PdfBuilder::new();
    let table = register_table(&mut builder, None).unwrap();
    let body = register_table_group(&mut builder, table, Role::TableBody).unwrap();
    let row = register(&mut builder, Role::TableRow, Some(body), None).unwrap();
    let first = register_table_cell(&mut builder, Role::TableData, row, false).unwrap();
    let second = register_table_cell(&mut builder, Role::TableData, row, false).unwrap();

    set_typed_table_cell_owner(&mut builder, first, "ledger", "net").unwrap();
    assert!(set_typed_table_cell_owner(&mut builder, second, "ledger", "net").is_err());
    assert!(set_typed_table_cell_owner(&mut builder, second, "", "tax").is_err());
    set_typed_table_cell_owner(&mut builder, second, "ledger", "tax").unwrap();

    assert_eq!(
        builder
            .structures
            .iter()
            .filter_map(|element| element
                .typed_table_identity
                .as_deref()
                .zip(element.typed_cell_identity.as_deref()))
            .collect::<Vec<_>>(),
        vec![("ledger", "net"), ("ledger", "tax")]
    );
}

#[test]
fn document_language_is_validated_and_published_with_the_structure_tree() {
    let mut flow = flow(300.0, 200.0);
    flow.builder.set_language(" en-US ").unwrap();
    assert!(flow.builder.set_language("en--US").is_err());
    assert!(flow.builder.set_language("12").is_err());
    assert!(flow.builder.set_language("en-u").is_err());
    assert!(flow.builder.set_language("en-u-ca-u-nu").is_err());
    flow.builder
        .set_language("zh-Hant-TW-u-nu-hanidec")
        .unwrap();
    flow.builder.set_language("en-US").unwrap();
    flow.add_paragraph(
        "Language",
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
    )
    .unwrap();
    flow.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&flow.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/Lang (en-US)"));
    assert!(pdf.contains("/StructTreeRoot"));
}

#[test]
fn footnote_body_materialization_retains_stable_note_structure_identity() {
    let mut flow = flow(220.0, 100.0);
    let text = "Body¹";
    let marker = text.find('¹').unwrap();
    flow.add_paragraph_with_footnotes(
        text,
        &TextStyle::unicode(10.0),
        &ParagraphStyle::new(),
        &[FlowFootnote::new(
            marker..marker + '¹'.len_utf8(),
            "A note.",
            TextStyle::unicode(8.0),
        )],
    )
    .unwrap();
    assert_eq!(
        flow.builder
            .structures
            .iter()
            .map(|element| element.role)
            .collect::<Vec<_>>(),
        vec![Role::Paragraph, Role::Note, Role::Span, Role::Reference]
    );
    let note = flow.builder.structures[1].id;
    let reference = flow.builder.structures[3].id;
    assert_eq!(
        flow.builder.structures[3].parent,
        Some(flow.builder.structures[0].id)
    );
    assert_eq!(flow.builder.structures[1].references, vec![reference]);
    assert_eq!(flow.builder.structures[3].references, vec![note]);
    assert!(flow
        .builder
        .pages
        .iter()
        .any(|page| page.commands.iter().any(
            |command| matches!(command, PageCommand::BeginStructure(id) if *id == reference)
        )));
    let mut materialized = notes::materialize(&flow.builder).unwrap();
    assert!(materialized.pages.iter().any(|page| page
        .commands
        .iter()
        .any(|command| matches!(command, PageCommand::BeginStructure(id) if *id == note))));
    materialized.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&materialized.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/Note"));
    assert!(pdf.contains("/Reference"));
    assert!(pdf.contains("/Ref"));
    assert!(pdf.contains("/ID (WFNote-"));
    assert!(pdf.contains("/IDTree"));
    assert!(pdf.contains("/Names"));
}

#[test]
fn tagged_authoring_rejects_unowned_painting_commands() {
    let mut flow = flow(300.0, 200.0);
    flow.add_paragraph("Owned", &TextStyle::unicode(10.0), &ParagraphStyle::new())
        .unwrap();
    flow.current_page_mut()
        .draw_rect(10.0, 10.0, 20.0, 20.0, &GraphicsStyle::fill(Color::black()));
    assert!(flow.builder.to_bytes().is_err());
}
