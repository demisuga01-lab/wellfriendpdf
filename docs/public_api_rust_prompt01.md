# Rust Public API — Prompt 01 Stabilization

Prompt 01 adds one stable public module, [`wellfriendpdf_engine::sdk`], as the shared
versioned-JSON report facade for the Python and C ABI bindings. It does **not**
rewrite the engine; it normalizes and safely exposes reports that already exist.

## Public API map

Three tiers, unchanged in spirit from the pre-existing layout:

| Tier | What | Stability |
| --- | --- | --- |
| **Curated integration** | `wellfriendpdf_engine::prelude` | The recommended embedder surface. |
| **SDK facade (new)** | `wellfriendpdf_engine::sdk` | Stable, versioned-JSON report layer that bindings call. |
| **Flat crate root** | `wellfriendpdf_engine::*` | Rich low-level building blocks (parser, renderer, writer, object model). Useful but pre-1.0. |
| **Crate-private** | `crate::...` internals | Not public. |

Bindings depend on the **SDK facade**, not on the flat root, so their behavior
cannot drift from a single well-tested layer.

## The `sdk` facade

Every function takes the document as `&[u8]` plus an optional password, and
returns a stable envelope string (see
[`report_schema_versioning_prompt01.md`](report_schema_versioning_prompt01.md)).

Read-only reports (`-> Result<String>`):

| Function | Report kind |
| --- | --- |
| `security_report_json` | `security_report` |
| `risky_content_report_json` | `risky_content_report` |
| `document_info_json` | `document_info` |
| `parser_report_json(mode)` | `parser_report` |
| `color_report_json(profile)` | `color_report` |
| `pdfa_validation_json(profile)` | `pdfa_validation` |
| `pdfua_validation_json` | `pdfua_validation` |
| `standards_profile_json(profile)` | `standards_profile` |
| `interactive_report_json` | `interactive_report` |
| `forms_report_json` | `forms_report` |
| `annotation_report_json` | `annotation_report` |
| `page_operations_report_json` | `page_operations_report` |
| `signature_report_json` | `signature_report` |
| `font_report_json` | `font_report` |
| `decode_budget_report_json(filter,w,h,c)` | `decode_budget_report` |
| `resource_dedup_report_json(&[Vec<u8>])` | `resource_dedup_report` |
| `text_semantic_json(pages)` | `text_semantic` |
| `chunk_report_json` | `chunk_set` |
| `semantic_document_json(pages)` | `semantic_document` |
| `feature_report_json()` | `feature_report` |

Output-producing operations (`-> Result<(Vec<u8>, String)>` = produced bytes +
report):

| Function | Report kind |
| --- | --- |
| `sanitize_json(policy)` | `sanitize_report` |
| `canonicalize_json(date_epoch)` | `canonicalize_report` |
| `redact_terms_json(terms, strict)` | `redaction_report` |

## Options normalization

The facade takes small typed inputs (byte slice, `Option<&[u8]>` password, and
string enum selectors like `"balanced"`/`"pdfa2b"`) rather than a dozen ad-hoc
booleans. String selectors map to the engine's own enums (`SanitizerOptions`,
`PdfAProfile`, `StandardsProfile`, `ColorValidationProfile`, `ParserMode`) with
documented defaults. Underlying option structs remain available on the flat root
for advanced Rust callers who need full control.

## Error normalization

The facade returns the engine's [`WellfriendError`], whose [`ErrorKind`] gives a
stable taxonomy: `io`, `malformed_pdf`, `parse`, `missing_object`,
`unsupported_feature`, `encrypted`, `cancelled`, `resource_limit`. A new
convenience constructor `WellfriendError::invalid_input(msg)` (categorized as
`parse`) is added for argument-validation failures in the facade and bindings.

## Feature availability

`feature_report_json()` reports the engine version, envelope version, the
compiled cargo capabilities (`parse`/`render`/`sign`/`pdfa`/`ocr`/…), and the
always-available report set. Bindings expose this so integrators query
availability instead of guessing.

## Determinism preserved

`canonicalize_json` threads the `fixed_source_date_epoch` option and produces
byte-deterministic output (the tests assert two runs are byte-equal). The full
`DeterministicSaveOptions` / `WriterMode` surface remains on the flat root.

## Tests

- `crates/engine/src/sdk.rs` `#[cfg(test)] mod tests` — 12 downstream-style
  tests that call the facade with bytes and assert the envelope + a report field,
  including invalid input and deterministic-canonicalize.
- `cargo run -p wellfriendpdf-engine --example sdk_reports` — runnable example that
  drives every facade area and can emit the smoke JSON.
