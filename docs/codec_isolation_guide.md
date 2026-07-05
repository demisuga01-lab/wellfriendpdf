# Codec Isolation Guide

Oxide's codec isolation posture is layered:

- Pure-Rust decoders remain the default for in-process decode.
- Lossless stream decoding is bounded by `DecodeLimits`.
- Prompt 05 routes non-render stream/image extraction through scheduler memory-token admission.
- Prompt 03 subprocess isolation remains the practical OS boundary for worker-backed codec paths.
- Prompt 04's native codec registry denies unsafe native backends by default.
- RLBox/WASM native-codec sandboxing remains documented as hard-blocked for this repository state.

Prompt 05 public artifacts:

- `oxide feature-report --pretty`
- `oxide parser-report input.pdf --include-decode --json`
- `target/prompt05-codec-closeout/codec-coverage-matrix.json`
- `target/prompt05-codec-closeout/closeout-verdict.json`

Failures must stay fail-closed. Worker timeout, output-cap violations,
malformed worker output, scheduler budget denial, malformed filters, and
unsupported native backends must return structured errors/reports rather than
silent fallback.

