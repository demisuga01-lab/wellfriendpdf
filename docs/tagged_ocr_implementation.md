# Whole-Figure tagged OCR — source continuation

Implemented over the existing uncommitted `main` candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. No compilation, builds, tests, PDF
workloads, rendering, browser/binding execution or deployment were run.
The full roadmap remains active, including more general tagged OCR migration.

## Implemented source behavior

An initial page-owned image may now carry explicitly selected invisible OCR
through the tagged linked-story route when **one existing selected Figure leaf
exclusively owns both the image paint and every selected OCR operand**, or when
the caller explicitly consumes one or more separate, content-only sibling
`P`/`Span` owners through exact, disjoint `merge_into_figure` span assignments.
The existing `StoryFigure.ocr` selection supplies exact span IDs and expected
font-decoded Unicode; `source_tags.figures` supplies the semantic owner.
There is no new flag that lets a caller bypass owner validation.

Preflight verifies:

- Exact page, Contents occurrence, source object, operand range and Tr=3.
- Every MCID surrounding a selected carrier resolves either to that Figure owner
  or to the exact separately approved owner assigned to that span.
- No selected carrier is also initial paragraph-replacement text.
- No optional-content, artifact, frame, different text owner or semantic
  subtree is silently absorbed into the Figure.
- Every nonempty text operand owned by the selected Figure is included; an
  unrelated-text decision cannot excuse leaving that Figure's text behind.
- Content-level ActualText on the image is allowed only when its complete
  glyph scope travels in the approved OCR group. The native carrier layer
  independently rejects partial/shared ActualText capture.
- All initial IDs are found exactly once. A Contents occurrence is part of
  the identity; shared objects at the same byte offset are not interchangeable.

The original-revision detachment batch moves the image and search operands,
preserves source advances and rebases disjoint paragraph scalar ranges. Existing
tag preparation removes the selected source Figure delimiters. In the native
search Form, a retained direct ActualText property keeps its text and other
property values but has its original page MCID replaced by null. Other old
semantic wrappers are neutralized by the existing carrier writer.

The destination's page-owned Figure marker then owns the entire native
image/search invocation. The search Form does **not** acquire a second semantic
owner or reuse the source page's MCID namespace. Final Figure /K, page MCR,
ParentTree, ordering before its separately owned caption, layout attributes and
saved native bindings are handled by the existing atomic story transaction.
This applies through continuation-page insertion, subsequent save/reopen moves
and explicit whole-group deletion. It is not sanitizing redaction.

The native search-program validator now also rejects non-null MCIDs, named
property dictionaries and optional-content markers. This prevents a reused
search capsule from introducing another unreviewed semantic namespace; it
already rejects visible text, image/shading paint and painted paths.

The final design keeps one Figure owner for the complete destination invocation.
A separate source owner is never inferred: every named owner must be a unique
selected direct sibling, contain only its exact assigned OCR content and basic
structural keys, and is removed atomically. The plural request partitions exact
span IDs between owners; unassigned spans must already belong to the Figure.
Owners with semantic attributes, relationships, subtrees or additional content
refuse. See `separate_ocr_owner_implementation.md`. The
[PDF Association's structure reference](https://pdfa.org/download-area/cheat-sheets/LogicalStructureObjects.pdf)
describes the separate page/content-stream parent namespaces; the
[Form XObject errata](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html)
distinguishes whole-object ownership from contained marked-content ownership.
Those references guide the implementation, not prove its correctness.

## Browser and binding behavior

The browser no longer disables all non-relocatable OCR candidates: it labels
them as requiring native Figure-owner validation. Users must still select the
exact carriers, bind the matching Figure owner and approve the resulting
preview. Ordinary untagged capture retains its existing restrictions. A failed
native preflight does not silently change the selected ownership.

Existing Rust, universal, WASM and native session routes carry the same request
fields into this writer; no new independent mutation API is introduced here.

## Regression source and remaining scope

Relevant unexecuted regressions cover:

- Outer-Figure and nested-Span ActualText, cross-Contents scopes, OCR before
  caption source ranges, growth through canonical insertion, contraction and
  repeated reopen/save with Figure/ParentTree and searchable-text assertions.
- Atomic image/search/Figure deletion while retaining caption and other OCR.
- Wrong/duplicate/stale/visible OCR selections, paragraph overlap, omitted OCR
  and attempting to replace the semantic owner with a newly invented Figure.
- Explicit separate-Span merge through direct and split streams, including two
  owners for one Figure, removal of consumed owners, ParentTree/search assertions
  and repeat saved editing.
- Omitted, wrong, nonselected or duplicate separate owners; mixed singular/plural
  syntax; empty, stale or overlapping span partitions; semantic attributes and
  additional owner content refusing before mutation.
- Unique and reused Form MCRs with one or multiple separate sibling owners,
  including consumed ownership, search/ParentTree checks and repeat saved
  editing; an explicitly split reused Form retains ordered adjacent residual
  Figure and OCR-owner leaves.

An existing native search-program regression was extended with forbidden MCID,
named-property and optional-content cases plus a permitted null-MCID ActualText
case. These are test source, not passing results or independent visual evidence.

Same-Figure invisible OCR inside an exact nested Form occurrence now uses the
separate source transaction described in `nested_form_ocr_implementation.md`.

Still incomplete: content-bearing/shared reused Figure subtrees, nested/malformed
semantic relationship containers, named/shared/partial ActualText transfer,
optional-content OCR, handwriting/background recovery,
cross-story transfer and the broader font/layout/rendering roadmap. This does
not claim general tagged OCR, PDF/UA conformance, universal editing or Adobe
superiority. All exact-revision build, binding, extraction, independent raster,
accessibility and corpus qualification remains pending.

## Checks performed in this source-only continuation

`rustfmt --check` succeeded for the changed OCR/tagged/story source and regression
files. Parse-only `rustfmt --emit stdout` succeeded for the capability registry;
`node --check` succeeded for `browser/story-editor.js`; and
`git -c core.safecrlf=false diff --check` found no whitespace errors.
These check syntax/formatting only, not Rust types, execution, semantics, pixels
or standards conformance. No regression was executed and no commit/push occurred.
