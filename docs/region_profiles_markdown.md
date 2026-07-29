# Region Extraction, Profiles, And Markdown Headings

Native Renderer closes three API-parity gaps across the Rust API, CLI, and Python
binding: scoped extraction, named extraction profiles, and explicit Markdown
heading control.

## Region / Scoped Extraction

Coordinate convention: regions use PDF user-space points, with origin at the
page's bottom-left and `y` increasing upward. This is the same coordinate system
used by layout JSON, table boxes, field boxes, and image placements.

Overlap rule: an item is included when its center is inside the region or at
least half of its bounding box overlaps the region. Partly out-of-page regions
are clamped to the page box. Regions that do not overlap the page return a clean
error.

Rust:

```rust
use wellfriendpdf_engine::{ContentEngine, PageRegion};

let engine = ContentEngine::open_path("input.pdf")?;
let region = PageRegion::new(0.0, 396.0, 306.0, 792.0)?;
let text = engine.extract_text_in_region(1, region)?;
let words = engine.extract_words_in_region(1, region)?;
let tables = engine.extract_tables_in_region(1, region)?;
let images = engine.find_page_image_regions(1, region)?;
```

CLI:

```powershell
wellfriendpdf extract-text input.pdf --region 0,396,306,792
wellfriendpdf extract-tables input.pdf --region 0,396,306,792 --format json
wellfriendpdf extract-images input.pdf --region 0,396,306,792 -o images.zip
```

Python:

```python
import wellfriendpdf

doc = wellfriendpdf.open("input.pdf")
top_left = doc.page(1).region(0, 396, 306, 792)
print(top_left.text)
print(top_left.words)
print(top_left.tables)
print(top_left.images)
```

Image-region extraction currently filters placed image XObjects. Inline images
are still exported by full-page image extraction, but are omitted from
region-filtered image extraction because their placement boxes are not exposed
yet.

## Extraction Profiles

Profiles are named bundles over existing engine options, not a separate parser:

| Profile | Intent |
| --- | --- |
| `fast-text` | Preserve the default fast text/document behavior. |
| `layout-faithful` | Prefer layout-aware text ordering and keep parse output more faithful to page structure. |
| `tables-focused` | Use table-preserving document parsing defaults. |
| `rag-chunks` | Omit furniture and normalize searchable text for retrieval workflows. |

Rust:

```rust
use wellfriendpdf_engine::{ContentEngine, ExtractionProfile};

let engine = ContentEngine::open_path("input.pdf")?;
let text = engine.get_page_text_with_profile(1, ExtractionProfile::LayoutFaithful)?;
```

CLI:

```powershell
wellfriendpdf extract-text input.pdf --profile layout-faithful
wellfriendpdf parse input.pdf --profile rag-chunks --format markdown
```

Python:

```python
doc.extract_text(profile="layout-faithful")
doc.to_markdown(profile="rag-chunks")
doc.page(1).text_with_profile("layout-faithful")
```

## Markdown Headings

Markdown heading detection is now explicit. With heading detection enabled,
Wellfriend serializes the document model's heuristic title/heading blocks to Markdown
heading syntax. With heading detection disabled, Markdown output falls back to a
flat text-like export.

CLI:

```powershell
wellfriendpdf parse input.pdf --format markdown --detect-headings=true
wellfriendpdf parse input.pdf --format markdown --detect-headings=false
```

Python:

```python
doc.to_markdown(detect_headings=True)
doc.to_markdown(detect_headings=False)
doc.page(1).markdown(detect_headings=True)
```

The heading detector remains heuristic unless the PDF supplies tagged structure.
The capability matrix entry is therefore "yes, heuristic" rather than a claim of
semantic authoring-grade headings for every untagged PDF.
