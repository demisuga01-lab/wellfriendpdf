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
