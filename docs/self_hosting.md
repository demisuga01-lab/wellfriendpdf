# Self-Hosting Wellfriend

Run the whole document-extraction stack on **your own machine or VPS** — CLI,
library, and HTTP API — with documents never leaving your hardware and no
per-page cloud fees. This guide covers the single-binary CLI, the self-hostable
server (with and without OCR), example workflows, and resource/privacy guidance.

> **TL;DR.** `cargo build --release` gives you a single `wellfriendpdf` binary (CLI) and
> an `wellfriendpdf-server` binary (HTTP API). The CLI needs no configuration. The
> server is **fail-closed**: it refuses to start until you set an API key (or
> explicitly opt into unauthenticated dev mode). OCR is an opt-in build feature
> that shells out to an external `tesseract` binary.

---

## 1. The single-binary CLI

### Build / install

```sh
# From a clone of the repo:
cargo build --release -p wellfriendpdf-cli      # produces target/release/wellfriendpdf
# Optionally with OCR (see §4):
cargo build --release -p wellfriendpdf-cli --features ocr
```

The result is one static-ish binary with no Python, no Poppler/Ghostscript, no
ML runtime. Drop it anywhere on your `PATH`.

Check what you built — `--version` reports the engine version and whether OCR
was compiled in:

```text
$ wellfriendpdf --version
wellfriendpdf 0.1.0
engine: 0.1.0
ocr: not compiled-in (rebuild with --features ocr to enable)
features: []
```

### Command groups

```sh
# PARSE / EXTRACT (the document parser)
wellfriendpdf parse          input.pdf --format markdown|json|html   # canonical model
wellfriendpdf chunk          input.pdf --target-tokens 512 --overlap 64   # RAG chunks (JSON)
wellfriendpdf extract-fields input.pdf --type auto|invoice|receipt|form|generic   # key-value (JSON)
wellfriendpdf extract-text   input.pdf [--structured|--semantic|--ocr]   # plain / layout / OCR text
wellfriendpdf extract-tables input.pdf --format csv|json|html

# STRUCTURAL (qpdf-class; read-and-rewrite)
wellfriendpdf merge a.pdf b.pdf -o merged.pdf
wellfriendpdf split input.pdf -o "page-%d.pdf"
wellfriendpdf extract-pages input.pdf 1,3,5-9 -o subset.pdf

# INSPECT
wellfriendpdf info   input.pdf [--json]      # pdfinfo-style + parser facts
wellfriendpdf fonts  input.pdf [--json]
wellfriendpdf detach input.pdf --list        # attachments

# RENDER (feeds OCR + previews)
wellfriendpdf render input.pdf --dpi 150 --format png
```

`--ocr`, `--ocr-lang`, and `--ocr-dpi` engage OCR on `parse`, `extract-fields`,
`chunk`, and `extract-text` when the binary was built `--features ocr` (see §4).
A binary built without OCR returns an actionable error if you pass `--ocr`.

> **Structural writes.** The CLI includes `encrypt`, `rotate`, `optimize`,
> `repair`, and qpdf-validated `linearize` output for the supported structural
> subset. Linearized object-stream packing, decrypt-as-write, and server
> mutation routes remain deliberate follow-ups (see `docs/manipulation.md`).
> `extract-tables` does not support `--ocr` (OCR'd table-grid reconstruction is
> a known gap; use `extract-fields --ocr` for scanned tabular data).

---

## 2. Running the server locally

The server (`wellfriendpdf-server`) is an HTTP API over the same engine: parse, chunk,
extract-fields, info, extract-text, analyze, render (pdf2img), and extract-images,
with auth, rate limiting, resource caps, and an async job queue for large inputs.

### Quick start (development)

The server is **fail-closed**: with no API keys configured it refuses to start,
to prevent a forgotten config from silently exposing every endpoint. For local
development you can either set a key or explicitly opt into unauthenticated mode:

```sh
# Option A — set an API key (recommended, mirrors production):
WELLFRIENDPDF_API_KEYS=dev-secret-key cargo run --release -p wellfriendpdf-server

# Option B — explicit dev escape hatch (NEVER in production):
WELLFRIENDPDF_ALLOW_UNAUTHENTICATED=true cargo run --release -p wellfriendpdf-server
```

Then call it (authenticated example):

```sh
# Parse a PDF to Markdown:
curl -sS -H "X-API-Key: dev-secret-key" \
  -F "file=@input.pdf" -F "format=markdown" \
  http://localhost:8080/api/v1/parse

# RAG chunks (JSON):
curl -sS -H "X-API-Key: dev-secret-key" \
  -F "file=@input.pdf" -F "target_tokens=512" \
  http://localhost:8080/api/v1/chunk

# Key-value fields (JSON):
curl -sS -H "X-API-Key: dev-secret-key" \
  -F "file=@invoice.pdf" -F "doc_type=invoice" \
  http://localhost:8080/api/v1/extract-fields

# Document metadata (JSON):
curl -sS -H "X-API-Key: dev-secret-key" -F "file=@input.pdf" \
  http://localhost:8080/api/v1/info
```

Health and readiness probes are auth-exempt:

```sh
curl -sS http://localhost:8080/health      # -> ok
curl -sS http://localhost:8080/readiness   # -> {"status":"ready",...}
```

### Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/api/v1/parse` | Canonical model → Markdown / JSON / HTML |
| POST | `/api/v1/chunk` | RAG-ready semantic chunks (JSON) |
| POST | `/api/v1/extract-fields` | Key-value fields (JSON) |
| POST | `/api/v1/info` | Document metadata (JSON) |
| POST | `/api/v1/extract-text` | Plain / structured text |
| POST | `/api/v1/analyze` | Text-layer / scanned detection |
| POST | `/api/v1/pdf2img` | Render pages to a ZIP of images |
| POST | `/api/v1/extract-images` | Extract embedded images (ZIP) |
| POST | `/api/v1/jobs/pdf2img`, `/api/v1/jobs/extract-images` | Async variants for large inputs |
| GET | `/api/v1/jobs/{id}`, `/api/v1/jobs/{id}/result` | Poll / download job result |
| GET | `/api/v1/version`, `/health`, `/readiness` | Versions / probes |

All `multipart/form-data`; the PDF is the `file` field. The parser endpoints
also accept `pages`, `password`, and op-specific fields (`format`, `doc_type`,
`target_tokens`, `overlap`, `keep_furniture`).

> **Large documents.** Parse/chunk/extract-fields/info run **synchronously**,
> bounded by `WELLFRIENDPDF_REQUEST_TIMEOUT_SECS`, `WELLFRIENDPDF_MAX_FILE_SIZE`, and
> `WELLFRIENDPDF_MAX_PAGES`. The async **job queue** currently wraps `pdf2img` and
> `extract-images` (the long, output-heavy render jobs); see `docs/jobs.md`.
> Asynchronous parse/chunk submission is a planned extension — for very large
> parse jobs today, raise `WELLFRIENDPDF_REQUEST_TIMEOUT_SECS` accordingly.

### Docker

A 2-stage `Dockerfile` and a `docker-compose.yml` build and run `wellfriendpdf-server`
(non-root user, `/health` healthcheck, `EXPOSE 8080`):

```sh
docker compose up --build
```

> **⚠️ The shipped `docker-compose.yml` will NOT start as-is.** It sets
> `WELLFRIENDPDF_API_KEYS=""` with no `WELLFRIENDPDF_ALLOW_UNAUTHENTICATED`, and the server is
> fail-closed — it refuses to boot without a key. **Before running**, edit the
> compose file (or your env) to set a real key:
>
> ```yaml
> environment:
>   WELLFRIENDPDF_API_KEYS: "your-strong-key-here"
>   WELLFRIENDPDF_CORS_ALLOWED_ORIGINS: "https://your-frontend.example.com"
> ```
>
> This non-booting default is intentional friction: it forces you to set a key
> rather than accidentally deploy an open API.

> **The default Docker image does NOT include OCR.** The `Dockerfile` builds
> `wellfriendpdf-server` without the `ocr` feature and does not install `tesseract`. To
> self-host OCR-enabled extraction, build with the feature **and** install
> Tesseract in the runtime image (see §4).

### Configuration reference

`.env.example` documents **every** `WELLFRIENDPDF_*` variable with secure defaults — it
is the canonical config reference. Highlights:

| Variable | Default | Purpose |
| --- | --- | --- |
| `WELLFRIENDPDF_API_KEYS` | *(empty → fail-closed)* | Comma-separated valid API keys |
| `WELLFRIENDPDF_ALLOW_UNAUTHENTICATED` | `false` | Dev-only: run with NO auth |
| `WELLFRIENDPDF_CORS_ALLOWED_ORIGINS` | *(empty → none)* | Browser cross-origin allowlist |
| `WELLFRIENDPDF_RATE_LIMIT_PER_MIN` | `60` | Per-key requests/min (0 = off) |
| `WELLFRIENDPDF_MAX_FILE_SIZE` | `52428800` (50 MiB) | Max upload size |
| `WELLFRIENDPDF_MAX_PAGES` | `200` | Max pages per request |
| `WELLFRIENDPDF_REQUEST_TIMEOUT_SECS` | `30` | Cooperative per-request deadline |
| `WELLFRIENDPDF_MAX_RENDER_PIXELS` | `100000000` | Pixel-explosion guard |
| `WELLFRIENDPDF_MAX_OUTPUT_BYTES` | `2147483648` (2 GiB) | Output-size cap |
| `WELLFRIENDPDF_JOB_*` | *(various)* | Async job queue sizing/retention |

Deploy checklist (also in `.env.example` and `docs/security.md`): set strong
`WELLFRIENDPDF_API_KEYS`; set `WELLFRIENDPDF_CORS_ALLOWED_ORIGINS` to your frontend; size the
timeouts/limits to your workload; **terminate TLS in front** (Wellfriend speaks plain
HTTP behind a reverse proxy / load balancer).

---

## 3. Browser-side extraction (WASM)

For client-side extraction with **no server at all** (documents never leave the
browser tab), build the WASM package and use `WellfriendPdf`:

```sh
cargo build -p wellfriendpdf-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web \
  --out-dir crates/wellfriendpdf-wasm/examples/browser/pkg \
  target/wasm32-unknown-unknown/release/wellfriendpdf_wasm.wasm
# serve crates/wellfriendpdf-wasm/examples/browser and open index.html
```

```js
const pdf = new WellfriendPdf(new Uint8Array(await file.arrayBuffer()));
const markdown = pdf.parseMarkdown();          // canonical model → Markdown
const chunkSet = JSON.parse(pdf.chunk(0, 0));   // RAG chunks (default 512/64)
const fields  = pdf.extractFieldsJson("auto");  // key-value fields
```

WASM is **digital-born only** — OCR needs the external Tesseract process and is
not available in the browser. See `crates/wellfriendpdf-wasm/examples/browser/README.md`.

---

## 4. OCR (optional, external Tesseract)

OCR is an **opt-in build feature** that drives the external `tesseract` binary
(no linked C). To enable it:

1. **Install Tesseract** and the language packs you need:
   - Debian/Ubuntu: `apt-get install tesseract-ocr tesseract-ocr-eng` (add
     `tesseract-ocr-deu`, etc. for other languages).
   - macOS: `brew install tesseract tesseract-lang`.
   - Windows: install the UB-Mannheim Tesseract build and ensure `tesseract` is
     on `PATH`.
2. **Build with the feature**:
   ```sh
   cargo build --release -p wellfriendpdf-cli --features ocr
   ```
3. **Use it** — scanned pages are recognized and flow through the same model as
   digital-born text:
   ```sh
   wellfriendpdf parse          scanned.pdf --ocr --ocr-lang eng --format markdown
   wellfriendpdf extract-fields scanned.pdf --ocr --type invoice
   wellfriendpdf extract-text   scanned.pdf --ocr
   ```

For an OCR-enabled **server/Docker** image, extend the runtime stage of the
`Dockerfile` to `apt-get install -y tesseract-ocr tesseract-ocr-eng` and build
`wellfriendpdf-server` with `--features ocr` (the server crate gains OCR the same way
the CLI does). Honest expectation: OCR quality is bounded by Tesseract and scan
quality; messy scans recover most text but key-value recall drops (see
`docs/parser_positioning.md`).

Tesseract is only the *default* backend, not the only option. OCR is a
**pluggable seam**: you can point Wellfriend at your own local vision model, an ONNX
runtime, or a hosted cloud vision API by implementing one small interface — in
Rust, Python, or C. See **[OCR Backends — Integrator Guide](ocr_backends.md)**
for the contract and worked examples on every surface. The copy-paste reference
templates live at `crates/wellfriendpdf-py/examples/local_ai_ocr_backend.py` and
`crates/engine/examples/ocr_http_backends.rs`.


---

## 5. Example workflows

### Batch a folder of PDFs → Markdown for a local RAG/LLM

```sh
for f in docs/*.pdf; do
  wellfriendpdf parse "$f" --format markdown -o "out/$(basename "$f" .pdf).md"
done
# Or chunk straight to JSON for an embedding pipeline:
for f in docs/*.pdf; do
  wellfriendpdf chunk "$f" --target-tokens 512 -o "out/$(basename "$f" .pdf).chunks.json"
done
```

### Self-hosted invoice → JSON pipeline

```sh
# Digital-born invoices:
wellfriendpdf extract-fields invoice.pdf --type invoice > invoice.json
# Scanned invoices (OCR build):
wellfriendpdf extract-fields scan.pdf --ocr --type invoice > scan.json
```

### Private document API on your VPS

```sh
WELLFRIENDPDF_API_KEYS=$(openssl rand -hex 32) \
WELLFRIENDPDF_CORS_ALLOWED_ORIGINS=https://app.example.com \
  ./wellfriendpdf-server     # behind nginx/Caddy terminating TLS
```

---

## 6. Resource guidance & privacy framing

- **Threads.** Text extraction and rendering parallelize across cores (rayon);
  the engine is shared via `Arc`, so per-page work scales without re-parsing.
- **Memory.** Render peak memory is roughly flat in page count (Arc-shared
  engine). The server's pixel-explosion guard, output-size cap, and image-count
  cap bound worst-case memory on untrusted input *before* allocation.
- **Timeouts.** The server uses cooperative cancellation: on the per-request /
  per-job deadline the engine's hot loops observe the cancel flag and bail,
  freeing the worker — it actually stops CPU-bound work rather than abandoning
  the wait. Tune `WELLFRIENDPDF_REQUEST_TIMEOUT_SECS` / `WELLFRIENDPDF_JOB_TIMEOUT_SECS`.
- **Privacy.** Everything runs on your hardware. Documents are never uploaded to
  a third party, there are no per-page cloud fees, and (for WASM) extraction can
  run entirely client-side. This is the thing teams pay Textract / Azure DI /
  Docling-cloud for — run it yourself, unmetered and private.

## See also

- `docs/parser_positioning.md` — honest wins/trails vs Docling/PyMuPDF/qpdf.
- `docs/parser_benchmark.md` — the reproducible extraction benchmark + numbers.
- `docs/security.md` — server security posture + deploy checklist.
- `docs/jobs.md` — async job API and its single-process/in-memory limitation.
- `docs/bindings.md` — C ABI and WASM embedding.
- `.env.example` — the complete `WELLFRIENDPDF_*` configuration reference.
