//! HTTP routes for `edit-metadata` and `pdf-to-epub`.
use std::io::Read;
use std::path::PathBuf;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("engine/tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

fn multipart(file: &[u8], options: Option<&str>) -> (String, Vec<u8>) {
    let boundary = "wellfriendpdf-meta-epub-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"input.pdf\"\r\n\
          Content-Type: application/pdf\r\n\r\n",
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

async fn post(
    path: &str,
    file: &[u8],
    options: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
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
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 64 << 20).await.unwrap();
    (status, headers, bytes.to_vec())
}

#[tokio::test]
async fn edit_metadata_route_returns_incremental_pdf() {
    let input = fixture("minimal.pdf");
    let (status, headers, bytes) = post(
        "/api/v1/edit-metadata",
        &input,
        Some(r#"{"title":"Route title","custom":{"Ticket":"42"}}"#),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        headers.get("x-signature-policy").unwrap(),
        "safe_incremental"
    );
    assert!(bytes.starts_with(&input));
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(bytes).unwrap();
    assert_eq!(
        engine.document_info().unwrap().title.as_deref(),
        Some("Route title")
    );
    let (info, xmp) = wellfriendpdf_engine::metadata_edit::read_metadata_views(&engine);
    assert_eq!(info["Ticket"], "42");
    assert!(xmp.unwrap().contains("<pdfx:Ticket>42</pdfx:Ticket>"));
}

#[tokio::test]
async fn edit_metadata_route_rejects_bad_options() {
    let input = fixture("minimal.pdf");
    let (status, _, body) = post("/api/v1/edit-metadata", &input, Some(r#"{"titel":"x"}"#)).await;
    assert!(status.is_client_error(), "{status}");
    assert!(String::from_utf8_lossy(&body).contains("invalid_options"));
    let (status, _, _) = post("/api/v1/edit-metadata", &input, None).await;
    assert!(status.is_client_error());
}

#[tokio::test]
async fn pdf_to_epub_route_returns_epub() {
    let (status, headers, bytes) = post(
        "/api/v1/pdf-to-epub",
        &fixture("tracemonkey.pdf"),
        Some(r#"{"split":"pages","pages_per_chapter":7,"include_images":false}"#),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(headers.get("content-type").unwrap(), "application/epub+zip");
    assert_eq!(headers.get("x-chapter-count").unwrap(), "2");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
    let mut chapter = String::new();
    archive
        .by_name("OEBPS/chapter-001.xhtml")
        .unwrap()
        .read_to_string(&mut chapter)
        .unwrap();
    assert!(chapter.contains("Trace"));
}
