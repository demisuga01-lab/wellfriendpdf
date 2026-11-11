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
process plates), the page is re-rendered with one plate per pass, unless the
page also uses a non-separable blend mode (below).

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

Spot overprint in a CMYK render: `pdftoppm -jpegcmyk` / `-overprint`
composite in `splashModeDeviceN8`, where the first four spots the page uses
(`SPOT_NCOMPS`) get their own plates, and on output
`SplashBitmap::getCMYKLine` converts each spot plate through its alternate
and *adds* it to the process plates, clamped (`dblToByte(clip01(c + ...))`).
So a spot overprinting a backdrop that already carries ink in the same
process plates adds to it (magenta 102 + 61 = 163) rather than replacing
it. When overprint is honoured and the page paints spots, the CMYK render
does the same: it renders the first four spots as separate plates (the
separation compositor above, overprint per plate), records each spot's
process ink at every tint byte (the alternate through the tint transform,
at Poppler's `byteToCol` tints; a DeviceN colorant uses its `/Colorants`
Separation, else the DeviceN alternate with that colorant alone), and
flattens the plates additively with Poppler's arithmetic. Further spots map
through their alternates as before. Without overprint the render is
unchanged (spots knock out, which the additive flattening would reproduce
anyway). The RGB overprint preview does not simulate spot overprint (it
imitates PDFium, which has no spot plates), so it has no such discrepancy.

Non-separable blend modes (Hue, Saturation, Color, Luminosity) follow the
PDF rule for CMYK (PDF 32000-2 11.3.5.3, Poppler `splashModeCMYK8`): the
RGB formulas run on the complements of C, M and Y, and K is the backdrop's
(Hue, Saturation, Color) or the source's (Luminosity). The C/M/Y pass holds
exactly those complements, and a pass with one plate holds it as a grey
triple, for which the formulas reduce to the backdrop or the source, so the
ordinary compositor computes both. Spot plates follow the K rule. A page
whose non-separable mode meets a pass that does not hold C, M and Y together
(K with spots, or one process plate per pass for overprint) is repeated
with C/M/Y together and every other plate alone.

In that layout the C/M/Y pass masks overprint per plate, like Poppler's
per-channel `overprintMask`: before an operator whose overprinting paint
names only some of C, M and Y, the pass saves the pixels, and once the
operator is done it restores the plates the paint left untouched. So the
non-separable blend sees all three process complements and overprint keeps
the unnamed plates. Operators that mix such a paint with a paint of a
different plate mask (for instance `B` with an OPM 1 fill and stroke naming
different plates) cannot be masked and fall back.

Transfer functions (`/TR`, `/TR2`) apply per plate to `255 - ink`, like
Poppler's `cmykTransfer*[i] = 255 - transfer[255 - i]`: C, M and Y take the
first three curves of a four-curve array, K and spot plates the fourth; a
single curve applies to every plate. They apply where the RGB render applies
them (paint colours, image samples, shadings, tiling-pattern results).

Colour-font glyphs: COLR/CPAL palette colours, COLRv1 gradient stops, SVG
glyph fills and bitmap glyph pixels are sRGB and convert to ink like
DeviceRGB paint; layers in the text colour (palette index 0xFFFF,
`currentColor`) keep its exact ink. Gradients interpolate in ink.

## Fallback

Pages that use something the plate compositor cannot reproduce are rendered
in RGB and converted with the formula above; `CmykRaster::native` is
`false`, `diagnostics` names the feature and the CLI prints a warning:

- an operator whose overprinting paints name different subsets of C, M and
  Y, on a page that also uses a non-separable blend mode;
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
  overprint, constant alpha, Multiply, CMYK images and shadings,
  separation plates; native Hue and Luminosity in CMYK and with spot plates,
  Hue on a page with OPM 1 overprint (per-plate masking), per-plate transfer
  curves, COLR palette and foreground ink.
- Synthetic page against `pdftoppm -jpegcmyk -overprint`: every probe point
  within JPEG noise (DeviceCMYK fill 51/102/0/26, OPM 1 overlap 255/255/0/0,
  spot overprint on cyan 255/61/240/0, CMYK image and shading values).
- Spot overprint onto process ink (2026-11, `render --cmyk --overprint
  preview -f tiff --tiff-compression none` against `pdftoppm -tiff
  -tiffcompression none -overprint`, Poppler 26.07): a page with magenta
  102, a spot (alternate 0/61/240/0) overprinting it, a second spot at half
  tint overlapping both, a knockout spot and OPM 1 yellow is byte-identical
  to Poppler over all 10,000 pixels (163 = 102 + 61, 201/255 where both
  spots add, clamped).
- tests/corpus/pdfs + engine fixtures (629 pages at 50 dpi): all render
  natively except one page with a `Color` blend mode (fallback as designed).
  Differences to Poppler's CMYK output track the RGB renderer's own
  differences to Poppler on the same pages.
- RGB output of tests/corpus/pdfs (596 pages at 100 dpi) is byte-identical to
  main.
