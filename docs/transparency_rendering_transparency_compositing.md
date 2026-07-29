# Transparency Rendering Transparency Compositing

Transparency Rendering introduced the native transparency foundation: bounded offscreen
surfaces, alpha state, blend modes, soft masks, isolated groups, knockout
groups, nested groups, and Poppler/PDFium/MuPDF comparison artifacts.

Transparency Closeout closes the remaining Transparency Rendering-owned items:

- `alpha_image` now applies graphics-state `/ca` to image XObject painting.
- Image soft-mask `/Matte` is unblended for common DeviceGray/RGB/CMYK matte
  values.
- ExtGState soft-mask `/BC` backdrop behavior remains implemented and measured.
- Luminosity masks are color-space-aware for DeviceGray, DeviceRGB, and
  DeviceCMYK through the current deterministic converter.
- Explicit DeviceGray/RGB/CMYK transparency-group color-space fixtures are
  recognized and measured.
- Interior knockout overlap for supported vector/Form groups uses the group
  initial backdrop instead of accumulated interior pixels.

Primary artifacts:

- `target/transparency_rendering-transparency-compositing/post-implementation-render-results.json`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-render-results.json`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-transparency-matrix.json`
- `target/transparency_rendering-transparency-compositing/transparency_closeout-html-report/index.html`

After Advanced Rendering, text clipping and common pattern/shading paints have native
coverage through the current clip/compositing pipeline. Remaining transparency
limits are advanced ICC/device-link/multicolor CMM and cropped-coordinate
offscreen surface exactness.
