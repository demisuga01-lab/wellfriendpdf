//! Integration tests for the progressive render session HTTP surface.
//!
//! These tests exercise the full lifecycle (start -> step -> pause -> resume ->
//! step-to-completion -> finish PNG) and the error paths (cancel, double-cancel,
//! status of unknown session) entirely through the axum router with no network.
//!
//! Each test builds ONE app and clones the returned router per request -
//! cloning shares the same progressive session store, so submit -> step ->
//! finish all hit the same running system.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::Value;
use tower::util::ServiceExt;
use wellfriendpdf_engine::{
    AuthorPageSize, ContentEngine, DeviceClip, ExactnessPolicy, PdfBuilder, RenderMode, TextStyle,
};

/// Build a minimal single-page PDF fixture in memory (no file I/O).
fn minimal_pdf() -> Vec<u8> {
    // A minimal valid PDF with one page (US-Letter, blank white page with text).
    let content = b"BT /F1 12 Tf 72 720 Td (Progressive test) Tj ET";
    let content_len = content.len();

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");

    // obj 1: catalog
    let obj1_offset = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // obj 2: pages
    let obj2_offset = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // obj 3: page
    let obj3_offset = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
    );

    // obj 4: content stream
    let obj4_offset = pdf.len();
    let stream_header = format!("4 0 obj\n<< /Length {} >>\nstream\n", content_len);
    pdf.extend_from_slice(stream_header.as_bytes());
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    // obj 5: font
    let obj5_offset = pdf.len();
    pdf.extend_from_slice(
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );

    // xref
    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n");
    pdf.extend_from_slice(b"0 6\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", obj1_offset).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", obj2_offset).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", obj3_offset).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", obj4_offset).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", obj5_offset).as_bytes());

    // trailer
    pdf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

    pdf
}

fn two_page_pdf() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder
        .add_page(AuthorPageSize::LETTER)
        .draw_text("Progressive page one", 72.0, 720.0, &TextStyle::default())
        .expect("write page one");
    builder
        .add_page(AuthorPageSize::LETTER)
        .draw_text("Progressive page two", 72.0, 720.0, &TextStyle::default())
        .expect("write page two");
    builder.to_bytes().expect("serialize two-page PDF")
}

/// Build a multipart body for the progressive/start endpoint.
fn start_multipart(pdf: &[u8], fields: &[(&str, &str)]) -> (String, Vec<u8>) {
    let boundary = "progressive-test-boundary";
    let mut body: Vec<u8> = Vec::new();

    // file field
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"test.pdf\"\r\n\
          Content-Type: application/pdf\r\n\r\n",
    );
    body.extend_from_slice(pdf);
    body.extend_from_slice(b"\r\n");

    // additional fields
    for (name, value) in fields {
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

fn test_config() -> wellfriendpdf_server::config::ServerConfig {
    wellfriendpdf_server::config::ServerConfig {
        allow_unauthenticated: true,
        rate_limit_per_min: 0,
        ..wellfriendpdf_server::config::ServerConfig::default()
    }
}

fn build_app() -> Router {
    wellfriendpdf_server::app::create_app_with_config(test_config())
}

// ---- Tests ----

#[tokio::test]
async fn progressive_start_returns_session_and_token() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "128"),
            ("tile_height", "128"),
        ],
    );

    let app = build_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["session_id"].is_string());
    assert_eq!(json["session_id"].as_str().unwrap().len(), 32);
    assert!(json["token"].is_object());
    assert_eq!(json["token"]["page_number"], 1);
    assert_eq!(json["token"]["dpi"], 72);
    assert_eq!(json["token"]["lifecycle_state"], "created");
}

#[tokio::test]
async fn progressive_start_accepts_adaptive_tile_size() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[("page", "1"), ("dpi", "72"), ("tile_size", "adaptive")],
    );

    let app = build_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    let tile_width = json["token"]["tile_width"].as_u64().unwrap();
    let tile_height = json["token"]["tile_height"].as_u64().unwrap();
    assert_eq!(tile_width, tile_height);
    assert!([128, 192, 256, 384, 512].contains(&tile_width));
    assert_eq!(
        json["token"]["tile_scheduler"]["selection_mode"],
        "adaptive"
    );
    assert_eq!(
        json["token"]["tile_scheduler"]["selected_tile_width"],
        serde_json::json!(tile_width)
    );
    assert!(
        json["token"]["tile_scheduler"]["complexity_score"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test]
async fn progressive_start_accepts_render_contract_json() {
    let pdf = minimal_pdf();
    let engine = ContentEngine::open_bytes(pdf.clone()).expect("open contract source PDF");
    let mut contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .expect("build default contract");
    contract.exactness = ExactnessPolicy::HighQualityExact;
    let contract_json = serde_json::to_string(&contract).expect("serialize render contract");
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("tile_width", "128"),
            ("tile_height", "128"),
            ("render_contract_json", &contract_json),
        ],
    );

    let app = build_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["token"]["page_number"], 1);
    assert_eq!(json["token"]["dpi"], 72);
    assert_eq!(json["token"]["render_mode"], "compat");
    assert_eq!(
        json["token"]["render_contract_fingerprint"],
        serde_json::json!(contract.cache_fingerprint())
    );
    assert_eq!(
        json["token"]["tile_scheduler"]["render_contract_fingerprint"],
        json["token"]["render_contract_fingerprint"]
    );
}

#[tokio::test]
async fn progressive_start_missing_file_returns_400() {
    let boundary = "progressive-test-boundary";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"page\"\r\n\r\n1\r\n--{}--\r\n",
        boundary, boundary
    );
    let ct = format!("multipart/form-data; boundary={}", boundary);

    let app = build_app();
    let response = app
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn progressive_full_lifecycle_step_to_finish() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "256"),
            ("tile_height", "256"),
        ],
    );

    let app = build_app();

    // Start
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    // Step until complete - for a 612x792 page at 72 DPI = 612x792 pixels,
    // with 256x256 tiles that's a 3x4 grid = 12 tiles. We step with max 20
    // to finish in one call.
    let step_body = serde_json::json!({ "max_tiles": 20 });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(step_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let step_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(step_json["lifecycle_state"], "completed");
    assert!(step_json["rendered_this_step"].as_u64().unwrap() > 0);

    // Finish - get the PNG
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/finish", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct_header = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct_header, "image/png");
    let png_bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    // Minimal PNG validity: starts with PNG signature
    assert!(png_bytes.len() > 8);
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n");
}

#[tokio::test]
async fn progressive_apply_render_invalidation_obsoletes_retained_tile_publication() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "256"),
            ("tile_height", "256"),
        ],
    );

    let app = build_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let step_body = serde_json::json!({ "max_tiles": 2 });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(step_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let step_json: Value = serde_json::from_slice(&bytes).unwrap();
    let publication = step_json["completed_tile_publications"][0].clone();
    let tile = publication["tile"].clone();
    let old_publication_identity = publication["publication_identity"]
        .as_str()
        .unwrap()
        .to_string();
    let old_tile_publication_identity = publication["tile_publication_identity"]
        .as_str()
        .unwrap()
        .to_string();
    let next_revision = publication["document_revision"].as_u64().unwrap() + 1;

    let invalidation_plan = serde_json::json!({
        "schema_version": "render-transaction-invalidation-plan.v1",
        "next_revision": next_revision,
        "affected_pages": [1],
        "mapped_source_ids": [],
        "source_cache_markers": [],
        "affected_tiles": [
            {
                "page": 1,
                "tile": tile
            }
        ],
        "conservative_reset_required": false
    });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/apply-render-invalidation",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(invalidation_plan.to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let invalidation_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(invalidation_json["applied"], true);
    assert_eq!(invalidation_json["affected_current_page"], true);
    assert_eq!(
        invalidation_json["previous_publication_identity"],
        old_publication_identity
    );
    assert_ne!(
        invalidation_json["current_publication_identity"],
        old_publication_identity
    );
    assert_eq!(
        invalidation_json["invalidated_completed_tiles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(invalidation_json["step_report"]["obsolete_publications"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["tile_publication_identity"].as_str()
            == Some(old_tile_publication_identity.as_str())));

    let evaluate_body = serde_json::json!({ "publication": publication });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/evaluate-publication",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(evaluate_body.to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let evaluate_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(evaluate_json["accepted"], false);
    assert_eq!(evaluate_json["reason"], "obsolete_tile_publication");
}

#[tokio::test]
async fn progressive_pause_resume_lifecycle() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    // Start
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    // Pause
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/pause", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let pause_json: Value = serde_json::from_slice(&bytes).unwrap();
    let token = &pause_json["token"];
    assert_eq!(token["lifecycle_state"], "paused");

    // Resume
    let resume_body = serde_json::json!({ "token": token });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/resume", session_id))
                .header("content-type", "application/json")
                .body(Body::from(resume_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let resume_json: Value = serde_json::from_slice(&bytes).unwrap();
    // After resume, state should be rendering (no tiles rendered yet in
    // created -> paused path).
    let state = resume_json["token"]["lifecycle_state"].as_str().unwrap();
    assert!(
        state == "rendering" || state == "completed",
        "expected rendering or completed, got {}",
        state
    );
}

#[tokio::test]
async fn progressive_cancel_prevents_further_steps() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "128"),
            ("tile_height", "128"),
        ],
    );

    let app = build_app();

    // Start
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    // Cancel
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/cancel", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let cancel_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cancel_json["cancelled"], true);

    // Step after cancel should fail
    let step_body = serde_json::json!({ "max_tiles": 1 });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(step_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    // The engine returns an error for terminal-state renders; the server maps
    // engine InvalidInput to a client error status.
    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "step after cancel should fail, got {}",
        response.status()
    );
}

#[tokio::test]
async fn progressive_revise_viewport_reports_obsolete_publication() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let step_body = serde_json::json!({ "max_tiles": 2 });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(step_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let first_step: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(first_step["completed_units"], 2);
    let old_identity = first_step["publication_identity"]
        .as_str()
        .unwrap()
        .to_string();

    let viewport_body = serde_json::json!({
        "viewport_hint_x": 576,
        "viewport_hint_y": 704,
        "viewport_hint_w": 32,
        "viewport_hint_h": 64
    });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/viewport", session_id))
                .header("content-type", "application/json")
                .body(Body::from(viewport_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let revision: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(revision["completed_units"], 0);
    assert_ne!(
        revision["publication_identity"].as_str().unwrap(),
        old_identity
    );
    assert_eq!(
        revision["obsolete_publications"][0]["publication_identity"],
        old_identity
    );
    assert_eq!(
        revision["obsolete_publications"][0]["reason"],
        "viewport_hint_revised"
    );

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let next_step: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(next_step["rendered_this_step"], 1);
    assert_eq!(
        next_step["completed_tile_publications"][0]["publication_identity"],
        revision["publication_identity"]
    );
}

#[tokio::test]
async fn progressive_evaluate_publication_rejects_stale_viewport_tile() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let first_step: Value = serde_json::from_slice(&bytes).unwrap();
    let publication = first_step["completed_tile_publications"][0].clone();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/evaluate-publication",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "publication": publication }).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let acceptance: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(acceptance["accepted"], true);
    assert_eq!(acceptance["reason"], "current");

    let viewport_body = serde_json::json!({
        "viewport_hint_x": 576,
        "viewport_hint_y": 704,
        "viewport_hint_w": 32,
        "viewport_hint_h": 64
    });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/viewport", session_id))
                .header("content-type", "application/json")
                .body(Body::from(viewport_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/evaluate-publication",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "publication": publication }).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let stale: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stale["accepted"], false);
    assert_eq!(stale["reason"], "obsolete_publication");
}

#[tokio::test]
async fn progressive_revise_render_context_obsoletes_publications() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let first_step: Value = serde_json::from_slice(&bytes).unwrap();
    let publication = first_step["completed_tile_publications"][0].clone();
    assert!(first_step["render_contract_fingerprint"].is_string());

    let revision_body = serde_json::json!({
        "render_contract_fingerprint": "contract:test-v2",
        "visibility_fingerprint": "ocg:view:manual=1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/render-context", session_id))
                .header("content-type", "application/json")
                .body(Body::from(revision_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let revision: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(revision["changed"], true);
    assert_eq!(
        revision["current_render_contract_fingerprint"],
        "contract:test-v2"
    );
    assert_eq!(
        revision["current_visibility_fingerprint"],
        "ocg:view:manual=1"
    );
    assert_eq!(
        revision["obsolete_publications"][0]["reason"],
        "render_context_revised"
    );
    assert_eq!(
        revision["step_report"]["render_contract_fingerprint"],
        "contract:test-v2"
    );

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/evaluate-publication",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "publication": publication }).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let stale: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stale["accepted"], false);
    assert_eq!(stale["reason"], "obsolete_publication");
}

#[tokio::test]
async fn progressive_revise_render_context_accepts_render_contract_json() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );
    let engine = ContentEngine::open_bytes(pdf.clone()).expect("open contract source PDF");
    let mut contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .expect("build default contract");
    contract.clip = Some(DeviceClip {
        x: 128,
        y: 96,
        width: 96,
        height: 80,
    });
    contract.width = 96;
    contract.height = 80;
    contract.stride = 96 * 4;
    let contract_json = serde_json::to_string(&contract).expect("serialize render contract");

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let first_step: Value = serde_json::from_slice(&bytes).unwrap();
    let publication = first_step["completed_tile_publications"][0].clone();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/render-context", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "render_contract_json": contract_json }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let revision: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(revision["changed"], true);
    assert_eq!(
        revision["current_render_contract_fingerprint"],
        serde_json::json!(contract.cache_fingerprint())
    );
    assert_eq!(
        revision["obsolete_publications"][0]["reason"],
        "render_contract_revised"
    );
    assert_eq!(revision["step_report"]["total_units"], 4);
    assert_eq!(revision["step_report"]["completed_units"], 0);

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let status: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["token"]["page_width"], 96);
    assert_eq!(status["token"]["page_height"], 80);
    assert_eq!(status["token"]["total_tiles"], 4);

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let revised_step: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        revised_step["render_contract_fingerprint"],
        serde_json::json!(contract.cache_fingerprint())
    );
    assert_eq!(
        revised_step["completed_tile_publications"][0]["tile"]["x"],
        128
    );
    assert_eq!(
        revised_step["completed_tile_publications"][0]["tile"]["y"],
        96
    );

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/evaluate-publication",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "publication": publication }).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let stale: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stale["accepted"], false);
    assert_eq!(stale["reason"], "obsolete_publication");
}

#[tokio::test]
async fn progressive_queue_reports_adjacent_page_prefetch_preview() {
    let pdf = two_page_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
            ("viewport_hint_x", "128"),
            ("viewport_hint_y", "128"),
            ("viewport_hint_w", "96"),
            ("viewport_hint_h", "96"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/queue", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let queue: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        queue["adjacent_page_prefetches"][0]["priority"],
        "adjacent_page_preview"
    );
    assert_eq!(queue["adjacent_page_prefetches"][0]["page_number"], 2);
    assert!(queue["viewer_queue_preview"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["priority"] == "center_visible_tile"));
    let adjacent_rank = queue["viewer_queue_preview"]
        .as_array()
        .unwrap()
        .iter()
        .position(|item| item["priority"] == "adjacent_page_preview")
        .expect("adjacent-page preview is in queue");
    let background_rank = queue["viewer_queue_preview"]
        .as_array()
        .unwrap()
        .iter()
        .position(|item| item["priority"] == "background_prefetch")
        .expect("background prefetch is in queue");
    assert!(adjacent_rank < background_rank);

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let status: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        status["viewer_queue_report"]["adjacent_page_prefetches"][0]["page_number"],
        2
    );

    let response = app
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/callbacks", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let callbacks: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(callbacks["no_callback_after_terminal_state"], false);
    assert!(callbacks["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["callback"] == "adjacent_page_prefetch_ready"));
    assert!(callbacks["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["callback"] == "viewer_queue_item_scheduled"));
}

#[tokio::test]
async fn progressive_queue_execute_advances_owned_work_and_defers_adjacent_prefetch() {
    let pdf = two_page_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "256"),
            ("tile_height", "256"),
            ("viewport_hint_x", "128"),
            ("viewport_hint_y", "128"),
            ("viewport_hint_w", "96"),
            ("viewport_hint_h", "96"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/queue/execute", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_items": 8 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 131_072).await.unwrap();
    let execution: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(execution["terminal_suppressed"], false);
    assert!(execution["rendered_current_page_tiles"].as_u64().unwrap() > 0);
    assert!(
        execution["queue_after"]["completed_units"]
            .as_u64()
            .unwrap()
            > execution["queue_before"]["completed_units"]
                .as_u64()
                .unwrap()
    );
    assert!(execution["executed_items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["result"] == "rendered_current_page_tile"
            && item["tile_publication"].is_object()));
    assert!(execution["deferred_queue_items"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |item| item["result"] == "deferred_adjacent_page_prefetch_requires_page_session"
                && item["page_number"] == 2
        ));
}

#[tokio::test]
async fn progressive_adjacent_prefetch_execute_creates_child_session() {
    let pdf = two_page_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "256"),
            ("tile_height", "256"),
            ("viewport_hint_x", "128"),
            ("viewport_hint_y", "128"),
            ("viewport_hint_w", "96"),
            ("viewport_hint_h", "96"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/queue", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let queue: Value = serde_json::from_slice(&bytes).unwrap();
    let prefetch_identity = queue["adjacent_page_prefetches"][0]["prefetch_identity"]
        .as_str()
        .unwrap()
        .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/progressive/{}/adjacent-prefetch/execute",
                session_id
            ))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "prefetch_identity": prefetch_identity,
                    "max_tiles": 2
                })
                .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let execution: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["source_session_id"], session_id);
    assert!(execution["prefetch_session_id"].is_string());
    assert_eq!(execution["report"]["executed"], true);
    assert_eq!(execution["report"]["page_number"], 2);
    assert!(
        execution["report"]["render_step_report"]["rendered_this_step"]
            .as_u64()
            .unwrap()
            > 0
    );

    let child_session_id = execution["prefetch_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", child_session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let child_status: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(child_status["token"]["page_number"], 2);
    assert!(
        child_status["viewer_queue_report"]["completed_units"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test]
async fn progressive_revise_dirty_region_reports_obsolete_tile_publication() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 2 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let first_step: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(first_step["completed_units"], 2);
    let old_identity = first_step["publication_identity"]
        .as_str()
        .unwrap()
        .to_string();

    let dirty_body = serde_json::json!({
        "dirty_region_x": 72,
        "dirty_region_y": 8,
        "dirty_region_w": 8,
        "dirty_region_h": 8
    });
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/dirty-region", session_id))
                .header("content-type", "application/json")
                .body(Body::from(dirty_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let revision: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(revision["completed_units"], 1);
    assert_ne!(
        revision["publication_identity"].as_str().unwrap(),
        old_identity
    );
    assert_eq!(
        revision["obsolete_publications"][0]["publication_identity"],
        old_identity
    );
    assert_eq!(
        revision["obsolete_publications"][0]["reason"],
        "dirty_region_revised"
    );
    assert_eq!(revision["obsolete_publications"][0]["tile"]["x"], 64);
    assert!(
        revision["obsolete_publications"][0]["tile_publication_identity"]
            .as_str()
            .unwrap()
            .contains("tile_index=1")
    );

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/step", session_id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "max_tiles": 1 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let next_step: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(next_step["rendered_this_step"], 1);
    assert_eq!(next_step["completed_units"], 2);
    let dirty_publication = next_step["completed_tile_publications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|publication| publication["tile"]["x"] == 64)
        .expect("dirty tile publication");
    assert_eq!(
        dirty_publication["publication_identity"],
        revision["publication_identity"]
    );
}

#[tokio::test]
async fn progressive_status_unknown_session_returns_error() {
    let app = build_app();
    let response = app
        .oneshot(
            Request::get("/api/v1/progressive/nonexistent-session-id/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Unknown session -> 400 (invalid parameter)
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn progressive_close_releases_session() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "128"),
            ("tile_height", "128"),
        ],
    );

    let app = build_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/progressive/{}/close", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let close_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(close_json["closed"], true);

    let response = app
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn progressive_sessions_are_scoped_to_caller_identity() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "128"),
            ("tile_height", "128"),
        ],
    );

    let app = build_app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .header("x-api-key", "owner-a")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .header("x-api-key", "owner-b")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .header("x-api-key", "owner-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn progressive_finish_before_complete_returns_error() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "64"),
            ("tile_height", "64"),
        ],
    );

    let app = build_app();

    // Start (don't step)
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    // Finish without stepping - should fail
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/finish", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let error_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error_json["error"], "invalid_parameter");
    assert!(error_json["message"]
        .as_str()
        .unwrap()
        .contains("progressive render cannot finish before all tiles are complete"));
}

#[tokio::test]
async fn progressive_status_returns_token_json() {
    let pdf = minimal_pdf();
    let (ct, body) = start_multipart(
        &pdf,
        &[
            ("page", "1"),
            ("dpi", "72"),
            ("tile_width", "128"),
            ("tile_height", "128"),
        ],
    );

    let app = build_app();

    // Start
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/progressive/start")
                .header("content-type", &ct)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let start_json: Value = serde_json::from_slice(&bytes).unwrap();
    let session_id = start_json["session_id"].as_str().unwrap().to_string();

    // Status
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/progressive/{}/status", session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let status_json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status_json["session_id"], session_id);
    assert!(status_json["state"].is_string());
    assert!(status_json["token"].is_object());
    assert_eq!(status_json["token"]["schema_version"], 1);
}
