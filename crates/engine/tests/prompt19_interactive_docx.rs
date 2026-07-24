use std::path::PathBuf;
use wellfriendpdf_engine::{
    flatten_calculated_values_pdf, form_action_graph, form_javascript_inventory,
    form_js_sanitize_pdf, word_pagination_audit, Color, ContentEngine, DocxLayout, FormJsLimits,
    FormJsPolicyMode, FormJsSanitizerOptions, PdfBuilder, StandardFont,
};

fn build_pdf(objects: Vec<String>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn action_fixture() -> Vec<u8> {
    build_pdf(vec![
        "<< /Type /Catalog /Pages 2 0 R /OpenAction 8 0 R /Names << /JavaScript 9 0 R >> /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /AA << /O 11 0 R >> /Annots [12 0 R] /Contents 4 0 R >>".to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        "<< /Fields [6 0 R 7 0 R] /CO [7 0 R] >>".to_string(),
        "<< /FT /Tx /T (A) /V (2) >>".to_string(),
        "<< /FT /Tx /T (Total) /V (0) /AA << /C 10 0 R /V 13 0 R >> >>".to_string(),
        "<< /Type /Action /S /JavaScript /JS (app.alert\\(\\'open\\'\\)) /Next [14 0 R 15 0 R] >>".to_string(),
        "<< /Names [(DocumentScript) 8 0 R] >>".to_string(),
        "<< /Type /Action /S /JavaScript /JS (event.value = this.getField\\(\\\"A\\\"\\).value * 2;) >>".to_string(),
        "<< /Type /Action /S /URI /URI (https://example.invalid/) >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 10 40 30] /A << /S /GoTo /D [3 0 R /Fit] >> >>".to_string(),
        "<< /Type /Action /S /JavaScript /JS (event.rc = this.getField\\(\\\"A\\\"\\).value > 0;) >>".to_string(),
        "<< /Type /Action /S /Launch /F (calc.exe) /Next 8 0 R >>".to_string(),
        "<< /Type /Action /S /Named /N /NextPage >>".to_string(),
    ])
}

#[test]
fn form_javascript_inventory_graph_and_rescan_are_real_pdf_backed() {
    let bytes = action_fixture();
    let engine = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let inventory = form_javascript_inventory(&engine, &FormJsLimits::default()).unwrap();
    assert!(inventory.script_count >= 3);
    assert!(inventory.action_count_by_type.contains_key("JavaScript"));
    assert!(inventory.action_count_by_type.contains_key("Launch"));
    assert!(inventory.action_count_by_type.contains_key("URI"));
    assert!(inventory
        .actions
        .iter()
        .any(|action| action.event == "calculate"));
    assert!(inventory.actions.iter().any(|action| action
        .diagnostic
        .as_deref()
        .is_some_and(|value| value.contains("cyclic"))));

    let graph = form_action_graph(&engine, &inventory).unwrap();
    assert_eq!(graph.calculation_order, vec!["Total"]);
    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.from_field == "A" && edge.to_field == "Total"));

    let options = FormJsSanitizerOptions {
        mode: FormJsPolicyMode::RemoveAllActiveActions,
        ..Default::default()
    };
    let (sanitized, report) = form_js_sanitize_pdf(&bytes, &options).unwrap();
    assert!(report.rescan_passed, "{report:#?}");
    assert_eq!(report.forbidden_remaining_count, 0);
    let reopened = ContentEngine::open_bytes(sanitized).unwrap();
    let rescan = form_javascript_inventory(&reopened, &FormJsLimits::default()).unwrap();
    assert!(rescan.actions.is_empty(), "{:#?}", rescan.actions);
}

#[test]
fn safe_calculation_flatten_updates_value_then_removes_scripts() {
    let bytes = action_fixture();
    let options = FormJsSanitizerOptions {
        mode: FormJsPolicyMode::FlattenCalculatedValuesThenRemove,
        ..Default::default()
    };
    let (flattened, report) = flatten_calculated_values_pdf(&bytes, &options).unwrap();
    assert_eq!(report.values_updated, 1, "{report:#?}");
    assert!(report
        .results
        .iter()
        .any(|result| result.target_field.as_deref() == Some("Total")
            && result.calculated_value.as_deref() == Some("4")));
    let reopened = ContentEngine::open_bytes(flattened).unwrap();
    let rescan = form_javascript_inventory(&reopened, &FormJsLimits::default()).unwrap();
    assert!(rescan.actions.is_empty());
    let forms = wellfriendpdf_engine::forms_report(&reopened).unwrap();
    assert!(forms
        .fields
        .iter()
        .any(|field| field.full_name == "Total" && field.value.as_deref() == Some("4")));
}

#[test]
fn docx_page_faithful_emits_mixed_size_sections_deterministically() {
    let mut builder = PdfBuilder::new();
    let style = wellfriendpdf_engine::TextStyle::standard(StandardFont::Helvetica, 12.0)
        .fill(Color::device_rgb(0.1, 0.1, 0.1));
    builder
        .add_page(wellfriendpdf_engine::authoring::PageSize::custom(
            300.0, 400.0,
        ))
        .draw_text("Portrait page", 24.0, 360.0, &style)
        .unwrap();
    builder
        .add_page(wellfriendpdf_engine::authoring::PageSize::custom(
            500.0, 280.0,
        ))
        .draw_text("Landscape page", 24.0, 240.0, &style)
        .unwrap();
    let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let report = word_pagination_audit(&engine, DocxLayout::PageFaithful).unwrap();
    assert_eq!(report.page_count, 2);
    assert_eq!(report.section_count, 2);
    assert_eq!(report.page_sizes_twips, vec![[6000, 8000], [10000, 5600]]);
    assert!(report.text_box_count >= 2);
    assert!(report.readback_ok);
    assert!(report.deterministic_repeat_match);
}

#[test]
fn docx_hyperlinks_use_real_relationships_when_parse_model_has_links() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("basicapi.pdf");
    let engine = ContentEngine::open_path(fixture).unwrap();
    let report = word_pagination_audit(&engine, DocxLayout::Hybrid).unwrap();
    assert!(report.hyperlink_count >= 1, "{report:#?}");
}
