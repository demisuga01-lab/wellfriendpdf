use std::path::PathBuf;

use wellfriendpdf_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};
use wellfriendpdf_engine::{
    canonicalize_pdf, encrypt, sanitize_pdf, security_report, validate_standards_profile, Color,
    ContentEngine, PdfBuilder, SanitizerOptions, SignatureValidity, StandardFont, StandardsProfile,
    ValidationStatus,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn authored_pdf() -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    let style = wellfriendpdf_engine::TextStyle::standard(StandardFont::Helvetica, 12.0)
        .fill(Color::device_rgb(0.1, 0.1, 0.1));
    let page = doc.add_page(wellfriendpdf_engine::authoring::PageSize::LETTER);
    page.draw_text("Prompt 09 security smoke", 72.0, 720.0, &style)
        .unwrap();
    doc.to_bytes().unwrap()
}

fn risky_pdf() -> Vec<u8> {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /OpenAction 5 0 R /Names << /EmbeddedFiles << /Names [(payload.txt) 7 0 R] >> >> /AcroForm << /XFA 9 0 R /AA << /K 5 0 R >> >> /Metadata 10 0 R >>".to_string(),
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [8 0 R] /Contents 4 0 R >>".to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        "<< /Type /Action /S /JavaScript /JS (app.alert('x')) >>".to_string(),
        "<< /Type /EmbeddedFile /Length 6 >>\nstream\nsecret\nendstream".to_string(),
        "<< /Type /Filespec /F (payload.txt) /EF << /F 6 0 R >> >>".to_string(),
        "<< /Type /Annot /Subtype /FileAttachment /Rect [10 10 20 20] /FS 7 0 R /A << /S /Launch /F (calc.exe) >> >>".to_string(),
        "(<xfa>dynamic</xfa>)".to_string(),
        "<< /Type /Metadata /Subtype /XML /Length 11 >>\nstream\n<xmp></xmp>\nendstream".to_string(),
    ];
    build_pdf(objects)
}

fn build_pdf(objects: Vec<String>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (idx, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", idx + 1, object).as_bytes());
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

#[test]
fn aes256_security_report_is_explicit_about_permissions() {
    let engine = ContentEngine::open_bytes(authored_pdf()).unwrap();
    let encrypted = encrypt(
        &engine,
        &EncryptParams {
            user_password: secret_bytes(b"user".to_vec()),
            owner_password: secret_bytes(b"owner".to_vec()),
            algorithm: EncryptAlgorithm::Aes256,
            permissions: -44,
            ..Default::default()
        },
    )
    .unwrap();
    let opened = ContentEngine::open_bytes_with_password(encrypted, b"user").unwrap();
    let report = security_report(&opened).unwrap();
    assert!(report.encrypted);
    assert_eq!(report.encryption.as_ref().unwrap().algorithm, "AES-256");
    assert!(report.permissions_note.contains("viewer-enforced"));
    assert!(!report.public_key_security_handler_detected);
    assert!(!report.aes_gcm_supported);
}

#[test]
fn signature_report_separates_byte_range_digest_cms_and_ltv_status() {
    let bytes = std::fs::read(fixture("sig_valid.pdf")).unwrap();
    let engine = ContentEngine::open_bytes(bytes).unwrap();
    let report = &engine.verify_signatures().unwrap()[0];
    assert_eq!(report.validity, SignatureValidity::Valid);
    assert!(report.checks.byte_range_present);
    assert!(report.checks.byte_range_well_formed);
    assert!(report.checks.byte_range_in_bounds);
    assert!(report.checks.byte_range_non_overlapping);
    assert!(report.checks.byte_range_covers_whole_file);
    assert!(report.checks.contents_present);
    assert!(report.checks.digest_matches);
    assert!(report.checks.cms_verified);
    assert!(!report.checks.chain_verified);
    assert!(!report.checks.timestamp_verified);
    assert!(!report.checks.ltv_verified);
    assert!(report.checks.signed_bytes > 0);
}

#[test]
fn strict_sanitizer_removes_active_content_payloads_and_metadata() {
    let engine = ContentEngine::open_bytes(risky_pdf()).unwrap();
    let before = security_report(&engine).unwrap();
    assert!(before.risky_content.risky_total() > 0);

    let (sanitized, report) = sanitize_pdf(&engine, &SanitizerOptions::strict()).unwrap();
    assert!(report.strict_passed, "{report:#?}");
    assert_eq!(report.output_risky_total, 0);
    assert!(report.removed.contains_key("action_object"));
    assert!(report.removed.contains_key("embedded_file"));
    assert!(report.removed.contains_key("file_attachment_annotation"));

    let reopened = ContentEngine::open_bytes(sanitized).unwrap();
    let after = security_report(&reopened).unwrap();
    assert_eq!(after.risky_content.risky_total(), 0);
}

#[test]
fn standards_profiles_report_supported_subsets_without_certification_claim() {
    let engine = ContentEngine::open_bytes(risky_pdf()).unwrap();
    let report = validate_standards_profile(&engine, StandardsProfile::All).unwrap();
    assert!(!report.certification_claimed);
    assert!(report
        .rules
        .iter()
        .any(|rule| rule.profile == "arlington" && rule.status == ValidationStatus::Pass));
    assert!(report
        .rules
        .iter()
        .any(|rule| rule.rule_id == "security.active_content"
            && rule.status == ValidationStatus::Fail));
    assert!(report
        .rules
        .iter()
        .any(|rule| rule.rule_id == "pdfx.output_intent" && rule.status == ValidationStatus::Fail));
}

#[test]
fn canonicalize_is_deterministic_and_reports_signature_impact() {
    let engine = ContentEngine::open_bytes(authored_pdf()).unwrap();
    let (first, first_report) = canonicalize_pdf(&engine, &Default::default()).unwrap();
    let (second, second_report) = canonicalize_pdf(&engine, &Default::default()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first_report.output_sha256, second_report.output_sha256);
    assert!(first_report.deterministic);
    assert_eq!(first_report.signature_impact, "no_signatures_detected");
}

#[test]
fn metamorphic_sanitize_preserves_safe_text() {
    let engine = ContentEngine::open_bytes(authored_pdf()).unwrap();
    let original_text = engine.get_page_text(1).unwrap();
    let (sanitized, report) = sanitize_pdf(&engine, &SanitizerOptions::balanced()).unwrap();
    assert!(report.strict_passed);
    let reopened = ContentEngine::open_bytes(sanitized).unwrap();
    assert_eq!(reopened.get_page_text(1).unwrap(), original_text);
}
