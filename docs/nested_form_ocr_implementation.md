# Nested Form OCR relocation - source implementation, not qualification

This increment closes the source-level refusal that previously prevented an
image occurrence inside a Form XObject from moving with invisible OCR text in
the same exact Form occurrence. It does not perform OCR recognition and it is
not executable or corpus qualification.

## Exact source binding

`OcrCarrierSelection.form_target` carries the revision-bound
`FormTextTarget` returned by `formTextSourcesJson`. The target must identify the
same page content slot and full ordered invocation path as the selected image.
Page-owned OCR rejects a Form target, and nested OCR rejects a missing, stale or
different target. Caller-owned outer `ActualText` remains a typed refusal
because it cannot be migrated by editing only the leaf Form.

The Form capture path reuses the inherited graphics/text state discovered by
the native Form-text analyzer. Span identities retain the occurrence prefix,
and destructive patches are expressed only in the selected leaf Form's local
stream. Form-local scalar ranges are explicitly excluded from page-logical
linked-story range rebasing.

## Visual and searchable capsule chains

The moved group contains two private chains:

- the existing visual chain replays the selected image through its original
  leaf, ancestor Form and page invocation state;
- the search chain uses the same invocation/resource chain, but its leaf
  contains only the approved invisible source glyph operands and their original
  fonts, matrices, spacing and complete direct `ActualText` scopes.

Every search-chain Form removes `StructParent` and `StructParents`, carries the
private `WFNestedOcrSearch` marker and is validated recursively. Each non-leaf
must contain exactly one approved `Do` edge to another marked search Form. With
that edge removed, the remainder must pass the existing no-visible-paint OCR
program validator. The leaf must pass that validator directly. Validation is
acyclic, depth-bounded and decoded-byte-budgeted; the group receipt binds the
leaf carrier-program hash and expected Unicode/span count.

## Atomic source rewrite and tagged ownership

Standalone clone-one moves and linked-story nested batches apply the image
removal and every OCR/`ActualText` patch to one cloned leaf Form before any
ancestor or page resource is redirected. Duplicate and overlapping patches
fail closed. Shared source Form definitions remain reachable by unselected
occurrences.

For tagged input, the selected Form spans must be invisible, exact-range
operands owned by the same approved Figure leaf as the image. The validator
requires complete coverage of that Figure's Form MCR items, rejects other text
or paint, and regenerates the moved page-level Figure ownership through the
existing tagged-story transaction. Reused Form semantics still require the
explicit residual-Figure and relationship policies already defined by the
tagged Figure path.

The browser worker now loads Form text inventories alongside image occurrences.
Selecting a nested image exposes only invisible spans from that exact Form
target and serializes the target with the OCR selection.

## Regression source and remaining evidence

Unexecuted regression source covers a uniquely invoked tagged nested Figure
whose image and invisible OCR share one Form owner, save/reopen/re-edit, and a
wrong invocation-path refusal. Existing page-owned OCR and nested Figure tests
remain in place.

No Cargo command, compiler, test, PDF workload, pixel rendering, assistive-
technology check or browser/device run was performed for this increment.
Required later evidence includes the exact-revision build and regressions,
real nested/reused Form documents, independent extraction/rendering, search and
copy behavior, malformed/cyclic resource graphs, cancellation, memory and all
binding/browser workflows.

