# Prompt 10B Color Glyph And CJK/RTL Closure Audit

Prompt 10B is a focused closure pass after Combined Prompt 10. It does not
reopen parser, codec, transparency, shading, pattern, annotation, OCG,
progressive, tile, band, or cache campaigns except where those paths are direct
regression targets for color glyphs and text fixtures.

## Artifact Root

All Prompt 10B evidence is written under:

```text
target/prompt10-cjk-rtl-color-glyph-reference
```

Primary artifacts:

- `prompt10b-closure-audit.json`
- `color-glyph-colr-cpal-matrix-prompt10b.json`
- `color-glyph-cbdt-cblc-matrix-prompt10b.json`
- `color-glyph-sbix-matrix-prompt10b.json`
- `color-glyph-svg-opentype-policy-prompt10b.json`
- `korean-render-fixture-matrix-prompt10b.json`
- `hebrew-render-fixture-matrix-prompt10b.json`
- `cid-keyed-cff-clipping-matrix-prompt10b.json`
- `hinting-posture-prompt10b.json`
- `prompt10b-multi-reference-render-results.json`
- `prompt10b-multi-reference-diff-metrics.json`
- `prompt10b-reference-disagreement-summary.json`
- `prompt10b-html-report/index.html`

## Running The Closure Harness

```powershell
python scripts/prompt10b_color_glyph_cjk_rtl_closure.py --dpi 72 --timeout 120
```

The harness uses the Prompt 06B/Prompt 10 target-local reference renderer
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

Prompt 10B currently audits five rendered fixture pages:

- COLR/CPAL vector color glyph fixture
- Korean embedded-font fixture
- Hebrew embedded-font fixture
- sbix PNG glyph fixture
- advanced CID-keyed CFF clipping fixture

The summary is recorded in
`prompt10b-reference-disagreement-summary.json`:

- Oxide outlier failures: `0`
- unclassified failures: `0`
- reference disagreement: sbix PNG, with Oxide inside the PDFium/MuPDF cluster
- unsupported exotic case: advanced CID-keyed CFF clipping geometry where a
  safe charstring clipping path is not exposed
