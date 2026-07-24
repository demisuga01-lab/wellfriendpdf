use wellfriendpdf_engine::prompt17::{
    NonAxisRedactionFallbackPolicy, NonAxisRedactionOptions, NonAxisRedactionRequest,
    RedactionCoordinateSpace,
};
use wellfriendpdf_engine::prompt18::{
    analyze_edit_policy_for_target, apply_signature_preserving_form_fill, associated_files_add_pdf,
    associated_files_inventory, associated_files_remove_owner_pdf,
    associated_files_update_owner_pdf, incremental_annotation_update_pdf,
    incremental_form_value_update_pdf, incremental_page_property_update_pdf,
    plan_signature_preserving_form_fill, AfRelationship, AssociatedFileAddRequest,
    AssociatedFileOwnerRemoveRequest, AssociatedFileOwnerType, AssociatedFileOwnerUpdateRequest,
    EditOperation, EditPolicyDecision, IncrementalAnnotationEdit, IncrementalPagePropertyEdit,
};
use wellfriendpdf_engine::{
    decode_stream_lossless, flate_encode, ContentEngine, PdfObject, VerifyOptions,
};

fn export_fixture(name: &str, bytes: &[u8]) {
    let Some(root) = std::env::var_os("WELLFRIENDPDF_PROMPT18B_EXPORT_FIXTURES") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(name), bytes).unwrap();
}

struct PdfBuilder {
    objects: Vec<Vec<u8>>,
    info: usize,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
            info: 0,
        }
    }

    fn add(&mut self, body: impl AsRef<[u8]>) -> usize {
        self.objects.push(body.as_ref().to_vec());
        self.objects.len()
    }

    fn stream(&mut self, dictionary: &str, bytes: &[u8]) -> usize {
        let mut body = format!("<< {dictionary} /Length {} >>\nstream\n", bytes.len()).into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\nendstream");
        self.add(body)
    }

    fn info(&mut self, body: &str) {
        self.info = self.add(body);
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R /ID [<18B1><18B1>] >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1,
                self.info
            )
            .as_bytes(),
        );
        pdf
    }
}

fn image_fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 440 100] /Resources << /XObject << /St 5 0 R /Idx 6 0 R /Icc 7 0 R >> >> /Contents 4 0 R >>",
    );
    let predicted = flate_encode(&[0, 10, 20], 9);
    let mut content = b"q 100 0 0 100 0 0 cm /St Do Q\nq 100 0 0 100 110 0 cm /Idx Do Q\nq 100 0 0 100 220 0 cm /Icc Do Q\nq 100 0 0 100 330 0 cm BI /W 2 /H 1 /CS /G /BPC 8 /F /Fl /DP << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns 2 >> ID\n".to_vec();
    content.extend_from_slice(&predicted);
    content.extend_from_slice(b"\nEI\nQ\n");
    builder.stream("", &content);
    builder.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 1 /ImageMask true /BitsPerComponent 1 /Decode [0 1]",
        &[0xF0],
    );
    builder.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 1 /ColorSpace [/Indexed /DeviceRGB 3 <FF000000FF000000FF000000>] /BitsPerComponent 2",
        &[0x1B],
    );
    builder.stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 1 /ColorSpace [/ICCBased 8 0 R] /BitsPerComponent 8",
        &[200, 10, 10, 10, 200, 10, 10, 10, 200, 120, 120, 120],
    );
    builder.stream("/N 3 /Alternate /DeviceRGB", b"bounded-profile-placeholder");
    builder.info("<< /Title (Prompt 18B images) >>");
    builder.build()
}

fn redaction(x0: f64, x1: f64) -> NonAxisRedactionRequest {
    NonAxisRedactionRequest {
        page: 1,
        polygon: vec![[x0, 0.0], [x1, 0.0], [x1, 100.0], [x0, 100.0]],
        coordinate_space: RedactionCoordinateSpace::PdfUserSpace,
        fallback_policy: NonAxisRedactionFallbackPolicy::FailIfNoSampleRewrite,
        fill: vec![0.0, 0.0, 0.0],
    }
}

#[test]
fn packed_indexed_iccbased_predictor_and_promotion_are_executable() {
    let input = image_fixture();
    export_fixture("advanced-input.pdf", &input);
    let options = NonAxisRedactionOptions {
        requests: vec![
            redaction(0.0, 50.0),
            redaction(110.0, 160.0),
            redaction(220.0, 270.0),
            redaction(330.0, 380.0),
        ],
        deterministic: true,
        fail_on_unsupported: true,
        promote_inline_images: true,
        signature_policy_override: false,
    };
    let (output, report) =
        wellfriendpdf_engine::redact_masked_images_pdf(&input, &options).unwrap();
    export_fixture("advanced-promoted.pdf", &output);
    let (again, _) = wellfriendpdf_engine::redact_masked_images_pdf(&input, &options).unwrap();
    assert_eq!(output, again);
    assert_eq!(report.security_proof_failures, 0);
    let engine = ContentEngine::open_bytes(output).unwrap();
    let images = engine.find_page_images(1).unwrap();
    assert!(images.iter().all(|image| !image.is_inline));
    assert!(images
        .iter()
        .any(|image| image.xobject_name.starts_with("OxP18Inline")));

    let reader = engine.document().reader();
    let mut saw_stencil = false;
    let mut saw_indexed = false;
    let mut saw_icc = false;
    let mut saw_predictor = false;
    for image in images
        .iter()
        .filter(|image| image.xobject_name.starts_with("OxP"))
    {
        let object = reader
            .get_object(image.object_number, image.generation_number)
            .unwrap();
        let dict = object.as_stream().unwrap().0;
        let decoded = decode_stream_lossless(&object, reader).unwrap().data;
        if dict.get_bool("ImageMask") == Some(true) {
            saw_stencil = true;
            assert_eq!(dict.get_integer("BitsPerComponent"), Some(1));
            assert_eq!(decoded, vec![0x30]);
        } else if dict
            .get("ColorSpace")
            .and_then(PdfObject::as_array)
            .and_then(|items| items.first())
            .and_then(PdfObject::as_name)
            == Some("Indexed")
        {
            saw_indexed = true;
            assert_eq!(dict.get_integer("BitsPerComponent"), Some(2));
            assert_eq!(decoded, vec![0xFB]);
        } else if dict
            .get("ColorSpace")
            .and_then(PdfObject::as_array)
            .and_then(|items| items.first())
            .and_then(PdfObject::as_name)
            == Some("ICCBased")
        {
            saw_icc = true;
            assert_eq!(&decoded[..6], &[0, 0, 0, 0, 0, 0]);
        } else if dict.get("DecodeParms").is_some() {
            saw_predictor = true;
            assert_eq!(decoded, vec![0, 20]);
        }
    }
    assert!(saw_stencil && saw_indexed && saw_icc && saw_predictor);

    let mut direct_options = options.clone();
    direct_options.promote_inline_images = false;
    let (direct, _) =
        wellfriendpdf_engine::redact_masked_images_pdf(&input, &direct_options).unwrap();
    export_fixture("advanced-direct.pdf", &direct);
    let promoted_render = engine.render_page(1, 72).unwrap().to_raw_image();
    let direct_render = ContentEngine::open_bytes(direct)
        .unwrap()
        .render_page(1, 72)
        .unwrap()
        .to_raw_image();
    assert_eq!(promoted_render.width, direct_render.width);
    assert_eq!(promoted_render.height, direct_render.height);
    assert_eq!(promoted_render.channels, direct_render.channels);
    assert_eq!(promoted_render.pixels, direct_render.pixels);
}

fn owners_fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Fm 6 0 R >> >> /Annots [5 0 R] /Contents 4 0 R >>");
    builder.stream("", b"q /Fm Do Q");
    builder.add("<< /Type /Annot /Subtype /FileAttachment /Rect [0 0 10 10] >>");
    builder.stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << >>",
        b"",
    );
    builder.add("<< /Type /StructElem /S /P /Pg 3 0 R >>");
    builder.info("<< /Title (Prompt 18B owners) >>");
    builder.build()
}

#[test]
fn owner_specific_add_update_remove_preserves_relationship_and_cleans_orphans() {
    let cases = [
        (AssociatedFileOwnerType::Catalog, None),
        (AssociatedFileOwnerType::Page, Some("3-0")),
        (AssociatedFileOwnerType::Annotation, Some("5-0")),
        (AssociatedFileOwnerType::StructureElement, Some("7-0")),
        (AssociatedFileOwnerType::FormXObject, Some("6-0")),
    ];
    for (owner, owner_ref) in cases {
        let request = AssociatedFileAddRequest {
            filename: "owner.dat".to_string(),
            description: Some("owner proof".to_string()),
            mime: "application/octet-stream".to_string(),
            relationship: Some(AfRelationship::Supplement),
            owner: Some(owner),
            owner_ref: owner_ref.map(str::to_string),
            deterministic: true,
            signature_policy_override: false,
        };
        let (output, report) =
            associated_files_add_pdf(&owners_fixture(), &request, b"owner-one").unwrap();
        assert!(report.output_reopened);
        let engine = ContentEngine::open_bytes(output).unwrap();
        let inventory = associated_files_inventory(&engine).unwrap();
        assert!(inventory.records.iter().any(|record| {
            record.owner_type == owner && record.relationship == AfRelationship::Supplement
        }));
    }

    let add = AssociatedFileAddRequest {
        filename: "page.dat".to_string(),
        description: Some("page".to_string()),
        mime: "application/octet-stream".to_string(),
        relationship: Some(AfRelationship::Data),
        owner: Some(AssociatedFileOwnerType::Page),
        owner_ref: Some("3-0".to_string()),
        deterministic: true,
        signature_policy_override: false,
    };
    let (added, _) = associated_files_add_pdf(&owners_fixture(), &add, b"first").unwrap();
    let inventory =
        associated_files_inventory(&ContentEngine::open_bytes(added.clone()).unwrap()).unwrap();
    let record = inventory
        .records
        .iter()
        .find(|record| record.owner_type == AssociatedFileOwnerType::Page)
        .unwrap();
    let update = AssociatedFileOwnerUpdateRequest {
        stable_id: record.stable_id.clone(),
        owner: AssociatedFileOwnerType::Page,
        owner_ref: record.owner_ref.clone(),
        filename: Some("page-updated.dat".to_string()),
        description: None,
        mime: None,
        relationship: None,
        signature_policy_override: false,
    };
    let (updated, _) = associated_files_update_owner_pdf(&added, &update, b"second").unwrap();
    let updated_inventory =
        associated_files_inventory(&ContentEngine::open_bytes(updated.clone()).unwrap()).unwrap();
    let updated_record = updated_inventory
        .records
        .iter()
        .find(|record| record.owner_type == AssociatedFileOwnerType::Page)
        .unwrap();
    assert_eq!(updated_record.relationship, AfRelationship::Data);
    assert_eq!(updated_record.filename, "page-updated.dat");
    let remove = AssociatedFileOwnerRemoveRequest {
        stable_id: updated_record.stable_id.clone(),
        owner: AssociatedFileOwnerType::Page,
        owner_ref: updated_record.owner_ref.clone(),
        signature_policy_override: false,
    };
    let (removed, _) = associated_files_remove_owner_pdf(&updated, &remove).unwrap();
    let final_inventory =
        associated_files_inventory(&ContentEngine::open_bytes(removed).unwrap()).unwrap();
    assert!(final_inventory
        .records
        .iter()
        .all(|record| record.owner_type != AssociatedFileOwnerType::Page));
}

fn signed_mutation_fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R /AcroForm 8 0 R /Perms << /DocMDP 9 0 R >> >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Annots [5 0 R 6 0 R] /Contents 4 0 R >>");
    builder.stream("", b"");
    builder.add(
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Open) /V (old) /Rect [0 0 40 10] /P 3 0 R >>",
    );
    builder.add("<< /Type /Annot /Subtype /Widget /FT /Tx /T (Locked) /V (old) /Rect [0 20 40 30] /P 3 0 R >>");
    builder.add("<< /FT /Sig /T (Certification) /V 9 0 R >>");
    builder.add("<< /Fields [5 0 R 6 0 R 7 0 R] /SigFlags 3 >>");
    builder.add("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] /Contents <00> /Reference [10 0 R 11 0 R] >>");
    builder.add("<< /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type /TransformParams /P 3 /V /1.2 >> >>");
    builder.add("<< /Type /SigRef /TransformMethod /FieldMDP /TransformParams << /Type /TransformParams /Action /Include /Fields [(Locked)] /V /1.2 >> >>");
    builder.info("<< /Title (Prompt 18B signed edits) >>");
    builder.build()
}

#[test]
fn signature_policy_executes_allowed_incremental_edits_and_blocks_prohibited_targets() {
    let input = signed_mutation_fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    assert_eq!(
        analyze_edit_policy_for_target(&engine, EditOperation::FormValueUpdate, Some("Locked"))
            .unwrap()
            .decision,
        EditPolicyDecision::BlockedBySignaturePolicy
    );
    assert!(incremental_form_value_update_pdf(&input, "Locked", "no", false).is_err());
    let (form, form_report) =
        incremental_form_value_update_pdf(&input, "Open", "allowed", false).unwrap();
    assert!(form.starts_with(&input));
    assert!(form_report.visible_after_reopen);
    assert!(!form_report.cryptographic_validity_claimed);

    let annotation = IncrementalAnnotationEdit::AddTextNote {
        page: 1,
        rect: [50.0, 50.0, 20.0, 20.0],
        contents: "allowed annotation".to_string(),
    };
    let (annotated, annotation_report) =
        incremental_annotation_update_pdf(&input, &annotation, false).unwrap();
    assert!(annotated.starts_with(&input));
    assert!(annotation_report.visible_after_reopen);

    assert!(incremental_page_property_update_pdf(
        &input,
        &IncrementalPagePropertyEdit::Rotate {
            page: 1,
            degrees: 90
        },
        false,
    )
    .is_err());
    let unsigned = owners_fixture();
    let (rotated, rotation_report) = incremental_page_property_update_pdf(
        &unsigned,
        &IncrementalPagePropertyEdit::Rotate {
            page: 1,
            degrees: 90,
        },
        false,
    )
    .unwrap();
    assert!(rotated.starts_with(&unsigned));
    assert!(rotation_report.visible_after_reopen);
}

#[test]
fn prompt25_signature_preserving_form_fill_plans_applies_and_revalidates() {
    let input = signed_mutation_fixture();
    let options = VerifyOptions::default();
    let blocked =
        plan_signature_preserving_form_fill(&input, "Locked", "denied", &options).unwrap();
    assert!(!blocked.allowed);
    assert_eq!(
        blocked.decision,
        EditPolicyDecision::BlockedBySignaturePolicy
    );

    let plan = plan_signature_preserving_form_fill(&input, "Open", "allowed", &options).unwrap();
    assert!(plan.allowed);
    assert_eq!(plan.decision, EditPolicyDecision::IncrementalWithWarning);
    assert_eq!(plan.before_signature_count, plan.before_signatures.len());

    let (output, result) =
        apply_signature_preserving_form_fill(&input, "Open", "allowed", &options, false).unwrap();
    assert!(output.starts_with(&input));
    assert!(result.post_edit.original_prefix_preserved);
    assert_eq!(
        result.post_edit.before_signature_count,
        result.plan.before_signature_count
    );
    assert_eq!(
        result.post_edit.after_signature_count,
        result.post_edit.post_edit_signatures.len()
    );
    assert!(
        !result
            .post_edit
            .original_signatures_mathematically_valid_after_edit,
        "the fixture signature is intentionally non-cryptographic; Prompt 25 must report, not fake, preservation"
    );
}

#[test]
fn prompt18b_public_report_has_zero_blocked_or_security_failures() {
    let report: serde_json::Value = serde_json::from_str(
        &wellfriendpdf_engine::sdk::prompt18b_report_json(&owners_fixture(), None).unwrap(),
    )
    .unwrap();
    assert_eq!(report["report"]["closure"]["failure"]["blocked"], 0);
    assert_eq!(report["report"]["closure"]["failure"]["security_proof"], 0);
    let feature: serde_json::Value =
        serde_json::from_str(&wellfriendpdf_engine::sdk::feature_report_json().unwrap()).unwrap();
    assert_eq!(
        feature["report"]["prompt18b_advanced_secure_mutation_closure"]["status"],
        "complete_with_exact_limits"
    );
}
