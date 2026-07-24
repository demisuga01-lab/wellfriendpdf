//! Tests for the local/cloud HTTP OCR reference backends.
//!
//! The backends live in `examples/` so integrators can copy them as templates,
//! but the example source is included here and exercised through the same
//! `ocr::dispatch` containment layer as any real backend. No external network is
//! used: every test talks to a one-shot localhost stub server.

#[path = "../examples/ocr_http_backends.rs"]
mod ocr_http_backends;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ocr_http_backends::{CloudHttpOcrBackend, CloudHttpOcrConfig, LocalHttpOcrBackend};
use wellfriendpdf_engine::ocr::dispatch::recognize_contained;
use wellfriendpdf_engine::{ErrorKind, OcrEngine, OcrImage, OcrOptions};

struct StubRequest {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl StubRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct StubResponse {
    status: u16,
    body: String,
    delay: Duration,
}

impl StubResponse {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(status: u16, body: &str, delay: Duration) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay,
        }
    }
}

struct StubServer {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    join: JoinHandle<()>,
}

impl StubServer {
    fn finish(self) {
        self.join.join().expect("stub server thread should finish");
    }
}

fn start_stub<F>(requests: usize, handler: F) -> StubServer
where
    F: Fn(usize, StubRequest) -> StubResponse + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local stub");
    let addr = listener.local_addr().expect("local addr");
    let endpoint = format!("http://{addr}/ocr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_thread = Arc::clone(&hits);
    let handler = Arc::new(handler);

    let join = std::thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().expect("accept OCR request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let n = hits_for_thread.fetch_add(1, Ordering::SeqCst) + 1;
            let request = read_request(&mut stream);
            let response = handler(n, request);
            if !response.delay.is_zero() {
                std::thread::sleep(response.delay);
            }
            write_response(&mut stream, response);
        }
    });

    StubServer {
        endpoint,
        hits,
        join,
    }
}

fn read_request(stream: &mut TcpStream) -> StubRequest {
    let mut raw = Vec::new();
    let mut buf = [0u8; 512];
    let mut header_end = None;
    let mut content_len = None;

    loop {
        let n = stream.read(&mut buf).expect("read request");
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if header_end.is_none() {
            if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = Some(pos);
                content_len = Some(parse_content_length(&raw[..pos]));
            }
        }
        if let (Some(pos), Some(len)) = (header_end, content_len) {
            if raw.len() >= pos + 4 + len {
                break;
            }
        }
    }

    let pos = header_end.expect("headers complete");
    let head = String::from_utf8_lossy(&raw[..pos]);
    let mut headers = Vec::new();
    for line in head.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let len = content_len.unwrap_or(0);
    let body_start = pos + 4;
    let body = raw[body_start..body_start + len].to_vec();
    StubRequest { headers, body }
}

fn parse_content_length(head: &[u8]) -> usize {
    let head = String::from_utf8_lossy(head);
    for line in head.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                return value.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}

fn write_response(stream: &mut TcpStream, response: StubResponse) {
    let reason = match response.status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        429 => "Too Many Requests",
        500 => "Server Error",
        _ => "Status",
    };
    let body = response.body.as_bytes();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn tiny_image() -> OcrImage {
    OcrImage {
        width: 2,
        height: 2,
        gray: vec![0, 128, 200, 255],
    }
}

fn ocr_opts() -> OcrOptions {
    OcrOptions {
        languages: vec!["eng".to_string()],
        dpi: 150,
        psm: Some(6),
    }
}

#[test]
fn local_http_backend_maps_stub_words() {
    let server = start_stub(1, |_n, request| {
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("\"encoding\":\"gray8-base64\""));
        assert!(body.contains("\"width\":2"));
        assert!(body.contains("\"dpi\":150"));
        StubResponse::json(
            200,
            r#"{"words":[{"text":"Local","bbox":[1,2,10,12],"confidence":0.91,"line_id":7}]}"#,
        )
    });

    let backend = LocalHttpOcrBackend::new(&server.endpoint)
        .unwrap()
        .with_timeout(Duration::from_secs(2))
        .with_max_concurrency(3);
    assert_eq!(backend.max_concurrency(), 3);
    let engine: Arc<dyn OcrEngine> = Arc::new(backend);
    let page = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_secs(2)),
    )
    .unwrap();

    assert_eq!(page.words.len(), 1);
    assert_eq!(page.words[0].text, "Local");
    assert_eq!(page.words[0].bbox, [1.0, 2.0, 10.0, 12.0]);
    assert!((page.words[0].confidence - 0.91).abs() < 0.001);
    assert_eq!(page.words[0].line_id, Some(7));
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    server.finish();
}

#[test]
fn cloud_http_backend_retries_429_with_backoff_and_auth() {
    let server = start_stub(2, |n, request| {
        if n == 1 {
            return StubResponse::json(429, r#"{"error":"slow down"}"#);
        }
        assert_eq!(
            request.header("authorization"),
            Some("Bearer example-token")
        );
        StubResponse::json(
            200,
            r#"{"words":[{"text":"Cloud","bbox":[3,4,30,20],"confidence":0.8}]}"#,
        )
    });
    let config = CloudHttpOcrConfig::new(&server.endpoint)
        .with_auth_header("Authorization", "Bearer example-token")
        .with_retries(1, Duration::from_millis(75))
        .with_timeout(Duration::from_secs(2))
        .with_max_concurrency(2);
    let engine: Arc<dyn OcrEngine> = Arc::new(CloudHttpOcrBackend::new(config).unwrap());

    let started = Instant::now();
    let page = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_secs(3)),
    )
    .unwrap();

    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "retry backoff should be observable"
    );
    assert_eq!(page.words[0].text, "Cloud");
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);
    server.finish();
}

#[test]
fn malformed_http_response_is_a_page_error() {
    let server = start_stub(1, |_n, _request| StubResponse::json(200, r#"{"items":[]}"#));
    let engine: Arc<dyn OcrEngine> =
        Arc::new(CloudHttpOcrBackend::new(CloudHttpOcrConfig::new(&server.endpoint)).unwrap());

    let err = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_secs(2)),
    )
    .unwrap_err();

    assert_eq!(err.kind(), ErrorKind::Parse);
    assert!(err.to_string().contains("words"));
    server.finish();
}

#[test]
fn auth_failure_is_actionable() {
    let server = start_stub(1, |_n, _request| {
        StubResponse::json(401, r#"{"error":"bad auth"}"#)
    });
    let config = CloudHttpOcrConfig::new(&server.endpoint)
        .with_auth_header("Authorization", "Bearer bad-token");
    let engine: Arc<dyn OcrEngine> = Arc::new(CloudHttpOcrBackend::new(config).unwrap());

    let err = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_secs(2)),
    )
    .unwrap_err();

    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
    assert!(err.to_string().contains("auth"));
    server.finish();
}

#[test]
fn backend_timeout_and_outer_dispatch_timeout_both_cut_off() {
    let backend_timeout_server = start_stub(1, |_n, _request| {
        StubResponse::delayed(
            200,
            r#"{"words":[{"text":"Late","bbox":[1,1,2,2]}]}"#,
            Duration::from_millis(300),
        )
    });
    let config = CloudHttpOcrConfig::new(&backend_timeout_server.endpoint)
        .with_timeout(Duration::from_millis(50))
        .with_retries(0, Duration::ZERO);
    let engine: Arc<dyn OcrEngine> = Arc::new(CloudHttpOcrBackend::new(config).unwrap());
    let started = Instant::now();
    let err = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_secs(2)),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
    backend_timeout_server.finish();

    let outer_timeout_server = start_stub(1, |_n, _request| {
        StubResponse::delayed(
            200,
            r#"{"words":[{"text":"Late","bbox":[1,1,2,2]}]}"#,
            Duration::from_millis(300),
        )
    });
    let config = CloudHttpOcrConfig::new(&outer_timeout_server.endpoint)
        .with_timeout(Duration::from_secs(2))
        .with_retries(0, Duration::ZERO);
    let engine: Arc<dyn OcrEngine> = Arc::new(CloudHttpOcrBackend::new(config).unwrap());
    let started = Instant::now();
    let err = recognize_contained(
        &engine,
        &tiny_image(),
        &ocr_opts(),
        Some(Duration::from_millis(50)),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
    outer_timeout_server.finish();
}
