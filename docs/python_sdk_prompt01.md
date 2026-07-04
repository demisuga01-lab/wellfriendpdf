# Python SDK — Prompt 01 Report Surfaces

The `oxide` Python module (crate `oxide-py`, built with maturin/pyo3) gains the
full report surface in Prompt 01, backed by the shared `oxide_engine::sdk`
facade. Every report method returns a **native Python dict** parsed from the
facade's versioned-JSON envelope `{"schema_version", "kind", "report"}`.

## Lifecycle

```python
import oxide

doc = oxide.open("in.pdf")          # path (os.PathLike) or bytes
doc = oxide.open(data)              # bytes
doc = oxide.Document.from_bytes(data, password="secret")
n = doc.page_count
text = doc.page(1).text
```

`Document` holds an `Arc<ContentEngine>`; page/region objects share it safely.
Invalid input raises `oxide.OxideError`; out-of-range pages raise `IndexError`.

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
| `oxide.feature_report()` | `feature_report` |
| `oxide.decode_budget_report(filter, width, height, components=3)` | `decode_budget_report` |
| `oxide.resource_dedup_report([b"..", ...])` | `resource_dedup_report` |

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
- All work runs under `catch_unwind`; a Rust panic surfaces as `OxideError`, not
  a process crash. Objects remain safe after the document goes out of scope.

## Honesty about limits

- Progress callbacks, cancellation tokens, and configurable recursion/object/
  timeout budgets are **not** yet Python parameters. They appear in the gap
  matrix as `unsupported_reported`/`partial_public`, not faked.
- Dynamic XFA is reported (via `forms_report`/`security_report`), not claimed as
  rendered.

## Version markers

```python
oxide.__version__                    # crate version
oxide.__report_envelope_version__    # report envelope version (== 1)
oxide.feature_report()["report"]["capabilities"]   # compiled features
```

## Tests & example

- `crates/oxide-py/tests/test_reports.py` — 12 tests (envelopes, fields, invalid
  input, deterministic canonicalize, redaction removal + verification, parity).
- `crates/oxide-py/tests/test_smoke.py` — pre-existing 6 tests still green.
- `crates/oxide-py/examples/sdk_reports.py` — runnable demo / smoke generator.

Build & test:

```sh
cd crates/oxide-py
python -m venv .venv && .venv/Scripts/python -m pip install maturin pytest
.venv/Scripts/python -m maturin develop --release
.venv/Scripts/python -m pytest tests/
```
