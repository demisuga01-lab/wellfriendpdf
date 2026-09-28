# Renderer resource scopes: source implementation

Status: implemented source increment, not runtime qualification or completion of
the universal-editor roadmap. Base remains `27e62db3a1b84804339e65b6025273fd003b3736`
with the pre-existing uncommitted candidate preserved. No build, compiler, test,
PDF workload, rasterization, benchmark, commit, push or deployment was run.

## Resource lookup versus inherited graphics state

The implementation follows the resource ownership rules in
[ISO 32000-2 clause 7.8.3 and its approved errata](https://pdf-issues.pdfa.org/32000-2-2020/clause07.html#783-resource-dictionaries).
An explicit resource dictionary is a complete namespace, including `<<>>`.
Legacy omitted Form and appearance resources use the original page's resources,
not the enclosing Form's dictionary. Type 3 lookup selects the first explicit
scope in glyph-stream, font, original-page order. Tiling patterns require their
own dictionary; the renderer does not silently repair a missing one by borrowing
unrelated caller resources.

These rules do not reset the graphics state. An already selected font, colour
space or pattern is a different thing from a name subsequently looked up in the
new scope. The renderer keeps those object-bound selections across a scope
switch and restores the caller after nested execution.

## Source changes

- `PageResources::from_content_owner` distinguishes absent/null resources from
  explicit dictionaries. Invalid top-level types, unresolved references and
  cyclic references return errors instead of becoming empty/fallback scopes.
- `RenderState` retains an immutable `Arc<PageResources>` for the original page.
  Both offscreen transparency and soft-mask children carry that same page scope.
- Form and annotation appearance retained plans compile using exactly the
  namespace later used for execution. Ordinary Forms, transparency groups and
  soft-mask Forms replace their complete lookup scope instead of merging maps.
- Tiling-pattern lookup follows the explicit pattern dictionary. Nested legacy
  Forms invoked from a pattern still see the original page as their fallback.
- Type 3 CharProc parsing retains the glyph stream's explicit resources.
  Full glyph replay uses glyph/font/page priority, and its program-cache byte
  estimate includes those retained resource dictionaries.
- Offscreen children inherit active object-bound font/colour/pattern selections.
  Raw `Tf` dispatch explicitly selects its local font object, including when
  the name matches an inherited font from another scope.
- Form program cache identity uses the original page scope rather than the
  transient caller. Appearance cache identity now includes that page scope.
  Cached invalid Form/appearance programs continue to report a fatal error on
  reuse rather than disappearing silently.
- Type 3 raster cache identity includes resolved resources, original page,
  inherited paint context, contract, optional-content state, viewport, surface
  and base transform. Soft-mask cache identity also includes inherited paint
  context and original-page resources. Position stays in the glyph context;
  broader translation-invariant cache reuse needs dependency analysis, not an
  assumption that arbitrary glyph programs are context-free.
- The fallback font-key cache checks the complete dictionary, not its allocation
  address. Replacing a same-named dictionary in place cannot reuse the previous
  font's cache key simply because the address stayed the same.
- The old merging/overlay helpers were removed. Four old regression functions
  were revised to assert scope replacement and restoration rather than merging.

## Regression source and checks

Fifteen new unexecuted functions in `render/resource_scope_tests.rs` cover:
absent/null/empty/malformed/cyclic scopes; same-name direct fonts and reference
metadata; nested page fallback with and without a transparency group; explicit
empty Form refusal; program-cache context and negative-cache reuse; appearance
fallback; inherited selected-font pixels; glyph/font/page Type 3 precedence;
Type 3 replay; soft-mask isolation; required pattern scope and nested fallback;
raster cache contexts; raw Tf rebinding; and dictionary-aware font-cache keys.

Pixel and replay assertions above describe the tests' code, not observed results.
No test was executed. `rustfmt` syntax parsing and `git diff --check` passed.
They do not perform Rust name resolution, typechecking, ownership checking,
linking or pixel validation.

## Still open

- All exact-revision builds, binding execution, regression execution,
  cross-renderer comparisons, real-document corpora and memory/latency evidence.
- A subsequent source increment implements scope-bound default device colours
  and selected paint-state corrections; see `default_colour_rendering_implementation.md`.
  General paint-state fidelity, arbitrary Type 3 clipping/compositing, full ICC
  domain handling, broader colour/transparency/codec coverage and renderer
  exactness remain open. Neither increment certifies those separate algorithms.
- The strict helper validates resource-scope ownership, not every nested font,
  colour-space, image or other resource graph. Existing typed capability limits
  and resource-specific validation still apply.
- A subsequent source increment addresses SVG/PostScript classification and
  export scopes; see `vector_export_scope_implementation.md`. Extraction parity,
  broader exporter correctness and all executable qualification remain open.
- Form text caller-owned ActualText/tag migration, cross-scope selection,
  non-page program editing, broader binding/planning integration and the other
  unfinished work listed in `universal_editor_roadmap_tracking.md`.

This is a correctness increment to the implementation, not evidence of universal
PDF editing, Adobe superiority or production readiness.
