# Prompt 10 SVG Color Glyph Security

SVG-in-OpenType remains a security-sensitive color glyph format. Prompt 10C
adds a static-subset classifier but does not execute a general SVG engine.

## Static Candidates

The classifier admits no-execution static candidates such as:

- `<svg>` root with bounded document size.
- `<g>` grouping with bounded depth.
- `<path>` data within the path command cap.
- simple shape and finite transform candidates for future primitive mapping.

These rows are classified as static subset candidates, not rendered by a general
SVG interpreter.

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
- filters
- masks
- path/depth bombs
- recursive references

Blocked SVG glyphs do not execute, dereference network/file resources, or fall
back silently to an unreported color glyph approximation.

Evidence:

- `color-glyph-svg-static-subset-matrix-prompt10c.json`
- `color-glyph-svg-security-policy-prompt10c.json`
- `color-glyph-svg-reference-results-prompt10c.json`
