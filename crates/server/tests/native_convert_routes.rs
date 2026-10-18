//! HTTP routes for the Rust-native text/Markdown/CSV/JSON/SVG to PDF converters.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::util::ServiceExt;

fn multipart(file: &[u8], options: Option<&str>) -> (String, Vec<u8>) {
    let boundary = "wellfriendpdf-native-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"input\"\r\n\
          Content-Type: application/octet-stream\r\n\r\n",
    );
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n");
    if let Some(options) = options {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"options\"\r\n\r\n{options}\r\n")
                .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn post(path: &str, file: &[u8], options: Option<&str>) -> (StatusCode, String, Vec<u8>) {
    let (content_type, body) = multipart(file, options);
    let response = wellfriendpdf_server::app::create_app()
        .oneshot(
            Request::post(path)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let pages = response
        .headers()
        .get("x-page-count")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = to_bytes(response.into_body(), 64 << 20).await.unwrap();
    (status, pages, bytes.to_vec())
}

fn assert_pdf(bytes: &[u8], expected_text: &str) {
    assert!(bytes.starts_with(b"%PDF-"));
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    assert!(engine.page_count().unwrap() >= 1);
    if !expected_text.is_empty() {
        let text = engine.get_page_text(1).unwrap();
        assert!(
            text.contains(expected_text),
            "missing {expected_text:?} in {text:?}"
        );
    }
}

#[tokio::test]
async fn text_to_pdf_route_returns_pdf() {
    let (status, pages, bytes) = post(
        "/api/v1/text-to-pdf",
        "Hello route\nПривет".as_bytes(),
        Some(r#"{"page_size":"letter","monospace":true}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pages, "1");
    assert_pdf(&bytes, "Hello route");
}

#[tokio::test]
async fn markdown_to_pdf_route_returns_pdf_and_rejects_base_dir() {
    let (status, _, bytes) =
        post("/api/v1/markdown-to-pdf", b"# Title\n\nBody *text*.", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_pdf(&bytes, "Title");
    let (status, _, body) = post(
        "/api/v1/markdown-to-pdf",
        b"# Title",
        Some(r#"{"base_dir":"/etc"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&body).contains("invalid_options"));
}

#[tokio::test]
async fn csv_to_pdf_route_reports_column_cap() {
    let (status, _, bytes) = post("/api/v1/csv-to-pdf", b"name;qty\nApple;3\n", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_pdf(&bytes, "Apple");
    let (status, _, body) = post(
        "/api/v1/csv-to-pdf",
        b"a,b,c,d\n1,2,3,4\n",
        Some(r#"{"max_columns":3}"#),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json.to_string().contains("csv_column_limit_exceeded"));
}

#[tokio::test]
async fn json_and_svg_routes_return_pdf() {
    let (status, _, bytes) = post("/api/v1/json-to-pdf", br#"{"route":"json"}"#, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_pdf(&bytes, "route");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="red"/></svg>"#;
    let (status, pages, bytes) = post(
        "/api/v1/svg-to-pdf",
        svg,
        Some(r#"{"page":"fit","page_size":"a4"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pages, "1");
    assert_pdf(&bytes, "");
}

#[tokio::test]
async fn invalid_options_are_rejected() {
    let (status, _, body) = post("/api/v1/text-to-pdf", b"x", Some(r#"{"font_size":40}"#)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&body).contains("font_size_out_of_range"));
}
