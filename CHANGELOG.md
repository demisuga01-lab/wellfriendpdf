# Changelog

All notable changes to Wellfriend are recorded here. The project follows the
stability policy in `docs/stability.md`.

## Unreleased

- XFA form scripts run before an XFA form is drawn, as in PDFium:
  `initialize`, `calculate` (re-run when their inputs change), form:ready,
  `docReady` and layout:ready (per page for master-page fields, so
  `xfa.layout.page(this)` numbers pages). FormCalc is a complete
  interpreter (control flow, functions, SOM accessors and its whole
  built-in library except the URL functions); JavaScript runs on the
  pure-Rust boa engine (new default `xfa-js` feature; not on wasm32) with
  form objects exposed through the XFA scripting object model. Scripts set
  values, presence, captions, choice items, colours and repeating-subform
  instances. Scripts have no network, file or host access and run within a
  per-document time and instruction budget; `ContentEngine::xfa_script_log`
  and `xfa_script_errors` report their messages and failures.
- XFA form scripts see the data DOM: `$data`, `$record`, `!name` and
  `xfa.datasets` reach the datasets packet's data groups and values from
  FormCalc and JavaScript, fields expose their bound `dataNode`, and writes
  stay in sync between bound fields and data values (PDFium's data DOM and
  `SyncValue`). Named containers without data get data nodes, as PDFium's
  data merge creates them.
- XFA validation runs without a user interface: validate scripts,
  `nullTest` and `formatTest` (`<validate><picture>`) run after the
  calculations and again for values changed later; failures are recorded
  in `ContentEngine::xfa_validation_results` (SOM expression, test,
  severity, message) instead of being shown. `execValidate` returns the
  outcome and `xfa.host.validationsEnabled` turns validation off.
- XFA barcode fields draw their symbols: Code 39, Code 128, Codabar, EAN-8,
  EAN-13, UPC-A, QR Code, PDF417 and Data Matrix. EAN and UPC human-readable
  digits are split around the full-length guard bars as PDFium lays them
  out.
- XFA dynamic layout places overflow and bookend leaders and trailers,
  break leaders/trailers and overflow/break targets for specific content
  areas, and keeps `keep next`/`keep previous` chains together (up to 512
  siblings). When a nested subform splits, its ancestors place their
  overflow leaders and trailers too.
- XFA picture clauses and default number/date formats follow the field's
  locale: the document's `localeSet` and PDFium's 15 built-in locales
  (de_DE, fr_FR, ja_JP, zh_CN, ...), with `date.long{}`-style locale
  patterns and `date(fr_FR){...}` locale overrides.
- Native CMYK raster output (Poppler `splashModeCMYK8`, `pdftoppm
  -jpegcmyk`): `ContentEngine::render_page_cmyk_with_contract` and
  `RenderContract::pixel_format = PixelFormat::Cmyk8` composite the page in
  CMYK instead of converting the RGB render. The page is rendered in plate
  passes (C/M/Y together, then K; one plate per pass when an overprinting
  paint names only some plates of a pass) through the normal compositor with
  every colour replaced by inverted plate ink, so coverage, clipping,
  constant and soft-mask alpha, groups and the separable blend modes
  composite each plate exactly (separable blends on complemented values are
  the PDF rule for subtractive spaces). DeviceCMYK fills, strokes, text,
  images (raw, DCT, JPX, Indexed, ICC N=4) and shadings keep their exact
  components; DeviceGray is black ink; RGB, ICC N=3, Lab and Cal spaces
  convert with Poppler's uncalibrated undercolour-removal formula (ICC
  through the CMM to sRGB first); Separation/DeviceN colorants named
  Cyan/Magenta/Yellow/Black paint their plates and spot colours go through
  their alternate and tint transform. Overprint (`OP`/`op`/`OPM`) is
  honoured natively per plate, unless `OverprintPolicy::Disabled`: a spot
  mapped through its alternate overprints the plates its full-tint
  alternate inks. Luminosity soft masks are still computed from RGB. Pages
  using what the plate compositor lacks — non-separable blend modes,
  transfer functions, colour-font (COLR/SVG/bitmap) glyphs, forced colour
  schemes, form-field highlights, linear-light compositing, proof/halftone/
  colour-scheme post-processing — fall back to RGB render plus conversion,
  reported in `CmykRaster::native` / `diagnostics`. The RGB render is
  unchanged.
- DeviceN plate output (Poppler `splashModeDeviceN8`):
  `render_page_separations_with_contract_and_cache` returns one 8-bit ink
  plate per colorant: the four process plates plus up to `max_spots` (at
  most 32) spot colours the page paints, found with the separation
  framebuffer. A separated spot paints only its own plate; further spots map
  to CMYK through their alternates.
- CLI: `render --cmyk` (jpg, tiff) now composites natively; `--cmyk-method
  convert` keeps the RGB conversion. `render --separations [--max-spots N]`
  writes one grey image per plate (`page-001-Cyan.png`, ...,
  `page-001-PANTONE_123_C.png`; full ink is black) for png, tiff and pnm.

- Damaged images keep more of their content. CCITT Group 3 streams
  resynchronise on the next end-of-line marker after a damaged row, as in
  Poppler, instead of dropping every later row. JBIG2 streams with a damaged
  middle segment leave out that segment (and segments referring to it) and
  still compose the other regions, instead of keeping only the segments
  before the damage.
- Colour-key `/Mask` arrays now apply to DCT and JPX images in colour spaces
  other than DeviceGray/DeviceRGB (CMYK, ICCBased, CalGray, Indexed JPX):
  the key is compared against the codec's decoded samples, before `/Decode`,
  as Poppler and PDFium do. Such images were drawn unmasked before.
- Transfer functions (`/TR`) on tiling patterns apply to the composited
  pixels of the filled area, like Poppler's Splash, instead of to each paint
  inside the pattern cell; soft-mask groups render without the masked
  object's transfer.
- Anti-aliased dashed strokes render about 25% faster (1,500 page-wide
  dashed rules: 103 ms to 77 ms), with identical pixels.
- Annotations are drawn by the render state that drew the page instead of a
  second one; output is unchanged.

- XFA forms render like PDFium. Static forms get their XFA fields (merged
  datasets values, captions, fonts, para alignment, borders with edges,
  corners and 3-D strokes, solid/linear/radial/pattern fills, check marks,
  list and combo boxes, comb and multi-line edits, images, picture-clause
  formatting) drawn over the page when its widgets have no appearances, or
  always with the new `RenderContract::xfa = XfaRenderPolicy::Foreground`.
  Dynamic forms lay out from their page sets with flowing subforms, breaks
  and overflow; `ContentEngine::xfa_page_count`, `render_xfa_page` and
  `xfa_layout_pdf` expose those pages, which may outnumber the PDF's own.
  PDFs without XFA render exactly as before.
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
