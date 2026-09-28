# Font coordinates, metric staging and color-instance consistency

Source increment over `27e62db3a1b84804339e65b6025273fd003b3736`, still
uncommitted. This is not a complete static-font instancer or roadmap closure.

## Production coordinate and color paths

The engine now pins ttf-parser 0.25.1, matching the version already selected by
Rustybuzz. Cargo.lock was aligned manually to that existing package and checksum;
the former 0.21.1 package entry and duplicate-version qualifiers were removed.
No Cargo resolver, build, typecheck or test was run. Previous unrelated dependency
changes remain intact. Source inspection of the upstream change log identified
per-axis avar fixes, phantom-point metrics and the unified COLR transform callback.
Engine call sites were inspected for the changed APIs; this is not compilation
evidence.

Explicit descriptor values such as weight 400 and width 100 are retained: they
are not necessarily the selected font's defaults. `apply_request_checked` validates
finite requests, raw fvar limits/record sizes and complete avar 1.0 maps, stages
changes in a cloned face, then publishes only after successful checks and
cancellation polling. This avoids trusting lazy table headers or min/max values
that the parser has already clamped. Zero-axis fvar remains non-variable.
Unknown descriptor axes remain ignorable for fallback fonts. Empty requests leave
the current face unchanged; this helper is not a full-font validator.

Renderer outline, strict advance, fallback coverage and color paint paths use
the checked helper. Legacy optional-outline APIs cannot return a detailed error
receipt and retain their compatibility behavior. The public bool adapter is
also retained, but engine production paths no longer call it.

COLR gradient stops now receive the same normalized coordinates as the selected
face; previously their offsets and alpha used the default instance. The collector
uses the parser's common matrix callback for translate/scale/rotate/skew and
composes local child transforms before accumulated parent transforms. This also
corrects displaced transform centers. Existing gradient/compositing limits remain;
these changes do not establish full COLR support or pixel equivalence.

## Internal metric staging

`fonts/mvar_instance.rs` stages the 38 registered MVAR targets in OS/2, hhea,
vhea, post and gasp using the shared source-bound variation evaluator. It respects
record strides, sorts/uniqueness requirements and the no-variation index. Private
or unknown tags are disclosed instead of being assigned invented targets.
Interpolated values are rounded once and range-checked for their signed or unsigned
fields. Terminal gasp ranges cannot vary, and resulting ranges must stay ordered.
Existing identical hhea/OS/2 vertical metrics stay synchronized; intentionally
different source tuples remain different.

All mutations occur in a separate stage. Source tables stay immutable, target
copies are bounded to 64 MiB with cancellation polling, and a later failure
discards the stage. LONG_WORDS rows are refused for MVAR. Removal of MVAR itself
belongs to the eventual complete-font transaction, not this helper.

**This primitive is not yet wired into the public font preparer.** It cannot by
itself turn a variable font into a valid static font. HVAR/VVAR, outline deltas,
GPOS/GDEF/BASE variable values, hints, names and retained dependent tables still
need to be composed before that operation can be exposed safely.

## Regression source and checks

26 new regression functions remain **unexecuted**:

- Ten coordinate cases cover nonlinear multi-axis mapping/order, repeated partial
  requests, explicit normal values against non-normal defaults, clamping, unknown
  axes, invalid raw tables, atomic failure, zero-axis fonts, capacity and cancellation.
- Thirteen MVAR cases cover all registered targets, field widths, fractional
  rounding, hhea synchronization, gasp invariants, extended strides, private tags,
  malformed data, no-op/sentinel handling, unsupported rows and cancellation.
- Three COLR cases cover parser-emitted centered transforms, parent/child ordering
  and selected-instance gradient stops/alpha. These inspect paint operations, not
  rendered pixels.

One existing descriptor regression was updated for explicit normal values.
Coordinate fixtures attach synthetic axis metadata to a static outline fixture;
they are not claimed to be complete conformant variable-font specimens.

Rustfmt formatting/parser checks and Git whitespace checks passed. No compiler,
build, typecheck, tests, PDF processing, rendering, benchmark, browser workload,
deployment, commit or push was performed.

## Open boundaries

The subsequent per-glyph increment composes this MVAR stage with source-bound
HVAR/VVAR, geometry and vertical-origin preparation. See
`font_glyph_metric_implementation.md`; it still does not expose whole-font output.

The pinned parser cannot set coordinates on fonts with 64 or more axes; checked
selection reports this instead of silently choosing defaults. Newer cross-axis
avar mappings and extended fvar layouts are not implemented. Malformed avar
identity-fallback recovery is not implemented by the backend, so the checked
path refuses those malformed maps. Broader whole-font instancing, editable CFF2
preparation, public/session integration and all executable qualification gates
remain open. The full roadmap remains active.

## Primary references

The [ttf-parser 0.25.1 change log](https://github.com/harfbuzz/ttf-parser/blob/v0.25.1/CHANGELOG.md)
documents the dependency/API changes. Raw metadata checks follow the
[fvar](https://learn.microsoft.com/en-us/typography/opentype/spec/fvar) and
[avar](https://learn.microsoft.com/en-us/typography/opentype/spec/avar) formats.
Metric targets and extension handling follow
[MVAR](https://learn.microsoft.com/en-us/typography/opentype/spec/mvar).
Color-instance and transform behavior follows the
[COLR specification](https://learn.microsoft.com/en-us/typography/opentype/spec/colr).
Preserving existing hhea/OS/2 equality follows the corresponding
[FontTools instancer policy](https://github.com/fonttools/fonttools/blob/main/Lib/fontTools/varLib/instancer/__init__.py).
