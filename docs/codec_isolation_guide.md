# Codec Isolation Guide

Wellfriend's codec isolation posture is layered:

- Pure-Rust decoders remain the default for in-process decode.
- Lossless stream decoding is bounded by `DecodeLimits`.
- Prompt 05 routes non-render stream/image extraction through scheduler memory-token admission.
- Prompt 03 subprocess isolation remains the practical OS boundary for worker-backed codec paths.
- Prompt 04's native codec registry denies unsafe native backends by default.
- RLBox/WASM native-codec sandboxing remains documented as hard-blocked for this repository state.
- Prompt 11B applies the same boundary discipline to native CMM: LittleCMS/lcms2
  is implemented only behind the explicit `native-cmm-lcms2` feature, is not
  linked in the default engine, is unavailable for WASM, and qcms remains the
  report-visible safe default preview path.

Prompt 05 public artifacts:

- `wellfriendpdf feature-report --pretty`
- `wellfriendpdf parser-report input.pdf --include-decode --json`
- `target/prompt05-codec-closeout/codec-coverage-matrix.json`
- `target/prompt05-codec-closeout/closeout-verdict.json`

Failures must stay fail-closed. Worker timeout, output-cap violations,
malformed worker output, scheduler budget denial, malformed filters, and
unsupported native backends must return structured errors/reports rather than
silent fallback.

Native CMM backends follow the same rule. The Prompt 11B LittleCMS backend is
optional, feature-gated, package-documented, report-visible, WASM-disabled, and
covered by native transform tests. Future native CMM expansion must keep the
same policy for device-link ICC, multicolor ICC, separations, spot plates, and
overprint simulation.
