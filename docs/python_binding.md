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
- Document/page text extraction.
- Lazy page properties: `page.text`, `page.words`, `page.tables`, `page.images`.
- Document-level tables, fields, document model, markdown, HTML, and rendering.

Region extraction, named extraction profiles, and explicit markdown heading
switches are added in Prompt 6.
