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

for table in page.tables:
    print(table["rows"])

fields = doc.extract_fields(doc_type="auto")
png_bytes = page.render(dpi=150)
```

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
  `Page.markdown`, and `Page.render`

Errors from the Rust engine are converted to `oxide.OxideError`. The binding
catches Rust panics at the Python boundary and converts them to that exception
instead of aborting the interpreter.

## Deferred

Region/scoped extraction, extraction profiles, and expanded markdown heading
controls are Prompt 6 work. Manipulation/signing surfaces remain Rust/CLI first
for now.
