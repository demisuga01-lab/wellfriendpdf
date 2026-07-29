# Multilingual Color Glyphs SVG Color Glyph Rendering

Colrv Svg Bitmap adds a safe static SVG-in-OpenType renderer. It is intentionally not
a browser, webview, external process, or general SVG engine.

## Rendered Static Subset

The renderer accepts:

- `<svg>` root metadata, including `viewBox`.
- `<g>` grouping with inherited static styles and transforms.
- `<path>` commands `M`, `L`, `H`, `V`, `C`, `Q`, and `Z`.
- `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, and `<polygon>`.
- `fill`, `stroke`, `stroke-width`, `opacity`, `fill-opacity`, and
  `stroke-opacity`.
- finite `matrix`, `translate`, `scale`, `rotate`, `skewX`, and `skewY`
  transforms.
- gzip-compressed SVGZ documents under the static byte cap.

The parsed geometry is routed through Wellfriend's existing path painter with the
current glyph transform, text matrix, CTM, graphics-state alpha, and clipping
state.

## Security Blocking

The renderer blocks scripts, event attributes, animation, `foreignObject`,
external images, network/file/javascript URLs, remote fonts, CSS blocks/imports,
filters, masks, recursive `<use>`, URL paint servers, and path/depth bombs.

Blocked SVG glyphs fail closed with diagnostics and do not silently fall back to
monochrome outline rendering.

## Current Static Limits

SVG gradients, `clipPath`, filters, masks, CSS styling blocks, `<use>`, and URL
paint servers remain exact unsupported/security rows. They are not broad
"SVG unsupported" cases.
