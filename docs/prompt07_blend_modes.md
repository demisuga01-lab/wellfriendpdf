# Prompt 07 Blend Modes

Blend modes are centralized in the render buffer/compositing layer instead of
being scattered through text, image, vector, or Form XObject drawing code. The
dispatch contract receives source color, backdrop color, source alpha, backdrop
alpha, the active blend mode, and the renderer's current device-space posture.

## Implemented Modes

Prompt 07 reports native support for:

- Normal
- Multiply
- Screen
- Overlay
- Darken
- Lighten
- ColorDodge
- ColorBurn
- HardLight
- SoftLight
- Difference
- Exclusion
- Hue
- Saturation
- Color
- Luminosity

The separable modes operate channel-by-channel with clamping. The nonseparable
modes use the renderer's RGB luminance/saturation helpers. This is a practical
device-space implementation, not a claim of full ICC-managed PDF transparency
group color conversion.

## Evidence

The Prompt 07 audit script generates one fixture per required blend mode plus a
combined 4 by 4 grid. Each fixture is rendered by Oxide, Poppler, PDFium, and
MuPDF. Pairwise metrics and visual hashes are written to:

- `target/prompt07-transparency-compositing/blend-mode-matrix.json`
- `target/prompt07-transparency-compositing/post-implementation-render-results.json`
- `target/prompt07-transparency-compositing/reference-disagreement-summary.json`

When references disagree, the matrix records the disagreement instead of
weakening the fixture or calling the Oxide result release-grade parity.

## Known Limits

Full color-managed nonseparable blending waits for the renderer color-management
layer to resolve group color spaces more completely. Prompt 08 may add new paint
sources, but not new one-off blend formulas.
