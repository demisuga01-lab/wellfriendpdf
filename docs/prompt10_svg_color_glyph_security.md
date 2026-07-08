# Prompt 10 SVG Color Glyph Security

SVG-in-OpenType remains a security-sensitive color glyph format. Prompt 10D
renders a narrow static subset inside Oxide's path renderer and continues to
block active or dynamic SVG behavior.

## Rendered Static Subset

The renderer admits:

- `<svg>` root metadata with bounded document size.
- `<g>` grouping with bounded depth.
- `<path>` commands `M`, `L`, `H`, `V`, `C`, `Q`, and `Z`.
- `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, and `<polygon>`.
- quoted fill/stroke/stroke-width and opacity attributes.
- finite `matrix`, `translate`, `scale`, `rotate`, `skewX`, and `skewY`
  transforms.

These rows are parsed without scripts, CSS execution, external fetches, or a
general SVG interpreter.

## Blocked Features

The renderer blocks:

- `<script>`
- event attributes
- `javascript:`, `file:`, `http:`, and `https:` URLs
- `<foreignObject>`
- animation elements
- CSS imports
- remote or embedded SVG fonts
- external images
- CSS style blocks
- filters
- masks
- URL paint-server references
- path/depth bombs
- recursive references

Blocked SVG glyphs do not execute, dereference network/file resources, or fall
back silently to an unreported color glyph approximation.

Evidence:

- `color-glyph-svg-static-subset-matrix-prompt10c.json`
- `color-glyph-svg-security-policy-prompt10c.json`
- `color-glyph-svg-reference-results-prompt10c.json`
- `svg-opentype-static-rendering-matrix-prompt10d.json`
- `svg-opentype-security-policy-prompt10d.json`
- `svg-opentype-reference-results-prompt10d.json`
