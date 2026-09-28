# Vector function semantics: source implementation, not qualification

This increment extends the accumulated uncommitted editor/rendering candidate on
`main`, based on `27e62db3a1b84804339e65b6025273fd003b3736`. It does not complete
the universal-editor roadmap or establish compatibility with any output viewer.

## Implemented

- Stitching functions retain their parent `/Range` in the parsed model, apply it
  after child evaluation, and carry it into RGB, gray-to-RGB, CMYK and named-tint
  PostScript emission. Unsupported exact-adapter ranges are not silently dropped.
- Stitching selection uses half-open intervals without an epsilon. Its terminal
  interval includes the upper domain endpoint; an empty final interval uses its
  Encode start. Selection is a binary partition rather than a linear search.
- Continuity checks compare the post-parent-clipping boundary values. A genuinely
  discontinuous function is not labelled continuous merely because its jump is
  small. Parent clipping can also make differing child endpoints continuous.
- Continuous linear gradient export now inserts analytic breakpoints for function
  domains, child domains reached through Encode, child/parent Range thresholds and
  device component clipping. Reversed Encode and shading domains are handled.
  Output clipping plateaus are no longer replaced by endpoint-to-endpoint ramps.
- Domain mapping uses the shared normalized interval helper and stable endpoint
  interpolation. Tiny nonzero domains/stops are not discarded or merged by an
  absolute epsilon. Exact PostScript component arrays are combined only when their
  domains and exponents really match.
- Vector parsing requires Bounds, validates parent range dimensions/order, bounds
  component and child counts, rejects fractional powers on negative domains, and
  does not treat a nearly unit exponent as a linear function.
- Stop accumulation is capped at 65,536 entries before pushing, using bounded
  child/component parameters and cooperative cancellation during parsing,
  breakpoint construction and sampling. This is not a total-RSS guarantee.
- PostScript Domain, Bounds, Encode, Range, exponents and coefficients retain
  round-trip numeric spellings when fixed precision would lose their values.
  Non-default shading domains are not omitted because they are close to `[0 1]`.
- SVG stop offsets retain narrow intervals. Non-eight-bit colours use RGB
  percentages; gradients explicitly select sRGB interpolation. PostScript stop
  colours likewise avoid fixed-four-decimal quantization.
- Varying nonlinear colour conversions (profiles, calibrated spaces, process inks,
  palettes and tint transformations) are no longer assumed to be linear between
  converted endpoint colours. They use the existing reported raster route unless
  an exact PostScript function/colour-space representation is retained. Constant
  source components can still emit a constant vector colour when resolvable.

## Source regression coverage

Seventeen new, **unexecuted** regression functions cover:

- Thirteen function/loader cases: clipping plateaus; device clipping; child-domain
  and parent clipping; reversal; native/vector boundary agreement; continuity after
  parent clipping; empty terminal intervals; narrow/extreme domains; adapter range
  retention; malformed metadata; stop budgets; nonlinear colour routing; and
  distinct component domains/exponents.
- Two PostScript cases: parent Range emission for all three output models and
  numeric/domain/colour preservation through serialization.
- Two SVG cases: close stop offsets and non-eight-bit stop colours.

Two existing PostScript fixtures also provide the newly required optional parent
range field. These are source assertions, not passing test results.

## Specification basis

Function clipping and stitching interval semantics follow the
[Adobe PDF Reference, section 3.9](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf).
Numeric emission uses the real-number syntax in the
[PostScript Language Reference, section 3.2.2](https://www.adobe.com/jp/print/postscript/pdfs/PLRM.pdf).
The SVG stop model is defined by
[SVG 1.1 gradient stops](https://www.w3.org/TR/SVG11/pservers.html#GradientStops).

## Still open

- SVG hard discontinuities remain raster-routed. Duplicate offsets can encode a
  jump, but reversing a gradient changes the exact-boundary owner; enabling this
  everywhere requires coordinated one-sided sampling and reversal/emission, not
  merely removing the parser guard. PostScript retains supported source Type 3
  discontinuities directly.
- The exact PS adapters still have bounded function families and unit component
  coefficient/range eligibility. Nested stitching, arbitrary function programs,
  nonlinear native SVG colour transforms and broad pattern/transparency semantics
  are not implemented by this increment.
- Arithmetic remains floating point. Narrow coordinates can exceed an output
  interpreter's precision/range even though serialization preserves the source
  value. No interval proof, target-interpreter certification or pixel proof is
  claimed. Coordinate/transform formatting outside function/stop emission remains
  a separate audit area.
- The subsequent native sampled/prepared-function increments implement bounded
  Order 3, paint-scoped graphs and shared/cumulative shading work accounting.
  Cold tint/transfer retention and aggregate memory reservation remain open; see
  `prepared_function_graph_implementation.md`. Typed calculator/operator behavior
  is now added in `calculator_semantics_implementation.md`, still unqualified.
  These changes remain unqualified. The wider
  editing/font/layout/tag/history/scan/UI and rendering roadmap remains open.

Only rustfmt parsing/formatting and whitespace inspection were performed.
No compiler/build/typecheck, test, PDF workload, rendering, browser/binding
execution, benchmark, commit, push or deployment was performed.
