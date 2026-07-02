//! Reference OCR backends that call an HTTP inference endpoint.
//!
//! This file is intentionally an example, not a built-in engine feature. It is a
//! copy-paste starting point for two common integrations:
//!
//! - `LocalHttpOcrBackend`: a self-hosted OCR/model server on localhost.
//! - `CloudHttpOcrBackend`: a provider-neutral hosted API adapter with explicit
//!   endpoint/auth, timeout, retry, and low concurrency defaults.
//!
//! The request shape is deliberately simple JSON:
//!
//! ```json
//! {
//!   "image": {
//!     "encoding": "gray8-base64",
//!     "width": 2480,
//!     "height": 3508,
//!     "dpi": 300,
//!     "data": "..."
//!   },
//!   "languages": ["eng"],
//!   "psm": null
//! }
//! ```
//!
//! The endpoint returns:
//!
//! ```json
//! {
//!   "words": [
//!     {"text": "Hello", "bbox": [10, 20, 80, 40], "confidence": 0.98, "line_id": 0}
//!   ]
//! }
//! ```
//!
//! Production cloud providers usually require HTTPS and provider-specific JSON.
//! Keep this adapter's `OcrEngine` implementation and replace `post_json` with
//! your production HTTP client/TLS stack and mapping code.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use oxide_engine::{OcrEngine, OcrImage, OcrOptions, OcrPage, OcrWord, OxideError, Result};
use serde_json::{json, Value};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
struct HttpEndpoint {
    host: String,
    host_header: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn parse(endpoint: &str) -> Result<Self> {
        if endpoint.trim().is_empty() {
            return Err(OxideError::UnsupportedFeature(
                "OCR HTTP endpoint is required; no default provider endpoint is bundled"
                    .to_string(),
            ));
        }
        if endpoint.starts_with("https://") {
            return Err(OxideError::UnsupportedFeature(
                "this reference backend uses plain http:// for local stub tests; replace \
                 post_json with a TLS-capable client for production HTTPS providers"
                    .to_string(),
            ));
        }
        let Some(rest) = endpoint.strip_prefix("http://") else {
            return Err(OxideError::UnsupportedFeature(
                "OCR HTTP endpoint must be an explicit http:// URL".to_string(),
            ));
        };

        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return Err(OxideError::UnsupportedFeature(
                "OCR HTTP endpoint URL is missing a host".to_string(),
            ));
        }

        let (host, port) = parse_authority(authority)?;
        Ok(Self {
            host,
            host_header: authority.to_string(),
            port,
            path,
        })
    }

    fn is_loopback(&self) -> bool {
        matches!(
            self.host.as_str(),
            "localhost" | "127.0.0.1" | "::1" | "[::1]"
        )
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16)> {
    if let Some(host) = authority.strip_prefix('[') {
        let Some((host, rest)) = host.split_once(']') else {
            return Err(OxideError::UnsupportedFeature(
                "invalid bracketed IPv6 OCR endpoint".to_string(),
            ));
        };
        let port = if let Some(port) = rest.strip_prefix(':') {
            port.parse::<u16>().map_err(|_| {
                OxideError::UnsupportedFeature("invalid OCR endpoint port".to_string())
            })?
        } else {
            80
        };
        return Ok((host.to_string(), port));
    }

    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            let port = port.parse::<u16>().map_err(|_| {
                OxideError::UnsupportedFeature("invalid OCR endpoint port".to_string())
            })?;
            Ok((host.to_string(), port))
        }
        _ => Ok((authority.to_string(), 80)),
    }
}

#[derive(Clone, Debug)]
struct HttpOcrBackend {
    endpoint: HttpEndpoint,
    auth_header: Option<(String, String)>,
    timeout: Duration,
    max_retries: usize,
    retry_backoff: Duration,
    max_concurrency: usize,
    name: &'static str,
}

impl HttpOcrBackend {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> Result<OcrPage> {
        if !image.is_valid() {
            return Err(OxideError::ParseError(
                "OCR image is empty or malformed".to_string(),
            ));
        }

        let body = serde_json::to_vec(&json!({
            "image": {
                "encoding": "gray8-base64",
                "width": image.width,
                "height": image.height,
                "dpi": opts.dpi,
                "data": base64_encode(&image.gray),
            },
            "languages": &opts.languages,
            "psm": opts.psm,
        }))
        .map_err(|e| OxideError::ParseError(format!("could not encode OCR HTTP request: {e}")))?;

        let response = self.post_with_retries(&body)?;
        parse_response_words(&response)
    }

    fn post_with_retries(&self, body: &[u8]) -> Result<Vec<u8>> {
        let mut attempt = 0usize;
        loop {
            let response = post_json(
                &self.endpoint,
                self.auth_header.as_ref(),
                body,
                self.timeout,
            )?;
            match response.status {
                200..=299 => return Ok(response.body),
                401 | 403 => {
                    return Err(OxideError::UnsupportedFeature(format!(
                        "OCR HTTP backend '{}' was rejected by the endpoint (HTTP {}); \
                         check the configured auth header",
                        self.name, response.status
                    )));
                }
                429 | 500 | 502 | 503 | 504 if attempt < self.max_retries => {
                    attempt += 1;
                    std::thread::sleep(scale_duration(self.retry_backoff, attempt));
                }
                429 => {
                    return Err(OxideError::Cancelled(format!(
                        "OCR HTTP backend '{}' was rate-limited after {} attempt(s)",
                        self.name,
                        attempt + 1
                    )));
                }
                status => {
                    let snippet = String::from_utf8_lossy(&response.body);
                    return Err(OxideError::ParseError(format!(
                        "OCR HTTP backend '{}' returned HTTP {status}: {}",
                        self.name,
                        snippet.trim()
                    )));
                }
            }
        }
    }
}

fn scale_duration(base: Duration, factor: usize) -> Duration {
    let millis = base.as_millis().saturating_mul(factor as u128);
    Duration::from_millis(millis.min(u64::MAX as u128) as u64)
}

/// A localhost inference-server backend for self-hosted OCR/model code.
#[derive(Clone, Debug)]
pub struct LocalHttpOcrBackend {
    inner: HttpOcrBackend,
}

impl LocalHttpOcrBackend {
    pub fn new(endpoint: impl AsRef<str>) -> Result<Self> {
        let endpoint = HttpEndpoint::parse(endpoint.as_ref())?;
        if !endpoint.is_loopback() {
            return Err(OxideError::UnsupportedFeature(
                "LocalHttpOcrBackend only accepts localhost/loopback endpoints; use \
                 CloudHttpOcrBackend for non-local providers"
                    .to_string(),
            ));
        }
        Ok(Self {
            inner: HttpOcrBackend {
                endpoint,
                auth_header: None,
                timeout: DEFAULT_TIMEOUT,
                max_retries: 0,
                retry_backoff: DEFAULT_BACKOFF,
                max_concurrency: 1,
                name: "local-http-ocr",
            },
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner.timeout = timeout;
        self
    }

    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.inner.max_concurrency = max_concurrency.max(1);
        self
    }
}

impl OcrEngine for LocalHttpOcrBackend {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> Result<OcrPage> {
        self.inner.recognize(image, opts)
    }

    fn name(&self) -> &str {
        self.inner.name
    }

    fn max_concurrency(&self) -> usize {
        self.inner.max_concurrency
    }
}

/// Provider-neutral cloud OCR config. No endpoint or key is bundled.
#[derive(Clone, Debug)]
pub struct CloudHttpOcrConfig {
    endpoint: String,
    auth_header: Option<(String, String)>,
    timeout: Duration,
    max_retries: usize,
    retry_backoff: Duration,
    max_concurrency: usize,
}

impl CloudHttpOcrConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            auth_header: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: 2,
            retry_backoff: DEFAULT_BACKOFF,
            max_concurrency: 2,
        }
    }

    pub fn with_auth_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.auth_header = Some((name.into(), value.into()));
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_retries(mut self, max_retries: usize, retry_backoff: Duration) -> Self {
        self.max_retries = max_retries;
        self.retry_backoff = retry_backoff;
        self
    }

    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency.max(1);
        self
    }
}

/// A provider-neutral hosted OCR adapter. Adapt the request/response mapping to
/// your provider and keep the explicit endpoint/auth/timeout/retry shape.
#[derive(Clone, Debug)]
pub struct CloudHttpOcrBackend {
    inner: HttpOcrBackend,
}

impl CloudHttpOcrBackend {
    pub fn new(config: CloudHttpOcrConfig) -> Result<Self> {
        Ok(Self {
            inner: HttpOcrBackend {
                endpoint: HttpEndpoint::parse(&config.endpoint)?,
                auth_header: config.auth_header,
                timeout: config.timeout,
                max_retries: config.max_retries,
                retry_backoff: config.retry_backoff,
                max_concurrency: config.max_concurrency.max(1),
                name: "cloud-http-ocr",
            },
        })
    }
}

impl OcrEngine for CloudHttpOcrBackend {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> Result<OcrPage> {
        self.inner.recognize(image, opts)
    }

    fn name(&self) -> &str {
        self.inner.name
    }

    fn max_concurrency(&self) -> usize {
        self.inner.max_concurrency
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn post_json(
    endpoint: &HttpEndpoint,
    auth_header: Option<&(String, String)>,
    body: &[u8],
    timeout: Duration,
) -> Result<HttpResponse> {
    let started = Instant::now();
    let addr = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            OxideError::UnsupportedFeature(format!(
                "could not resolve OCR endpoint host '{}'",
                endpoint.host
            ))
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            OxideError::Cancelled(format!(
                "OCR HTTP endpoint connect exceeded {}ms",
                timeout.as_millis()
            ))
        } else {
            OxideError::Io(e)
        }
    })?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .unwrap_or_else(|| Duration::from_millis(1));
    stream.set_read_timeout(Some(remaining))?;
    stream.set_write_timeout(Some(remaining))?;

    let mut request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        endpoint.path,
        endpoint.host_header,
        body.len()
    );
    if let Some((name, value)) = auth_header {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .map_err(timeout_or_io)?;
    stream.write_all(body).map_err(timeout_or_io)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(timeout_or_io)?;
    parse_http_response(&raw)
}

fn timeout_or_io(e: std::io::Error) -> OxideError {
    if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock {
        OxideError::Cancelled("OCR HTTP request exceeded its configured timeout".to_string())
    } else {
        OxideError::Io(e)
    }
}

fn parse_http_response(raw: &[u8]) -> Result<HttpResponse> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| OxideError::ParseError("malformed OCR HTTP response".to_string()))?;
    let (head, body_with_sep) = raw.split_at(split);
    let body = body_with_sep[4..].to_vec();
    let head = String::from_utf8_lossy(head);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| OxideError::ParseError("missing OCR HTTP status code".to_string()))?;
    Ok(HttpResponse { status, body })
}

fn parse_response_words(body: &[u8]) -> Result<OcrPage> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| OxideError::ParseError(format!("OCR HTTP response is not valid JSON: {e}")))?;
    let words = value
        .get("words")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OxideError::ParseError("OCR HTTP response must contain a 'words' array".to_string())
        })?;

    let mut out = Vec::with_capacity(words.len());
    for item in words {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OxideError::ParseError("OCR HTTP word is missing string 'text'".to_string())
            })?
            .to_string();
        let bbox_values = item
            .get("bbox")
            .and_then(Value::as_array)
            .ok_or_else(|| OxideError::ParseError("OCR HTTP word is missing 'bbox'".to_string()))?;
        if bbox_values.len() != 4 {
            return Err(OxideError::ParseError(
                "OCR HTTP word bbox must have four numbers".to_string(),
            ));
        }
        let mut bbox = [0.0f64; 4];
        for (i, v) in bbox_values.iter().enumerate() {
            bbox[i] = v.as_f64().ok_or_else(|| {
                OxideError::ParseError("OCR HTTP bbox values must be numeric".to_string())
            })?;
        }
        let confidence = item
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0) as f32;
        let line_id = item
            .get("line_id")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());

        out.push(OcrWord {
            text,
            bbox,
            confidence,
            line_id,
        });
    }
    Ok(OcrPage::new(out))
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(not(test))]
fn main() {
    eprintln!(
        "This example exports LocalHttpOcrBackend and CloudHttpOcrBackend templates. \
         Copy the file into your integration and adapt the request/response mapping."
    );
}
