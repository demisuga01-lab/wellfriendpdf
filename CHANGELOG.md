# Changelog

All notable changes to Wellfriend are recorded here. The project follows the
stability policy in `docs/stability.md`.

## Unreleased

- DOCX and EPUB embed each distinct image once, for its largest placement:
  baseline JPEGs are copied, PNG-predicted Flate images are wrapped as PNG
  without inflating, the rest are decoded (at reduced resolution when far
  larger than needed at 200 dpi) and written as JPEG or PNG. EPUB images are
  no longer all full-resolution PNG. Image placements come from the content
  the conversion pass already parsed.
- `pdf-to-json` serializes the large arrays in parallel; the bytes are
  unchanged.
- With `--threads` 2 or fewer the CLI keeps memory near its live data
  (one glibc arena per worker, large buffers mapped, freed heap trimmed);
  allocator tunables set by the environment are respected.
- `watermark --incremental`, `add-page-numbers --incremental` and the new
  `page-labels` command append incremental updates. Deleting or reordering
  pages stays a full rewrite.
- Conversions to Markdown, JSON, DOCX, XLSX and EPUB run page-parallel in one
  process and read each page's content once: classification, layout, marked
  text and the tagged route share a single parse. Pages are assembled in page
  order, so the output is byte-identical for any thread count. A global
  `--threads N` sizes the worker pool (default and `0`: every core).
  `pdf-to-docx` and `pdf-to-xlsx` take `-p/--pages`.
- A structure tree that yields no content for the selected pages (for example
  an empty `Document` element) no longer produces an empty conversion: the
  pages are laid out geometrically, as for an untagged file.
- The per-page content operation limit scales with the page's stored content
  size (one operation per stored byte, floor 1,000,000, ceiling 8,000,000), so
  dense map pages parse while decompression bombs keep the floor. The page
  pass bounds the stored content it expands at once.
- The content tokenizer reads a page's decoded streams from one buffer and
  scans names, operators and numbers in place; numbers parse exactly as
  `str::parse` does without its general machinery.
- JSON results of the conversions and of `rotate`, `organize`, `merge`,
  `extract-pages`, `watermark`, `add-page-numbers`, `optimize`, `repair`,
  `linearize`, `encrypt` and `decrypt` carry `output_sha256` (computed while
  writing) and `output_pages`; the `pdf-to-epub` result carries `sha256`.
- `rotate --incremental` appends the changed page dictionaries as an
  incremental update after an unchanged copy of the input and reports
  `incremental`, `baseLength` and `deltaLength`.
- `pdf-to-epub` opens its input in place instead of reading it into memory
  twice.
- Added Rust-native conversions to PDF: `text-to-pdf`, `markdown-to-pdf`
  (CommonMark + GFM), `csv-to-pdf` (repeated headers, column cap error instead
  of truncation), `json-to-pdf` and `svg-to-pdf` (vector, via `svg2pdf`), on
  the CLI, HTTP server and C API.
- Added `pdf-to-epub` (EPUB 3 with navigation, chapters and images) and
  `edit-metadata` (Info and XMP kept in sync, batch mode, incremental update
  that keeps existing signatures valid).
- Added native PDF/A conversion (`pdfa-convert`: PDF/A-1b, 2b, 2u, 3b, 3u)
  with font substitution and subsetting that preserves text positions, an
  OutputIntent, XMP, forbidden-feature removal, and a gate that returns a file
  only when the built-in PDF/A validator passes it. `pdfa-validate` gains a
  JSON `--input/--options` form; the old form keeps working.
- The CLI conversion commands print one JSON result line
  (`{"ok":true,"pages","bytes"}` or `{"ok":false,"code","message"}`).
- Windows builds no longer need the `sha2` assembly feature; it stays enabled
  on other targets. The Tesseract crate is no longer a default workspace member.
- Added the main CI gate, scheduled/manual sanitizer gate, and tag-driven
  release pipeline with checksummed CLI artifacts and crate publish dry-runs.
- Added the documented public API overview and stability policy.
- Added `ErrorKind` plus `WellfriendError::kind()`, `WellfriendError::code()`, and
  `WellfriendError::is_input_error()` for programmatic error handling.
- Added RSA/SHA-256 PDF signing through incremental update and the
  `sign_document` example.
