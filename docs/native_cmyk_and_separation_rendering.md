# Native CMYK and separation (DeviceN) rendering

Wellfriend can composite a page in CMYK, like Poppler's `splashModeCMYK8`
(`pdftoppm -jpegcmyk`), and can write one 8-bit plate per colorant, like
Poppler's `splashModeDeviceN8`. PDFium has neither. The RGB renderer is
unchanged by this work; its output stays byte-identical.

## API

| Entry point | Output |
| --- | --- |
| `ContentEngine::render_page_cmyk_with_contract(_and_cache)` | `CmykRaster`: interleaved C, M, Y, K ink bytes (0 = no ink), `native`, `diagnostics` |
| `RenderContract::pixel_format = PixelFormat::Cmyk8` with `render_page_into_buffer` | CMYK rows at the contract stride |
| `ContentEngine::render_page_separations_with_contract_and_cache(.., max_spots, ..)` | `SeparationRaster`: Cyan, Magenta, Yellow, Black, then each separated spot plate |
| CLI `render --cmyk` (jpg: Adobe CMYK, tiff: Separated) | native CMYK; `--cmyk-method convert` keeps the old RGB-then-convert path |
| CLI `render --separations [--max-spots N]` (png, tiff, pnm) | `page-NNN-<colorant>.<ext>`, 8-bit grey, full ink black |

The contract's geometry, page box, optional content, annotations, forms,
rendering intent, colour management and overprint policy apply. Its pixel
format and background do not: CMYK output is opaque, on blank paper. The CLI
keeps `--overprint disabled` as default (as pdftoppm without `-overprint`);
pass `--overprint preview` to honour overprint.

## How it composites

A *plate pass* renders the page through the ordinary compositor with every
paint colour replaced by the inverted ink of up to three plates, one per
R/G/B channel (`255 - ink`; paper = 255). Coverage and anti-aliasing,
clipping, constant alpha, soft-mask alpha, isolated and knockout groups and
the separable blend modes are per-channel operations, so each plate
composites exactly as in a planar CMYK/DeviceN compositor. Separable blend
modes on complemented values are the PDF rule for subtractive spaces
(PDF 32000-1 11.3.5), so `Multiply`, `Screen`, `Darken`, ... are exact.

CMYK pages take two passes (C/M/Y, then K). A separation render takes
`ceil((4 + spots) / 3)` passes. When an overprinting paint names only some of
the plates of a pass (OPM 1 with a zero component, or a spot overprinting the
process plates), the page is re-rendered with one plate per pass.

Colour to ink:

| Source | Ink |
| --- | --- |
| DeviceCMYK, ICCBased with N = 4 (paint, raw/DCT/JPX images, shadings) | components unchanged (`round(c * 255)`, as Poppler `colToByte`) |
| DeviceGray, ICCBased with N = 1 | black only: `K = 1 - gray` |
| DeviceRGB | Poppler's uncalibrated conversion: `c = 1 - r, m = 1 - g, y = 1 - b, k = min(c, m, y)`, then `c, m, y -= k` |
| ICCBased N = 3, Lab, CalRGB, CalGray | through the CMM (qcms or lcms2) to sRGB, then the formula above |
| Indexed | the palette entry in its base space (Lab/Cal bases via sRGB) |
| Separation/DeviceN colorant named Cyan, Magenta, Yellow or Black | that process plate directly |
| Separation/DeviceN spot that is separated (`--separations`) | its own plate only |
| Other spots | tint transform to the alternate space, then that space's ink |
| `/All` | every plate at the tint; `/None`: nothing |

There is no CMM-based RGB to CMYK conversion (no output-intent CMYK profile
is used); Poppler without a CMS uses the same formula.

Overprint (`OP`, `op`, `OPM`) is honoured per plate unless the contract's
overprint policy is `Disabled`: a paint leaves untouched every plate its
colour space does not name. Process spaces name the four process plates
(with OPM 1, a DeviceCMYK component of 0 names nothing); a separated spot
names its own plate; a spot mapped through its alternate names the plates
its alternate colour inks at full tint (Poppler's overprint mask). Images
and shadings overprint as a whole by their colour space. Luminosity soft
masks are still computed from the mask group's RGB rendering.

## Fallback

Pages that use something the plate compositor cannot reproduce are rendered
in RGB and converted with the formula above; `CmykRaster::native` is
`false`, `diagnostics` names the feature and the CLI prints a warning:

- non-separable blend modes (Hue, Saturation, Color, Luminosity), detected
  when an ExtGState selects them;
- transfer functions (`/TR`, `/TR2`);
- colour-font glyphs (COLR/CPAL, SVG, bitmap emoji);
- forced colour schemes and form-field highlights;
- linear-light (`high`) compositing;
- proof print profile, halftone screen and colour-scheme post-processing.

A separation render that falls back fills the process plates from the
converted RGB render and leaves spot plates empty.

## Known limits

- Retained vector fills in DeviceGray/DeviceRGB carry an 8-bit RGB colour,
  so their ink can be one level off Poppler on half-way values (gray 0.5 is
  127 instead of 128 units of black). DeviceCMYK fills are exact.
- Transparency groups composite in the plate space; a group `/CS` is not
  honoured (Poppler's CMYK mode does the same).
- `/OPM 1` applies to DeviceCMYK paint only, not to images.
- Spot plates come from a report-only separation-framebuffer pass at 9 dpi
  that records the colorants the page paints; at most 32 spots are
  separated, further spots map to CMYK.

## Verification (2026-11)

- `render::plate` unit and end-to-end tests: exact DeviceCMYK paint, Gray and
  RGB conversion, OPM 1 and spot overprint per plate, knockout without
  overprint, constant alpha, Multiply, CMYK images and shadings, the
  non-separable blend fallback, separation plates.
- Synthetic page against `pdftoppm -jpegcmyk -overprint`: every probe point
  within JPEG noise (DeviceCMYK fill 51/102/0/26, OPM 1 overlap 255/255/0/0,
  spot overprint on cyan 255/61/240/0, CMYK image and shading values).
- tests/corpus/pdfs + engine fixtures (629 pages at 50 dpi): all render
  natively except one page with a `Color` blend mode (fallback as designed).
  Differences to Poppler's CMYK output track the RGB renderer's own
  differences to Poppler on the same pages.
- RGB output of tests/corpus/pdfs (596 pages at 100 dpi) is byte-identical to
  main.
