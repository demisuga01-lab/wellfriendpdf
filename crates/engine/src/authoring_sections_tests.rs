//! Source-only section/master regressions. No PDF workload was executed.
use super::*;

fn running(parts: Vec<RunningTextPart>, align: TextAlign) -> RunningText {
    RunningText::new(parts, TextStyle::unicode(10.0))
        .align(align)
        .baseline_from_edge(16.0)
}

fn words(page: &PdfPageBuilder) -> String {
    page.commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::TextGroup { logical_text, .. } => Some(logical_text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn numbered_master(label: &str) -> SectionPageMaster {
    SectionPageMaster::new()
        .header(RunningText::literal(label, TextStyle::unicode(10.0)).baseline_from_edge(16.0))
        .footer(running(
            vec![
                RunningTextPart::Text("Page ".into()),
                RunningTextPart::Field(PageNumberField::SectionPage),
                RunningTextPart::Text(" of ".into()),
                RunningTextPart::Field(PageNumberField::SectionLastPage),
            ],
            TextAlign::Center,
        ))
}

#[test]
fn first_odd_even_masters_and_final_counts_materialize_as_artifacts() {
    let section = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
        .first_master(numbered_master("FIRST"))
        .odd_master(numbered_master("ODD"))
        .even_master(numbered_master("EVEN"));
    let mut flow = FlowDocument::from_section(section).unwrap();
    flow.add_page_break().add_page_break();
    let materialized = materialize(flow.builder()).unwrap();
    assert_eq!(words(&materialized.pages[0]), "FIRST|Page 1 of 3");
    assert_eq!(words(&materialized.pages[1]), "EVEN|Page 2 of 3");
    assert_eq!(words(&materialized.pages[2]), "ODD|Page 3 of 3");
    for page in &materialized.pages {
        assert_eq!(
            page.commands
                .iter()
                .filter(|command| matches!(command, PageCommand::BeginArtifact))
                .count(),
            2
        );
        assert_eq!(
            page.commands
                .iter()
                .filter(|command| matches!(command, PageCommand::EndArtifact))
                .count(),
            2
        );
    }
    assert!(flow
        .builder
        .pages
        .iter()
        .all(|page| page.commands.is_empty()));
}

#[test]
fn odd_section_start_keeps_parity_blank_in_previous_section() {
    let first = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
        .odd_master(numbered_master("OLD"))
        .even_master(numbered_master("OLD"));
    let mut flow = FlowDocument::from_section(first).unwrap();
    let second = FlowSection::new(PageSize::custom(300.0, 240.0), Margins::all(36.0))
        .first_master(numbered_master("SECOND"))
        .page_numbering(3, PageNumberStyle::LowerRoman);
    flow.start_section_on(second, FlowPageBreak::NextOddPage)
        .unwrap();
    assert_eq!(flow.builder.pages.len(), 3);
    assert_eq!(flow.builder.pages[0].section_index, Some(0));
    assert_eq!(flow.builder.pages[1].section_index, Some(0));
    assert_eq!(flow.builder.pages[2].section_index, Some(1));
    assert!(!flow.builder.pages[0].suppress_section_master);
    assert!(flow.builder.pages[1].suppress_section_master);
    assert!(!flow.builder.pages[2].suppress_section_master);
    assert_eq!(flow.builder.pages[2].size, PageSize::custom(300.0, 240.0));
    let materialized = materialize(flow.builder()).unwrap();
    assert_eq!(words(&materialized.pages[0]), "OLD|Page 1 of 2");
    assert_eq!(words(&materialized.pages[1]), "");
    assert_eq!(words(&materialized.pages[2]), "SECOND|Page iii of iii");

    let PdfObject::Dictionary(labels) = page_labels(&materialized).unwrap().unwrap() else {
        panic!()
    };
    let nums = labels.get("Nums").and_then(PdfObject::as_array).unwrap();
    assert_eq!(nums[0].as_integer(), Some(0));
    assert_eq!(nums[2].as_integer(), Some(2));
    assert_eq!(
        nums[3].as_dict().and_then(|dict| dict.get_name("S")),
        Some("r")
    );
    assert_eq!(
        nums[3]
            .as_dict()
            .and_then(|dict| dict.get("St"))
            .and_then(PdfObject::as_integer),
        Some(3)
    );

    let output = flow.builder.to_bytes().unwrap();
    let engine = crate::ContentEngine::open_bytes(output).unwrap();
    assert!(engine
        .document()
        .get_catalog()
        .unwrap()
        .contains_key("PageLabels"));
    assert!(engine
        .collect_page_text_chunks(2)
        .unwrap()
        .iter()
        .all(|chunk| !chunk.text.contains("OLD")));
    assert!(engine
        .collect_page_text_chunks(3)
        .unwrap()
        .iter()
        .any(|chunk| chunk.text.contains("SECOND")));
}

#[test]
fn invalid_or_overflowing_master_never_mutates_authored_pages() {
    let invalid = FlowSection::new(PageSize::custom(100.0, 100.0), Margins::all(10.0)).odd_master(
        SectionPageMaster::new().header(
            RunningText::literal("bad\nheader", TextStyle::unicode(10.0)).baseline_from_edge(5.0),
        ),
    );
    assert!(FlowDocument::from_section(invalid).is_err());

    let section = FlowSection::new(PageSize::custom(100.0, 100.0), Margins::all(10.0)).odd_master(
        SectionPageMaster::new().header(
            RunningText::literal("This running line cannot fit", TextStyle::unicode(10.0))
                .baseline_from_edge(5.0),
        ),
    );
    let flow = FlowDocument::from_section(section).unwrap();
    assert!(materialize(flow.builder()).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn document_and_section_fields_use_distinct_authorities() {
    let master = SectionPageMaster::new().footer(running(
        vec![
            RunningTextPart::Field(PageNumberField::DocumentPage),
            RunningTextPart::Text("/".into()),
            RunningTextPart::Field(PageNumberField::DocumentPages),
            RunningTextPart::Text(" section ".into()),
            RunningTextPart::Field(PageNumberField::SectionPage),
            RunningTextPart::Text("/".into()),
            RunningTextPart::Field(PageNumberField::SectionPages),
            RunningTextPart::Text(" last ".into()),
            RunningTextPart::Field(PageNumberField::SectionLastPage),
        ],
        TextAlign::Right,
    ));
    let first = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
        .odd_master(master.clone())
        .even_master(master.clone());
    let mut flow = FlowDocument::from_section(first).unwrap();
    flow.add_page_break();
    flow.start_section(
        FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
            .odd_master(master.clone())
            .even_master(master)
            .page_numbering(7, PageNumberStyle::Decimal),
    )
    .unwrap();
    let materialized = materialize(flow.builder()).unwrap();
    assert_eq!(words(&materialized.pages[0]), "1/3 section 1/2 last 2");
    assert_eq!(words(&materialized.pages[1]), "2/3 section 2/2 last 2");
    assert_eq!(words(&materialized.pages[2]), "3/3 section 7/1 last 7");
}

#[test]
fn materialization_is_private_and_idempotent() {
    let section = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
        .odd_master(numbered_master("ODD"))
        .even_master(numbered_master("EVEN"));
    let mut flow = FlowDocument::from_section(section).unwrap();
    flow.add_page_break();

    let once = materialize(flow.builder()).unwrap();
    let twice = materialize(&once).unwrap();
    assert_eq!(words(&once.pages[0]), words(&twice.pages[0]));
    assert_eq!(words(&once.pages[1]), words(&twice.pages[1]));
    assert_eq!(once.pages[0].commands.len(), twice.pages[0].commands.len());
    assert_eq!(once.pages[1].commands.len(), twice.pages[1].commands.len());
    assert!(flow
        .builder
        .pages
        .iter()
        .all(|page| page.commands.is_empty()));
}

#[test]
fn invalid_section_transition_rolls_back_before_mutation() {
    let mut flow = FlowDocument::new(PageSize::custom(200.0, 200.0), Margins::all(30.0));
    let pages = flow.builder.pages.len();
    let sections = flow.builder.sections.len();
    let invalid = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(120.0));
    assert!(flow
        .start_section_on(invalid, FlowPageBreak::NextOddPage)
        .is_err());
    assert_eq!(flow.builder.pages.len(), pages);
    assert_eq!(flow.builder.sections.len(), sections);
    assert_eq!(flow.current_page, 0);
    assert_eq!(flow.current_section, 0);
}

#[test]
fn default_single_section_omits_redundant_page_labels() {
    let mut flow = FlowDocument::new(PageSize::custom(200.0, 200.0), Margins::all(30.0));
    flow.add_page_break();
    assert!(page_labels(flow.builder()).unwrap().is_none());
}

#[test]
fn post_layout_section_geometry_changes_fail_closed() {
    let mut flow = FlowDocument::new(PageSize::custom(200.0, 200.0), Margins::all(30.0));
    flow.current_section_mut().page_size = PageSize::custom(300.0, 200.0);
    assert!(materialize(flow.builder()).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn unrepresentable_roman_running_number_fails_without_partial_paint() {
    let master = SectionPageMaster::new().footer(running(
        vec![RunningTextPart::Field(PageNumberField::SectionPage)],
        TextAlign::Center,
    ));
    let section = FlowSection::new(PageSize::custom(200.0, 200.0), Margins::all(30.0))
        .odd_master(master)
        .page_numbering(4000, PageNumberStyle::UpperRoman);
    let flow = FlowDocument::from_section(section).unwrap();
    assert!(materialize(flow.builder()).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn mirrored_section_margins_follow_physical_page_sides_for_every_continuation() {
    let margins = Margins {
        left: 40.0,
        right: 20.0,
        top: 30.0,
        bottom: 24.0,
    };
    let section = FlowSection::new(PageSize::custom(220.0, 200.0), margins).mirrored_margins(true);
    let mut flow = FlowDocument::from_section(section).unwrap();
    flow.add_page_break().add_page_break();
    assert_eq!(flow.builder.pages[0].margins, margins);
    assert_eq!(
        flow.builder.pages[1].margins,
        Margins {
            left: 20.0,
            right: 40.0,
            top: 30.0,
            bottom: 24.0,
        }
    );
    assert_eq!(flow.builder.pages[2].margins, margins);
    assert_eq!(flow.margins, margins);
    assert!(materialize(flow.builder()).is_ok());
}
