# OCR in Oxide

Oxide is offline by default. The core engine does not bundle a model, phone
home, require provider SDKs, or choose a cloud vendor. It owns the PDF side of
OCR and exposes one backend seam:

1. Classify pages as digital-born, searchable scan, or scanned.
2. Render only pages selected by the OCR policy.
3. Preprocess each rendered page as grayscale image data.
4. Call an `OcrEngine`.
5. Merge returned word boxes into the canonical document model.

Tesseract is the first-class shipped backend because it is local and free. The
local-AI and cloud-AI paths are reference implementations: copy them, adapt the
model/provider mapping, and keep the same seam contract.

## The Backend Contract

Rust backends implement `oxide_engine::OcrEngine`:

```rust
use oxide_engine::{OcrEngine, OcrImage, OcrOptions, OcrPage, OcrWord};

struct MyBackend;

impl OcrEngine for MyBackend {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions)
        -> oxide_engine::Result<OcrPage>
    {
        let _ = (image, opts);
        Ok(OcrPage::new(vec![OcrWord {
            text: "Hello".to_string(),
            bbox: [72.0, 60.0, 140.0, 88.0],
            confidence: 0.98,
            line_id: Some(0),
        }]))
    }

    fn name(&self) -> &str { "my-backend" }
    fn max_concurrency(&self) -> usize { 1 }
}
```

`recognize` receives one page image:

| Field | Meaning |
| --- | --- |
| `image.gray` | Raw 8-bit grayscale bytes, row-major. |
| `image.width`, `image.height` | Image dimensions in pixels. |
| `opts.dpi` | Render DPI, usually 300. |
| `opts.languages` | Language hints such as `["eng"]`. |
| `opts.psm` | Optional page segmentation hint. |

Return words with:

| Field | Meaning |
| --- | --- |
| `text` | Recognized word text. |
| `bbox` | `[x0, y0, x1, y1]` in the same image-pixel frame, origin top-left, y down. |
| `confidence` | `0.0..1.0`, clamped by Oxide. |
| `line_id` | Optional line grouping. Recommended for readable line assembly. |

Do not return PDF points. Do not flip y coordinates. Oxide maps image-pixel
boxes back into page space.

## Policy And Activation

OCR is never implicit. Callers choose `OcrPolicy`:

| Policy | Behavior |
| --- | --- |
| `Off` | Never OCR. Byte-identical to no backend. |
| `Auto` | OCR only classifier-detected scanned pages. |
| `Force` | OCR every selected page, including digital-born pages. |

`ParseOptions` owns the backend and policy:

```rust
use std::sync::Arc;
use oxide_engine::{ContentEngine, OcrPolicy, ParseOptions};

# fn main() -> oxide_engine::Result<()> {
# let bytes = Vec::new();
# let backend = Arc::new(MyBackend);
let engine = ContentEngine::open_bytes(bytes)?;
let doc = engine.parse_document(&ParseOptions {
    ocr: Some(backend),
    ocr_policy: OcrPolicy::Auto,
    ocr_dpi: 300,
    ocr_timeout: Some(std::time::Duration::from_secs(60)),
    ..Default::default()
})?;
# let _ = doc;
# Ok(())
# }
```

## Robustness Guarantees

Every backend call goes through `ocr::dispatch::recognize_contained`:

- A backend panic becomes a per-page error.
- An engine-side timeout can bound a hung backend.
- A backend error fails that page only.
- The run continues unless the caller's outer policy cancels the operation.

Backends should still enforce their own efficient timeout when they own a
killable resource. The Tesseract backend does this by killing and reaping the
subprocess before the outer engine timeout is needed.

## Memory And Concurrency

Oxide renders and OCRs pages in a bounded window. Backends must not accumulate
all page images.

`OcrEngine::max_concurrency()` tells Oxide how many pages may be recognized at
once:

- Tesseract returns a CPU-tied process count with a sane cap.
- Python backends currently return `1`, because the wrapper enters Python under
  the GIL.
- Cloud APIs should usually return `1` or `2` to respect rate limits.

Returning `0` is treated as `1`.

## Tesseract Backend

The shipped backend lives in `crates/oxide-ocr-tesseract`.

It drives the external `tesseract` binary as a child process. It links no C
library. Per page it writes a temporary PGM, invokes Tesseract TSV output, parses
word-level boxes/confidence, and deletes the temporary file.

Install requirements:

- Tesseract binary on `PATH`, or use `TesseractEngine::with_path`.
- Language data for each requested language, such as `eng.traineddata`.

Rust usage:

```rust
use std::sync::Arc;
use oxide_engine::{ContentEngine, OcrPolicy, ParseOptions};
use oxide_ocr_tesseract::TesseractEngine;

# fn main() -> oxide_engine::Result<()> {
# let bytes = Vec::new();
let backend = TesseractEngine::new()?
    .with_timeout(std::time::Duration::from_secs(60));
let engine = ContentEngine::open_bytes(bytes)?;
let doc = engine.parse_document(&ParseOptions {
    ocr: Some(Arc::new(backend)),
    ocr_policy: OcrPolicy::Auto,
    ocr_dpi: 300,
    ..Default::default()
})?;
# let _ = doc;
# Ok(())
# }
```

Typed failure cases:

| Failure | Surface |
| --- | --- |
| Binary missing | `UnsupportedFeature` with install guidance. |
| Language data missing | `UnsupportedFeature` naming tessdata guidance. |
| Timeout | `Cancelled`; child process is killed and reaped. |
| Nonzero exit / bad TSV | `ParseError` with stderr/context. |

Live tests auto-skip when Tesseract or language data is absent. They do not
fabricate recognition results.

## CLI

Build the CLI with the OCR feature:

```sh
cargo build --release -p oxide-cli --features ocr
```

Use optional-value `--ocr` flags:

```sh
oxide parse scanned.pdf --ocr --ocr-lang eng --ocr-dpi 300
oxide parse scanned.pdf --ocr force --format html
oxide extract-text scanned.pdf --ocr auto
oxide extract-fields invoice-scan.pdf --ocr --type invoice
oxide chunk scanned.pdf --ocr --target-tokens 512
```

`--ocr` alone means `auto`; explicit values are `off`, `auto`, and `force`.

If the CLI is not compiled with OCR, `--ocr` returns an actionable
"rebuild with --features ocr" error. If it is compiled with OCR but Tesseract is
missing, backend construction returns install guidance.

`extract-tables --ocr` is currently an explicit unsupported path: table-grid
reconstruction from OCR word boxes remains a known gap. Use `extract-fields
--ocr` for scanned key-value/line-item extraction or `parse --ocr` for recovered
text and structure.

## Server

The server has one process-wide OCR hook. Build with:

```sh
cargo build --release -p oxide-server --features ocr
```

Then set:

```sh
OXIDE_OCR=auto
```

Accepted values are `off`, `auto`, and `force` (`1`, `on`, and `true` map to
`auto`). With the `ocr` feature enabled, the server discovers Tesseract on
`PATH` and registers it through `crates/server/src/ocr.rs`.

Embedded users can call:

```rust
oxide_server::ocr::set_backend(backend, oxide_engine::OcrPolicy::Auto);
```

Do not add a second configuration path. Parser endpoints receive the registered
backend through the existing hook.

## Python Local-AI Backend

Python integrations pass any object with:

```python
def recognize(self, image_bytes: bytes, info: dict) -> list[dict]: ...
```

`image_bytes` is raw 8-bit grayscale in `width * height` row-major order.
`info` contains `width`, `height`, `dpi`, `languages`, and `psm`.

Example:

```python
import oxide

class MyModel:
    name = "my-local-model"
    version = "2026-07"

    def recognize(self, image_bytes, info):
        # YOUR MODEL HERE
        return [
            {"text": "Hello", "bbox": [72, 60, 140, 88],
             "confidence": 0.98, "line_id": 0},
        ]

doc = oxide.open("scan.pdf")
markdown = doc.to_markdown(ocr=MyModel(), ocr_lang="eng", ocr_dpi=300)
print(markdown)
```

A runnable local-AI template is provided at:

```text
crates/oxide-py/examples/local_ai_ocr_backend.py
```

The template marks the `YOUR MODEL HERE` boundary and uses real `pytesseract`
when `pillow` and `pytesseract` are installed. If those packages are absent, it
raises a setup error instead of fabricating words.

Python concurrency is intentionally `1` in the current binding. The wrapper
enters Python under the GIL; if your model releases the GIL or uses a separate
process pool, own that concurrency in Python or write a Rust backend that
advertises a wider `max_concurrency`.

## Local HTTP Backend

Use this when your model runs behind a self-hosted inference server.

Template:

```text
crates/engine/examples/ocr_http_backends.rs
```

`LocalHttpOcrBackend` posts page images to an explicit loopback endpoint and
maps the JSON response back to `OcrPage`.

```rust
use std::sync::Arc;
use oxide_engine::{ContentEngine, OcrPolicy, ParseOptions};

#[path = "ocr_http_backends.rs"]
mod ocr_http_backends;
use ocr_http_backends::LocalHttpOcrBackend;

# fn main() -> oxide_engine::Result<()> {
# let bytes = Vec::new();
let backend = LocalHttpOcrBackend::new("http://127.0.0.1:9000/ocr")?
    .with_max_concurrency(1);
let engine = ContentEngine::open_bytes(bytes)?;
let doc = engine.parse_document(&ParseOptions {
    ocr: Some(Arc::new(backend)),
    ocr_policy: OcrPolicy::Auto,
    ..Default::default()
})?;
# let _ = doc;
# Ok(())
# }
```

The reference request uses base64-encoded grayscale bytes plus width, height,
DPI, languages, and PSM. Adapt that request shape to your server.

## Cloud HTTP Backend

Use this as a provider-neutral template for hosted OCR/vision APIs.

The cloud template requires explicit configuration:

- Endpoint URL.
- Optional auth header.
- Request timeout.
- Retry count and backoff for 429/5xx responses.
- Low `max_concurrency`, defaulting to `2`.

It has no default endpoint, no bundled keys, and no provider SDK.

```rust
use ocr_http_backends::{CloudHttpOcrBackend, CloudHttpOcrConfig};

# fn build() -> oxide_engine::Result<CloudHttpOcrBackend> {
let config = CloudHttpOcrConfig::new("http://127.0.0.1:9000/provider-ocr")
    .with_auth_header("Authorization", "Bearer ${TOKEN}")
    .with_timeout(std::time::Duration::from_secs(20))
    .with_retries(2, std::time::Duration::from_millis(250))
    .with_max_concurrency(2);
CloudHttpOcrBackend::new(config)
# }
```

Production providers normally require HTTPS and provider-specific payloads. Keep
the `OcrEngine` implementation and replace the example's tiny HTTP client with
your production TLS-capable client and exact request/response mapping.

Privacy note: cloud OCR sends page images outside your machine. Oxide guarantees
the plumbing, coordinate merge, and containment. Your provider contract governs
recognition accuracy, retention, cost, rate limits, and compliance.

## C ABI

The C ABI exposes a function-pointer backend:

```c
static int my_recognize(void* userdata,
                        const uint8_t* gray, uint32_t width, uint32_t height,
                        uint32_t dpi,
                        void* sink, OxideOcrEmitWordFn emit) {
    emit(sink, "Hello", 72.0, 60.0, 140.0, 88.0, 0.98, 0);
    return 0;
}

OxideOcrBackend backend = {
    .userdata = NULL,
    .recognize = my_recognize,
    .max_concurrency = 1,
    .name = "my-c-ocr",
};
oxide_document_set_ocr_backend(doc, backend, &err);
oxide_document_parse_markdown_ocr(doc, &markdown, &err);
```

The plain non-OCR C parse functions ignore the backend. Use the `_ocr` variants
for OCR-aware parse/JSON output.

## WebAssembly

There is no in-browser OCR backend. The browser build has no Tesseract process,
no Python runtime, no native subprocess timeout, and no default remote provider.

Today, render pages in the browser and run OCR out of band if needed. A future
JS callback backend can mirror the Python seam, but it is intentionally not part
of the current WASM API.

## Output, Provenance, And Limits

Recovered words flow into the same document model as digital-born text. That
means Markdown, JSON, HTML, chunking, and field extraction consume OCR text
through the normal pipeline.

Oxide records OCR source/provenance and propagates confidence. If the mean OCR
confidence for a page is below `ocr_low_confidence_warn`, Oxide adds a warning
block so consumers can treat the page as unreliable.

Honest limits:

- OCR quality belongs to the backend and the scan.
- Oxide guarantees bounded rendering, backend dispatch, coordinate merging,
  confidence propagation, and per-page failure containment.
- OCR remains additive. With `OcrPolicy::Off`, behavior is byte-identical to no
  backend.
