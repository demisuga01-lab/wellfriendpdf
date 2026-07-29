# C ABI — Binding Surface Report Surfaces

The C ABI (crate `wellfriendpdf-capi`, header `crates/wellfriendpdf-capi/include/wellfriendpdf.h`) gains
report, output, and version/capability functions in Binding Surface, backed by the
shared `wellfriendpdf_engine::sdk` facade. The JSON returned is byte-identical to what
the Python bindings return for the same document.

## Design: boring and stable

- **Opaque handle** `WellfriendDocument *` (from `wellfriendpdf_document_open_from_bytes`).
- **Integer status codes**: `WELLFRIENDPDF_STATUS_OK` (0), `WELLFRIENDPDF_STATUS_NULL` (1),
  `WELLFRIENDPDF_STATUS_ERROR` (2), `WELLFRIENDPDF_STATUS_PANIC` (3). A panic is caught and
  returned as a code — never undefined behavior.
- **Rich reports → versioned JSON** through `char **out_json`.
- **Scalar helpers** where stable: `wellfriendpdf_document_page_count`, `wellfriendpdf_version`,
  `wellfriendpdf_abi_version`.
- **Caller-owned buffers**: `WellfriendBuffer { data, len }` for produced PDFs.

## Ownership & lifetime (contract in `wellfriendpdf.h`)

- On success, `*out_json` is a heap NUL-terminated UTF-8 string **owned by the
  caller** — free with `wellfriendpdf_string_free`.
- `*error_out` (if non-null on entry) receives an owned message on error — free
  with `wellfriendpdf_error_free`. Cleared to null on success.
- Output ops also write an `WellfriendBuffer` **owned by the caller** — free with
  `wellfriendpdf_buffer_free`.
- A null document handle yields `WELLFRIENDPDF_STATUS_ERROR` and a message, never a
  crash. Every allocation has exactly one matching free function, and the inline
  tests call them.

## New report functions (all `-> int` status, `out_json` owned)

| Function | `mode`/`profile` param | Kind |
| --- | --- | --- |
| `wellfriendpdf_document_security_report_json` | — | `security_report` |
| `wellfriendpdf_document_parser_report_json` | `mode` (NULL→repair) | `parser_report` |
| `wellfriendpdf_document_color_report_json` | `profile` (NULL→generic) | `color_report` |
| `wellfriendpdf_document_validate_json` | `profile` (NULL→all) | `standards_profile` |
| `wellfriendpdf_document_forms_report_json` | — | `forms_report` |
| `wellfriendpdf_document_annotations_report_json` | — | `annotation_report` |
| `wellfriendpdf_document_pages_report_json` | — | `page_operations_report` |
| `wellfriendpdf_document_interactive_report_json` | — | `interactive_report` |
| `wellfriendpdf_document_chunks_json` | — | `chunk_set` |

Pre-existing report-ish functions retained: `wellfriendpdf_document_info_json`,
`wellfriendpdf_document_fonts_json`, `wellfriendpdf_document_signatures_json`,
`wellfriendpdf_document_extract_semantic_json`, `wellfriendpdf_document_parse_json`.

## New output-producing functions (buffer + report)

| Function | Params | Kind |
| --- | --- | --- |
| `wellfriendpdf_document_sanitize_json` | `policy` (NULL→balanced) | `sanitize_report` |
| `wellfriendpdf_document_canonicalize_json` | `date_epoch`, `has_date_epoch` | `canonicalize_report` |
| `wellfriendpdf_document_redact_terms_json` | `terms[]`, `terms_len`, `strict` | `redaction_report` |

## New version / capability query (no document)

- `int wellfriendpdf_feature_report_json(char **out_json, char **error_out)` — engine
  version, envelope version, compiled capabilities. Free `*out_json`.
- `char *wellfriendpdf_version(void)` — engine semver string (free with
  `wellfriendpdf_string_free`). Safe to call.
- `uint32_t wellfriendpdf_abi_version(void)` — report envelope version. Safe to call.

## Thread safety

`WellfriendDocument` wraps a `Send + Sync` engine (compile-time asserted). A single
handle may be read concurrently from multiple threads; the report calls do not
mutate it. Do not free a handle while another thread is using it.

## Usage sketch

```c
WellfriendDocument *doc = wellfriendpdf_document_open_from_bytes(buf, len, &err);
char *json = NULL;
if (wellfriendpdf_document_security_report_json(doc, &json, &err) == WELLFRIENDPDF_STATUS_OK) {
    /* parse json ... */
    wellfriendpdf_string_free(json);
}

WellfriendBuffer out = {0};
char *report = NULL;
if (wellfriendpdf_document_sanitize_json(doc, "balanced", &out, &report, &err)
        == WELLFRIENDPDF_STATUS_OK) {
    /* out.data / out.len is the sanitized PDF */
    wellfriendpdf_buffer_free(out);
    wellfriendpdf_string_free(report);
}
wellfriendpdf_document_free(doc);
```

## Tests & example

- Inline `#[cfg(test)]` tests in `crates/wellfriendpdf-capi/src/lib.rs`:
  `capi_read_only_report_envelopes`, `capi_parametrized_reports`,
  `capi_sanitize_and_canonicalize_output_and_report`,
  `capi_redact_terms_output_and_report`, `capi_feature_and_version`,
  `capi_report_null_document_is_error_not_panic` — each frees every allocation.
- `crates/wellfriendpdf-capi/examples/sdk_reports.c` — compiles against `wellfriendpdf.h`, links
  the built `wellfriendpdf_capi` cdylib, opens a fixture, calls the report/output/version
  functions, frees everything, and can dump a smoke JSON. Verified compiled with
  MSVC and run against the real DLL in this roadmap task.

Build the cdylib: `cargo build -p wellfriendpdf-capi`. Run the C-ABI test suite:
`cargo test -p wellfriendpdf-capi`.
