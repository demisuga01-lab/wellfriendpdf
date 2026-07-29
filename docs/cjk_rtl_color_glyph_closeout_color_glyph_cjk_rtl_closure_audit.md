# CJK RTL Color Glyph Closeout Color Glyph And CJK/RTL Closure Audit

CJK RTL Color Glyph Closeout is a focused closure pass after roadmap closure 10. It does not
reopen parser, codec, transparency, shading, pattern, annotation, OCG,
progressive, tile, band, or cache campaigns except where those paths are direct
regression targets for color glyphs and text fixtures.

## Artifact Root

All CJK RTL Color Glyph Closeout evidence is written under:

```text
target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference
```

Primary artifacts:

- `cjk_rtl_color_glyph_closeout-closure-audit.json`
- `color-glyph-colr-cpal-matrix-cjk_rtl_color_glyph_closeout.json`
- `color-glyph-cbdt-cblc-matrix-cjk_rtl_color_glyph_closeout.json`
- `color-glyph-sbix-matrix-cjk_rtl_color_glyph_closeout.json`
- `color-glyph-svg-opentype-policy-cjk_rtl_color_glyph_closeout.json`
- `korean-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json`
- `hebrew-render-fixture-matrix-cjk_rtl_color_glyph_closeout.json`
- `cid-keyed-cff-clipping-matrix-cjk_rtl_color_glyph_closeout.json`
- `hinting-posture-cjk_rtl_color_glyph_closeout.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-render-results.json`
- `cjk_rtl_color_glyph_closeout-multi-reference-diff-metrics.json`
- `cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`
- `cjk_rtl_color_glyph_closeout-html-report/index.html`

## Running The Closure Harness

```powershell
python scripts/cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure.py --dpi 72 --timeout 120
```

The harness uses the Reference Renderer/Multilingual Color Glyphs target-local reference renderer
discipline. Poppler, PDFium, and MuPDF are required reference renderers for the
full audit; missing tools are not treated as a pass.

## Closure Status

The closure audit has no `partial_blocker` rows. Current rows are:

- COLR/CPAL v0 rendering: `implemented_and_proven`
- COLR/CPAL v1 posture: `unsupported_reported_exotic_case`
- CBDT/CBLC bitmap glyphs: `implemented_and_proven`
- sbix PNG glyphs: `implemented_and_proven`
- SVG-in-OpenType: `unsupported_reported_security_policy`
- Korean rendered-page fixture: `implemented_and_proven`
- Hebrew rendered-page fixture: `implemented_and_proven`
- CID-keyed CFF clipping: `unsupported_reported_exotic_case`
- optional native hinting posture: `implemented_and_proven`
- multi-reference audit: `implemented_and_proven`
- public feature report: `implemented_and_proven`

## Multi-Reference Result

CJK RTL Color Glyph Closeout currently audits five rendered fixture pages:

- COLR/CPAL vector color glyph fixture
- Korean embedded-font fixture
- Hebrew embedded-font fixture
- sbix PNG glyph fixture
- advanced CID-keyed CFF clipping fixture

The summary is recorded in
`cjk_rtl_color_glyph_closeout-reference-disagreement-summary.json`:

- Wellfriend outlier failures: `0`
- unclassified failures: `0`
- reference disagreement: sbix PNG, with Wellfriend inside the PDFium/MuPDF cluster
- unsupported exotic case: advanced CID-keyed CFF clipping geometry where a
  safe charstring clipping path is not exposed
