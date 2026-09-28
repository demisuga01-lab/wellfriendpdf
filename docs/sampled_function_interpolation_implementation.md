# Sampled-function interpolation: implemented in source, unqualified

This is an uncommitted increment on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`. It does not close the full
editor/rendering roadmap.

## Changes

- Function Type 0 now validates `/Order`: absent/null means linear, `1` means
  linear, `3` means cubic, and other types/values fail validation. The existing
  parameter normalizer resolves indirect Order values before this check.
- Cubic evaluation uses a tensor product of cardinal Hermite stencils with
  centered slopes (Catmull-Rom). Exterior samples repeat the nearest endpoint.
  Each axis with fewer than four samples uses linear interpolation; singleton
  axes use their sole sample. Exact sample coordinates use one tap.
- Sample-point indexing remains dimension-zero-fastest, with all output channels
  interleaved at each point. Reversed Encode and non-unit/tiny input domains keep
  the shared domain mapping.
- Packed values are interpolated in normalized sample space. Decode is applied
  afterward, followed by Range clipping. Cubic overshoot is not prematurely
  clipped to `[0,1]`, which would change functions with explicit Decode ranges.
- The old per-corner index-vector allocation is removed for both orders. Fixed
  axis stencils merge boundary aliases, a mixed-radix counter visits tensor taps,
  and two output-sized vectors hold compensated sums. No full interpolation
  tensor or recursive output-buffer tree is allocated.
- Random-access sample reads assemble at most five bytes, instead of reading
  individual bits. All eight supported packed widths remain supported.
- Shape validation now rejects overflow/over-budget table dimensions before
  decoding. The existing 4,194,304 sample-value cap also bounds tap/channel reads
  per evaluation; the eight-dimensional limit is unchanged. The sampled-stream
  decoder receives an additional 16 MiB decoded-byte ceiling.
- Cancellation is checked before interpolation and every 256 sample reads. A
  cancelled/malformed evaluation does not return a partial colour.
- Direct function streams are borrowed when passed to the decoder, avoiding the
  previous extra full stream-object clone. Indirect resolution still follows the
  canonical reader. This is not a decoded-sample cache.

## Mathematical and specification basis

The polynomial weights were derived from the cubic Hermite endpoint basis with
slopes `(next - previous) / 2`, then combined across dimensions. Compensated
summation reduces cancellation from the negative cubic lobes; it is not an exact
arithmetic or error-bound proof.

[PDF 32000-1 section 7.10.2](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf?from=20423&from_column=20423)
defines Order 1/3, Encode/Decode, sample storage and the small-Size fallback.
The specification does not provide a complete numerical kernel and boundary
recipe. The chosen centered-slope/endpoint-extension convention is documented
here rather than presented as a uniquely mandated kernel.
[Artifex's sampled-function implementation](https://github.com/ArtifexSoftware/ghostpdl/blob/master/base/gsfunc0.c)
was consulted for interoperability context; this implementation independently
derives its stencils and does not copy that implementation's source. Actual
agreement with Ghostscript, Acrobat or another renderer has not been tested.

## Regression source

Seventeen new regression functions are present but **unexecuted**:

- Sixteen sample/function cases cover a cubic-vs-linear counterexample, exact
  sample points, exterior extension/overshoot, Decode/Range ordering, short axes,
  tensor/channel indexing, eight dimensions, all bit widths, packed-reader parity,
  reversed Encode/tiny domains, invalid/indirect Order, component arrays, size and
  decode limits, cancellation, extreme Decode values and Separation tint use.
- One render-entry regression covers both raw and compiled dispatch for axial and
  two-input function shadings, asserting expected output pixels rather than only
  a successful operation report.

Only formatting/parser and whitespace checks were performed. No compilation,
type checking, test, PDF workload, rendering, browser/binding execution, benchmark,
commit, push or deployment was performed.

## Remaining work

- The subsequent `prepared_function_graph_implementation.md` increment retains
  sampled tables/calculator programs across shading samples and shares execution
  budgets across nested/component functions. Shading function work now debits
  the cumulative render allowance. Cold tint/transfer retention, cross-paint
  caches, shared memory reservations and broader diagnostics remain open.
- Native vector export of sampled functions is separate; the existing explicit
  raster routing is unchanged. Other function families, colour management,
  codecs, transparency and the wider editing/UI roadmap remain incomplete.
- Cubic numerical/edge conventions and performance require actual independent
  renderer, corpus and regression qualification. This is not a universal,
  production-ready or better-than-Acrobat claim.
