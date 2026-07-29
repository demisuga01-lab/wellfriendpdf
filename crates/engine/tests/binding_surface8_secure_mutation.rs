use wellfriendpdf_engine::annotation_media_redaction::{
    NonAxisRedactionFallbackPolicy, NonAxisRedactionOptions, NonAxisRedactionRequest,
    RedactionCoordinateSpace,
};
use wellfriendpdf_engine::images::decoder::ImageDecoder;
use wellfriendpdf_engine::secure_mutation::{
    analyze_edit_policy, associated_file_extract, associated_files_add_pdf,
    associated_files_inventory, associated_files_sanitize_pdf, incremental_metadata_update_pdf,
    mask_redaction_inventory, AssociatedFileAddRequest, AssociatedFileSanitizerOptions,
    AssociatedFileSanitizerPolicy, EditOperation, EditPolicyDecision,
};
use wellfriendpdf_engine::ContentEngine;

struct PdfBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
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
                "trailer\n<< /Size {} /Root 1 0 R /Info 8 0 R /ID [<1801><1801>] >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

fn fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 220 100] \
         /Resources << /XObject << /Masked 5 0 R >> >> /Contents 4 0 R >>",
    );
    let mut content = b"q 100 0 0 100 0 0 cm /Masked Do Q\nq 100 0 0 100 110 0 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID\n".to_vec();
    content.extend_from_slice(&[241, 17, 31, 11, 223, 47, 91, 103, 211, 37, 59, 181]);
    content.extend_from_slice(b"\nEI\nQ\n");
    builder.stream("", &content);
    builder.stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 6 0 R",
        &[229, 19, 43, 13, 197, 61, 89, 107, 173, 31, 71, 151],
    );
    builder.stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Matte [1 1 1]",
        &[255, 128, 64, 0],
    );
    builder.add("<< /Producer (SecureMutation fixture) >>");
    builder.add("<< /Title (SecureMutation) >>");
    builder.build()
}

fn signed_policy_fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [9 0 R] /SigFlags 3 >> /Perms << /DocMDP 10 0 R >> >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>");
    builder.stream("", b"");
    builder.add("null");
    builder.add("null");
    builder.add("null");
    builder.add("<< /Title (Signed policy) >>");
    builder.add("<< /FT /Sig /T (Certification) /V 10 0 R >>");
    builder.add("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 2 3] /Contents <00> /Reference [11 0 R 12 0 R] >>");
    builder.add("<< /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type /TransformParams /P 2 /V /1.2 >> >>");
    builder.add("<< /Type /SigRef /TransformMethod /FieldMDP /TransformParams << /Type /TransformParams /Action /Include /Fields [(Locked)] /V /1.2 >> >>");
    builder.build()
}

fn indirect_names_fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R /Names 9 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>");
    builder.stream("", b"");
    builder.add("null");
    builder.add("null");
    builder.add("null");
    builder.add("<< /Title (Indirect names) >>");
    builder.add("<< /Dests << /Names [(home) [3 0 R /Fit]] >> >>");
    builder.build()
}

fn request(x0: f64, x1: f64) -> NonAxisRedactionRequest {
    NonAxisRedactionRequest {
        page: 1,
        polygon: vec![[x0, 0.0], [x1, 0.0], [x1, 100.0], [x0, 100.0]],
        coordinate_space: RedactionCoordinateSpace::PdfUserSpace,
        fallback_policy: NonAxisRedactionFallbackPolicy::SecureRewriteOrRemove,
        fill: vec![0.0, 0.0, 0.0],
    }
}

#[test]
fn masked_and_inline_redaction_reopen_are_deterministic_and_reachable_data_is_scrubbed() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let inventory = mask_redaction_inventory(&engine).unwrap();
    assert!(inventory.rows.iter().any(|row| row.soft_mask.is_some()));

    let options = NonAxisRedactionOptions {
        requests: vec![request(0.0, 50.0), request(110.0, 160.0)],
        deterministic: true,
        fail_on_unsupported: false,
        promote_inline_images: false,
        signature_policy_override: false,
    };
    let (first, report) =
        wellfriendpdf_engine::secure_mutation::redact_masked_images_pdf(&input, &options).unwrap();
    let (second, _) =
        wellfriendpdf_engine::secure_mutation::redact_masked_images_pdf(&input, &options).unwrap();
    assert_eq!(first, second);
    assert!(report.output_reopened);
    assert_eq!(report.security_proof_failures, 0);
    assert_eq!(report.overlay_only_success_claims, 0);

    let output = ContentEngine::open_bytes(first).unwrap();
    let images = output.find_page_images(1).unwrap();
    let rewritten = images
        .iter()
        .find(|image| image.xobject_name.starts_with("OxP17RedactIm"))
        .expect("rewritten masked XObject");
    let object = output
        .document()
        .reader()
        .get_object(rewritten.object_number, rewritten.generation_number)
        .unwrap();
    let dictionary = object.as_stream().unwrap().0;
    assert!(dictionary.get("SMask").is_some());
    assert!(dictionary.get("Mask").is_none());

    let inline = images
        .iter()
        .find(|image| image.is_inline)
        .expect("inline image retained");
    let raw = ImageDecoder::decode_inline(
        &inline.inline_data.as_ref().unwrap().bytes,
        inline.width,
        inline.height,
        inline.bits_per_component,
        &inline.color_space,
        &inline.filter.iter().map(String::as_str).collect::<Vec<_>>(),
        None,
    )
    .unwrap();
    assert_eq!(&raw.pixels[0..3], &[0, 0, 0]);
    assert_eq!(&raw.pixels[3..6], &[11, 223, 47]);
}

#[test]
fn associated_file_add_extract_dedup_remove_and_rescan_are_real_mutations() {
    let input = fixture();
    let request = AssociatedFileAddRequest {
        filename: "../evidence.txt".to_string(),
        description: Some("bounded evidence".to_string()),
        mime: "text/plain".to_string(),
        relationship: Some(wellfriendpdf_engine::AfRelationship::Data),
        owner: Some(wellfriendpdf_engine::AssociatedFileOwnerType::Catalog),
        owner_ref: None,
        deterministic: true,
        signature_policy_override: false,
    };
    let (once, first_report) =
        associated_files_add_pdf(&input, &request, b"SECURE_MUTATION-EVIDENCE").unwrap();
    assert!(first_report.output_reopened);
    let (twice, second_report) =
        associated_files_add_pdf(&once, &request, b"SECURE_MUTATION-EVIDENCE").unwrap();
    assert_eq!(second_report.duplicate_streams_collapsed, 1);
    let engine = ContentEngine::open_bytes(twice.clone()).unwrap();
    let inventory = associated_files_inventory(&engine).unwrap();
    assert_eq!(
        inventory.records.iter().filter(|row| row.internal).count(),
        1
    );
    assert_eq!(
        inventory
            .records
            .iter()
            .filter(|row| row.internal)
            .filter_map(|row| row.stream_ref.as_deref())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1,
        "duplicate file specs must share one embedded stream"
    );
    let record = inventory.records.iter().find(|row| row.internal).unwrap();
    assert_eq!(record.filename, "evidence.txt");
    let (safe_name, payload) = associated_file_extract(&engine, &record.stable_id).unwrap();
    assert_eq!(safe_name, "evidence.txt");
    assert_eq!(payload, b"SECURE_MUTATION-EVIDENCE");

    let (removed, report) = associated_files_sanitize_pdf(
        &twice,
        &AssociatedFileSanitizerOptions {
            policy: AssociatedFileSanitizerPolicy::RemoveAllEmbeddedFiles,
            ..AssociatedFileSanitizerOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.after_count, 0);
    let reopened = ContentEngine::open_bytes(removed).unwrap();
    assert!(associated_files_inventory(&reopened)
        .unwrap()
        .records
        .iter()
        .all(|record| !record.internal));
}

#[test]
fn associated_file_add_preserves_unrelated_indirect_name_trees() {
    let request = AssociatedFileAddRequest {
        filename: "data.txt".to_string(),
        description: None,
        mime: "text/plain".to_string(),
        relationship: Some(wellfriendpdf_engine::AfRelationship::Data),
        owner: Some(wellfriendpdf_engine::AssociatedFileOwnerType::Catalog),
        owner_ref: None,
        deterministic: true,
        signature_policy_override: false,
    };
    let (output, _) =
        associated_files_add_pdf(&indirect_names_fixture(), &request, b"data").unwrap();
    let engine = ContentEngine::open_bytes(output).unwrap();
    let catalog = engine.document().get_catalog().unwrap();
    let names = engine
        .document()
        .reader()
        .resolve(catalog.get("Names").unwrap().clone())
        .unwrap();
    let names = names.as_dict().unwrap();
    assert!(names.get("Dests").is_some());
    assert!(catalog.get("AF").is_some());
    assert!(names.get("EmbeddedFiles").is_none());
}

#[test]
fn signature_policy_separates_structural_crypto_and_incremental_prefix() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let safe = analyze_edit_policy(&engine, EditOperation::MetadataUpdate).unwrap();
    assert_eq!(safe.decision, EditPolicyDecision::SafeIncremental);
    assert!(safe.impact.cryptographic_validity_evaluated);
    let destructive = analyze_edit_policy(&engine, EditOperation::Redaction).unwrap();
    assert_eq!(
        destructive.decision,
        EditPolicyDecision::FullRewriteRequired
    );

    let (output, report) =
        incremental_metadata_update_pdf(&input, "Subject", "SecureMutation", false).unwrap();
    assert!(output.starts_with(&input));
    assert!(report.original_prefix_preserved);
    assert!(report.byte_range_covered_bytes_untouched);
    assert!(report.signature_dictionary_untouched);
    ContentEngine::open_bytes(output).unwrap();
}

#[test]
fn docmdp_fieldmdp_are_structural_and_do_not_overclaim_crypto() {
    let engine = ContentEngine::open_bytes(signed_policy_fixture()).unwrap();
    let annotation = analyze_edit_policy(&engine, EditOperation::AnnotationAdd).unwrap();
    assert_eq!(
        annotation.decision,
        EditPolicyDecision::BlockedBySignaturePolicy
    );
    assert_eq!(annotation.structural_policies[0].docmdp_p, Some(2));
    assert_eq!(
        annotation.structural_policies[0].fieldmdp_action.as_deref(),
        Some("Include")
    );
    assert!(annotation.impact.cryptographic_validity_evaluated);
    assert!(!annotation.cryptographic_reports.is_empty());
    assert!(annotation
        .crypto_validation_requirement
        .contains("does not establish validity"));
}

#[test]
fn sdk_and_feature_report_have_secure_mutation_parity() {
    let input = fixture();
    let report: serde_json::Value = serde_json::from_str(
        &wellfriendpdf_engine::sdk::secure_mutation_report_json(&input, None).unwrap(),
    )
    .unwrap();
    assert_eq!(report["kind"], "secure_mutation_report");
    let feature: serde_json::Value =
        serde_json::from_str(&wellfriendpdf_engine::sdk::feature_report_json().unwrap()).unwrap();
    let section = &feature["report"]["secure_mutation_mask_inline_associated_signature_safe_edits"];
    assert_eq!(section["failure"]["blocked"], 0);
    assert_eq!(section["security"]["signature_crypto_overclaim"], 0);
}
