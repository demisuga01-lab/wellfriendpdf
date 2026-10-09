# Wellfriend PDF SDK

Build PDF products with one coherent engine instead of stitching together a
parser, renderer, editor, converter, and signing stack. Wellfriend opens the
original document structure, turns it into reliable pixels and data, edits the
source content, and writes the result through the same Rust core.

Use that core directly from Rust or ship it through the CLI, HTTP API, C,
Python, WebAssembly, .NET, and Java. One document model carries the work from
uploaded bytes to the PDF your customer downloads.

## Benchmark

The [complete benchmark report](docs/reports/final-benchmark-vps-20261008/README.md) publishes the reproducible result and its hashed per-file evidence.

Renderer revision `1fd74f722222b12f5278e1b8dd886943770d6c0d` and parser/document-facility revision `fbd455829e7c726bf2457116e1723da058b26af3` are measured on 150 real PDFs (2.036 GiB, 9,214 pages) on one Linux VPS. Every tool receives the same corpus and operation contract. A facility passes only when its output reopens and its operation-specific structural, semantic, security, or visual postconditions hold; unsupported operations remain unsupported.

### Parsing and document facilities

Resident parsing opens the in-memory document and resolves page count after warm-up; fresh parsing includes process startup. Facility timings include quality-qualified passes only. Conversion token F1 is agreement with Poppler extraction, not semantic ground truth.

| Benchmark | Wellfriend SDK | PDFium | Poppler | MuPDF | qpdf |
| --- | --- | --- | --- | --- | --- |
| Parse correctness | 150/150 | 150/150 | 150/150 | 150/150 | 150/150 |
| Resident parse P50 | 1.15 ms | 0.72 ms | 3.18 ms | 3.02 ms | 5.04 ms |
| Resident parse P95 | 3.49 ms | 2.15 ms | 6.20 ms | 113.76 ms | 30.49 ms |
| Resident parse P99 | 11.58 ms | 6.15 ms | 13.40 ms | 454.79 ms | 59.19 ms |
| Resident parse maximum | 50.90 ms | 41.65 ms | 22.93 ms | 820.54 ms | 61.64 ms |
| Fresh-process parse P50 | 18.62 ms | 18.86 ms | 55.28 ms | 30.37 ms | 41.00 ms |
| Fresh-process parse P95 | 149.46 ms | 164.20 ms | 188.29 ms | 316.52 ms | 173.01 ms |
| Fresh-process parse maximum | 759.66 ms | 937.51 ms | 891.96 ms | 1,582.42 ms | 674.40 ms |
| merge | 150/150 qualified<br>P50 213.76 ms; P95 3,654.58 ms; max 17,311.86 ms | N/A | 150/150 qualified<br>P50 733.77 ms; P95 12,083.91 ms; max 58,064.18 ms | 143/150 qualified; 7 failed<br>P50 142.84 ms; P95 1,460.92 ms; max 7,108.10 ms | 150/150 qualified<br>P50 107.55 ms; P95 1,062.49 ms; max 3,866.50 ms |
| split | 150/150 qualified<br>P50 84.32 ms; P95 716.31 ms; max 3,678.69 ms | N/A | 147/150 qualified; 3 failed<br>P50 339.00 ms; P95 2,173.56 ms; max 12,078.04 ms | 141/150 qualified; 9 failed<br>P50 53.06 ms; P95 108.38 ms; max 215.05 ms | 150/150 qualified<br>P50 52.21 ms; P95 276.64 ms; max 1,105.69 ms |
| extract-pages | 150/150 qualified<br>P50 77.37 ms; P95 804.59 ms; max 4,233.58 ms | N/A | 150/150 qualified<br>P50 630.29 ms; P95 3,325.53 ms; max 13,161.39 ms | 141/150 qualified; 9 failed<br>P50 29.73 ms; P95 55.13 ms; max 120.57 ms | 150/150 qualified<br>P50 49.39 ms; P95 254.88 ms; max 1,417.04 ms |
| lock | 150/150 qualified<br>P50 203.28 ms; P95 2,173.27 ms; max 9,356.47 ms | N/A | N/A | 150/150 qualified<br>P50 125.60 ms; P95 1,456.04 ms; max 6,570.45 ms | 150/150 qualified<br>P50 150.36 ms; P95 1,167.82 ms; max 5,531.15 ms |
| unlock | 150/150 qualified<br>P50 175.11 ms; P95 1,795.93 ms; max 7,890.87 ms | N/A | N/A | 150/150 qualified<br>P50 199.29 ms; P95 1,533.24 ms; max 7,571.06 ms | 150/150 qualified<br>P50 141.90 ms; P95 1,466.27 ms; max 5,935.34 ms |
| rotate | 150/150 qualified<br>P50 137.32 ms; P95 2,434.94 ms; max 24,874.89 ms | N/A | N/A | N/A | 150/150 qualified<br>P50 86.16 ms; P95 726.35 ms; max 3,062.44 ms |
| repair | 150/150 qualified<br>P50 116.56 ms; P95 757.80 ms; max 4,458.00 ms | N/A | N/A | 150/150 qualified<br>P50 73.13 ms; P95 1,177.30 ms; max 2,375.86 ms | 150/150 qualified<br>P50 435.01 ms; P95 10,296.60 ms; max 34,872.36 ms |
| organize | 150/150 qualified<br>P50 80.93 ms; P95 924.32 ms; max 4,103.92 ms | N/A | 150/150 qualified<br>P50 684.00 ms; P95 3,143.91 ms; max 15,258.38 ms | 141/150 qualified; 9 failed<br>P50 30.86 ms; P95 63.91 ms; max 123.01 ms | 150/150 qualified<br>P50 56.19 ms; P95 380.25 ms; max 1,631.32 ms |
| linearize | 150/150 qualified<br>P50 189.91 ms; P95 2,371.69 ms; max 9,991.37 ms | N/A | N/A | N/A | 150/150 qualified<br>P50 118.50 ms; P95 1,044.90 ms; max 5,157.87 ms |
| flatten | 150/150 qualified<br>P50 115.84 ms; P95 1,116.39 ms; max 5,896.10 ms | N/A | N/A | N/A | 150/150 qualified<br>P50 85.64 ms; P95 864.83 ms; max 2,948.51 ms |
| watermark | 150/150 qualified<br>P50 154.66 ms; P95 1,737.82 ms; max 8,878.64 ms | N/A | N/A | N/A | N/A |
| page-numbers | 150/150 qualified<br>P50 155.61 ms; P95 1,725.49 ms; max 8,423.41 ms | N/A | N/A | N/A | N/A |
| metadata | 150/150 qualified<br>P50 107.39 ms; P95 2,577.70 ms; max 10,633.06 ms | N/A | N/A | N/A | N/A |
| crop | 150/150 qualified<br>P50 124.03 ms; P95 1,523.96 ms; max 7,427.88 ms | N/A | N/A | N/A | N/A |
| resize | 150/150 qualified<br>P50 397.77 ms; P95 1,658.04 ms; max 4,049.19 ms | N/A | N/A | N/A | N/A |
| nup | 150/150 qualified<br>P50 694.93 ms; P95 2,338.78 ms; max 3,999.76 ms | N/A | N/A | N/A | N/A |
| optimize | 150/150 qualified<br>P50 167.39 ms; P95 2,163.10 ms; max 8,446.07 ms | N/A | N/A | 149/150 qualified; 1 failed<br>P50 103.00 ms; P95 5,885.34 ms; max 66,341.09 ms | 150/150 qualified<br>P50 1,497.49 ms; P95 15,557.53 ms; max 47,407.92 ms |
| canonicalize | 150/150 qualified<br>P50 214.85 ms; P95 3,661.61 ms; max 16,631.86 ms | N/A | N/A | 150/150 qualified<br>P50 67.92 ms; P95 572.99 ms; max 2,236.47 ms | 150/150 qualified<br>P50 363.59 ms; P95 3,735.10 ms; max 11,450.83 ms |
| sanitize | 150/150 qualified<br>P50 201.60 ms; P95 3,226.29 ms; max 14,906.27 ms | N/A | N/A | N/A | N/A |
| sign | 150/150 qualified<br>P50 219.17 ms; P95 3,542.30 ms; max 15,642.65 ms | N/A | 150/150 qualified<br>P50 141.61 ms; P95 1,098.55 ms; max 5,268.58 ms | N/A | N/A |
| text | 150/150 qualified<br>P50 262.26 ms; P95 2,298.74 ms; max 17,715.96 ms<br>token F1 P50 0.950251 | N/A | 150/150 qualified<br>P50 188.97 ms; P95 1,754.79 ms; max 29,320.31 ms<br>token F1 P50 1.000000 | 149/150 qualified; 1 failed<br>P50 166.31 ms; P95 1,167.06 ms; max 7,640.46 ms<br>token F1 P50 0.972429 | N/A |
| html | 150/150 qualified<br>P50 491.02 ms; P95 9,858.02 ms; max 89,235.22 ms<br>token F1 P50 0.918116 | N/A | 150/150 qualified<br>P50 198.76 ms; P95 1,898.32 ms; max 11,790.75 ms<br>token F1 P50 0.886217 | 150/150 qualified<br>P50 183.27 ms; P95 1,447.25 ms; max 11,156.80 ms<br>token F1 P50 0.960763 | N/A |
| markdown | 150/150 qualified<br>P50 521.60 ms; P95 9,689.52 ms; max 87,113.97 ms<br>token F1 P50 0.921787 | N/A | N/A | N/A | N/A |
| json | 150/150 qualified<br>P50 493.43 ms; P95 9,810.18 ms; max 91,872.99 ms<br>token F1 P50 0.418431 | N/A | N/A | N/A | N/A |
| docx | 150/150 qualified<br>P50 1,212.57 ms; P95 55,662.01 ms; max 625,898.94 ms<br>token F1 P50 0.924338 | N/A | N/A | N/A | N/A |
| pptx | 150/150 qualified<br>P50 1,233.87 ms; P95 53,456.90 ms; max 603,236.77 ms<br>token F1 P50 0.924338 | N/A | N/A | N/A | N/A |
| xlsx | 150/150 qualified<br>P50 487.86 ms; P95 9,571.48 ms; max 92,791.51 ms<br>token F1 P50 0.924132 | N/A | N/A | N/A | N/A |
| svg-first-page | 150/150 qualified<br>P50 2,393.40 ms; P95 13,130.54 ms; max 127,353.42 ms | N/A | N/A | 150/150 qualified<br>P50 73.14 ms; P95 1,112.60 ms; max 1,808.32 ms | N/A |
| postscript-first-page | 150/150 qualified<br>P50 2,228.03 ms; P95 12,650.90 ms; max 124,749.18 ms | N/A | 150/150 qualified<br>P50 109.77 ms; P95 1,321.53 ms; max 1,773.57 ms | N/A | N/A |
| eps-first-page | 150/150 qualified<br>P50 3,131.30 ms; P95 12,093.42 ms; max 122,045.52 ms | N/A | 150/150 qualified<br>P50 113.77 ms; P95 903.26 ms; max 2,244.92 ms | N/A | N/A |
| png-first-page | 150/150 qualified<br>P50 367.04 ms; P95 1,572.35 ms; max 4,219.85 ms | N/A | 150/150 qualified<br>P50 330.87 ms; P95 737.59 ms; max 2,238.46 ms | 150/150 qualified<br>P50 111.67 ms; P95 342.70 ms; max 478.89 ms | N/A |
| jpeg-first-page | 150/150 qualified<br>P50 327.95 ms; P95 1,412.03 ms; max 5,165.54 ms | N/A | 150/150 qualified<br>P50 143.50 ms; P95 327.36 ms; max 932.34 ms | N/A | N/A |
| tables-json | 150/150 qualified<br>P50 447.71 ms; P95 5,820.16 ms; max 126,528.43 ms | N/A | N/A | N/A | N/A |
| pdfa-2b-validation | 150/150 qualified<br>P50 179.04 ms; P95 1,699.35 ms; max 8,540.30 ms | N/A | N/A | N/A | N/A |
| pdfa-2b-conversion | 7/150 qualified; 143 refused<br>P50 149.66 ms; P95 1,461.22 ms; max 1,461.22 ms<br>veraPDF verified 7/7 qualified outputs; 8 refused artifacts rejected | N/A | N/A | N/A | N/A |

Wellfriend and veraPDF both execute PDF/A-2b validation reports across all 150 inputs; veraPDF classifies the source corpus as 0/150 compliant. Wellfriend converts 7/150 inputs to independently verified PDF/A-2b and refuses 143 files whose source fonts cannot be legally reconstructed as embedded fonts. Ghostscript emits 150 structurally readable files under the matched command, but veraPDF accepts 0/150 as PDF/A-2b.

### Visual rendering

Each renderer opens a document once and emits every page as 72-DPI RGB8. Timing includes only documents whose emitted page count matches the qpdf inventory. Thumbnail fidelity is measured on every matched page; first, middle, and last pages also receive full-resolution comparison. qpdf is a structural transformer, not a raster renderer.

| Benchmark | Wellfriend SDK | PDFium | Poppler | MuPDF | qpdf |
| --- | --- | --- | --- | --- | --- |
| Complete documents | 150/150 | 150/150 | 150/150 | 150/150 | N/A |
| Pages emitted / expected | 9,214/9,214 | 9,214/9,214 | 9,214/9,214 | 9,214/9,214 | N/A |
| All-page document P50 | 5,323.72 ms | 1,078.92 ms | 1,809.67 ms | 889.34 ms | N/A |
| All-page document P90 | 27,374.63 ms | 7,158.15 ms | 8,928.46 ms | 4,010.87 ms | N/A |
| All-page document P95 | 84,881.12 ms | 21,137.28 ms | 28,582.82 ms | 11,804.36 ms | N/A |
| All-page document P99 | 342,587.75 ms | 166,584.06 ms | 139,079.79 ms | 80,484.38 ms | N/A |
| All-page document maximum | 373,962.68 ms | 185,995.95 ms | 152,191.38 ms | 88,210.12 ms | N/A |
| Per-page P50 | 231.23 ms | 42.81 ms | 86.48 ms | 47.89 ms | N/A |
| Per-page P90 | 742.36 ms | 179.58 ms | 170.82 ms | 114.47 ms | N/A |
| Per-page P95 | 832.56 ms | 209.28 ms | 220.28 ms | 142.99 ms | N/A |
| Per-page P99 | 1,057.56 ms | 340.79 ms | 485.95 ms | 182.70 ms | N/A |
| Per-page maximum | 1,478.15 ms | 365.36 ms | 818.00 ms | 264.65 ms | N/A |
| Thumbnail SSIM vs Wellfriend (P50 / P05) | 1.000000 / 1.000000 | 0.999663 / 0.979476 | 0.985956 / 0.910557 | 0.998321 / 0.983639 | N/A |
| Thumbnail PSNR vs Wellfriend (P50) | Infinity | 54.440 | 37.169 | 47.721 | N/A |
| Full-resolution SSIM vs Wellfriend (P50) | 1.000000 | 0.939976 | 0.793696 | 0.964249 | N/A |

The comparison sheets below are inspected at source resolution before publication. They show the lowest-agreement page, a lower-tail page, the median page, and an upper-tail page selected from the measured distribution.

![Lowest-agreement visual sample](docs/reports/final-benchmark-vps-20261008/render/visual/01-worst.webp)

![Lower-tail visual sample](docs/reports/final-benchmark-vps-20261008/render/visual/02-lower-tail.webp)

![Median visual sample](docs/reports/final-benchmark-vps-20261008/render/visual/03-median.webp)

![Upper-tail visual sample](docs/reports/final-benchmark-vps-20261008/render/visual/04-upper-tail.webp)

Exact distributions and tool-level outcomes are recorded in [summary.json](docs/reports/final-benchmark-vps-20261008/summary.json). The compressed per-file JSONL records are the authoritative evidence; [SHA256SUMS](docs/reports/final-benchmark-vps-20261008/evidence/SHA256SUMS) binds every published artifact.

Generated: `2026-10-09T01:31:45.582543+00:00`.

## Capabilities

| Build with Wellfriend | What the engine handles |
|---|---|
| Understand documents | Page trees, content streams, text, tables, forms, annotations, attachments, metadata, fonts, images, signatures, and logical structure |
| Render pages | PNG, JPEG, WebP, raw pixels, SVG, PostScript, and EPS with transparency, masks, color management, clipping, patterns, shadings, and Type 3 fonts |
| Edit real content | Source-linked text and object editing, paragraph and linked-story reflow, image and vector changes, scanned-word reconstruction, forms, annotations, watermarking, and redaction |
| Rebuild documents | Merge, split, extract, rotate, crop, resize, organize, flatten, linearize, optimize, sanitize, encrypt, decrypt, and sign |
| Deliver useful formats | Text, Markdown, JSON, HTML, DOCX, PPTX, XLSX, images, and PDF/A workflows |

## Requirements

| Component | Requirement |
|---|---|
| Rust workspace | Rust 1.95 or newer and Cargo |
| Python binding | Python 3.9+ and `maturin` |
| WASM binding | `wasm-pack` and a wasm32 Rust target |
| .NET binding | .NET 8 SDK |
| Java binding | JDK 25 with preview FFM enabled |
| Native bindings | A locally built `wellfriendpdf_capi` shared library |
| Optional OCR | Tesseract development and runtime libraries |
| Optional native CMM | Build with `native-cmm-lcms2` |

```powershell
git clone https://github.com/demisuga01-lab/wellfriendpdf.git
cd wellfriendpdf
cargo build --workspace --jobs 2
```

The engine default features are `parse`, `render`, and `structural`. The
`full` feature adds extraction, creation, editing, signing, standards, and
OCR-facing APIs.

## Packages

| Component | Path | Primary use |
|---|---|---|
| Rust engine | `crates/engine` | Core document APIs |
| CLI | `crates/cli` | Shell workflows and JSON reports |
| HTTP server | `crates/server` | Authenticated multipart APIs and progressive sessions |
| SIMD kernels | `crates/render-simd` | CPU and wasm32 raster kernels |
| C ABI | `crates/wellfriendpdf-capi` | Native integration boundary |
| Python | `crates/wellfriendpdf-py` | PyO3/maturin package |
| WASM | `crates/wellfriendpdf-wasm` | Browser, WebWorker, and Node APIs |
| .NET | `bindings/dotnet/WellfriendPdf` | .NET 8 wrapper |
| Java | `bindings/java` | JDK 25 FFM wrapper |
| OCR | `crates/wellfriendpdf-ocr-tesseract` | Optional Tesseract provider |
| Visual comparison | `tools/renderer-visual-diff` | Raster normalization and image metrics |

## Rust Engine

`ContentEngine` is the main document entry point. Page numbers are 1-based.

```rust
use wellfriendpdf_engine::ContentEngine;

fn main() -> wellfriendpdf_engine::Result<()> {
    let engine = ContentEngine::open_path("input.pdf")?;
    println!("pages: {}", engine.page_count()?);
    println!("{}", engine.get_page_text(1)?);

    let png = engine.render_page_png_fast(1, 150)?;
    std::fs::write("page-1.png", png)?;
    Ok(())
}
```

Add the local crate from another workspace:

```toml
[dependencies]
wellfriendpdf-engine = { path = "../wellfriendpdf/crates/engine" }
```

### Render contracts and caller-owned surfaces

A `RenderContract` carries every output-affecting field into validation and
cache identity: page/revision, DPI, box, transform, clip, surface layout,
background, backend, compositing, optional content, smoothing, print/color
policy, exactness, determinism, and resource budgets.

```rust
use wellfriendpdf_engine::{CancelToken, ContentEngine};
use wellfriendpdf_engine::render::RenderMode;

fn render() -> wellfriendpdf_engine::Result<()> {
    let engine = ContentEngine::open_path("input.pdf")?;
    let contract = engine.default_render_contract(1, 150, RenderMode::Compat)?;

    let png = engine.render_page_png_with_contract(
        &contract,
        &CancelToken::none(),
    )?;
    std::fs::write("contract-page.png", png)?;

    let mut surface = vec![0_u8; contract.stride * contract.height as usize];
    engine.render_page_into_buffer(
        &contract,
        &CancelToken::none(),
        &mut surface,
    )?;
    Ok(())
}
```

Use `RenderDocumentCache` overloads when repeated renders belong to the same
document/revision and contract policy. Apply the render-invalidation plan
returned by supported editing transactions before publishing cached output.

### Progressive rendering

Progressive jobs render a deterministic tile queue and reject stale
publications with revision, contract, visibility, scheduler, and tile identity.

```rust
use wellfriendpdf_engine::{CancelToken, ContentEngine};
use wellfriendpdf_engine::render::RenderMode;

fn progressive() -> wellfriendpdf_engine::Result<()> {
    let engine = ContentEngine::open_path("input.pdf")?;
    let contract = engine.default_render_contract(1, 150, RenderMode::Compat)?;
    let mut job = engine.progressive_render_job_with_contract(contract, 256, 256)?;

    while !job.is_complete() {
        let report = job.render_next(4, &CancelToken::none())?;
        for publication in report.completed_tile_publications {
            println!("publish {}", publication.publication_identity);
        }
    }

    let pixels = job.finish_checked()?;
    println!("{}x{}", pixels.width, pixels.height);
    Ok(())
}
```

Use `revise_viewport_hint`, `revise_dirty_region`, or
`revise_render_contract` when viewer state changes. Full contract revision
rebuilds the output region and tile grid, invalidates prior publications, and
returns an obsolete-publication report. Use `request_cancel`, `pause`, `resume`,
and `close` for lifecycle control.

### Editing and invalidation

The shared SDK facade exposes JSON transaction methods when a language binding
needs a versioned transport:

```rust
let (edited_pdf, result_json) = wellfriendpdf_engine::sdk::
    editing_transactions_transaction_apply_with_render_invalidation_json(
        &pdf_bytes,
        request_json,
        Some(render_invalidation_options_json),
        None,
    )?;
# let _ = (edited_pdf, result_json);
```

The result includes edited bytes/report data plus source IDs, affected pages,
dirty regions/tiles, and cache-pruning instructions when exact dependency
coverage is available. Unknown dependencies deliberately broaden invalidation.

## Command-Line Interface

Build once, then invoke `target/release/wellfriendpdf` (add `.exe` on Windows):

```powershell
cargo build -p wellfriendpdf-cli --release --jobs 2
target\release\wellfriendpdf.exe --mode standard capabilities
```

Common workflows:

```powershell
# Document facts and text
target\release\wellfriendpdf.exe --mode standard info input.pdf --json
target\release\wellfriendpdf.exe --mode standard extract-text input.pdf

# Raster output and a reusable schema-v1 contract
target\release\wellfriendpdf.exe --mode standard render input.pdf --pages 1 --dpi 150 --format png --output pages.zip
target\release\wellfriendpdf.exe --mode standard render input.pdf --pages 1 --dpi 150 --write-contract-json --output pages.zip
target\release\wellfriendpdf.exe --mode standard render input.pdf --contract-json contract.json --format raw --output page.raw

# Renderer architecture/capability reports
target\release\wellfriendpdf.exe --mode standard feature-report
target\release\wellfriendpdf.exe --mode standard document-views-report input.pdf
target\release\wellfriendpdf.exe --mode standard backend-plan-arena-report input.pdf --page 1 --dpi 72
target\release\wellfriendpdf.exe --mode standard image-decode-capability-report input.pdf

# Source-linked editing and a machine-readable report
target\release\wellfriendpdf.exe --mode standard edit-text-operator input.pdf --source-text "Original" --replacement-text "Updated" --output edited.pdf --report edit-report.json

# Security and standards reporting
target\release\wellfriendpdf.exe --mode standard security-report input.pdf --json
target\release\wellfriendpdf.exe --mode standard validate input.pdf --profile all --json
```

Use `wellfriendpdf <command> --help` to inspect that operation's inputs,
outputs, and machine-readable result schema.

## HTTP Server

The server listens on port `8080` by default. Production instances use API
keys; loopback development can enable the explicit unauthenticated mode.

```powershell
$env:WELLFRIENDPDF_ALLOW_UNAUTHENTICATED = "true"
$env:WELLFRIENDPDF_PORT = "8080"
cargo run -p wellfriendpdf-server --jobs 2
```

For a protected instance, unset `WELLFRIENDPDF_ALLOW_UNAUTHENTICATED`, set
`WELLFRIENDPDF_API_KEYS` to a comma-separated key list, and send either
`X-API-Key: <key>` or `Authorization: Bearer <key>`.

Build and consume a render contract with PowerShell 7:

```powershell
$form = @{ file = Get-Item .\input.pdf; page = "1"; dpi = "150" }
$built = Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/api/v1/render-contract -Form $form

$renderForm = @{
  file = Get-Item .\input.pdf
  contract_json = $built.contract_json
}
Invoke-WebRequest -Method Post -Uri http://127.0.0.1:8080/api/v1/render-contract/png -Form $renderForm -OutFile page.png
```

Primary endpoint groups:

| Group | Endpoints |
|---|---|
| Health/runtime | `GET /health`, `/readiness`, `/api/v1/version`, `/api/v1/capabilities`, `/api/v1/runtime-config`, `/api/v1/providers` |
| Parse/extract | `POST /api/v1/info`, `/parse`, `/extract-text`, `/extract-images`, `/extract-fields`, `/analyze` |
| Render contract | `POST /api/v1/render-contract`, `/png`, `/raw`, report variants, `/backend-plan-arena-report` |
| Progressive page render | `POST /api/v1/progressive/start`, `/:id/step`, `/pause`, `/resume`, `/viewport`, `/dirty-region`, `/render-context`, `/cancel`, `/finish`, `/close` |
| Viewer scheduling | `POST /api/v1/progressive/:id/queue/execute`, `/adjacent-prefetch/execute`, `/callbacks`, `/evaluate-publication` |
| Decode/prepress | `POST /api/v1/image-decode/capability-report`, `/progressive-image-decode/lifecycle-report`, `/prepress/plate-report` |
| Editing/cache | `POST /api/v1/editing-transactions/apply-with-render-invalidation`, `/progressive/:id/apply-render-invalidation` |

Multipart document routes accept a `file` part. Contract render routes accept a
canonical `contract_json` part. Progressive `/render-context` accepts either a
full `render_contract_json`/`contract_json` revision or the legacy fingerprint
fields, but not both in one request.

## C ABI

Build the native library and include the checked-in header:

```powershell
cargo build -p wellfriendpdf-capi --release --jobs 2
```

Artifacts are `wellfriendpdf_capi.dll`, `libwellfriendpdf_capi.so`, or
`libwellfriendpdf_capi.dylib`; the public header is
[`crates/wellfriendpdf-capi/include/wellfriendpdf.h`](crates/wellfriendpdf-capi/include/wellfriendpdf.h).

The normal call flow is:

1. Open caller-owned bytes with `wellfriendpdf_document_open_from_bytes`.
2. Query or transform through `wellfriendpdf_document_*` functions.
3. Build/round-trip schema-v1 contracts with the `wellfriendpdf_render_contract_*` functions.
4. Start progressive jobs with `wellfriendpdf_document_progressive_render_new_with_contract_json` and revise them with `wellfriendpdf_progressive_render_revise_render_contract_json`.
5. Free returned `char *` values with `wellfriendpdf_string_free`, returned `WellfriendBuffer` values with `wellfriendpdf_buffer_free`, and every opaque handle with its matching free function.

Every fallible function takes or returns an error channel. Never free Rust-owned
memory with the C allocator.

## Python

Build a local ABI3 wheel with maturin:

```powershell
python -m pip install maturin
python -m maturin build --release --manifest-path crates\wellfriendpdf-py\Cargo.toml --out target\wheels
$wheel = Get-ChildItem target\wheels\wellfriendpdf-0.1.0-*.whl |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
python -m pip install $wheel.FullName
```

```python
import wellfriendpdf

doc = wellfriendpdf.open("input.pdf")
print(doc.page_count)
print(doc.page(1).text)
print(doc.security_report())

png = doc.page(1).render(dpi=150)
with open("page.png", "wb") as output:
    output.write(png)

contract_json = doc.default_render_contract_json(1, dpi=150)
cache = doc.render_cache()
png, cache_report = doc.render_contract_png_with_render_cache_report(
    contract_json,
    cache,
)
```

The binding also exposes cancellation, caller-owned `bytearray` rendering,
font-substitution/telemetry reports, progressive page jobs, progressive image
decode lifecycle reports, transaction invalidation, semantic/chunk/search
reports, sanitization, redaction, and Office export helpers. See
[`crates/wellfriendpdf-py/README.md`](crates/wellfriendpdf-py/README.md) for the
full binding surface.

## WebAssembly

```powershell
wasm-pack build crates\wellfriendpdf-wasm --target web --out-dir pkg
wasm-pack build crates\wellfriendpdf-wasm --target nodejs --out-dir pkg-node
```

```typescript
import init, { WellfriendPdf } from "./pkg/wellfriendpdf_wasm.js";

await init();
const pdf = new WellfriendPdf(new Uint8Array(await file.arrayBuffer()));
console.log(JSON.parse(pdf.securityReportJson()));
const png = pdf.renderPagePng(1, 150);
pdf.close();
```

WASM accepts bytes and returns bytes or JSON. `ProgressiveRenderJob` supports
cancellation checks, queue execution, callback reports, adjacent-page prefetch,
and `reviseRenderContractJson`. The TypeScript declaration source is
[`crates/wellfriendpdf-wasm/wellfriendpdf.d.ts`](crates/wellfriendpdf-wasm/wellfriendpdf.d.ts).

## .NET

Build the C ABI first and point the managed resolver at it:

```powershell
cargo build -p wellfriendpdf-capi --release --jobs 2
$env:WELLFRIENDPDF_NATIVE_LIBRARY = (Resolve-Path target\release\wellfriendpdf_capi.dll)
dotnet build bindings\dotnet\WellfriendPdf\WellfriendPdf.csproj
```

```csharp
using WellfriendPdf;

using var doc = WellfriendDocument.Open("input.pdf", password: null);
var contract = doc.DefaultRenderContract(pageNumber: 1, dpi: 150)
    .WithResourceBudget(maxPixels: 20_000_000);

File.WriteAllBytes("page.png", doc.RenderPagePng(contract));
var surface = new byte[checked((int)contract.SurfaceByteLength)];
doc.RenderPageIntoBuffer(contract, surface);

using var session = doc.ProgressiveRenderSession(contract, tileWidth: 256, tileHeight: 256);
session.ReviseRenderContract(contract.WithBackground(248, 250, 252));
```

Use `using`/`Dispose()` for document, cache, cancellation, and progressive
handles. `RenderContract` is an immutable-style managed value object. See
[`bindings/dotnet/WellfriendPdf/README.md`](bindings/dotnet/WellfriendPdf/README.md).

## Java

The binding uses the JDK 25 Foreign Function and Memory API.

```powershell
cargo build -p wellfriendpdf-capi --release --jobs 2
$env:WELLFRIENDPDF_NATIVE_LIBRARY = (Resolve-Path target\release\wellfriendpdf_capi.dll)
cd bindings\java
mvn test package
```

```java
try (var doc = WellfriendPdf.Document.open(Path.of("input.pdf"), null)) {
    var contract = doc.defaultRenderContract(1, 150)
        .withResourceBudget(20_000_000L, null, null, null);
    Files.write(Path.of("page.png"), doc.renderPagePng(contract));

    ByteBuffer surface = ByteBuffer.allocateDirect(
        Math.toIntExact(contract.surfaceByteLength()));
    doc.renderPageIntoBuffer(contract, surface);

    try (var session = doc.progressiveRenderSession(contract, 256, 256)) {
        session.reviseRenderContract(contract.withBackground(248, 250, 252));
    }
}
```

Run Java with `--enable-preview --enable-native-access=ALL-UNNAMED`. Maven and
Gradle project files are both checked in; see
[`bindings/java/README.md`](bindings/java/README.md) for native loading and smoke
commands.

## OCR, SIMD, and Color Management

OCR is optional. Enable the native adapter explicitly:

```powershell
cargo build -p wellfriendpdf-cli --features ocr --release --jobs 2
cargo build -p wellfriendpdf-server --features ocr --release --jobs 2
```

`wellfriendpdf-render-simd` is an internal implementation crate. Call renderer
APIs normally; runtime dispatch selects guarded scalar, portable-wide, native
CPU, or wasm32 `simd128` rows and preserves scalar-equivalence fallbacks.

Portable color management is the default. Enable native LittleCMS support only
where that dependency is available:

```powershell
cargo build -p wellfriendpdf-cli --features native-cmm-lcms2 --release --jobs 2
```

## Verification Tools

`tools/renderer-visual-diff` normalizes pixel format, alpha, background, crop,
and dimensions and calculates image metrics. `tools/pdfium-harness` provides a
direct PDFium comparator. Reports pin the source commit, dataset manifest,
binary hashes, comparator versions, command lines, outcomes, and raw evidence.

## Error and Safety Model

- Page numbers are 1-based across public APIs.
- Passwords are operation-scoped inputs and are not retained by managed wrappers.
- Errors include typed machine-readable context and complete paint accounting.
- Exact render mode validates source operations and resources before pixel publication.
- Caller-owned surfaces validate dimensions, stride, pixel format, alpha mode, and byte length.
- Progressive consumers must evaluate publication identity after viewport, revision, visibility, or contract changes.
- Editing callers must apply returned invalidation plans before reusing renderer caches.
- Server deployments should configure API keys, request limits, timeouts, and CORS explicitly.

## Development Checks

Run lightweight checks serially when resources are constrained:

```powershell
cargo fmt --all --check
cargo check --workspace --all-targets --jobs 1
cargo clippy --workspace --all-targets --jobs 1 -- -D warnings
cargo check --workspace --all-features --all-targets --jobs 1
cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings
cargo test --workspace --all-targets --all-features --no-fail-fast --jobs 1 -- --test-threads=1
```

Focused tests live beside the corresponding Rust modules and under each
binding/server test directory. Large PDF corpora, visual adjudication,
performance measurements, and competitor comparisons are separate validation
campaigns rather than ordinary source checks.

## Documentation

- [`docs/api_overview.md`](docs/api_overview.md): Rust API orientation.
- [`docs/stability.md`](docs/stability.md): pre-1.0 API stability policy.
- [`docs/renderer/final-universal-renderer-implementation-report.md`](docs/renderer/final-universal-renderer-implementation-report.md): renderer internals.
- [`docs/renderer/complete-algorithm-and-method-inventory.md`](docs/renderer/complete-algorithm-and-method-inventory.md): algorithm and entry-point inventory.

## License

Wellfriend PDF SDK is available under the MIT License. See [`LICENSE`](LICENSE).
