# Wellfriend PDF SDK

Wellfriend PDF SDK is an MIT-licensed PDF engine for parsing, raster and vector
rendering, extraction, source-linked editing, document reflow, forms,
annotations, redaction, standards analysis, and multi-language embedding. The
canonical implementation is Rust; the CLI, HTTP server, C ABI, Python, WASM,
.NET, and Java packages use that same engine.

> The repository is currently source-first and pre-1.0 (`0.1.0`). Packages are
> built from this checkout; no package publication is implied by this README.

## Benchmark Results

### Current RAPTOR qualification — 2026-09-30

This stage-separated 100-PDF VPS campaign uses nearest-rank percentiles after
collapsing repeated fresh-process measurements to one median per document. The
[full report](docs/reports/raptor-vps-20260930/README.md) contains raw JSONL,
source/corpus hashes, timestamps, 5,412-test workspace logs, 198 page-level
four-renderer sheets, contact sheets, and amplified differences.

#### Parsing

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Accepted | 100/100 | 100/100 | 95/100 | Not measured | 100/100 |
| Workload | xref open + indexed page count | structural check | document inventory | Not measured | metadata + page count |
| P50 | 1.959 ms | 159.358 ms | 21.008 ms | — | 38.457 ms |
| P90 | 3.474 ms | 1,121.336 ms | 39.362 ms | — | 46.655 ms |
| P95 | 3.826 ms | 1,801.061 ms | 44.569 ms | — | 51.971 ms |
| P99 | 20.176 ms | 7,271.322 ms | 265.436 ms | — | 60.162 ms |
| Maximum | 44.338 ms | 9,283.166 ms | 265.436 ms | — | 63.557 ms |

The commands perform different work and are not semantic-equivalence claims.
Wellfriend's indexed structural path passes the requested 15/30/50/75/200 ms
upper bounds. Requested-page, full-tree, page-program, semantic, cold-raster,
and retained-raster stages remain separate in the full report.

#### Editing

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Applicable and verified after reopen | 98/98 | — | — | — | — |
| Typed non-applicable | 2/100 | — | — | — | — |
| Plan P50 / P95 / max | 1,208.958 / 2,465.182 / 4,456.134 ms | — | — | — | — |
| Apply P50 / P95 / max | 1,306.047 / 3,161.706 / 5,182.854 ms | — | — | — | — |
| Verified E2E P50 / P95 / max | 2,624.005 / 6,196.141 / 9,193.955 ms | — | — | — | — |

Every applicable output changed bytes, reopened, removed the selected reachable
source occurrence, and exposed the replacement. The requested edit latency
gates still fail.

#### Visual rendering — original inputs

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Rendered outputs | 100/100 | — | 100/100 | 100/100 | 100/100 |
| P50 | 406.167 ms | — | 946.182 ms | 694.410 ms | 841.898 ms |
| P90 | 632.020 ms | — | 1,047.133 ms | 774.717 ms | 1,073.674 ms |
| P95 | 732.437 ms | — | 1,070.129 ms | 803.991 ms | 1,104.522 ms |
| P99 / maximum | 1,264.523 / 1,310.240 ms | — | 1,208.693 / 1,671.627 ms | 1,014.655 / 1,102.913 ms | 1,292.722 / 1,716.905 ms |

#### Visual rendering — verified edited outputs

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Rendered outputs | 98/98 | — | 98/98 | 98/98 | 98/98 |
| P50 | 407.149 ms | — | 906.762 ms | 682.421 ms | 848.388 ms |
| P90 | 663.924 ms | — | 1,030.038 ms | 772.669 ms | 1,081.573 ms |
| P95 | 875.002 ms | — | 1,113.652 ms | 821.685 ms | 1,135.360 ms |
| P99 / maximum | 1,317.898 / 1,317.898 ms | — | 1,444.954 / 1,444.954 ms | 1,086.931 / 1,086.931 ms | 1,710.013 / 1,710.013 ms |

Wellfriend leads at P50 and P90, but the required 10% lead fails at the original
P95/P99/max and edited P95/P99/max gates. Pixel differences are diagnostics,
not objective error rates. This corpus does not establish universal editing or
superiority over Adobe.

![Edited four-renderer comparison](docs/reports/raptor-vps-20260930/visual-edited/pages/094-i1040gi-pdf.webp)

<details>
<summary>Previous RAPTOR qualification — 2026-09-29</summary>

The current result is a stage-separated 100-PDF VPS campaign. Tools are
columns; measurements are rows. Percentiles use nearest rank. Full per-file
evidence, commands, hashes, timestamps, readable comparison sheets, and the
methodology are in the
[RAPTOR qualification report](docs/reports/raptor-vps-20260929/README.md).

#### Parsing

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Accepted | 100/100 | 100/100 | 95/100 | Not run | 100/100 |
| P50 | 20.020 ms | 175.558 ms | 22.057 ms | — | 40.703 ms |
| P90 | 93.424 ms | 1,193.706 ms | 39.310 ms | — | 49.993 ms |
| P95 | 158.914 ms | 1,976.645 ms | 49.202 ms | — | 52.476 ms |
| P99 | 254.819 ms | 7,436.913 ms | 260.429 ms | — | 57.263 ms |
| Maximum | 315.642 ms | 11,081.225 ms | 260.429 ms | — | 59.346 ms |

These parsing workloads are deliberately labeled and are not semantically
equivalent: Wellfriend performs structural open/page-tree work, qpdf checks the
file, MuPDF inventories resources, and Poppler reads metadata. Wellfriend missed
the requested 15/30/50/75/200 ms structural-parse SLO at every gate.

#### Editing

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Attempted | 100 | — | — | — | — |
| Applicable and verified after reopen | 98/98 | — | — | — | — |
| Typed non-applicable | 2/100 | — | — | — | — |
| Apply P50 / P90 | 2,115.375 / 4,387.261 ms | — | — | — | — |
| Apply P95 / P99 / max | 5,208.620 / 8,207.122 / 8,207.122 ms | — | — | — | — |
| Verified E2E P50 / P90 | 3,586.492 / 6,796.927 ms | — | — | — | — |
| Verified E2E P95 / P99 / max | 7,756.644 / 12,481.508 / 12,481.508 ms | — | — | — | — |

Every applicable result changed bytes, reopened, removed the selected source
occurrence from the reachable revision, and exposed the replacement. The two
non-applicable files were retained in the denominator. All requested edit
latency gates failed.

#### Visual rendering time

The table below is the corrected 98-file edited-output campaign at 144 DPI,
using one fresh process per tool and page.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Matching-dimension output | 98/98 | — | 98/98 | 98/98 | 98/98 |
| P50 | 432.789 ms | — | 944.190 ms | 705.232 ms | 871.752 ms |
| P90 | 716.201 ms | — | 1,156.429 ms | 851.732 ms | 1,139.882 ms |
| P95 | 1,005.492 ms | — | 1,185.603 ms | 867.605 ms | 1,209.075 ms |
| P99 / maximum | 43,689.996 ms | — | 1,526.179 ms | 1,112.247 ms | 1,759.373 ms |

Wellfriend was at least 10% faster than the fastest reference at P50 and P90,
but failed at P95, P99, and maximum because `i1040gi.pdf` took 43.690 seconds.
The overall 10%-faster claim is therefore **failed**, not passed.

#### Visual rendering quality diagnostics

| Edited-output `changed > 8` | Wellfriend vs MuPDF | Wellfriend vs PDFium | Wellfriend vs Poppler |
|---|---:|---:|---:|
| P50 | 8.371346% | 9.955664% | 9.158321% |
| P90 | 11.375035% | 14.046511% | 12.903184% |
| P95 | 11.794105% | 14.402553% | 13.451663% |
| P99 / maximum | 15.750179% | 18.501807% | 17.794829% |

`Changed > 8` is a raster-policy diagnostic, not a percentage of objectively
wrong pixels. The formerly misplaced replacement now stays at its source
position, and the inspected comparison pages are recognizable in all four
renderers. This bounded evidence does not establish pixel identity, universal
editing, or superiority over Adobe.

![Corrected four-renderer edited output](docs/reports/raptor-vps-20260929/visual/edited/075-arxiv-stat-ap-2609-28419v1-fa39e6badcda-pdf.webp)

</details>

<details>
<summary>Historical 2026-09-28 campaign and pre-RAPTOR evidence</summary>

The 2026-09-28 qualification used implementation commit [`e2a83fc`](https://github.com/demisuga01-lab/wellfriendpdf/commit/e2a83fc),
release CLI SHA-256
`879e2bc695790fc7523cf6a835b7149b2d6b7ee7a6dd542466930ecb18f83943`,
and a fixed 100-PDF corpus on the project VPS. The corpus contains 81 arXiv
papers, 16 IRS forms, one DARPA SafeDocs sample, one Mozilla PDF.js regression
file, and one veraPDF conformance file.

### Editing and parsing

Tools are columns; benchmark workloads are rows. A dash means that the tool was
not used for that workload.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Semantic parse, originals | 100/100 accepted | 100/100 accepted: 97 clean, 3 warning | 95/100 accepted | — | — |
| Semantic parse, edited outputs | 100/100 accepted | 100/100 accepted: 99 clean, 1 warning | 95/100 accepted | — | — |
| Revision-bound source edit | 100/100 applied and verified | Structural verification | — | — | Independent text verification |

Every counted edit had to produce changed bytes, report an applied mutation,
prove the selected source occurrence was replaced, reopen through Wellfriend PDF,
expose the replacement through independent Poppler extraction, and introduce no
new qpdf structural diagnostic. There were no timeouts, typed refusals, or
verification failures in this particular workflow.

MuPDF rejected `f1040.pdf`, `f1040sa.pdf`, `f1040sc.pdf`, `f1040sd.pdf`, and
`f1040se.pdf` with `syntax error after element name`. It rejected the same five
files before and after editing; Wellfriend PDF and qpdf accepted all five.

### Timing percentiles

These are process durations from the retained 100 per-file observations. P50,
P90, P95, and P99 are latency percentiles in milliseconds; they are unrelated
to the visual-divergence percentiles below.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Parse originals, P50 | 1,164.201 ms | 182.814 ms | 26.593 ms | — | — |
| Parse originals, P90 | 4,550.653 ms | 1,331.799 ms | 49.605 ms | — | — |
| Parse originals, P95 | 6,944.989 ms | 2,029.282 ms | 58.166 ms | — | — |
| Parse originals, P99 | 14,024.827 ms | 7,946.815 ms | 93.001 ms | — | — |
| Parse originals, maximum | 16,136.160 ms | 11,974.864 ms | 280.164 ms | — | — |
| Parse edited outputs, P50 | 1,109.898 ms | 191.145 ms | 26.404 ms | — | — |
| Parse edited outputs, P90 | 4,727.693 ms | 1,305.994 ms | 49.009 ms | — | — |
| Parse edited outputs, P95 | 6,484.281 ms | 2,138.305 ms | 55.323 ms | — | — |
| Parse edited outputs, P99 | 15,025.047 ms | 8,167.515 ms | 166.890 ms | — | — |
| Parse edited outputs, maximum | 15,919.602 ms | 11,679.947 ms | 327.036 ms | — | — |
| Verified edit end-to-end, P50 | 33,016.260 ms | — | — | — | — |
| Verified edit end-to-end, P90 | 55,890.344 ms | — | — | — | — |
| Verified edit end-to-end, P95 | 68,940.164 ms | — | — | — | — |
| Verified edit end-to-end, P99 | 98,026.668 ms | — | — | — | — |
| Verified edit end-to-end, maximum | 111,736.765 ms | — | — | — | — |

The parser rows are not equivalent-operation speed comparisons: Wellfriend PDF emits
a semantic document model, qpdf performs structural checks, and `mutool info`
inventories document resources. End-to-end edit time includes extraction,
planning, approval, mutation, qpdf checks, reopen/extraction, and independent
Poppler verification.

### Rendering against PDFium, MuPDF, and Poppler

Page one of every original and edited PDF was rendered at 144 DPI. A comparison
was counted only when Wellfriend PDF and the reference renderer both produced an image
with identical dimensions.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| First-page raster, 100 originals | 100/100 produced | — | 100/100 produced | 100/100 produced | 100/100 produced |
| First-page raster, 100 edited outputs | 100/100 produced | — | 100/100 produced | 100/100 produced | 100/100 produced |

**These are output-production and same-dimension results, not visual-fidelity
passes.** The retained visual rerun exposes a severe Wellfriend PDF layout failure in
the maximum-divergence document. The full four-renderer sheets, heatmaps,
timestamps, raster hashes, and timing measurements are in the
[human-viewable visual evidence report](docs/reports/ecbes-vps-20260929-visual/README.md).

#### Current post-fix render

![Corrected Wellfriend PDF/PDFium/MuPDF/Poppler comparison](docs/reports/ecbes-vps-20260929-visual/fixes/type1-font-matrix/comparison.webp)

The catastrophic case was traced to an ignored non-default Type 1
`/FontMatrix`: the font used approximately 1/2048 glyph-space scaling while the
renderer assumed 1/1000. A source correction now applies the complete embedded
font matrix. The exact input was rerendered on the VPS and is readable; its
`changed > 8` divergence fell from 31.29-32.42% to 10.00-12.80% across the
three references. See the [focused before/after correction report](docs/reports/ecbes-vps-20260929-visual/fixes/type1-font-matrix/README.md).
The full 100-file corpus has not yet been rerun with this correction, so the
original campaign is retained below as historical failure evidence.

<details>
<summary>Historical pre-fix failure: oversized and overlapping Type 1 text</summary>

![Historical maximum-divergence comparison](docs/reports/ecbes-vps-20260929-visual/originals/pages/047-arxiv-eess-iv-2609-28194v1-5e65af65d2ee-pdf.webp)

</details>

`Changed > 8` is the percentage of pixels where at least one RGB channel
differs from the reference by more than 8. Antialiasing and color-management
policy can contribute to the value, so it is a diagnostic—not a percentage of
objectively incorrect pixels. These P50/P90/P95/P99 columns are distributions
of pixel divergence across files, not render-time percentiles.

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Originals, changed > 8 P50 | Comparison source | — | 8.379496% | 10.087342% | 9.161623% |
| Originals, changed > 8 P90 | Comparison source | — | 11.293026% | 14.145231% | 12.973691% |
| Originals, changed > 8 P95 | Comparison source | — | 11.791062% | 14.624750% | 13.444442% |
| Originals, changed > 8 P99 | Comparison source | — | 15.770450% | 18.511040% | 17.801277% |
| Originals, changed > 8 maximum, pre-fix | Comparison source | — | 31.288504% | 32.420260% | 31.454685% |
| Exact formerly broken page, post-fix | Comparison source | — | 10.000130% | 12.801973% | 11.236840% |
| Edited, changed > 8 P50 | Comparison source | — | 8.366498% | 10.078393% | 9.155820% |
| Edited, changed > 8 P90 | Comparison source | — | 11.277501% | 14.142704% | 12.949965% |
| Edited, changed > 8 P95 | Comparison source | — | 11.794105% | 14.618561% | 13.439491% |
| Edited, changed > 8 P99 | Comparison source | — | 15.751366% | 18.493297% | 17.781729% |
| Edited, changed > 8 maximum, pre-fix | Comparison source | — | 31.297379% | 32.425445% | 31.462513% |

The edited-output distribution closely follows the original distribution,
which is evidence against a broad rendering regression in this edit workflow.
It is not pixel identity with the reference engines.

The original aggregate campaign did not retain per-page durations. A timestamped
visual rerun retained individual engine durations and produced these percentiles:

| Benchmark | Wellfriend PDF | qpdf | MuPDF | PDFium | Poppler |
|---|---:|---:|---:|---:|---:|
| Original render, P50 | 495.160 ms | — | 957.148 ms | 735.010 ms | 900.829 ms |
| Original render, P90 | 773.481 ms | — | 1,133.023 ms | 868.279 ms | 1,146.898 ms |
| Original render, P95 | 997.802 ms | — | 1,181.795 ms | 913.387 ms | 1,168.173 ms |
| Original render, P99 | 1,489.815 ms | — | 1,287.178 ms | 1,089.188 ms | 1,393.087 ms |
| Original render, maximum | 40,854.901 ms | — | 1,455.695 ms | 1,154.583 ms | 1,634.043 ms |
| Edited render, P50 | 490.340 ms | — | 965.530 ms | 730.140 ms | 923.954 ms |
| Edited render, P90 | 813.415 ms | — | 1,103.554 ms | 850.722 ms | 1,146.064 ms |
| Edited render, P95 | 1,164.499 ms | — | 1,184.153 ms | 918.240 ms | 1,217.007 ms |
| Edited render, P99 | 1,546.844 ms | — | 1,328.426 ms | 1,069.498 ms | 1,280.089 ms |
| Edited render, maximum | 39,961.103 ms | — | 1,435.239 ms | 1,139.994 ms | 1,788.012 ms |

The Wellfriend PDF maximum is `i1040gi.pdf`; it took about 40 seconds in both corpora,
while the next-slowest Wellfriend PDF page was about 1.5 seconds.

### Timestamped result history

| Campaign date | Source state | Semantic parse | Compatibility render | Verified source edit | Visual result |
|---|---|---:|---:|---:|---|
| 2026-09-27 | Research snapshot over `27e62db` | 99/100 | 99/100 | 6/100 glyph-preserving | Poppler median 9.611%, P95 13.444%; PDFium median 10.215%, P95 14.625% |
| 2026-09-28 | Implementation `e2a83fc` | 100/100 originals and edited | 100/100 originals and edited | 100/100 | Full P50/P90/P95/P99 table above |

The implementation commit timestamp is `2026-09-28T22:50:39+05:30`; the
evidence publication commit timestamp is `2026-09-28T22:55:36+05:30`. The
original harness did not record wall-clock campaign start and finish timestamps,
so the report does not invent them from copied-file modification times.

### Build and binding gates

| Gate | Result |
|---|---:|
| Focused renderer regression | 232 passed, 0 failed |
| Full Rust workspace | 5,399 passed, 0 failed |
| Optimized CLI and C ABI | Built successfully |
| Java binding | 3 passed, 0 failed |
| .NET binding | 17 passed, 0 failed |
| Python optimized binding | 32 passed, 0 failed, 1 skipped |
| WASM/TypeScript | `npm ci` and typecheck passed |

The complete methodology, exact tool versions, corpus manifest, per-file edit
proofs, parser diagnostics, and page-level render measurements are published in
the [100-PDF VPS qualification report](docs/reports/ecbes-vps-20260928/README.md).
Human-viewable Wellfriend PDF/PDFium/MuPDF/Poppler sheets and heatmaps are in the
[visual evidence report](docs/reports/ecbes-vps-20260929-visual/README.md).
Raw evidence is available in
[`editing-results.jsonl`](docs/reports/ecbes-vps-20260928/editing-results.jsonl),
[`parse-original.json`](docs/reports/ecbes-vps-20260928/parse-original.json),
[`parse-edited.json`](docs/reports/ecbes-vps-20260928/parse-edited.json),
[`render-original.json`](docs/reports/ecbes-vps-20260928/render-original.json),
[`render-edited.json`](docs/reports/ecbes-vps-20260928/render-edited.json), and
the derived [`timing-summary.json`](docs/reports/ecbes-vps-20260928/timing-summary.json).

These results establish 100/100 operational success for the published corpus
and workflow. They do not prove that every valid PDF or every possible edit is
supported, and Acrobat was not part of the comparator set. The project does not
claim universal PDF editing, pixel-perfect equivalence, or superiority over
Adobe from this campaign alone.

</details>

## Repository Status

The renderer source includes schema-v1 render contracts, caller-owned surfaces,
bounded document caches, packed retained plans, transaction-driven
invalidation, persistent clip state, transparency and soft-mask execution,
display/print/proof policies, deterministic tile scheduling, progressive image
and page lifecycles, native CPU/WASM SIMD kernels, font-substitution reports,
Type 3 retained programs, JPX capability reporting, SVG/PostScript regional
output, and visual-normalization tooling.

Unsupported PDF or backend cases fail through typed errors or explicitly
reported policies. Codec-native region, tile, component, reduction, and
progressive capabilities are reported from the selected decoder rather than
being simulated.

The latest runtime qualification and exact current verdict are in the
[2026-09-28 VPS report](docs/reports/ecbes-vps-20260928/README.md). The renderer
architecture remains documented in
[`docs/renderer/final-universal-renderer-implementation-report.md`](docs/renderer/final-universal-renderer-implementation-report.md),
and universal-editing implementation details are recorded in
[`docs/universal_editing_v2_implementation_report.md`](docs/universal_editing_v2_implementation_report.md).

## Requirements

| Component | Requirement |
|---|---|
| Rust workspace | Rust 1.95 or newer; Cargo |
| Python binding | Python 3.9+ and `maturin` |
| WASM binding | `wasm-pack` and a wasm32 Rust target |
| .NET binding | .NET 8 SDK |
| Java binding | JDK 25 with preview FFM enabled |
| Native bindings | A locally built `wellfriendpdf_capi` shared library |
| Optional OCR | Tesseract development/runtime libraries for the native OCR adapter |
| Optional native CMM | Build with the `native-cmm-lcms2` feature |

Clone and build the default workspace:

```powershell
git clone https://github.com/demisuga01-lab/wellfriendpdf.git
cd wellfriendpdf
cargo build --workspace --jobs 2
```

For a smaller production build, select only the component you need. The engine
default features are `parse`, `render`, and `structural`; its `full` feature
adds extraction, creation, editing, signing, standards, and OCR-facing APIs.

## Component Map

| Component | Package/path | Use it for |
|---|---|---|
| Rust engine | `crates/engine` / `wellfriendpdf-engine` | Native library integration and all core APIs |
| CLI | `crates/cli` / `wellfriendpdf-cli` | Shell workflows and JSON reports |
| HTTP server | `crates/server` / `wellfriendpdf-server` | Authenticated multipart HTTP API and progressive sessions |
| SIMD kernels | `crates/render-simd` | Internal CPU and wasm32 `simd128` raster kernels |
| C ABI | `crates/wellfriendpdf-capi` | Stable native boundary used by C, .NET, and Java |
| Python | `crates/wellfriendpdf-py` | PyO3/maturin package named `wellfriendpdf` |
| WASM | `crates/wellfriendpdf-wasm` | Browser, WebWorker, and Node byte-oriented APIs |
| .NET | `bindings/dotnet/WellfriendPdf` | `net8.0` managed wrapper over the C ABI |
| Java | `bindings/java` | JDK 25 FFM wrapper over the C ABI |
| Native OCR | `crates/wellfriendpdf-ocr-tesseract` | Optional Tesseract-backed OCR provider |
| Visual normalization | `tools/renderer-visual-diff` | Future same-policy image normalization and comparison |
| PDFium harness | `tools/pdfium-harness` | Future direct comparator harness; not required by the engine |

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

Add the local crate from another workspace while packages remain unpublished:

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

Run `wellfriendpdf <command> --help` before automating a command: mutation
commands have explicit output/refusal policies and do not all share the same
arguments.

## HTTP Server

The server listens on port `8080` by default. It refuses unauthenticated startup
unless API keys are configured or local-development access is explicitly
enabled.

```powershell
$env:WELLFRIENDPDF_ALLOW_UNAUTHENTICATED = "true"
$env:WELLFRIENDPDF_PORT = "8080"
cargo run -p wellfriendpdf-server --jobs 2
```

For a protected instance, unset `WELLFRIENDPDF_ALLOW_UNAUTHENTICATED`, set
`WELLFRIENDPDF_API_KEYS` to a comma-separated key list, and send either
`X-API-Key: <key>` or `Authorization: Bearer <key>`. Do not expose development
mode publicly.

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

WASM accepts bytes and returns bytes/JSON; it does not read host paths, fetch
URLs, spawn OCR workers, or load native libraries. `ProgressiveRenderJob`
supports cancellation checks, queue execution, callback reports, adjacent-page
prefetch, and `reviseRenderContractJson`. The TypeScript declaration source is
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

The CLI and server do not require OCR. Enable the native adapter explicitly:

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
and dimensions before future comparisons. `tools/pdfium-harness` is a direct C
harness reserved for a separately provisioned PDFium verification environment.
Neither tool is required to build or use Wellfriend.

Do not publish benchmark claims from these tools without the exact source
commit, dataset manifest, comparator version, normalization manifest, command
line, failures, and raw evidence.

## Error and Safety Model

- Page numbers are 1-based across public APIs.
- Passwords are operation-scoped inputs and are not retained by managed wrappers.
- Unsupported or malformed rendering cases return typed errors instead of silently dropping paint.
- High-quality exact rendering refuses material-degrading compatibility substitutions.
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
- [`docs/renderer/final-universal-renderer-implementation-report.md`](docs/renderer/final-universal-renderer-implementation-report.md): renderer architecture.
- [`docs/renderer/complete-algorithm-and-method-inventory.md`](docs/renderer/complete-algorithm-and-method-inventory.md): algorithm and entry-point inventory.
- [`docs/renderer/final-fallback-closure-report.md`](docs/renderer/final-fallback-closure-report.md): typed refusal and fallback policy.
- [`docs/renderer/final-local-implementation-closure.md`](docs/renderer/final-local-implementation-closure.md): local source checks and final verdict.
- [`docs/renderer/independent-closure-audit-2026-09-07.md`](docs/renderer/independent-closure-audit-2026-09-07.md): independent failure discovery, remediation evidence, code-line accounting, and proof boundary.

## License

Wellfriend PDF SDK is available under the MIT License. See [`LICENSE`](LICENSE).
