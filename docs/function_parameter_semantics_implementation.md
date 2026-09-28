# PDF function domains and indirect render parameters

Source continuation on `main` over
`27e62db3a1b84804339e65b6025273fd003b3736`. The accumulated roadmap changes are
preserved and remain uncommitted. This is not a completed universal-editor or
production-renderer qualification report.

## Correctness changes

The shared native PDF function evaluator previously normalized Type 2 input to
the unit interval and clamped every result to that interval. Both operations
were wrong for a general Type 2 function. It now evaluates the declared
exponential on the domain-clipped input itself. An absent Range does not imply
unit-range clipping. Signed and non-unit components survive until the selected
colour-space converter applies its own rules.

The common evaluator now requires the declared input count, clips each input
before evaluation (including calculator functions), and checks ordered finite
Domain/Range pairs. Type 2 exponent/domain compatibility, component counts and
Range dimensions are checked. Stitching functions check ordered boundaries,
consistent child output sizes and parent Range dimensions before execution.
Component-function arrays require one output per child during shape validation,
not only after sampling. Parent stitching Range is applied to the child result.

Sampled-function encoding and stitching encoding no longer treat a tiny
nonzero interval as zero using a fixed epsilon. Relative positions and decoded
values use overflow-aware interpolation. Stitching selection retains exact
half-open intervals and the defined terminal zero-width interval behavior.
Recursive shape traversal has a shared visit limit and cancellation checks;
this is not a total execution-time bound for arbitrary calculator programs.

These changes serve the native shared function entry points used by shading,
colour/tint and transfer-function consumers. They do not replace every separate
vector-export function implementation.

## Parameter resolution

`render/parameter_dictionary.rs` resolves recognized shading, function and
pattern fields with copy-on-write dictionaries:

- Scalars, arrays and array elements may be indirect, including bounded alias
  chains handled by the reader's existing cycle/depth checks.
- A dictionary null is treated as an absent entry. Required fields still fail
  their validator; array nulls are not turned into numeric defaults.
- Unknown/private entries remain untouched. Nonnull function/resource object
  references retain their source identity; there is no eager graph flattening.
- Parameter arrays have explicit bounds. Function parameters are limited to
  8,192 entries, stitching children to 4,096, and shape traversal to 4,096 visits
  and the existing depth limit. Shading vectors use their smaller shape limits.
- Normalization occurs before mesh-type dispatch, decode selection and shading
  validation. Pattern fill/stroke, tiling parameter consumption, direct/retained
  shading and SVG/PostScript parameter loaders use the same resolver.
- Function object, array and stream lookups follow reference chains. The page
  and vector dictionary/stream helpers also use the bounded reader resolver.

Array limits and cancellation checks bound this resolution work; they do not
bound the reader's full object cache, native decoder memory, or total RSS.

## Specification basis

The input/output clipping and Type 2 equation follow ISO 32000-1 section 7.10:
[Adobe-hosted PDF 32000-1, function dictionaries and exponential interpolation](https://raw.githubusercontent.com/adobe/dc-acrobat-sdk-docs/master/docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).

Stitching uses ordered, half-open subdomains and a special encoding value for
the empty final interval:
[Adobe PDF Reference 1.4, section 3.9.3](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf).

## Regression source, not execution evidence

Twenty-six new regression functions cover:

- Six parameter-resolution cases: COW/direct input, indirect scalars/arrays/
  elements, null semantics, reference cycles, size limits and preserved children.
- Eleven function cases: non-unit/signed Type 2 results, explicit Range,
  exponent domains, calculator input clipping, tiny sampled domains, stitching
  boundaries/Range, empty final interval, malformed dimensionality, indirect
  functions, traversal limits and extreme domain mapping.
- Three shading-entry cases: all seven families with indirect parameters,
  optional indirect-null mesh Function, and non-unit exponential pixel values.
- Three page-render cases: raw and fully compiled descriptor dispatch with
  culling enabled, pattern fill/stroke, and indirect mesh type/stream decoding.
- Three vector-loader cases: SVG/PostScript parameter parity, referenced
  function arrays/pattern matrices, and native/vector Type 2 sample parity.

Two prior tests were updated: one now describes input-domain clipping rather
than universal unit-output clipping; component-array shape validation now
rejects a multi-output child before evaluating it.

Only rustfmt parsing/formatting and whitespace checks were performed. No build,
compiler/type check, tests, PDF/rendering workload, binding execution, benchmark,
commit, push or deployment was performed. The source assertions are unexecuted.

## Remaining work

- Paint-scoped prepared programs/sample data and shared nested execution budgets
  are now implemented in `prepared_function_graph_implementation.md`; cold tint/
  transfer retention and aggregate memory reservations remain open. Typed
  calculator/operator handling is now added in `calculator_semantics_implementation.md`,
  with numerical/interpreter qualification still pending. The subsequent
  `sampled_function_interpolation_implementation.md` increment adds bounded
  tensor cubic Order 3 and packed-sample work/decoder controls. Its source
  regressions remain unexecuted.
- Vector export has separate function-to-gradient/PS logic. The subsequent
  `vector_function_semantics_implementation.md` increment addresses parent Range,
  boundary selection, analytic clipping stops and numeric serialization, and
  routes non-affine colour conversions without exact PS representations to
  raster. Broader native colour fidelity and SVG hard transitions remain open;
  neither increment has executable qualification.
- Pattern parameter resolution does not establish all pattern paint-state,
  uncoloured-pattern binding, ext-state or transparency semantics.
- Floating-point arithmetic is not exact/interval-certified; the earlier
  analytic-geometry limits and wider rendering/codec/export gaps remain.
- The full editing/font/layout/tag/history/scan/UI roadmap and every current
  executable qualification gate remain active and incomplete.
