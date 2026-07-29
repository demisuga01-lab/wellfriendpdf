# Multilingual Color Glyphs Bitmap Color Glyphs

Colrv Svg Bitmap preserves CJK RTL Color Glyph Closeout/10C bitmap color glyph rendering and implements
the safe non-PNG path where the existing decoder stack already supports it.

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
- JPEG strikes through the existing bounded DCT decoder
- duplicate-glyph references that resolve to a supported strike

Unsupported:

- TIFF when no existing safe TIFF decoder is available
- PDF
- mask payloads
- unknown `graphicType` tags
- malformed or oversized strike records

Known unsupported bitmap payloads fail closed and use a color-glyph cache mode
that cannot alias with monochrome outline rendering.

Evidence:

- `color-glyph-bitmap-payload-matrix-color_glyph_hinting.json`
- `color-glyph-cbdt-cblc-results-color_glyph_hinting.json`
- `color-glyph-sbix-results-color_glyph_hinting.json`
- `bitmap-color-glyph-nonpng-matrix-colrv_svg_bitmap.json`
- `cbdt-cblc-nonpng-results-colrv_svg_bitmap.json`
- `sbix-nonpng-results-colrv_svg_bitmap.json`
