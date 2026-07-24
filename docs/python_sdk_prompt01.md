# Python SDK — Prompt 01 Report Surfaces

The `wellfriendpdf` Python module (crate `wellfriendpdf-py`, built with maturin/pyo3) gains the
full report surface in Prompt 01, backed by the shared `wellfriendpdf_engine::sdk`
facade. Every report method returns a **native Python dict** parsed from the
facade's versioned-JSON envelope `{"schema_version", "kind", "report"}`.

## Lifecycle

```python
import wellfriendpdf

doc = wellfriendpdf.open("in.pdf")          # path (os.PathLike) or bytes
doc = wellfriendpdf.open(data)              # bytes
doc = wellfriendpdf.Document.from_bytes(data, password="secret")
n = doc.page_count
text = doc.page(1).text
```

`Document` holds an `Arc<ContentEngine>`; page/region objects share it safely.
Invalid input raises `wellfriendpdf.WellfriendError`; out-of-range pages raise `IndexError`.

## Report methods (return dict)

| Method | Envelope kind |
| --- | --- |
| `doc.security_report()` | `security_report` |
| `doc.risky_content_report()` | `risky_content_report` |
| `doc.parser_report(mode="repair"\|"strict"\|"audit")` | `parser_report` |
| `doc.color_report(profile="generic"\|"pdfa"\|"pdfx")` | `color_report` |
| `doc.validate_pdfa(profile="pdfa2b"...)` | `pdfa_validation` |
| `doc.validate_pdfua()` | `pdfua_validation` |
| `doc.validate(profile="all"\|"pdfa"\|"pdfua"\|"pdfx"\|"security")` | `standards_profile` |
| `doc.interactive_report()` | `interactive_report` |
| `doc.forms_report()` | `forms_report` |
| `doc.annotations_report()` | `annotation_report` |
| `doc.pages_report()` | `page_operations_report` |
| `doc.signature_report()` | `signature_report` |
| `doc.font_report()` | `font_report` |
| `doc.text_semantic(pages=None)` | `text_semantic` |
| `doc.chunks()` | `chunk_set` |
| `doc.semantic_document(pages=None)` | `semantic_document` |

Module-level (no document):

| Function | Envelope kind |
| --- | --- |
| `wellfriendpdf.feature_report()` | `feature_report` |
| `wellfriendpdf.decode_budget_report(filter, width, height, components=3)` | `decode_budget_report` |
| `wellfriendpdf.resource_dedup_report([b"..", ...])` | `resource_dedup_report` |

## Output-producing methods (return `(bytes, dict)`)

```python
data, report = doc.sanitize(policy="balanced", output="clean.pdf")
data, report = doc.canonicalize(date_epoch=0)           # deterministic
data, report = doc.redact(["SECRET"], strict=True)      # raises if a term survives
```

The `bytes` is the produced PDF (also written to `output` when given); `report`
is the parsed envelope dict (`sanitize_report`, `canonicalize_report`,
`redaction_report`). Redaction verifies the terms are absent from the output.

## Memory & lifetime

- Report methods copy the document bytes out of the reader once per call, run the
  facade, and return a fresh dict — no borrowed Rust memory escapes into Python.
- Output methods return an owned `bytes`; the dict is plain JSON-loaded data.
- All work runs under `catch_unwind`; a Rust panic surfaces as `WellfriendError`, not
  a process crash. Objects remain safe after the document goes out of scope.

## Honesty about limits

- Progress callbacks, cancellation tokens, and configurable recursion/object/
  timeout budgets are **not** yet Python parameters. They appear in the gap
  matrix as `unsupported_reported`/`partial_public`, not faked.
- Dynamic XFA is reported (via `forms_report`/`security_report`), not claimed as
  rendered.

## Version markers

```python
wellfriendpdf.__version__                    # crate version
wellfriendpdf.__report_envelope_version__    # report envelope version (== 1)
wellfriendpdf.feature_report()["report"]["capabilities"]   # compiled features
```

## Tests & example

- `crates/wellfriendpdf-py/tests/test_reports.py` — 12 tests (envelopes, fields, invalid
  input, deterministic canonicalize, redaction removal + verification, parity).
- `crates/wellfriendpdf-py/tests/test_smoke.py` — pre-existing 6 tests still green.
- `crates/wellfriendpdf-py/examples/sdk_reports.py` — runnable demo / smoke generator.

Build & test:

```sh
cd crates/wellfriendpdf-py
python -m venv .venv && .venv/Scripts/python -m pip install maturin pytest
.venv/Scripts/python -m maturin develop --release
.venv/Scripts/python -m pytest tests/
```
