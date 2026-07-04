use oxide_engine::{
    annotation_report, apply_form_data_pdf, export_form_data, forms_report, page_operations_report,
    parse_form_data, redaction_verification_report, AttachmentRedactionPolicy,
    AuthorPageSize as PageSize, ContentEngine, EditMode, FormDataField, FormDataFormat,
    FormDataSet, ImageRect, PdfBuilder, PdfEditor, RedactionOptions, StandardFont,
    TextSearchOptions, TextStyle,
};

struct PdfFixtureBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfFixtureBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: &str) -> usize {
        self.objects.push(body.as_bytes().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, stream: &[u8]) -> usize {
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
        self.objects.len()
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (idx, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref_start = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                self.objects.len() + 1,
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }
}

fn interactive_fixture_pdf() -> Vec<u8> {
    let mut b = PdfFixtureBuilder::new();
    b.add(
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 8 0 R /Outlines 9 0 R \
         /PageLabels 11 0 R \
         /Names << /Dests << /Names [(home) [3 0 R /Fit]] >> \
                  /EmbeddedFiles << /Names [(note.txt) 13 0 R] >> >> >>",
    );
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
         /Resources << /Font << /F1 15 0 R >> >> /Contents 4 0 R \
         /Annots [5 0 R 6 0 R 7 0 R] >>",
    );
    b.add_stream(b"BT /F1 10 Tf 1 0 0 1 40 150 Tm (Visible form page) Tj ET\n");
    b.add("<< /Type /Annot /Subtype /Widget /Rect [60 120 240 140] /F 4 /AP << /N 4 0 R >> >>");
    b.add(
        "<< /Type /Annot /Subtype /Link /Rect [40 40 120 60] \
         /A << /S /JavaScript /JS (app.alert('unsafe')) >> >>",
    );
    b.add(
        "<< /Type /Annot /Subtype /Highlight /Rect [40 150 140 165] /Contents (marked) \
         /C [1 1 0] /QuadPoints [40 165 140 165 40 150 140 150] >>",
    );
    b.add(
        "<< /Fields [10 0 R] /NeedAppearances true /SigFlags 3 /CO [16 0 R] \
         /XFA [(template) 12 0 R] >>",
    );
    b.add("<< /First 14 0 R /Last 14 0 R /Count 1 >>");
    b.add("<< /T (parent) /FT /Tx /DA (/F1 10 Tf 0 g) /Kids [16 0 R] >>");
    b.add("<< /Nums [0 << /S /D /St 1 >>] >>");
    b.add_stream(b"<template xmlns=\"http://www.xfa.org/schema/xfa-template/2.8/\"/>");
    b.add("<< /Type /Filespec /F (note.txt) /UF (note.txt) >>");
    b.add("<< /Title (Start) /Parent 9 0 R /Dest [3 0 R /Fit] >>");
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    b.add("<< /T (child) /Parent 10 0 R /V (Alice) /DV (Default) /Kids [5 0 R] >>");
    b.build()
}

#[test]
fn forms_report_merges_field_tree_inheritance_widgets_and_xfa() {
    let engine = ContentEngine::open_bytes(interactive_fixture_pdf()).unwrap();
    let report = forms_report(&engine).unwrap();

    assert!(report.has_acroform);
    assert!(report.need_appearances);
    assert_eq!(report.sig_flags, Some(3));
    assert_eq!(report.calculation_order_len, 1);
    assert!(report.xfa.present);
    assert!(!report.xfa.supported);

    let field = report
        .fields
        .iter()
        .find(|field| field.full_name == "parent.child")
        .expect("inherited child field");
    assert_eq!(field.field_type, "text");
    assert_eq!(field.value.as_deref(), Some("Alice"));
    assert!(field
        .attributes
        .iter()
        .any(|attr| attr.name == "FT" && attr.inherited));
    assert!(field
        .widgets
        .iter()
        .any(|widget| widget.page == Some(1) && widget.has_appearance));
    assert!(report
        .diagnostics
        .iter()
        .any(|diag| diag.code == "form.xfa.detected"));
}

#[test]
fn fdf_xfdf_form_exchange_roundtrips_field_values() {
    let source = interactive_fixture_pdf();
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();

    let fdf = export_form_data(&engine, FormDataFormat::Fdf).unwrap();
    let parsed = parse_form_data(&fdf, FormDataFormat::Fdf).unwrap();
    assert!(parsed
        .fields
        .iter()
        .any(|field| field.name == "parent.child" && field.value == "Alice"));

    let xfdf = export_form_data(&engine, FormDataFormat::Xfdf).unwrap();
    let xfdf_text = String::from_utf8(xfdf).unwrap();
    assert!(xfdf_text.contains("<field name=\"parent.child\""));

    let update = serde_json::to_vec(&FormDataSet {
        fields: vec![FormDataField {
            name: "parent.child".to_string(),
            value: "Bob".to_string(),
        }],
    })
    .unwrap();
    let (filled, report) = apply_form_data_pdf(source, &update, FormDataFormat::Json).unwrap();
    assert_eq!(report.applied_fields, 1);
    let filled_report = forms_report(&ContentEngine::open_bytes(filled).unwrap()).unwrap();
    let field = filled_report
        .fields
        .iter()
        .find(|field| field.full_name == "parent.child")
        .unwrap();
    assert_eq!(field.value.as_deref(), Some("Bob"));
}

#[test]
fn annotations_report_classifies_quadpoints_and_unsafe_actions() {
    let engine = ContentEngine::open_bytes(interactive_fixture_pdf()).unwrap();
    let report = annotation_report(&engine).unwrap();

    assert_eq!(report.by_subtype.get("Widget"), Some(&1));
    assert_eq!(report.by_subtype.get("Link"), Some(&1));
    assert_eq!(report.by_subtype.get("Highlight"), Some(&1));
    assert_eq!(report.unsafe_actions, 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diag| diag.code == "annotation.action.unsafe"));

    let highlight = report
        .annotations
        .iter()
        .find(|annotation| annotation.subtype == "Highlight")
        .expect("highlight annotation");
    assert_eq!(highlight.quad_points.len(), 1);
    assert_eq!(highlight.contents.as_deref(), Some("marked"));

    let link = report
        .annotations
        .iter()
        .find(|annotation| annotation.subtype == "Link")
        .expect("link annotation");
    assert_eq!(link.action.as_ref().unwrap().kind, "JavaScript");
    assert!(!link.action.as_ref().unwrap().safe);
}

#[test]
fn annotation_flattening_removes_common_annotations_but_keeps_widgets() {
    let mut editor = PdfEditor::open_bytes(interactive_fixture_pdf()).unwrap();
    editor.flatten_annotations();
    let flattened = editor.save_to_bytes(EditMode::FullRewrite).unwrap();
    let report = annotation_report(&ContentEngine::open_bytes(flattened.clone()).unwrap()).unwrap();
    assert_eq!(report.by_subtype.get("Widget"), Some(&1));
    assert!(!report.by_subtype.contains_key("Highlight"));
    assert!(!report.by_subtype.contains_key("Link"));
    assert!(ContentEngine::open_bytes(flattened)
        .unwrap()
        .render_page_png_fast(1, 72)
        .is_ok());
}

#[test]
fn page_operations_report_exposes_preservation_inputs() {
    let engine = ContentEngine::open_bytes(interactive_fixture_pdf()).unwrap();
    let report = page_operations_report(&engine).unwrap();

    assert_eq!(report.page_count, 1);
    assert_eq!(report.pages[0].annotations, 3);
    assert!(report.outlines_present);
    assert_eq!(report.outline_count, 1);
    assert!(report.page_labels_present);
    assert!(report.named_destinations_present);
    assert!(report.embedded_files_present);
    assert!(report.acroform_present);
    assert!(report.signatures_may_be_invalidated_by_rewrite);
}

#[test]
fn attachment_removal_policy_drops_embedded_file_name_tree() {
    let mut editor = PdfEditor::open_bytes(interactive_fixture_pdf()).unwrap();
    editor
        .redact(
            1,
            ImageRect::new(1.0, 1.0, 10.0, 10.0),
            RedactionOptions {
                attachment_policy: AttachmentRedactionPolicy::RemoveAll,
                ..RedactionOptions::default()
            },
        )
        .unwrap();
    let scrubbed = editor.save_to_bytes(EditMode::FullRewrite).unwrap();
    let engine = ContentEngine::open_bytes(scrubbed).unwrap();
    assert!(engine.list_attachments().unwrap().is_empty());
    assert!(
        !page_operations_report(&engine)
            .unwrap()
            .embedded_files_present
    );
}

#[test]
fn semantic_search_redaction_verifies_terms_are_gone() {
    let mut doc = PdfBuilder::new();
    doc.add_page(PageSize::LETTER)
        .draw_text(
            "Public SECRET text",
            72.0,
            720.0,
            &TextStyle::standard(StandardFont::Helvetica, 20.0),
        )
        .unwrap();
    let source = doc.to_bytes().unwrap();
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    let hits = engine
        .search_text(
            &[1],
            "SECRET",
            TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                ..TextSearchOptions::default()
            },
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    let bbox = oxide_engine::TextQuad::union(&hits[0].quads).unwrap();

    let mut editor = PdfEditor::open_bytes(source).unwrap();
    editor
        .redact(
            1,
            ImageRect::new(
                bbox.x0 - 0.5,
                bbox.y0 - 0.5,
                bbox.x1 - bbox.x0 + 1.0,
                bbox.y1 - bbox.y0 + 1.0,
            ),
            RedactionOptions::default(),
        )
        .unwrap();
    let redacted = editor.save_to_bytes(EditMode::FullRewrite).unwrap();

    let verification = redaction_verification_report(&redacted, &["SECRET".to_string()]).unwrap();
    assert!(verification.verified_absent, "{verification:?}");
    let remaining = ContentEngine::open_bytes(redacted)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert!(!remaining.contains("SECRET"), "{remaining}");
    assert!(remaining.contains("Public"), "{remaining}");
}
