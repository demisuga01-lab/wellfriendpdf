# Prompt 10 CJK/RTL/Color Glyph Reference Harness

Prompt 10 extends the renderer parity campaign into complex text rasterization
and direct reference-renderer harnessing. It does not change the PDF imaging
model: existing page content streams are painted from encoded PDF glyph codes,
CMap/CID/GID mapping, font data, text matrices, and explicit positioning.
Unicode shaping is used only when Wellfriend owns text generation or fallback text
layout.

## Artifact Root

All Prompt 10 artifacts are written under:

```text
target/prompt10-cjk-rtl-color-glyph-reference
```

Important outputs:

- `reference-tool-manifest-prompt10.json`: Poppler, PDFium, and MuPDF discovery,
  versions, checksum posture, command templates, DPI, timeout, and output format.
- `corpus-manifest-prompt10.json`: available CJK/RTL fixtures plus explicit
  policy rows for missing or unsupported color-glyph cases.
- `prompt10-capability-matrix.json`: CJK, RTL, color glyph, PDFium, and MuPDF
  status matrix.
- `multi-reference-render-results-prompt10.json`: per-page Wellfriend/Poppler/PDFium/
  MuPDF render commands, artifacts, and raw classifications.
- `multi-reference-diff-metrics-prompt10.json`: per-page pairwise image metrics.
- `reference-disagreement-summary-prompt10.json`: reference disagreement and
  Wellfriend outlier summary.
- `html-report/index.html`: browsable summary table.
- `public-feature-report-prompt10.json`: feature report captured through the CLI.
- `binding-report-parity-prompt10.json`: shared public report surface mapping.

## Running The Harness

Build the CLI first when no existing `target/debug/wellfriendpdf.exe` is present:

```powershell
cargo build -p wellfriendpdf-cli
```

Run the full Prompt 10 harness:

```powershell
python scripts/prompt10_cjk_rtl_color_glyph_reference_harness.py --dpi 72 --timeout 120
```

Useful development options:

```powershell
python scripts/prompt10_cjk_rtl_color_glyph_reference_harness.py --skip-render
python scripts/prompt10_cjk_rtl_color_glyph_reference_harness.py --limit 3
```

The full harness requires all three reference renderers. It reuses the Prompt
06B target-local bootstrap policy through
`scripts/prompt06b_bootstrap_reference_renderers.ps1`, with Prompt 10-specific
tool and artifact paths. Missing Poppler, PDFium, or MuPDF is a failed Prompt
10 audit, not a silent skip.

## CJK Raster And Hinting Posture

The Prompt 10 corpus includes real-safe fixtures for Type0/CID fonts,
predefined CMaps, Identity-H/Identity-V, vertical Japanese, mixed Latin/CJK,
CIDToGID-related paths, ToUnicode independence, malformed CMap handling, and
CJK text clipping regressions.

Visual rendering remains independent from extraction-only ToUnicode behavior.
ToUnicode helps text extraction and accessibility, but glyph painting follows
the PDF font program and text-state path. Missing CMap data, unsupported
predefined CMaps, missing descendants, vertical CMap use, and missing Type0
ToUnicode are reported through `fonts_report`.

The default raster posture remains pure Rust with analytic/light grid-fitting
behavior. No native hinting or font engine dependency is enabled silently.

## RTL Shaping Boundary

Arabic and Hebrew generated text paths use the rustybuzz complex-script
boundary when the engine owns Unicode-to-glyph layout. Existing PDF content
streams are not reshaped blindly, because many PDFs already contain positioned
or pre-shaped glyph streams. The harness keeps Arabic page-content fixtures
separate from generated/fallback text posture.

## Color Glyph Posture

The font report detects OpenType color glyph tables:

- `COLR`
- `CPAL`
- `CBDT`
- `CBLC`
- `sbix`
- `SVG`

Prompt 10B supersedes the original detection-only posture. The renderer now
supports the bounded safe/common paths documented in
`docs/prompt10_color_glyph_rendering.md`: COLR/CPAL v0 solid layers and bounded
embedded bitmap color glyph payloads, including sbix PNG fixtures. Detected
color glyph tables are exposed through:

- `color_font_tables`
- `color_glyph_status`
- `color_glyph_supported_tables`
- `color_glyph_unsupported_tables`
- format-specific diagnostics

Unsupported formats are reported precisely rather than silently replaced.
COLR/CPAL v1 complex paint graphs remain an exotic unsupported case, malformed
or unsafe bitmap payloads fail closed, and SVG-in-OpenType is blocked by
security policy. SVG glyph documents are not executed, and external references
are not dereferenced.

## Public Surfaces

Prompt 10 status is exposed through the shared SDK feature report facade, so the
same additive section is visible to Rust SDK, CLI, Python, C ABI, WASM, .NET,
Java Maven, and Java Gradle surfaces.

The additive feature section is:

```text
prompt10_cjk_rtl_color_glyph_reference_harness
```

## Known Bounded Limits

- COLR/CPAL v1 complex paint graphs remain unsupported unless they fit the
  supported solid-layer path.
- SVG-in-OpenType remains blocked by security policy.
- Complex CID-keyed CFF geometry under real-world text clipping is classified as
  a bounded unsupported edge when it falls outside the reference cluster.
- Existing PDF glyph painting is not reshaped as authoring-time text.
- Native hinting remains outside the default dependency boundary.
- Prompt 10B closes the Korean and Hebrew visual page fixture gaps.
