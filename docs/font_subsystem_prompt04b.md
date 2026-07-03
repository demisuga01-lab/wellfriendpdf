# Prompt 04B Font Fidelity Closure

Prompt 04B closes the font-foundation leftovers that can be handled safely in
the current pure-Rust engine, and makes the remaining boundaries explicit.

## Closure Matrix

| Area | Outcome | Evidence |
|---|---|---|
| FreeType-grade hinting | Outcome B/C: keep pure Rust because `oxide-engine` forbids unsafe code; use existing analytic coverage plus bounded light grid fitting for TrueType outlines | `crates/engine/src/lib.rs` has `#![forbid(unsafe_code)]`; glyph tests cover deterministic cache and hint policy |
| HarfBuzz-style generated output | Outcome B: rustybuzz remains the shaping backend and authoring now consumes shaped glyph IDs for complex scripts | `FontBuildPlan` stores shaped CID entries, `/ActualText`, CIDToGIDMap, `/W`, and ToUnicode cluster entries |
| True font subsetting | Outcome B/C: Type0 CID subset maps are generated, but sfnt glyph-program rewriting remains disabled until a safe subset writer is added | authoring emits used-CID `/W`, CIDToGIDMap, ToUnicode, and full font program fallback with `font.subset.sfnt_deferred` diagnostics |
| Predefined CJK CMaps | Outcome B: bounded common UTF-16 predefined CMaps are classified and treated as two-byte Unicode-preserving CMaps | `Identity-H/V`, `UniJIS-UTF16-H/V`, `UniGB-UTF16-H/V`, `UniCNS-UTF16-H/V`, and `UniKS-UTF16-H/V` |
| Vertical writing | Outcome B: vertical mode flows through shared text decode and raster/SVG/PostScript text advancement uses `/W2` or default vertical displacement | `DecodedGlyph` now carries vertical metrics; renderers advance on Y for vertical Type0 text |
| Color glyphs | Outcome C: detect COLR/CPAL/CBDT/CBLC/sbix/SVG tables and report monochrome-outline fallback | `font.color_glyphs.detected` diagnostic; full color-font rendering belongs with later color/image work |

## Implemented Code Paths

- `crates/engine/src/fonts/predefined_cmap.rs` adds compact predefined-CMap
  metadata and tests.
- `crates/engine/src/fonts/resolver.rs` uses supported predefined CMap names to
  choose code size, writing mode, and Unicode-preserving fallback decoding.
- `crates/engine/src/render/text_decode.rs` carries vertical advance/origin
  metadata in `DecodedGlyph`.
- `crates/engine/src/render/page_renderer.rs`, `svg.rs`, and `postscript.rs`
  advance vertical Type0 text on the Y axis.
- `crates/engine/src/authoring.rs` plans shaped glyph CIDs for complex-script
  generated text, emits shaped CID streams, writes `/ActualText`, and maps each
  shaped CID through ToUnicode and CIDToGIDMap.
- `crates/engine/src/fonts/provider.rs` honors explicit bold/italic style hints
  against bundled fallback faces before declaring synthetic styling.
- `crates/engine/src/fonts_report.rs` exposes predefined CMap, color-font,
  rasterization, and embedding-policy fields in JSON reports.

## Shaped Generated PDF Output

The generated-output path is now:

Unicode input -> selected embedded font bytes -> `TextShaper`/rustybuzz ->
shaped glyph IDs and clusters -> stable CIDs -> Type0/CIDFontType2 content
stream -> CIDToGIDMap -> ToUnicode cluster map -> `/ActualText` logical text.

Latin/simple text still uses the existing scalar CID path for deterministic
compatibility. Complex shaping activates only when the selected font produces
nonzero shaped glyph IDs; otherwise the writer falls back to scalar CID output
instead of embedding unusable `.notdef` glyphs.

Arabic generated-output coverage is tested with bundled DejaVu Sans. Indic
shaping remains backend-ready but needs a deterministic bundled Indic font or a
user-provided registered font to be production-proven.

## Subsetting Boundary

Prompt 04B does not rewrite sfnt `glyf`/`loca`/`cmap`/`hmtx`/`maxp`/`head`
tables. That is a deliberate safety boundary: a partial table-rewriter can
produce PDFs that open in a smoke test but corrupt composite glyphs, checksums,
or cmap-dependent consumers. Current generated output is therefore:

- subset CID maps: implemented;
- subset `/W`: implemented;
- subset ToUnicode: implemented;
- subset CIDToGIDMap: implemented;
- full font program fallback: still used;
- true sfnt glyph-program subsetting: bounded 04C candidate.

Reports surface this as `font.subset.sfnt_deferred` rather than hiding it.

## Predefined CMap Coverage

Supported built-in metadata:

- `Identity-H`, `Identity-V`
- `UniJIS-UTF16-H`, `UniJIS-UTF16-V`
- `UniGB-UTF16-H`, `UniGB-UTF16-V`
- `UniCNS-UTF16-H`, `UniCNS-UTF16-V`
- `UniKS-UTF16-H`, `UniKS-UTF16-V`

Legacy maps such as `90ms-RKSJ-H` are detected as predefined-looking but
unsupported unless an embedded CMap or ToUnicode map supplies the real mapping.
Reports emit `font.cmap.predefined.unsupported` for that case.

## Color Glyph Posture

OpenType color tables are detected from the sfnt table directory:

- `COLR`
- `CPAL`
- `CBDT`
- `CBLC`
- `sbix`
- `SVG`

The renderer uses monochrome outline fallback today. Full layered/vector/bitmap
color glyph rendering is not implemented in Prompt 04B because it intersects
the Prompt 05 color pipeline and image compositing.

## Benchmark Baseline

Prompt 04 baseline on the text/font Poppler slice:

- files: 24
- visual pass: 45.83%
- weighted score: 45.21
- determinism: 5/5
- peak Oxide memory: 11.72 MB

Prompt 04B reran the same capped slice after rebuilding `target\release\oxide.exe`:

- files: 24
- visual pass: 45.83%
- weighted score: 45.21
- determinism: 5/5
- peak Oxide memory: 11.28 MB
- artifact path: `target/prompt04b-font-render-benchmark/`

This is not a visual-fidelity improvement over Prompt 04. The code changes close
authoring, reporting, CMap, vertical-advance, and diagnostic gaps, but the
remaining Poppler slice failures are still dominated by renderer/rasterizer and
font-substitution differences. The `real_pdfjs_vertical` artifact is also a good
example of why the score cannot be chased blindly: Oxide renders visible vertical
text while the Poppler reference image for that fixture is effectively blank.

## Known Limits

- Native FreeType bytecode hinting and native HarfBuzz bindings are not linked
  into `oxide-engine`; the engine crate remains safe Rust only.
- True sfnt glyph-program subsetting remains a named implementation campaign.
- Predefined legacy CJK CMap packs are not vendored.
- Indic generated-output shaping requires a deterministic registered/bundled
  Indic font to prove end-to-end output.
- Full vertical glyph substitution and vertical punctuation alternates are not
  implemented.
- Color glyph rendering is detected and diagnosed, not rendered in color.
