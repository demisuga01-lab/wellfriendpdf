use wellfriendpdf_engine::annotation_media_redaction::{
    apply_nonaxis_image_redaction_pdf, apply_rich_media_policy_pdf, export_annotation_xfdf,
    generate_annotation_appearances_pdf, import_annotation_xfdf_pdf, parse_annotation_xfdf,
    plan_nonaxis_image_redaction, rich_media_inventory, AnnotationAppearanceOptions,
    AnnotationConflictPolicy, AnnotationDeletePolicy, AnnotationXfdfImportOptions,
    NonAxisRedactionFallbackPolicy, NonAxisRedactionOptions, NonAxisRedactionRequest,
    RedactionCoordinateSpace, RichMediaCustomPolicy, RichMediaLimits, RichMediaPolicyMode,
};
use wellfriendpdf_engine::ContentEngine;

struct PdfFixtureBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfFixtureBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: impl AsRef<[u8]>) -> usize {
        self.objects.push(body.as_ref().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, dict: &str, stream: &[u8]) -> usize {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
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
                "trailer\n<< /Size {} /Root 1 0 R /ID [<00112233><44556677>] >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

fn fixture_pdf() -> Vec<u8> {
    let mut b = PdfFixtureBuilder::new();
    assert_eq!(b.add("<< /Type /Catalog /Pages 2 0 R >>"), 1);
    assert_eq!(b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"), 2);
    assert_eq!(
        b.add(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 220 180] /CropBox [10 10 210 170] /Rotate 90 \
             /Resources << /XObject << /Im1 5 0 R /ImStencil 16 0 R >> >> /Contents 4 0 R \
             /Annots [6 0 R 7 0 R 8 0 R 9 0 R 14 0 R 15 0 R] >>"
        ),
        3
    );
    assert_eq!(
        b.add_stream(
            "",
            b"q 80 20 -15 70 40 30 cm /Im1 Do Q\nq 20 0 0 20 180 140 cm /ImStencil Do Q\n"
        ),
        4
    );
    let pixels: Vec<u8> = (0..16)
        .flat_map(|index| [index * 7 + 20, 180 - index * 5, 80 + index * 3])
        .collect();
    assert_eq!(
        b.add_stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8",
            &pixels,
        ),
        5
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /Text /NM (note-1) /Rect [20 20 38 38] \
             /T (Alice) /Subj (Review) /Contents (hello & safe) /C [1 1 0] /Popup 7 0 R >>"
        ),
        6
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /Popup /NM (popup-1) /Rect [40 20 120 80] /Parent 6 0 R >>"
        ),
        7
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /FreeText /NM (free-1) /Rect [50 100 180 140] \
             /Contents (annotation/media redaction) /DA (/Helv 12 Tf 0 g) /C [0 0 1] /CA 0.8 >>"
        ),
        8
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /RichMedia /NM (media-1) /Rect [120 30 200 90] \
             /AP << /N 10 0 R >> /RichMediaContent 11 0 R \
             /RichMediaSettings << /Activation << /Condition /PV >> >> \
             /A << /S /Rendition /R << /S /MR /C << /S /MCD /D (https://example.invalid/media.mp4) >> >> >> >>"
        ),
        9
    );
    assert_eq!(
        b.add_stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 80 60] /Resources << >>",
            b"q 0.2 0.5 0.8 rg 0 0 80 60 re f Q\n",
        ),
        10
    );
    assert_eq!(
        b.add("<< /Type /RichMediaContent /Assets << /Names [(clip.mp4) 12 0 R] >> >>"),
        11
    );
    assert_eq!(
        b.add("<< /Type /Filespec /F (clip.mp4) /EF << /F 13 0 R >> >>"),
        12
    );
    assert_eq!(
        b.add_stream(
            "/Type /EmbeddedFile /Subtype /video#2Fmp4",
            b"UNTRUSTED-MEDIA-PAYLOAD"
        ),
        13
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /Square /NM (cloud-1) /Rect [20 82 72 132] \
             /C [0.8 0.1 0.1] /IC [1 0.9 0.7] /BS << /W 2 /S /D /D [3 2] >> \
             /BE << /S /C /I 1 >> >>"
        ),
        14
    );
    assert_eq!(
        b.add(
            "<< /Type /Annot /Subtype /Redact /NM (redact-preview-1) /Rect [78 82 190 110] \
             /Contents (REDACT) /IC [0 0 0] /C [1 1 1] /Repeat true >>"
        ),
        15
    );
    assert_eq!(
        b.add_stream(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ImageMask true /BitsPerComponent 1",
            &[0xA0, 0x50, 0xA0, 0x50],
        ),
        16
    );
    b.build()
}

#[test]
fn secure_xfdf_parser_blocks_dtd_entities_and_malformed_geometry() {
    let malicious = br#"<?xml version="1.0"?>
        <!DOCTYPE xfdf [<!ENTITY xxe SYSTEM "file:///secret">]>
        <xfdf xmlns="http://ns.adobe.com/xfdf/"><annots><text page="0" name="x"><contents>&xxe;</contents></text></annots></xfdf>"#;
    let error = parse_annotation_xfdf(malicious).unwrap_err().to_string();
    assert!(error.contains("DTD/entity declarations are forbidden"));

    let malformed = br#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>
        <square page="0" name="bad" rect="0,0,NaN,20"/>
        </annots></xfdf>"#;
    assert!(parse_annotation_xfdf(malformed).is_err());
}

#[test]
fn annotation_xfdf_roundtrip_is_stable_and_preserves_relationships() {
    let input = fixture_pdf();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (first, export) = export_annotation_xfdf(&engine).unwrap();
    let (second, _) = export_annotation_xfdf(&engine).unwrap();
    assert_eq!(first, second);
    assert_eq!(export.annotation_count, 6);
    let text = String::from_utf8(first.clone()).unwrap();
    assert!(text.contains("name=\"note-1\""));
    assert!(text.contains("popup-for=\"note-1\""));
    assert!(text.contains("hello &amp; safe"));
    assert!(text.contains("name=\"cloud-1\""));
    assert!(text.contains("style=\"D\""));
    assert!(text.contains("dashes=\"3,2\""));
    assert!(text.contains("cloudy=\"C\""));
    assert!(text.contains("name=\"redact-preview-1\""));
    assert!(text.contains("repeat=\"true\""));

    let imported = parse_annotation_xfdf(&first).unwrap();
    assert_eq!(imported.annotations.len(), 6);
    let cloudy = imported
        .annotations
        .iter()
        .find(|annotation| annotation.id == "cloud-1")
        .unwrap();
    assert_eq!(cloudy.border_style.as_deref(), Some("D"));
    assert_eq!(cloudy.border_width, Some(2.0));
    assert_eq!(cloudy.border_dash, vec![3.0, 2.0]);
    assert_eq!(cloudy.border_effect.as_deref(), Some("C"));
    let (output, report) = import_annotation_xfdf_pdf(
        &input,
        &first,
        &AnnotationXfdfImportOptions {
            conflict_policy: AnnotationConflictPolicy::MergeSafeFields,
            delete_policy: AnnotationDeletePolicy::Disabled,
            ..AnnotationXfdfImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.updated, 6);
    assert!(report.appearances_regenerated >= 4);
    let reopened = ContentEngine::open_bytes(output).unwrap();
    let (roundtrip, _) = export_annotation_xfdf(&reopened).unwrap();
    let parsed = parse_annotation_xfdf(&roundtrip).unwrap();
    assert_eq!(parsed.annotations.len(), 6);
    assert!(parsed
        .annotations
        .iter()
        .any(|annotation| annotation.popup_for.as_deref() == Some("note-1")));
}

#[test]
fn exotic_appearances_generate_deterministically_and_render() {
    let input = fixture_pdf();
    let options = AnnotationAppearanceOptions::default();
    let (first, report) = generate_annotation_appearances_pdf(&input, &options).unwrap();
    let (second, _) = generate_annotation_appearances_pdf(&input, &options).unwrap();
    assert_eq!(first, second);
    assert!(report.generated >= 4);
    let engine = ContentEngine::open_bytes(first).unwrap();
    let rendered = engine.render_page(1, 72).unwrap();
    assert!(!rendered.to_rgba_bytes().is_empty());
}

#[test]
fn rich_media_inventory_never_decodes_and_policy_rescan_removes_active_media() {
    let input = fixture_pdf();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let limits = RichMediaLimits::default();
    let inventory = rich_media_inventory(&engine, &limits).unwrap();
    assert_eq!(inventory.counts.rich_media_annotations, 1);
    assert!(inventory.counts.embedded_media > 0);
    assert_eq!(inventory.payloads_decoded, 0);
    assert_eq!(inventory.network_requests, 0);

    let (removed, report) = apply_rich_media_policy_pdf(
        &input,
        RichMediaPolicyMode::RemoveAllMedia,
        &RichMediaCustomPolicy::default(),
        &limits,
    )
    .unwrap();
    assert!(report.rescan_passed, "{report:#?}");
    assert_eq!(report.after.rich_media_annotations, 0);
    assert_eq!(report.after.embedded_media, 0);
    ContentEngine::open_bytes(removed).unwrap();

    let (flattened, flattened_report) = apply_rich_media_policy_pdf(
        &input,
        RichMediaPolicyMode::FlattenStaticPoster,
        &RichMediaCustomPolicy::default(),
        &limits,
    )
    .unwrap();
    assert!(flattened_report.flattened_items >= 1);
    assert!(flattened_report.rescan_passed);
    let flattened_engine = ContentEngine::open_bytes(flattened).unwrap();
    let rendered = flattened_engine.render_page(1, 72).unwrap();
    assert!(!rendered.to_rgba_bytes().is_empty());
}

#[test]
fn nonaxis_redaction_maps_polygon_and_rewrites_or_removes_securely() {
    let input = fixture_pdf();
    let request = NonAxisRedactionRequest {
        page: 1,
        polygon: vec![[55.0, 50.0], [95.0, 60.0], [90.0, 95.0], [48.0, 82.0]],
        coordinate_space: RedactionCoordinateSpace::PdfUserSpace,
        fallback_policy: NonAxisRedactionFallbackPolicy::SecureRewriteOrRemove,
        fill: vec![0.0, 0.0, 0.0],
    };
    let options = NonAxisRedactionOptions {
        requests: vec![
            request,
            NonAxisRedactionRequest {
                page: 1,
                polygon: vec![
                    [182.0, 142.0],
                    [195.0, 142.0],
                    [195.0, 155.0],
                    [182.0, 155.0],
                ],
                coordinate_space: RedactionCoordinateSpace::PdfUserSpace,
                fallback_policy: NonAxisRedactionFallbackPolicy::SecureRewriteOrRemove,
                fill: vec![0.0],
            },
        ],
        deterministic: true,
        fail_on_unsupported: false,
        promote_inline_images: false,
        signature_policy_override: false,
    };
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let plan = plan_nonaxis_image_redaction(&engine, &options).unwrap();
    assert_eq!(plan.overlay_only_claims, 0);
    assert!(plan.rows[0].planned_strategy.contains("inverse_affine"));
    let (first, report) = apply_nonaxis_image_redaction_pdf(&input, &options).unwrap();
    let (second, _) = apply_nonaxis_image_redaction_pdf(&input, &options).unwrap();
    assert_eq!(first, second);
    assert!(report.output_reopened);
    assert_eq!(report.security_proof_failures, 0);
    assert_eq!(report.overlay_only_success_claims, 0);
    let output_engine = ContentEngine::open_bytes(first).unwrap();
    let images = output_engine.find_page_images(1).unwrap();
    assert!(
        images
            .iter()
            .any(|image| image.xobject_name.starts_with("OxP17RedactIm")),
        "image names after redaction: {:?}",
        images
            .iter()
            .map(|image| image.xobject_name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn rotated_crop_coordinate_mapping_is_bounded() {
    let input = fixture_pdf();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let options = NonAxisRedactionOptions {
        requests: vec![NonAxisRedactionRequest {
            page: 1,
            polygon: vec![[20.0, 20.0], [60.0, 20.0], [60.0, 60.0], [20.0, 60.0]],
            coordinate_space: RedactionCoordinateSpace::RotatedCropSpace,
            fallback_policy: NonAxisRedactionFallbackPolicy::RemoveIntersectingInvocation,
            fill: vec![0.0],
        }],
        deterministic: true,
        fail_on_unsupported: false,
        promote_inline_images: false,
        signature_policy_override: false,
    };
    let plan = plan_nonaxis_image_redaction(&engine, &options).unwrap();
    assert_eq!(plan.rows[0].page_rotation, 90);
    assert_eq!(plan.rows[0].page_polygon[0], [190.0, 30.0]);
}

#[test]
fn annotation_media_redaction_sdk_envelopes_and_feature_report_are_additive() {
    let input = fixture_pdf();
    let rich: serde_json::Value = serde_json::from_str(
        &wellfriendpdf_engine::sdk::rich_media_report_json(&input, None).unwrap(),
    )
    .unwrap();
    assert_eq!(rich["kind"], "rich_media_report");
    assert_eq!(
        rich["report"]["schema_version"],
        "annotation_media_redaction.annotation-xfdf-media-redaction.v1"
    );
    let annotation_media_redaction: serde_json::Value = serde_json::from_str(
        &wellfriendpdf_engine::sdk::annotation_media_redaction_report_json(&input, None).unwrap(),
    )
    .unwrap();
    assert_eq!(
        annotation_media_redaction["kind"],
        "annotation_media_redaction_report"
    );
    let feature: serde_json::Value =
        serde_json::from_str(&wellfriendpdf_engine::sdk::feature_report_json().unwrap()).unwrap();
    let section =
        &feature["report"]["annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction"];
    assert_eq!(section["status"], "complete_bounded_foundation");
    assert_eq!(section["failure"]["blocked"], 0);
    assert_eq!(
        section["security"]["overlay_only_redaction_success_claims"],
        0
    );
}
