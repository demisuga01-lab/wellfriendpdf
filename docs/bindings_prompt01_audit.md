# Combined Prompt 01 — Binding Core Surfaces: Audit Note

This document is the evidence trail for the binding surface audit and the
Rust / Python / C-ABI stabilization performed in Combined Prompt 01
(roadmap items 001–004). It is updated during the work, not only at the end.

## Starting checkpoint

- Starting HEAD: `cd67cbc` (`Complete Prompt 09 enterprise trust hardening`)
- Starting worktree status: **clean** (`git status --short` empty)
- Toolchain: cargo/rustc 1.95.0, Python 3.14.3, maturin present, MSVC `cl`
  available (no unix `cc`/`gcc`/`clang` on PATH).

## Design principle chosen

The engine (`wellfriendpdf-engine`) is already large and mostly public: `ContentEngine`
is the main facade, and nearly every report type derives `serde::Serialize`.
The core problem this prompt solves is **binding divergence**: the CLI can
produce security / sanitizer / redaction / parser / color / validation /
canonicalize reports, but Python and C ABI cannot, and each surface reimplements
its own wiring.

Rather than hand-write dozens of divergent Python methods and C functions, this
prompt introduces **one shared Rust facade** —
[`wellfriendpdf_engine::sdk`](../crates/engine/src/sdk.rs) — that returns **versioned
JSON reports** from a small, stable set of functions. Python and the C ABI both
call the *same* facade functions, so a security report requested from Python and
from C is byte-identical JSON (guaranteeing cross-surface parity), and future
prompts extend one place instead of three.

- Rich reports → versioned JSON (`schema_version` on the envelope).
- Common scalar metadata (page count, encrypted flag, signature count) → direct
  scalar helpers, unchanged where they already exist.
- Destructive operations (sanitize, redaction apply, canonicalize) → return the
  output bytes AND the report, and are clearly named as producing output.
- Unsupported / partial capabilities → honest diagnostics inside the report,
  never a fake success.

This is **binding/API stabilization**, not an engine rewrite. The only engine
change is the additive `sdk` facade module plus its `pub use` re-export; no
existing behavior is modified.

## Crates and surfaces inspected

| Surface | Location | Entry point | Notes |
| --- | --- | --- | --- |
| Rust public API | `crates/engine/src/lib.rs` | `ContentEngine`, `prelude`, flat re-exports | Rich, mostly stable pre-1.0. |
| Rust SDK facade (new) | `crates/engine/src/sdk.rs` | `wellfriendpdf_engine::sdk::*` | Versioned-JSON report layer shared by bindings. |
| Python SDK | `crates/wellfriendpdf-py/src/lib.rs` | `wellfriendpdf.Document`, module fns | pyo3; ergonomic but report-light before this prompt. |
| C ABI | `crates/wellfriendpdf-capi/src/lib.rs` + `include/wellfriendpdf.h` | `WellfriendDocument *` + `WellfriendBuffer` | opaque handle, owned buffers, explicit free fns. |
| CLI | `crates/cli/src/main.rs` (5314 lines) | subcommands | Superset of report capability; source of truth for gaps. |
| WASM | `crates/wellfriendpdf-wasm` | wasm-bindgen | Out of scope for this prompt (documented gap). |
| .NET / Java | `bindings/dotnet`, `bindings/java` | thin over C ABI | Out of scope; inherit C ABI additions. |

## Existing binding coverage (pre-Prompt-01 baseline)

**Python (`wellfriendpdf` module):** `Document` (open path/bytes, page_count, metadata,
pages/iteration, extract_text, extract_tables, extract_fields, document_model,
to_markdown, to_html, render), `Page`/`RegionPage` (text, words, tables, images,
region), plus module functions: merge_pdfs, extract_pages, rotate_pdf,
decrypt_pdf, encrypt_pdf, optimize_pdf, repair_pdf, linearize_pdf, pdf_to_images,
images_to_pdf, pdf_to_{xlsx,pptx,docx}, {docx,xlsx,pptx}_to_pdf, watermark_pdf,
add_page_numbers, organize_pdf, fonts, verify_signatures.

**C ABI:** open_from_bytes/free, page_count, extract_text, parse_markdown/json
(+ `_ocr`), extract_fields_json, extract_semantic_json, info_json,
render_page_png/jpeg, extract_pages/organize/rotate/optimize/linearize/decrypt/
encrypt_aes256, to_html/xlsx/pptx/docx, {docx,xlsx,pptx}_to_pdf, fonts_json,
signatures_json, watermark_text, add_page_numbers, images_to_pdf,
merge_pdfs_from_bytes, set_ocr_backend. Buffer/string/error free fns present.

**Gaps common to both bindings (the core of this prompt):** security report,
sanitizer (policy + apply + rescan), redaction (plan/apply/verify), parser
report (repair/xref/revisions/linearization/encryption discovery), color report,
PDF/A + PDF/UA + standards-profile validation, canonicalize, forms report,
annotation report, page-operations report, resource-dedup report, deterministic
writer report, feature-availability + version reporting.

## Shared facade functions added (`wellfriendpdf_engine::sdk`)

All return `String` (UTF-8 JSON) or `(Vec<u8>, String)` for output-producing ops.
Each JSON envelope carries `schema_version` and `report`. Input is bytes +
optional password, so bindings need only pass the document bytes.

| Facade fn | Underlying engine call | Kind |
| --- | --- | --- |
| `security_report_json` | `security::security_report` | read report |
| `risky_content_report_json` | `security::scan_risky_content` | read report |
| `sanitize_json` | `security::sanitize_pdf` | output + report |
| `canonicalize_json` | `security::canonicalize_pdf` | output + report |
| `parser_report_json` | `parser_report::parser_report_bytes_with_password` | read report |
| `color_report_json` | `color_report::color_report_bytes` | read report |
| `pdfa_validation_json` | `compliance::validate_pdfa` | read report |
| `pdfua_validation_json` | `compliance::validate_pdfua` | read report |
| `standards_profile_json` | `standards::validate_standards_profile` | read report |
| `interactive_report_json` | `interactive::interactive_report` | read report |
| `forms_report_json` | `interactive::forms_report` | read report |
| `annotation_report_json` | `interactive::annotation_report` | read report |
| `page_operations_report_json` | `interactive::page_operations_report` | read report |
| `document_info_json` | `info::DocumentInfo::gather` | read report |
| `redact_terms_json` | `editing::PdfEditor` + `redaction_verification_report` | output + report |
| `signature_report_json` | `ContentEngine::verify_signatures` | read report |
| `font_report_json` | `ContentEngine::list_fonts` | read report |
| `decode_budget_report_json` | `filters::decode_image_budget_report` | read report |
| `resource_dedup_report_json` | `versioning::resource_dedup_report` | read report |
| `text_semantic_json` | `ContentEngine::extract_text_semantic_model` | read report |
| `semantic_document_json` | `ContentEngine::extract_semantic_document` | read report |
| `chunk_report_json` | `Document::chunk` | read report |
| `feature_report_json` | build-time cfg + engine version | capability query |

Actual `sdk` module: 20 read-report functions + 3 output-producing functions
(`sanitize_json`, `canonicalize_json`, `redact_terms_json`), all validated by
`crates/engine/src/sdk.rs` tests.

See [`public_api_rust_prompt01.md`](public_api_rust_prompt01.md),
[`python_sdk_prompt01.md`](python_sdk_prompt01.md),
[`c_abi_prompt01.md`](c_abi_prompt01.md), and
[`report_schema_versioning_prompt01.md`](report_schema_versioning_prompt01.md).

## Follow-up / bounded limits (for later combined prompts)

- WASM and .NET/Java parity with the new report facade — **deferred** to a later
  binding prompt; C ABI additions are the substrate the managed bindings wrap.
- Streaming / progress callbacks and cancellation tokens over the C ABI and
  Python are **partial**: engine has `CancelToken`; wiring a C function-pointer
  cancel token and a Python callback is out of this prompt's core scope and is
  recorded in the matrix as `partial_public` with an action.
- Tile/band/progressive rendering, display-list extraction, glyph positioning,
  MCID/ParentTree deep access remain Rust-level and are matrixed accordingly.

## Validation results

All commands run on this host (win-msvc, cargo 1.95.0, Python 3.14.3):

| Command | Result |
| --- | --- |
| `cargo fmt --check` | pass (0 diffs) |
| `git diff --check` | pass (only benign LF→CRLF warnings) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass (0 failures across all crates) |
| `cargo test -p wellfriendpdf-engine --lib sdk` | pass (12/12 facade tests) |
| `cargo test -p wellfriendpdf-capi` | pass (12/12, incl. 6 new report tests) |
| `cargo build -p wellfriendpdf-capi` (cdylib) | pass |
| `maturin develop --release` (wellfriendpdf-py) | pass |
| `pytest crates/wellfriendpdf-py/tests/` | pass (18/18: 12 new + 6 existing) |
| C example compile+run (MSVC, real DLL) | pass (`sdk_reports.c` over fixture) |
| CLI smokes (security/parser/validate/forms/info/extract/sanitize/canonicalize) | pass (unchanged behavior) |
| Cross-surface parity (rust/python/c-abi report bodies) | pass (byte-identical) |

Gap matrix: 180 features. Headline (best of rust/python/c_abi per feature):
`implemented_public=112`, `partial_public=37`, `implemented_internal=29`,
`cli_only=1`, `missing=1`. Per-surface Python: 109 public / 40 partial /
7 unsupported_reported / 24 missing (missing = deep low-level Rust internals
deliberately not exposed to bindings). No `blocked` or `deferred-without-reason`
rows; every non-public row carries an action or reason.

Smoke artifacts (regenerated by `scripts/gen_binding_gap_matrix.py` and the three
`sdk_reports` examples; under gitignored `target/`):
`binding-gap-matrix.json`, `rust-api-smoke.json`, `python-api-smoke.json`,
`c-abi-smoke.json`.
