use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::path::Path;
use tower::util::ServiceExt;

fn fixture_pdf(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../engine/tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

fn make_multipart(filename: &str, pdf: &[u8], extra: &[(&str, &str)]) -> (String, Vec<u8>) {
    let boundary = "wellfriendpdf-test-boundary-xyz";
    let mut body: Vec<u8> = Vec::new();

    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n\
             Content-Type: application/pdf\r\n\r\n",
            filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(pdf);
    body.extend_from_slice(b"\r\n");

    for (name, value) in extra {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}",
                name, value
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    let ct = format!("multipart/form-data; boundary={}", boundary);
    (ct, body)
}

fn build_prepress_plate_pdf() -> Vec<u8> {
    let content = "/CS1 cs 0.25 scn 10 10 20 20 re f\n\
                   /CS1 CS 0.75 SCN 40 10 m 80 10 l S\n\
                   /CS2 cs 0.20 0.80 scn 10 40 20 20 re f\n";
    let type4 = "{ 0 }";
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB 5 0 R] /CS2 [/DeviceN [/Cyan /SpotGreen] /DeviceRGB 6 0 R] >> >> /Contents 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content).into_bytes(),
        b"<< /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0.5 0] /N 1 >>".to_vec(),
        format!(
            "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{}\nendstream",
            type4.len(),
            type4
        )
        .into_bytes(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0usize];
    for (idx, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let startxref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            startxref
        )
        .as_bytes(),
    );
    pdf
}

fn build_text_edit_pdf(content: &[u8]) -> Vec<u8> {
    use wellfriendpdf_engine::writer::{OutputObject, PdfWriter};
    use wellfriendpdf_engine::PdfObject;

    let mut catalog = wellfriendpdf_engine::PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".into()));
    catalog.insert(
        "Pages",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );
    let mut pages = wellfriendpdf_engine::PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".into()));
    pages.insert("Count", PdfObject::Integer(1));
    pages.insert(
        "Kids",
        PdfObject::Array(vec![PdfObject::Reference {
            number: 3,
            generation: 0,
        }]),
    );
    let mut font = wellfriendpdf_engine::PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name("Courier".into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    let mut fonts = wellfriendpdf_engine::PdfDictionary::empty();
    fonts.insert(
        "F1",
        PdfObject::Reference {
            number: 5,
            generation: 0,
        },
    );
    let mut resources = wellfriendpdf_engine::PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut page = wellfriendpdf_engine::PdfDictionary::empty();
    page.insert("Type", PdfObject::Name("Page".into()));
    page.insert(
        "Parent",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );
    page.insert(
        "MediaBox",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(200),
            PdfObject::Integer(200),
        ]),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    page.insert(
        "Contents",
        PdfObject::Reference {
            number: 4,
            generation: 0,
        },
    );
    let mut stream = wellfriendpdf_engine::PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(content.len() as i64));
    PdfWriter::new(
        vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: stream,
                    raw: content.to_vec(),
                },
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(font),
            },
        ],
        1,
    )
    .write()
    .expect("text edit fixture")
}

fn build_one_image_pdf() -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode /Length 4 >>\nstream\nxxxx\nendstream".to_vec(),
        b"<< /Length 19 >>\nstream\nq 1 0 0 1 0 0 cm /Im1 Do Q\nendstream".to_vec(),
    ];
    let mut offsets = vec![0usize];
    for (idx, obj) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        pdf.extend_from_slice(obj);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let startxref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            startxref
        )
        .as_bytes(),
    );
    pdf
}

fn multipart_report_response(
    content_type: &str,
    body: &[u8],
    expected_part_name: &str,
) -> (Value, Vec<u8>) {
    let boundary = content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .unwrap();
    let delimiter = format!("--{}", boundary).into_bytes();
    let mut next_delimiter = b"\r\n".to_vec();
    next_delimiter.extend_from_slice(&delimiter);
    let mut cursor = find_bytes(body, &delimiter).unwrap();
    let mut metadata = None;
    let mut payload = None;

    loop {
        cursor += delimiter.len();
        if body[cursor..].starts_with(b"--") {
            break;
        }
        assert!(body[cursor..].starts_with(b"\r\n"));
        cursor += 2;

        let header_end = cursor + find_bytes(&body[cursor..], b"\r\n\r\n").unwrap();
        let raw_headers = std::str::from_utf8(&body[cursor..header_end]).unwrap();
        let body_start = header_end + 4;
        let body_end = body_start + find_bytes(&body[body_start..], &next_delimiter).unwrap();
        let part_body = &body[body_start..body_end];

        if raw_headers.contains("name=\"metadata\"") {
            metadata = Some(serde_json::from_slice::<Value>(part_body).unwrap());
        } else if raw_headers.contains(&format!("name=\"{}\"", expected_part_name)) {
            payload = Some(part_body.to_vec());
        }
        cursor = body_end + 2;
    }

    (metadata.unwrap(), payload.unwrap())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn assert_report_metadata(metadata: &Value, expected_content_type: &str, expected_part: &str) {
    assert_eq!(metadata["contract_schema_version"], 1);
    assert_eq!(metadata["cache_fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(metadata["rendered_content_type"], expected_content_type);
    assert_eq!(metadata["body_part_name"], expected_part);
    assert!(metadata["rendered_byte_length"].as_u64().unwrap() > 0);
    assert!(metadata["font_substitution_report"]["events"].is_array());
    let telemetry = &metadata["render_telemetry_report"];
    assert_eq!(telemetry["scope"], "one_shot_render_contract_report");
    assert_eq!(
        telemetry["cache_fingerprint"],
        metadata["cache_fingerprint"]
    );
    assert!(telemetry["resource_budget_max_cache_bytes"].is_number());
    assert!(telemetry["aggregate_resource_cache_bytes"].is_number());
    assert!(telemetry["glyph_cache"]["hits"].is_number());
    assert!(telemetry["font_bytes_cache"]["misses"].is_number());
    assert!(telemetry["display_list_cache"]["bytes"].is_number());
    assert!(telemetry["display_list_raster_cache"]["bytes"].is_number());
    assert!(telemetry["image_xobject_cache"]["evictions"].is_number());
    assert!(telemetry["transparent_page_group_entries"].is_number());
    if let Some(event) = metadata["font_substitution_report"]["events"]
        .as_array()
        .and_then(|events| events.first())
    {
        assert!(event["requested_pdf_font"].is_string());
        assert!(event["selected_replacement"].is_string());
        assert!(event["reason"].is_string());
        assert!(event["metric_posture"].is_string());
        assert!(event["embedded_state"].is_string());
        assert!(event["encoding"].is_string());
        assert!(event["resolution_source"].is_string());
        assert!(event["selection_reason"].is_string());
        assert!(event["required_glyph_coverage"].is_object());
        assert!(event["missing_glyphs"].is_number());
        assert!(event["visual_risk_category"].is_string());
        assert!(event["extraction_impact"].is_string());
        assert!(event["editing_impact"].is_string());
        assert!(event["font_policy_identity"].is_string());
    }
    assert!(metadata["font_substitution_report"]["overflow_count"].is_number());
}

#[tokio::test]
async fn health_check_returns_ok() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn readiness_endpoint_returns_ready() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/readiness").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["status"], "ready");
    assert!(json["version"].is_string());
}

#[test]
fn server_config_defaults_are_sane() {
    let cfg = wellfriendpdf_server::config::ServerConfig::default();
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.max_dpi, 600);
    assert!(cfg.max_file_size > 0);
    assert!(cfg.max_pages > 0);
}

#[test]
fn config_default_port_and_max_dpi_are_sane() {
    let cfg = wellfriendpdf_server::config::ServerConfig::default();
    assert_eq!(cfg.port, 8080, "default port should be 8080");
    assert_eq!(cfg.max_dpi, 600);
}

#[test]
fn server_config_max_dpi_default_is_capped_at_600() {
    let cfg = wellfriendpdf_server::config::ServerConfig::default();
    assert!(cfg.max_dpi <= 600, "max_dpi should never exceed 600");
}

#[tokio::test]
async fn health_still_returns_ok() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_readiness_alias_works() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::get("/api/v1/readiness")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_health_alias_works() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn version_endpoint_returns_json() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/api/v1/version").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["product"], "Wellfriend");
    assert!(json["version"].is_string());
}

#[tokio::test]
async fn capabilities_endpoint_exposes_renderer_cache_pressure_and_concurrency_policy() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::get("/api/v1/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let policy = &json["renderer_cache_pressure_policy"];
    assert_eq!(policy["public_endpoint"], "/api/v1/capabilities");
    assert!(policy["cache_classes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("render_tiles")));
    assert!(policy["pressure_actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("evict_recomputable_tiles")));
    assert_eq!(policy["correctness_preserved"], true);
    assert!(policy["remaining_limitation"]
        .as_str()
        .unwrap()
        .contains("external_runtime_cache_telemetry_validation_deferred"));
    let matrix = &json["renderer_concurrency_cache_matrix"];
    assert_eq!(matrix["public_endpoint"], "/api/v1/capabilities");
    assert!(matrix["thread_classes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["name"].as_str() == Some("tile_render")));
    assert!(matrix["cache_rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["class"].as_str() == Some("render_tiles")));
    assert_eq!(matrix["correctness_preserved"], true);
    assert!(matrix["remaining_limitation"]
        .as_str()
        .unwrap()
        .contains("external_runtime_thread_cache_matrix_validation_deferred"));
}

#[tokio::test]
async fn extract_text_missing_file_returns_400() {
    let app = wellfriendpdf_server::app::create_app();
    let boundary = "test-boundary";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"pages\"\r\n\r\nall\r\n--{}--\r\n",
        boundary, boundary
    );
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "missing_file");
}

#[tokio::test]
async fn extract_text_with_flate_pdf_returns_text() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("page_markers", "false")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let resp_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8(resp_bytes.to_vec()).unwrap();
    assert!(
        !text.trim().is_empty(),
        "extracted text should not be empty"
    );
}

#[tokio::test]
async fn extract_text_json_format_returns_valid_json() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("output_format", "json")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        json["pages"].is_array(),
        "response should have 'pages' array"
    );
    assert!(json["total_pages"].is_number());
    assert!(json["has_text_layer"].is_boolean());
    assert!(json["is_likely_scanned"].is_boolean());
    let pages = json["pages"].as_array().unwrap();
    assert!(!pages.is_empty(), "pages array should not be empty");
    let first_page = &pages[0];
    assert_eq!(first_page["page"], 1);
    assert!(first_page["text"].is_string());
    assert!(first_page["line_count"].is_number());
    assert!(first_page["char_count"].is_number());
}

#[tokio::test]
async fn extract_text_scanned_pdf_returns_422() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("scanned.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "no_text_layer");
}

#[tokio::test]
async fn extract_text_invalid_page_range_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("pages", "999")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn analyze_endpoint_returns_analysis() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/analyze")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        json["has_text_layer"].as_bool().unwrap_or(false),
        "flate.pdf should have text layer"
    );
    assert!(
        !json["is_likely_scanned"].as_bool().unwrap_or(true),
        "flate.pdf should not be scanned"
    );
    assert!(json["total_pages"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(json["recommendation"], "UseExtractText");
}

#[tokio::test]
async fn analyze_missing_file_returns_400() {
    let app = wellfriendpdf_server::app::create_app();
    let boundary = "test-boundary";
    let body = format!("--{}--\r\n", boundary);
    let response = app
        .oneshot(
            Request::post("/api/v1/analyze")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn extract_text_page_markers_false_omits_marker() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("page_markers", "false")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8_lossy(&body_bytes);
    assert!(
        !text.contains("--- Page"),
        "page_markers=false should produce no page markers"
    );
}

#[tokio::test]
async fn extract_text_specific_page_range() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("pages", "1"), ("page_markers", "false")],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(
        !body_bytes.is_empty(),
        "specific page request should return text"
    );
}

#[tokio::test]
async fn extract_text_invalid_boolean_param_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("page_markers", "maybe")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "invalid_parameter");
}

#[tokio::test]
async fn extract_text_invalid_output_format_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("output_format", "pdf")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn extract_images_returns_zip() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let count = response
        .headers()
        .get("x-image-count")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(
        body_bytes.starts_with(b"PK"),
        "response body should be a ZIP file; got {:?}",
        &body_bytes[..4.min(body_bytes.len())]
    );
    if count > 0 {
        assert!(
            body_bytes.len() > 100,
            "non-empty ZIP should be more than 100 bytes"
        );
    }
}

#[tokio::test]
async fn extract_images_json_mode_returns_metadata_only() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("output_format", "json")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(json["image_count"].is_number(), "should have image_count");
    assert!(
        json["pages_processed"].is_number(),
        "should have pages_processed"
    );
    assert!(json["images"].is_array(), "should have images array");
    if let Some(images) = json["images"].as_array() {
        for img in images {
            assert!(
                img.get("data").is_none(),
                "JSON mode should not include image bytes"
            );
            assert!(
                img.get("bytes").is_none(),
                "JSON mode should not include byte payloads"
            );
        }
    }
}

#[tokio::test]
async fn extract_images_missing_file_returns_400() {
    let app = wellfriendpdf_server::app::create_app();
    let boundary = "test-boundary";
    let body = format!("--{}--\r\n", boundary);
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "missing_file");
}

#[tokio::test]
async fn extract_images_invalid_format_returns_400() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("format", "bmp")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn extract_images_text_only_pdf_returns_empty_zip() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "text-only PDF should return 200, not error"
    );
    let count = response
        .headers()
        .get("x-image-count")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    assert_eq!(count, 0, "text-only PDF should have X-Image-Count: 0");
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(body_bytes.starts_with(b"PK"));
}

#[tokio::test]
async fn extract_images_with_format_png_returns_zip_with_png_files() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("format", "png")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(body_bytes.starts_with(b"PK"), "should return a ZIP file");
}

#[tokio::test]
async fn extract_images_with_format_webp_returns_zip_and_succeeds() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("format", "webp")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    // WebP is now supported (no longer a 400).
    assert_eq!(response.status(), StatusCode::OK);
    let images_encoded = response
        .headers()
        .get("x-images-encoded")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        images_encoded >= 1,
        "image_only.pdf should encode at least one WebP image"
    );
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(body_bytes.starts_with(b"PK"), "should return a ZIP file");
}

#[tokio::test]
async fn pdf2img_returns_zip_with_png_pages() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let x_page_count = response
        .headers()
        .get("x-page-count")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let x_pages_rendered = response
        .headers()
        .get("x-pages-rendered")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let x_dpi = response
        .headers()
        .get("x-dpi")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    println!(
        "pdf2img png headers: page_count={}, pages_rendered={}, dpi={}",
        x_page_count, x_pages_rendered, x_dpi
    );

    assert!(x_page_count > 0);
    assert_eq!(x_pages_rendered, x_page_count);
    assert_eq!(x_dpi, 72);

    let body_bytes = to_bytes(response.into_body(), 50_000_000).await.unwrap();
    assert!(body_bytes.starts_with(b"PK"));
    let cursor = std::io::Cursor::new(&body_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert!(!archive.is_empty());

    let mut page_file = archive.by_index(0).unwrap();
    assert!(page_file.name().ends_with(".png"));
    let mut content = Vec::new();
    use std::io::Read;
    page_file.read_to_end(&mut content).unwrap();
    assert!(content.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn render_contract_builder_returns_valid_contract_json() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("render_mode", "high-quality"),
            ("page_box", "Media"),
            ("pixel_format", "Rgb8"),
            ("alpha_mode", "Opaque"),
            ("width", "12"),
            ("height", "9"),
            ("clip_x", "1"),
            ("clip_y", "2"),
            ("clip_width", "10"),
            ("clip_height", "7"),
            ("transform_a", "1"),
            ("transform_b", "0"),
            ("transform_c", "0"),
            ("transform_d", "1"),
            ("transform_e", "3"),
            ("transform_f", "4"),
            ("background_r", "12"),
            ("background_g", "34"),
            ("background_b", "56"),
            ("background_a", "255"),
            ("execution_mode", "Research"),
            ("backend", "ScalarReference"),
            ("compositing", "Compatibility"),
            ("annotations", "Exclude"),
            ("forms", "Exclude"),
            ("optional_content", "ocg:server"),
            ("smoothing", "Disabled"),
            ("text_smoothing", "Subpixel"),
            ("image_smoothing", "Antialiased"),
            ("path_smoothing", "Disabled"),
            ("subpixel_text", "Subpixel"),
            ("color_scheme", "Dark"),
            ("print_profile", "Print"),
            ("halftone", "Screen"),
            ("overprint", "Preview"),
            ("rendering_intent", "Perceptual"),
            ("color_management", "DeterministicFallback"),
            ("exactness", "HighQualityExact"),
            ("determinism", "BestEffortResearch"),
            ("max_pixels", "1000000"),
        ],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["surface_byte_length"], 12 * 9 * 3);
    assert_eq!(json["builder"], "server_render_contract_builder");
    assert_eq!(json["cache_fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(json["contract"]["schema_version"], 1);
    assert_eq!(json["contract"]["page_number"], 1);
    assert_eq!(json["contract"]["dpi"], 72);
    assert_eq!(json["contract"]["page_box"], "Media");
    assert_eq!(json["contract"]["width"], 12);
    assert_eq!(json["contract"]["height"], 9);
    assert_eq!(json["contract"]["stride"], 36);
    assert_eq!(json["contract"]["pixel_format"], "Rgb8");
    assert_eq!(json["contract"]["alpha_mode"], "Opaque");
    assert_eq!(json["contract"]["clip"]["x"], 1);
    assert_eq!(json["contract"]["clip"]["y"], 2);
    assert_eq!(json["contract"]["clip"]["width"], 10);
    assert_eq!(json["contract"]["clip"]["height"], 7);
    assert_eq!(json["contract"]["background"]["r"], 12);
    assert_eq!(json["contract"]["background"]["g"], 34);
    assert_eq!(json["contract"]["background"]["b"], 56);
    assert_eq!(json["contract"]["background"]["a"], 255);
    assert_eq!(json["contract"]["execution_mode"], "Research");
    assert_eq!(json["contract"]["backend"], "ScalarReference");
    assert_eq!(json["contract"]["compositing"], "Compatibility");
    assert_eq!(json["contract"]["annotations"], "Exclude");
    assert_eq!(json["contract"]["forms"], "Exclude");
    assert_eq!(json["contract"]["optional_content"], "ocg:server");
    assert_eq!(json["contract"]["text_smoothing"], "Subpixel");
    assert_eq!(json["contract"]["image_smoothing"], "Antialiased");
    assert_eq!(json["contract"]["path_smoothing"], "Disabled");
    assert_eq!(json["contract"]["subpixel_text"], "Subpixel");
    assert_eq!(json["contract"]["color_scheme"], "Dark");
    assert_eq!(json["contract"]["print_profile"], "Print");
    assert_eq!(json["contract"]["halftone"], "Screen");
    assert_eq!(json["contract"]["overprint"], "Preview");
    assert_eq!(json["contract"]["rendering_intent"], "Perceptual");
    assert_eq!(
        json["contract"]["color_management"],
        "DeterministicFallback"
    );
    assert_eq!(json["contract"]["exactness"], "HighQualityExact");
    assert_eq!(json["contract"]["determinism"], "BestEffortResearch");
    assert_eq!(json["contract"]["resource_budget"]["max_pixels"], 1_000_000);

    let contract_json = json["contract_json"].as_str().unwrap();
    let round_trip: Value = serde_json::from_str(contract_json).unwrap();
    assert_eq!(round_trip, json["contract"]);
}

#[tokio::test]
async fn render_contract_builder_accepts_research_hybrid_backend() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("page", "1"), ("dpi", "72"), ("backend", "research-hybrid")],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["contract"]["backend"], "ResearchHybrid");
    assert_eq!(json["contract"]["schema_version"], 1);
    assert_eq!(json["cache_fingerprint"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn render_contract_backend_plan_arena_report_route_returns_json() {
    let pdf = fixture_pdf("multi_stream.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("page", "1"), ("dpi", "72"), ("render_mode", "compat")],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract/backend-plan-arena-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["kind"], "backend_plan_arena_report");
    assert_eq!(json["report"]["schema_version"], 1);
    assert_eq!(json["report"]["page_number"], 1);
    assert!(json["report"]["document_revision"].as_u64().is_some());
    assert!(json["report"]["hot_operation_count"].as_u64().unwrap() > 0);
    assert!(json["report"]["descriptor_kinds"].as_object().is_some());
}

#[tokio::test]
async fn prepress_plate_report_route_returns_json() {
    let pdf = build_prepress_plate_pdf();
    let (ct, body_bytes) = make_multipart("prepress.pdf", &pdf, &[("page", "1"), ("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/prepress/plate-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["kind"], "prepress_plate_report");
    assert_eq!(json["report"]["true_separation_framebuffer"], true);
    assert_eq!(json["report"]["page_number"], 1);
    assert_eq!(json["report"]["plate_count"], 3);
    assert_eq!(json["report"]["contribution_count"], 4);
    assert_eq!(
        json["report"]["deterministic_plane_order"],
        serde_json::json!(["Cyan", "SpotGreen", "SpotOrange"])
    );
    assert!(json["report"]["cache_fingerprint"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
}

#[tokio::test]
async fn document_views_report_route_returns_json() {
    let pdf = fixture_pdf("multi_stream.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/document-views/report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["kind"], "document_views_report");
    assert_eq!(json["report"]["schema_version"], 1);
    assert_eq!(json["report"]["views"].as_array().unwrap().len(), 5);
    assert!(json["report"]["views"]
        .as_array()
        .unwrap()
        .iter()
        .any(|view| view["name"] == "render"));
    assert_eq!(json["report"]["materialization"]["semantic_pages"], 0);
}

#[tokio::test]
async fn render_contract_png_route_renders_canonical_contract() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("page", "1"), ("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let contract_json = json["contract_json"].as_str().unwrap().to_string();

    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("contract_json", contract_json.as_str())],
    );
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract/png")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-type").unwrap(), "image/png");
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(body_bytes.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn render_contract_png_report_route_returns_png_and_report_part() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("page", "1"), ("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let contract_json = json["contract_json"].as_str().unwrap().to_string();

    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("contract_json", contract_json.as_str())],
    );
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract/png-with-font-substitution-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.starts_with("multipart/mixed; boundary="));
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let (metadata, image) = multipart_report_response(&content_type, &body_bytes, "image");
    assert_report_metadata(&metadata, "image/png", "image");
    assert_eq!(
        metadata["rendered_byte_length"].as_u64().unwrap() as usize,
        image.len()
    );
    assert!(image.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn render_contract_raw_route_renders_caller_surface_contract() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("pixel_format", "Rgb8"),
            ("alpha_mode", "Opaque"),
            ("width", "12"),
            ("height", "9"),
            ("clip_x", "0"),
            ("clip_y", "0"),
            ("clip_width", "12"),
            ("clip_height", "9"),
        ],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let contract_json = json["contract_json"].as_str().unwrap().to_string();

    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("contract_json", contract_json.as_str())],
    );
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract/raw")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-pixel-format")
            .unwrap(),
        "Rgb8"
    );
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-alpha-mode")
            .unwrap(),
        "Opaque"
    );
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-surface-bytes")
            .unwrap(),
        "324"
    );
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert_eq!(body_bytes.len(), 12 * 9 * 3);
}

#[tokio::test]
async fn render_contract_raw_report_route_returns_surface_and_report_part() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("pixel_format", "Rgb8"),
            ("alpha_mode", "Opaque"),
            ("width", "12"),
            ("height", "9"),
            ("clip_x", "0"),
            ("clip_y", "0"),
            ("clip_width", "12"),
            ("clip_height", "9"),
        ],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/render-contract")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let contract_json = json["contract_json"].as_str().unwrap().to_string();

    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("contract_json", contract_json.as_str())],
    );
    let response = app
        .oneshot(
            Request::post("/api/v1/render-contract/raw-with-font-substitution-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-pixel-format")
            .unwrap(),
        "Rgb8"
    );
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-alpha-mode")
            .unwrap(),
        "Opaque"
    );
    assert_eq!(
        response
            .headers()
            .get("x-render-contract-surface-bytes")
            .unwrap(),
        "324"
    );
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.starts_with("multipart/mixed; boundary="));
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let (metadata, surface) = multipart_report_response(&content_type, &body_bytes, "surface");
    assert_report_metadata(&metadata, "application/octet-stream", "surface");
    assert_eq!(metadata["contract_surface_byte_length"], 12 * 9 * 3);
    assert_eq!(
        metadata["rendered_byte_length"].as_u64().unwrap() as usize,
        surface.len()
    );
    assert_eq!(surface.len(), 12 * 9 * 3);
}

#[tokio::test]
async fn editing_transaction_apply_route_returns_render_invalidation_plan() {
    let pdf = build_text_edit_pdf(b"BT /F1 12 Tf 10 150 Td (HELLO) Tj ET\n");
    let request = r#"{
        "requested_mode":"operator_preserving",
        "page":1,
        "source_text":"HELLO",
        "replacement_text":"WORLD"
    }"#;
    let options = r#"{
        "page_number":1,
        "dpi":72,
        "tile_width":64,
        "tile_height":64
    }"#;
    let (ct, body_bytes) = make_multipart(
        "edit.pdf",
        &pdf,
        &[
            ("request_json", request),
            ("render_invalidation_options_json", options),
        ],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/editing-transactions/apply-with-render-invalidation")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(content_type.starts_with("multipart/mixed; boundary="));
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let (metadata, document) = multipart_report_response(&content_type, &body_bytes, "document");
    assert!(document.starts_with(b"%PDF-"));
    assert_eq!(
        metadata["kind"],
        "editing_transactions_transaction_apply_with_render_invalidation"
    );
    assert_eq!(
        metadata["report"]["render_invalidation"]["schema_version"],
        "render-transaction-invalidation-plan.v1"
    );
    assert_eq!(
        metadata["report"]["render_invalidation"]["dirty_region_conversion"]["requested"],
        true
    );
    assert!(
        metadata["report"]["render_invalidation"]["cache_application_entry_points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "ContentEngine::invalidate_for_transaction_with_tiles")
    );
}

#[tokio::test]
async fn image_decode_capability_report_route_returns_json() {
    let pdf = build_one_image_pdf();
    let (ct, body_bytes) = make_multipart("image.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/image-decode/capability-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let value: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(value["kind"], "image_decode_capability_report");
    assert_eq!(value["report"]["schema_version"], 1);
    assert_eq!(value["report"]["image_count"], 1);
    assert_eq!(value["report"]["images"][0]["name"], "Im1");
    assert!(value["report"]["native_metadata_inspection_count"]
        .as_u64()
        .is_some());
    assert!(value["report"]["renderer_boundary_memory_budget_count"]
        .as_u64()
        .is_some());
}

#[tokio::test]
async fn progressive_image_decode_lifecycle_route_returns_report() {
    let pdf = build_one_image_pdf();
    let request = r#"{
        "image_index":0,
        "max_retained_bytes":2048,
        "actions":["start","continue","pause","resume","cancel","close"]
    }"#;
    let (ct, body_bytes) = make_multipart("image.pdf", &pdf, &[("request_json", request)]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/progressive-image-decode/lifecycle-report")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let value: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(value["kind"], "progressive_image_decode_lifecycle_report");
    assert_eq!(value["report"]["image_count"], 1);
    assert_eq!(value["report"]["image"]["name"], "Im1");
    assert_eq!(
        value["report"]["reports"][1]["phase"],
        "full_decode_required"
    );
    assert_eq!(value["report"]["reports"][4]["state"], "cancelled");
    assert_eq!(value["report"]["reports"][5]["state"], "closed");
}

#[tokio::test]
async fn pdf2img_with_jpeg_format_returns_jpeg_pages() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("dpi", "72"), ("format", "jpg")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 50_000_000).await.unwrap();
    assert!(body_bytes.starts_with(b"PK"));

    let cursor = std::io::Cursor::new(&body_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert!(!archive.is_empty());
    let mut page_file = archive.by_index(0).unwrap();
    assert!(page_file.name().ends_with(".jpg"));
    let mut content = Vec::new();
    use std::io::Read;
    page_file.read_to_end(&mut content).unwrap();
    assert_eq!(&content[..2], &[0xFF, 0xD8]);
}

#[tokio::test]
async fn pdf2img_missing_file_returns_400() {
    let app = wellfriendpdf_server::app::create_app();
    let boundary = "bound";
    let body = format!("--{}--\r\n", boundary);
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pdf2img_invalid_dpi_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("dpi", "2")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pdf2img_invalid_format_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("format", "bmp")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pdf2img_page_range_limits_pages() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("pages", "1"), ("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let x_pages = response
        .headers()
        .get("x-pages-rendered")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(999);
    assert_eq!(x_pages, 1);
}

#[tokio::test]
async fn pdf2img_high_dpi_page_has_larger_dimensions() {
    async fn render_zip_size(pdf: &[u8], dpi: &str) -> usize {
        let (ct, body_bytes) = make_multipart("test.pdf", pdf, &[("dpi", dpi)]);
        let app = wellfriendpdf_server::app::create_app();
        let response = app
            .oneshot(
                Request::post("/api/v1/pdf2img")
                    .header("content-type", ct)
                    .body(Body::from(body_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 50_000_000)
            .await
            .unwrap()
            .len()
    }

    let pdf = fixture_pdf("flate.pdf");
    let size_72 = render_zip_size(&pdf, "72").await;
    let size_144 = render_zip_size(&pdf, "144").await;
    println!("pdf2img zip sizes: dpi72={}, dpi144={}", size_72, size_144);
    assert!(
        size_144 > size_72,
        "144 DPI output ({}) should exceed 72 DPI output ({})",
        size_144,
        size_72
    );
}

#[tokio::test]
async fn pdf2img_renders_pages_in_correct_order() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 50_000_000).await.unwrap();
    let cursor = std::io::Cursor::new(&body_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    let mut names = Vec::new();
    for i in 0..archive.len() {
        names.push(archive.by_index(i).unwrap().name().to_string());
    }

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[tokio::test]
async fn pdf2img_default_dpi_header_is_150() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body_bytes) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let x_dpi = response
        .headers()
        .get("x-dpi")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(x_dpi, "150");
}

#[tokio::test]
async fn content_type_header_is_correct_for_txt() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body_bytes) = make_multipart(
        "test.pdf",
        &pdf,
        &[("output_format", "txt"), ("page_markers", "false")],
    );
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/plain"),
        "txt output should have text/plain content-type, got: {}",
        content_type
    );
    assert!(
        content_type.contains("utf-8") || content_type.contains("UTF-8"),
        "should specify utf-8 charset"
    );
}

#[tokio::test]
async fn auth_disabled_by_default_allows_all_requests() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn rate_limit_disabled_by_default_allows_rapid_health_requests() {
    let app = wellfriendpdf_server::app::create_app();
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn version_endpoint_returns_version_string() {
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(Request::get("/api/v1/version").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(json["version"].is_string(), "version field should exist");
}

#[tokio::test]
async fn e2e_extract_text_returns_content() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(
        text.contains("Hi"),
        "extract-text should include fixture text"
    );
}

#[tokio::test]
async fn e2e_pdf2img_produces_valid_zip_with_png() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let zip_bytes = to_bytes(response.into_body(), 10_000_000).await.unwrap();
    assert!(zip_bytes.starts_with(b"PK"), "should be ZIP");

    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert!(!archive.is_empty(), "ZIP should have at least 1 page");
    let mut page_file = archive.by_index(0).unwrap();
    assert!(page_file.name().ends_with(".png"));
    use std::io::Read;
    let mut content = Vec::new();
    page_file.read_to_end(&mut content).unwrap();
    assert!(content.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[tokio::test]
async fn e2e_analyze_detects_text_layer() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(json["has_text_layer"].is_boolean());
}

#[tokio::test]
async fn e2e_all_endpoints_return_200_for_valid_pdf() {
    let pdf = fixture_pdf("flate.pdf");
    let app = wellfriendpdf_server::app::create_app();

    for (path, extra) in [
        ("/api/v1/extract-text", Vec::<(&str, &str)>::new()),
        ("/api/v1/extract-images", Vec::<(&str, &str)>::new()),
        ("/api/v1/analyze", Vec::<(&str, &str)>::new()),
        ("/api/v1/pdf2img", vec![("dpi", "72")]),
    ] {
        let (ct, body) = make_multipart("test.pdf", &pdf, &extra);
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", ct)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{} failed", path);
    }
}

#[tokio::test]
async fn e2e_non_pdf_bytes_returns_error() {
    let garbage = b"this is not a pdf file, definitely not";
    let (ct, body) = make_multipart("garbage.pdf", garbage, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_client_error());
    assert_ne!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn e2e_extract_images_from_image_pdf() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("format", "png")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-images")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let img_count = response
        .headers()
        .get("x-image-count")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        img_count > 0,
        "image_only.pdf should have at least one image"
    );
    let zip_bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(zip_bytes.starts_with(b"PK"));
}

#[tokio::test]
async fn e2e_render_image_pdf_contains_non_white_pixels() {
    let pdf = fixture_pdf("image_only.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let zip_bytes = to_bytes(response.into_body(), 10_000_000).await.unwrap();
    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert_eq!(archive.len(), 1, "one page");

    let mut file = archive.by_index(0).unwrap();
    use std::io::Read;
    let mut png_bytes = Vec::new();
    file.read_to_end(&mut png_bytes).unwrap();
    assert!(png_bytes.starts_with(&[0x89, b'P', b'N', b'G']));

    let decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    let has_non_white = pixels
        .chunks(3)
        .any(|p| p[0] != 255 || p[1] != 255 || p[2] != 255);
    assert!(
        has_non_white,
        "rendered image_only.pdf should have non-white pixels"
    );
}

// ---------------------------------------------------------------------------
// Encryption: optional `password` form field
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unencrypted_pdf_with_password_param_still_works() {
    // Supplying a password for a non-encrypted PDF must not break it.
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("password", "any_password")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "non-encrypted PDF should succeed even with a password param"
    );
}

#[tokio::test]
async fn analyze_with_password_param_on_unencrypted_pdf_works() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("password", "ignored")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/analyze")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn invalid_pdf_returns_client_or_server_error() {
    let garbage = b"%PDF-1.4 ... this is not real PDF content";
    let (ct, body) = make_multipart("test.pdf", garbage, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "invalid PDF should return an error status, got {}",
        response.status()
    );
}

#[tokio::test]
async fn password_protected_pdf_without_password_returns_422() {
    // A genuine V2/R3 encrypted PDF whose /U does not match the empty password.
    // Synthesised inline so the test needs no external fixture.
    let pdf = build_password_protected_pdf();
    let (ct, body) = make_multipart("protected.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-text")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "password-protected PDF without a password should map to 422"
    );
    let body_bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"], "encrypted");
}

/// Build a minimal V2/R3 encrypted PDF whose `/U` deliberately does not match
/// the empty user password, so the server cannot open it without a password.
fn build_password_protected_pdf() -> Vec<u8> {
    fn hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{:02X}", b));
        }
        s
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut offsets = [0usize; 3];
    bytes.extend_from_slice(b"%PDF-1.4\n");
    offsets[1] = bytes.len();
    bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offsets[2] = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 3\n0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    let owner_o = vec![0xABu8; 32];
    let user_u = vec![0xCDu8; 32]; // will not verify against the empty password
    let file_id = b"0123456789abcdef";
    let trailer = format!(
        "trailer\n<< /Size 3 /Root 1 0 R /Encrypt << /Filter /Standard /V 2 /R 3 /Length 128 \
         /P -3904 /O <{}> /U <{}> >> /ID [<{}> <{}>] >>\nstartxref\n{}\n%%EOF\n",
        hex(&owner_o),
        hex(&user_u),
        hex(file_id),
        hex(file_id),
        xref
    );
    bytes.extend_from_slice(trailer.as_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// Form XObject rendering
// ---------------------------------------------------------------------------

/// Same minimal Form-XObject PDF as the engine integration test (a 50×50 gray
/// square centred on a 100×100 page). Duplicated here because test helpers do
/// not cross crate boundaries.
fn build_form_xobject_pdf() -> Vec<u8> {
    let form_stream_content: &[u8] = b"0.5 g\n0 0 50 50 re\nf\n";
    let page_stream_content: &[u8] = b"q\n1 0 0 1 25 25 cm\n/Fm0 Do\nQ\n";

    let mut pdf: Vec<u8> = Vec::new();
    let mut offsets = [0usize; 6];
    pdf.extend_from_slice(b"%PDF-1.4\n");

    offsets[1] = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    offsets[2] = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    offsets[5] = pdf.len();
    pdf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Type /XObject /Subtype /Form /BBox [0 0 50 50] \
             /Resources << /ProcSet [/PDF] >> /Length {} >>\nstream\n",
            form_stream_content.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(form_stream_content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    offsets[4] = pdf.len();
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n",
            page_stream_content.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(page_stream_content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    offsets[3] = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] \
          /Contents 4 0 R \
          /Resources << /XObject << /Fm0 5 0 R >> /ProcSet [/PDF] >> >>\nendobj\n",
    );

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );
    pdf
}

#[tokio::test]
async fn pdf2img_form_xobject_pdf_returns_valid_png() {
    let pdf_bytes = build_form_xobject_pdf();
    let (ct, body) = make_multipart("form.pdf", &pdf_bytes, &[("dpi", "72")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/pdf2img")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let zip_bytes = to_bytes(response.into_body(), 5_000_000).await.unwrap();
    assert!(zip_bytes.starts_with(b"PK"), "should be a ZIP archive");

    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert_eq!(archive.len(), 1, "one page");
    let mut file = archive.by_index(0).unwrap();
    use std::io::Read;
    let mut png = Vec::new();
    file.read_to_end(&mut png).unwrap();
    assert!(
        png.starts_with(&[0x89, b'P', b'N', b'G']),
        "should be a PNG"
    );

    // The rendered page should contain non-white (gray) pixels from the Form.
    let decoder = png::Decoder::new(std::io::Cursor::new(&png));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    let has_non_white = pixels
        .chunks(3)
        .any(|p| p[0] != 255 || p[1] != 255 || p[2] != 255);
    assert!(has_non_white, "Form XObject should paint non-white pixels");
}

// --- Parser endpoints (parse / chunk / extract-fields / info) ---------------

#[tokio::test]
async fn parse_markdown_returns_text() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("format", "markdown")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/parse")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let md = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!md.trim().is_empty(), "parsed markdown should not be empty");
}

#[tokio::test]
async fn parse_json_is_canonical_schema() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("format", "json")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/parse")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    // Canonical Document model: schema_version + body present (matches CLI/C/WASM).
    assert!(
        json["schema_version"].is_string(),
        "parse json must carry the canonical schema_version"
    );
    assert!(json.get("body").is_some(), "parse json must have a body");
}

#[tokio::test]
async fn parse_invalid_format_returns_400() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("format", "yaml")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/parse")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn parse_missing_file_returns_400() {
    let boundary = "wellfriendpdf-test-boundary-xyz";
    let body = format!("--{0}--\r\n", boundary).into_bytes();
    let ct = format!("multipart/form-data; boundary={}", boundary);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/parse")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn chunk_returns_chunkset_json() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[("target_tokens", "256")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/chunk")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        json["chunks"].is_array(),
        "chunk response must have a chunks array"
    );
    assert!(json["schema_version"].is_string());
}

#[tokio::test]
async fn extract_fields_returns_json_on_acroform() {
    let pdf = fixture_pdf("form_160f.pdf");
    let (ct, body) = make_multipart("form.pdf", &pdf, &[("doc_type", "auto")]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/extract-fields")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        json["fields"].is_array(),
        "extract-fields must return a fields array"
    );
    let fields = json["fields"].as_array().unwrap();
    assert!(!fields.is_empty(), "AcroForm document should yield fields");
}

#[tokio::test]
async fn info_returns_metadata_json() {
    let pdf = fixture_pdf("flate.pdf");
    let (ct, body) = make_multipart("test.pdf", &pdf, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/info")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json.is_object(), "info must return a JSON object");
    assert!(
        json.get("page_count").is_some(),
        "info must report page_count"
    );
}

#[tokio::test]
async fn parse_garbage_input_does_not_crash() {
    // Untrusted/garbage bytes must produce a clean 4xx/5xx, never panic the server.
    let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03];
    let (ct, body) = make_multipart("not.pdf", &garbage, &[]);
    let app = wellfriendpdf_server::app::create_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/parse")
                .header("content-type", ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "garbage input should be rejected, got {}",
        response.status()
    );
}
