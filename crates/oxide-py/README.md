# oxide-py

Python bindings for the Oxide PDF engine, built with PyO3 and maturin.

```python
import oxide

doc = oxide.open("input.pdf")
print(doc.page_count)
print(doc.extract_text())

page = doc.page(1)
print(page.text)
print(page.words[:5])

left_half = page.region(0, 0, 306, 792)
print(left_half.text)

for table in page.tables:
    print(table["rows"])

fields = doc.extract_fields(doc_type="auto")
markdown = doc.to_markdown(detect_headings=True, profile="rag-chunks")
png_bytes = page.render(dpi=150)

docx_bytes = oxide.pdf_to_docx("input.pdf", output="input.docx")
pdf_bytes = oxide.docx_to_pdf("input.docx", output="from_docx.pdf")
```

### Report surfaces

Every report method returns a native dict (versioned-JSON envelope
`{"schema_version", "kind", "report"}`), backed by the shared
`oxide_engine::sdk` facade the C ABI also uses:

```python
doc.security_report()          # encryption, signatures, risky content
doc.parser_report(mode="audit")# repair/xref/revisions/linearization/encryption
doc.color_report()             # ICC, output intents, spot/DeviceN, overprint
doc.forms_report()             # AcroForm fields, XFA status
doc.annotations_report()       # annotations, appearances, unsafe actions
doc.pages_report()             # boxes, labels, destinations
doc.interactive_report()       # forms + annotations + page ops
doc.signature_report()         # validity, trust, coverage, LTV
doc.font_report()              # fonts, embedding, subsetting
doc.validate_pdfa(); doc.validate_pdfua(); doc.validate(profile="all")
doc.text_semantic(); doc.chunks(); doc.semantic_document()

# Output-producing (return (bytes, report)):
data, rep = doc.sanitize(policy="balanced", output="clean.pdf")
data, rep = doc.canonicalize(date_epoch=0)          # deterministic
data, rep = doc.redact(["SECRET"], strict=True)     # verifies absence

# No-document queries:
oxide.feature_report()                               # version + capabilities
oxide.decode_budget_report("DCTDecode", 4096, 4096, 3)
oxide.resource_dedup_report([b"a", b"a", b"b"])
```

See `../../docs/python_sdk_prompt01.md` and `examples/sdk_reports.py`.

Scanned-page OCR can be supplied by any Python object implementing
`recognize(image_bytes, info) -> list[dict]`:

```python
class MyOcr:
    def recognize(self, image_bytes, info):
        return [
            {"text": "Hello", "bbox": [72, 60, 140, 88],
             "confidence": 0.98, "line_id": 0},
        ]

markdown = doc.to_markdown(ocr=MyOcr(), ocr_lang="eng", ocr_dpi=300)
```

See `examples/local_ai_ocr_backend.py` for a runnable local-AI template with a
real `pytesseract` fallback when those Python packages are installed.

## Build And Install

```powershell
python -m pip install maturin
cd crates\oxide-py
python -m maturin build
python -m pip install target\wheels\oxide_pdf-0.1.0-*.whl
python -c "import oxide; print(oxide.__version__)"
```

`oxide.open()` accepts a filesystem path or raw `bytes`. Password-protected PDFs
can be opened with `password="..."`.

## Exposed In This Binding

- `oxide.open(path_or_bytes, password=None)`
- `Document.page_count`, `Document.metadata`, `Document.page(n)`,
  iteration over pages, and `doc[index]`
- `Document.extract_text`, `extract_tables`, `extract_fields`,
  `document_model`, `to_markdown`, `to_html`, and `render`
- `Page.text`, `Page.words`, `Page.tables`, `Page.images`,
- `Page.markdown`, `Page.render`, `Page.text_with_profile`, and
  `Page.region(...)` / `Page.within(...)`
- `RegionPage.text`, `RegionPage.words`, `RegionPage.tables`, and
  `RegionPage.images`
- Module helpers for structural ops and conversions including
  `pdf_to_xlsx`, `pdf_to_pptx`, `pdf_to_docx`, `docx_to_pdf`,
  `xlsx_to_pdf`, and `pptx_to_pdf`

Region coordinates are PDF user-space points with origin at the page's
bottom-left. Scoped extraction includes an item when its center is in the region
or at least half of its box overlaps the region.

Named profiles are `fast-text`, `layout-faithful`, `tables-focused`, and
`rag-chunks`. Profiles are convenience bundles over Rust engine options; they do
not reimplement parsing in Python.

Errors from the Rust engine are converted to `oxide.OxideError`. The binding
catches Rust panics at the Python boundary and converts them to that exception
instead of aborting the interpreter.

## Deferred

Cross-platform prebuilt wheels are future CI work; the local platform wheel is
built with maturin.
