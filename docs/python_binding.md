# Python Binding

Oxide now has a PyO3/maturin binding in `crates/oxide-py` that installs a Python
module named `oxide`.

```python
import oxide

doc = oxide.open("invoice.pdf")
print(doc.page_count)
print(doc.extract_text())
print(doc.page(1).tables)
print(doc.extract_fields(doc_type="auto"))
print(doc.page(1).region(0, 0, 306, 792).text)
print(doc.to_markdown(detect_headings=True, profile="rag-chunks"))
png = doc.page(1).render(dpi=150)
```

The API is intentionally Python-native: text is `str`, rendered pages and images
are `bytes`, and tables/fields/document models are returned as lists and dicts.
Malformed-input errors become `oxide.OxideError`; Rust panics are caught at the
FFI boundary and converted to the same exception type.

## Build

```powershell
python -m pip install maturin
cd crates\oxide-py
python -m maturin build
python -m pip install target\wheels\oxide_pdf-0.1.0-*.whl
python -c "import oxide; print(oxide.__version__)"
```

Verified locally on Windows with Python 3.14.3. Prebuilt cross-platform wheels
are not claimed yet; CI wheel publishing is future packaging work.

## Current Coverage

- Open from path or bytes, optionally with a password.
- Document/page text extraction, with named profiles:
  `fast-text`, `layout-faithful`, `tables-focused`, and `rag-chunks`.
- Lazy page properties: `page.text`, `page.words`, `page.tables`, `page.images`.
- Document-level tables, fields, document model, markdown, HTML, and rendering.
- Region/scoped extraction through `page.region(x0, y0, x1, y1)` and
  `page.within(...)`. The returned scoped object exposes `.text`, `.words`,
  `.tables`, and `.images`.
- Explicit markdown heading control through
  `to_markdown(detect_headings=True|False)` and `page.markdown(...)`.

Region coordinates are PDF user-space points with origin at the page's
bottom-left. Scoped extraction includes an item when its center is inside the
region or at least half of its bounding box overlaps it. Markdown heading
detection is heuristic unless the source PDF supplies tagged structure.
