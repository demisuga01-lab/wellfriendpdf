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
- `Page.markdown`, `Page.render`, `Page.text_with_profile`, and
  `Page.region(...)` / `Page.within(...)`
- `RegionPage.text`, `RegionPage.words`, `RegionPage.tables`, and
  `RegionPage.images`

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

Manipulation/signing surfaces remain Rust/CLI first for now. Cross-platform
prebuilt wheels are also future CI work; the local platform wheel is built with
maturin.
