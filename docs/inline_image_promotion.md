# Inline image promotion

`redact-inline-image --promote` converts a safely decoded inline image into a deterministic Image XObject before secure rewriting. It adds an `OxP18Inline*` resource to the affected scope and replaces only `BI … ID … EI` with `/Name Do`; the CTM, clipping, ImageMask fill color, and graphics state remain active.

The promoted object preserves dimensions, decode semantics, stencil bit depth, supported device color semantics, and deterministic predictor-aware Flate output. The original inline payload is absent from rewritten content. Names and object ordering are deterministic.

Malformed or ambiguous inline data is never promoted. Disallowed resource mutation or signature policy returns an exact failure or full-rewrite requirement.
