# Bindings

Wellfriend exposes the engine beyond Rust through Python, a native C ABI, and a
`wasm-bindgen` browser wrapper.

## C ABI

Crate: `crates/wellfriendpdf-capi`

Build:

```sh
cargo build -p wellfriendpdf-capi
```

Header: `crates/wellfriendpdf-capi/include/wellfriendpdf.h`

The C API uses an opaque `WellfriendDocument *` handle and caller-owned return
buffers:

- `wellfriendpdf_document_open_from_bytes`
- `wellfriendpdf_document_open_from_bytes_with_password`
- `wellfriendpdf_document_page_count`
- `wellfriendpdf_document_extract_text`
- `wellfriendpdf_document_parse_markdown` — parse → canonical model → Markdown (RAG-facing)
- `wellfriendpdf_document_parse_json` — parse → canonical `Document` JSON (schema 1.1)
- `wellfriendpdf_document_extract_fields_json` — key-value fields → JSON (`doc_type`:
  null/`auto`/`invoice`/`receipt`/`form`/`generic`)
- `wellfriendpdf_document_extract_semantic_json` — **legacy** (older semantic model;
  prefer `wellfriendpdf_document_parse_json` for new code)
- `wellfriendpdf_document_info_json`
- `wellfriendpdf_document_render_page_png`
- `wellfriendpdf_document_render_page_jpeg`
- `wellfriendpdf_document_extract_pages_pdf` / `wellfriendpdf_document_organize_pdf`
- `wellfriendpdf_document_rotate_pdf`
- `wellfriendpdf_document_optimize_pdf`
- `wellfriendpdf_document_linearize_pdf`
- `wellfriendpdf_document_decrypt_pdf`
- `wellfriendpdf_document_encrypt_aes256_pdf`
- `wellfriendpdf_document_to_html`
- `wellfriendpdf_document_to_xlsx`
- `wellfriendpdf_document_to_pptx`
- `wellfriendpdf_document_to_docx`
- `wellfriendpdf_docx_to_pdf`
- `wellfriendpdf_xlsx_to_pdf`
- `wellfriendpdf_pptx_to_pdf`
- `wellfriendpdf_document_fonts_json`
- `wellfriendpdf_document_signatures_json`
- `wellfriendpdf_document_watermark_text_pdf`
- `wellfriendpdf_document_add_page_numbers_pdf`
- `wellfriendpdf_images_to_pdf`
- `wellfriendpdf_merge_pdfs_from_bytes`
- `wellfriendpdf_document_free`
- `wellfriendpdf_string_free` / `wellfriendpdf_error_free`
- `wellfriendpdf_buffer_free`

`wellfriendpdf_document_parse_markdown`, `wellfriendpdf_document_parse_json`, and the WASM
`parseMarkdown` / `parseJson` bindings all emit the **same canonical `Document`
schema** the CLI `wellfriendpdf parse` and the server `POST /api/v1/parse` produce, so
output is consistent across every surface. The parser ops over C are
digital-born only (OCR is not wired through the C ABI). Returned strings are
freed with `wellfriendpdf_string_free`.

`wellfriendpdf_document_open_from_bytes_with_password` accepts a UTF-8 password as
pointer plus byte length. `password == NULL && password_len == 0` means no
password; a non-null pointer with zero length means an explicit empty password.
The C ABI reads the password only for the open operation and does not log or
retain it. Existing callers can keep using `wellfriendpdf_document_open_from_bytes`.

### Report / version surfaces (Prompt 01)

The C ABI, Python, and Rust `wellfriendpdf_engine::sdk` facade share one versioned-JSON
report layer (envelope `{"schema_version","kind","report"}`). New C functions
returning owned JSON strings (free with `wellfriendpdf_string_free`):
`wellfriendpdf_document_security_report_json`, `wellfriendpdf_document_parser_report_json`,
`wellfriendpdf_document_color_report_json`, `wellfriendpdf_document_validate_json`,
`wellfriendpdf_document_forms_report_json`, `wellfriendpdf_document_annotations_report_json`,
`wellfriendpdf_document_pages_report_json`, `wellfriendpdf_document_interactive_report_json`,
`wellfriendpdf_document_chunks_json`, plus output ops
`wellfriendpdf_document_sanitize_json`, `wellfriendpdf_document_canonicalize_json`,
`wellfriendpdf_document_redact_terms_json` (owned `WellfriendBuffer` + report), and
version/capability queries `wellfriendpdf_feature_report_json`, `wellfriendpdf_version`,
`wellfriendpdf_abi_version`. See [`c_abi_prompt01.md`](c_abi_prompt01.md),
[`python_sdk_prompt01.md`](python_sdk_prompt01.md),
[`public_api_rust_prompt01.md`](public_api_rust_prompt01.md),
[`report_schema_versioning_prompt01.md`](report_schema_versioning_prompt01.md),
and the gap matrix [`bindings_prompt01_gap_matrix.md`](bindings_prompt01_gap_matrix.md).

Every exported function catches Rust panics before the FFI boundary and returns
one of:

- `WELLFRIENDPDF_STATUS_OK`
- `WELLFRIENDPDF_STATUS_NULL`
- `WELLFRIENDPDF_STATUS_ERROR`
- `WELLFRIENDPDF_STATUS_PANIC`

The sample `crates/wellfriendpdf-capi/examples/extract_text.c` opens a PDF from bytes,
extracts page 1 text, and frees all returned resources. The sample
`crates/wellfriendpdf-capi/examples/parse_document.c` opens a PDF, prints the parsed
Markdown, and prints extracted key-value fields as JSON.

Verified on this host:

```bat
cargo test -p wellfriendpdf-capi
cargo build -p wellfriendpdf-capi
call "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64
cl /I crates\wellfriendpdf-capi\include crates\wellfriendpdf-capi\examples\extract_text.c /Fe:target\debug\wellfriendpdf_capi_extract_text_example.exe /link target\debug\wellfriendpdf_capi.dll.lib
target\debug\wellfriendpdf_capi_extract_text_example.exe crates\engine\tests\fixtures\minimal.pdf

cl /I crates\wellfriendpdf-capi\include crates\wellfriendpdf-capi\examples\parse_document.c /Fe:target\debug\wellfriendpdf_capi_parse_example.exe /link target\debug\wellfriendpdf_capi.dll.lib
target\debug\wellfriendpdf_capi_parse_example.exe crates\engine\tests\fixtures\form_160f.pdf
```

The `parse_document` example was run on `form_160f.pdf`: `parse_markdown`
emitted structured Markdown (headings, paragraphs, a recovered borderless-table
grid) and `extract_fields_json` returned 67 AcroForm fields with
`"doc_type":"form"` — exercising the full parser surface over the C ABI.

`cbindgen` was not installed on this machine, so the header is committed along
with `crates/wellfriendpdf-capi/cbindgen.toml` for regeneration in environments that
have `cbindgen`.

## WASM

Crate: `crates/wellfriendpdf-wasm`

Build verified:

```sh
rustup target add wasm32-unknown-unknown
cargo build -p wellfriendpdf-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir crates/wellfriendpdf-wasm/examples/browser/pkg target/wasm32-unknown-unknown/release/wellfriendpdf_wasm.wasm
```

The wrapper exposes a JS class:

- `new WellfriendPdf(Uint8Array)`
- `pageCount()`
- `extractText(page)`
- `extractStructuredText(page)`
- `extractSemanticJson()`
- `infoJson()`
- `renderPagePng(page, dpi)` returning PNG bytes

The wrapper uses only in-memory engine operations. Server routes, job queues,
filesystem opening, and package-side temp-file workflows are excluded. The
engine dependency graph builds for wasm32 after enabling `getrandom`'s `js`
feature in the WASM crate; native engine defaults are unchanged. Rayon remains
in the engine dependency graph, but the WASM wrapper does not expose the
parallel all-pages extractor.

Browser example: `crates/wellfriendpdf-wasm/examples/browser/index.html`

Prompt H browser verification:

```sh
cd crates/wellfriendpdf-wasm/examples/browser
py -m http.server 8765
```

Headless Chrome, driven through `puppeteer-core`, loaded the local demo, selected
`tests/corpus/pdfs/generated/generated_basic_text.pdf`, extracted 116
characters, and rendered a nonblank 1020x1320 page-1 PNG. The only browser
console error was the static server's missing favicon. The in-app Browser tool
was unavailable in this session, so the live verification used standalone Chrome
instead.

## Python

Crate: `crates/wellfriendpdf-py`

Build:

```sh
maturin build --manifest-path crates/wellfriendpdf-py/Cargo.toml
```

The module exposes a `Document` class:

- `Document(bytes)` / `Document.from_path(path)`
- `len(doc)` / `doc.page_count`
- `doc.extract_text(page=None, profile="fast-text")`
- `doc.document_model(...) -> dict`
- `doc.to_markdown(...)` / `doc.to_html(...)`
- `doc.render(page, dpi=150) -> bytes`
- `doc[0]`, `doc.page(1)`, page `text`/`words`/`tables`/`images`

Module-level Phase 3/4 helpers mirror the CLI/Rust utility surface:

- `merge_pdfs(inputs, output=None, passwords=None) -> bytes`
- `extract_pages(pdf, pages, output=None, password=None) -> bytes`
- `rotate_pdf(pdf, angle, pages="all", relative=False, output=None, password=None) -> bytes`
- `encrypt_pdf(...)`, `decrypt_pdf(...)`, `optimize_pdf(...)`, `repair_pdf(...)`, `linearize_pdf(...)`
- `pdf_to_images(pdf, out_dir, pages="all", dpi=150, quality=85, format="jpg", password=None) -> list[dict]`
- `images_to_pdf(images, output=None, page_size="a4", margin=0.0) -> bytes`
- `pdf_to_xlsx(pdf, output=None, layout="pages", password=None) -> bytes`
- `pdf_to_pptx(pdf, output=None, include_images=True, password=None) -> bytes`
- `pdf_to_docx(pdf, output=None, include_images=True, password=None) -> bytes`
- `docx_to_pdf(docx, output=None) -> bytes`
- `xlsx_to_pdf(xlsx, output=None) -> bytes`
- `pptx_to_pdf(pptx, output=None) -> bytes`
- `watermark_pdf(pdf, text=None, image=None, output=None, ...) -> bytes`
- `add_page_numbers(pdf, output=None, ...) -> bytes`
- `organize_pdf(pdf, order="all", output=None, password=None) -> bytes`
- `fonts(pdf, password=None) -> list[dict]`
- `verify_signatures(pdf, password=None) -> list[dict]`

Errors map to `wellfriendpdf.WellfriendError` for engine failures and normal Python
`ValueError`/`IndexError` for bad binding-level arguments.
