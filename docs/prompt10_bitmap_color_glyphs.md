# Prompt 10 Bitmap Color Glyphs

Prompt 10C preserves Prompt 10B bitmap color glyph rendering and narrows
non-PNG payload behavior to exact policy rows.

## CBDT/CBLC

Supported:

- PNG `RasterGlyphImage` payloads.
- bounded raw/grayscale/color bitmap strikes when the font parser exposes safe
  metadata.

Unsupported:

- ambiguous compressed payloads
- malformed strike tables
- oversized dimensions
- invalid offsets or lengths
- mismatched glyph strike references

## sbix

Supported:

- PNG strikes
- duplicate-glyph references that resolve to a PNG strike

Unsupported:

- JPEG
- TIFF
- PDF
- mask payloads
- unknown `graphicType` tags
- malformed or oversized strike records

Known unsupported bitmap payloads fail closed and use a color-glyph cache mode
that cannot alias with monochrome outline rendering.

Evidence:

- `color-glyph-bitmap-payload-matrix-prompt10c.json`
- `color-glyph-cbdt-cblc-results-prompt10c.json`
- `color-glyph-sbix-results-prompt10c.json`
