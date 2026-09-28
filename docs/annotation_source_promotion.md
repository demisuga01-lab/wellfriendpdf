# Exact direct-annotation source promotion

Status: implemented in source, not compiled or runtime-qualified. This is one
increment of the active editor roadmap, not its completion.

## Transaction and integration

`annotation_promotion.rs` selects a direct dictionary by the original revision,
page object and `Annots` ordinal. Equal dictionaries in different slots remain
different occurrences. It materializes the selected dictionary as an indirect
object and replaces only that slot, preserving source order and native fields.
Actions, appearances, opaque values and resource references are not reconstructed
through XFDF or executed. Shared appearances remain shared.

The selected source popup/reply component is traversed once with a bounded graph
walk. Every participant's editing ID is persisted before any revision change.
Unrelated anonymous IDs are compared through their source occurrence, not their
new hash. The writer allocates above both live objects and the declared xref
Size, includes cancellation polls and bounds dictionary verification bytes.
Reopening checks dictionary hashes, source count/order, identities and the actual
relationship graph. These checks are implementation, not evidence that they ran.

Native geometry invokes materialization privately when a selected component
contains a direct annotation. XFDF resolves IDs, deletion IDs and relationship
targets against the original PDF first, then privately materializes all bound
operands before update/delete/create planning. Thus one direct operand cannot
stale an anonymous indirect operand in the same request. Intermediate bytes are
not returned when the enclosing transaction rejects an edit.

Story discovery now includes direct annotations with supported Rect geometry.
Story staging verifies the original selected dictionaries/slots even when an
unrelated image-staging revision intervenes, materializes selected components,
and persists their IDs before canonical page insertion.

Public Rust entry point, re-exported by `annotation_media_redaction`:

```text
promote_annotation_sources_pdf(input, source_sha256, annotation_ids)
```

It accepts 1..4096 distinct IDs and requires the displayed input SHA-256.
The existing generic document-subsystems JSON surface also routes:

```json
{
  "kind": "annotation_promote_sources",
  "source_sha256": "<input SHA-256>",
  "annotation_ids": ["<discovered exact source ID>"]
}
```

Use the annotation-appearance subsystem and the existing approved operation
request envelope. The report lists persisted and promoted IDs and affected pages.
Native geometry and XFDF relationship receipts separately list promoted source
IDs; normal outer input/output authority remains the original request and final
output, not an internal staging revision.

## Deliberate remaining boundaries

- The subsequent `annotation_owner_promotion.md` increment supports direct
  widgets/tagged annotations with unique exact reachable field/OBJR ownership.
  Direct field ancestors are covered by `annotation_field_materialization.md`.
  Unique direct structure roots/elements are covered by
  `annotation_structure_materialization.md`. Missing/ambiguous owners need repair;
  unrelated equal dictionaries are not adopted merely because they match.
- Direct relationship values, dangling/cyclic graphs and contradictory page P
  references require a separate repair workflow. Source promotion does not
  silently repair malformed ownership.
- Before materialization, story page-pruning planning conservatively retains
  pages containing direct annotations. Movement still works; this increment does
  not claim automatic deletion of their vacated continuation pages.
- Standalone appearance generation may still report direct sources unsupported;
  explicit promotion provides a source-normalization route before that tool.
- Extended geometry, appearance typography, foreign-document ID remapping,
  shared tagged appearance cloning, standards/signature/permission preservation
  and encrypted incremental writing remain separate boundaries.
- Incremental history remains. Promotion is not sanitizing redaction.

The occurrence/owner distinction follows PDF's separate page annotation,
interactive-form and logical-structure relationships; see Adobe's
[PDF Reference 1.6](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf),
sections 8.4, 8.6 and 10.6. The source ID scheme is SDK metadata, not a PDF standard
identity claim.

## Evidence boundary

Eight new unexecuted regression functions in `annotation_promotion_tests.rs`
cover duplicate direct dictionaries, idempotence, native geometry with shared
appearances/actions, mixed direct/indirect batches, XFDF update/delete/reply
creation, stale/intermediate revisions, inferred popup ownership, owner-mapping
refusals, story movement and generic JSON routing. The previous direct-source
XFDF refusal regression now asserts one update without duplicate creation.

Only formatting/parser and whitespace checks were run. No compiler, tests,
binding execution, PDF workloads, rendering or benchmarks were run. The full
roadmap remains in `universal_editor_roadmap_tracking.md`.
