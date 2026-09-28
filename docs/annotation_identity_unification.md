# Shared annotation identities and native geometry transactions

Source-only continuation on dirty `main`, base
`27e62db3a1b84804339e65b6025273fd003b3736`. This is implementation evidence,
not a compiling, tested or renderer-qualified release. No Cargo, builds,
typechecking, tests, PDF workloads, rendering, benchmarks or deployment ran.
No commit or push was made.

## Shared source selection

`annotation_identity.rs` now allocates the IDs used by story discovery, XFDF
export/import, standalone geometry edits and appearance-generation reports.
The ID is separate from the page-local PDF `/NM`. Unique unambiguous names keep
legacy IDs. Duplicate names and anonymous objects receive revision/object-bound
IDs. Selected objects persist their IDs as valid UTF-16BE PDF text strings in
`WFStoryAnnotationID`; existing names, including absent/null names, are retained.
Indirect text-string names resolve before decoding. Duplicate page ownership or
duplicate persisted IDs are not silently assigned to an arbitrary occurrence.

The common inventory includes direct and geometry-less annotations even though
those are not native geometry-edit targets. They still reserve destination NM
names. The story name planner accounts for them both before page insertion and
when validating the final namespace.

XFDF export now includes namespace-qualified `source-sha256` on the root and
`pdf-name` where there is an original NM. The standard `name` attribute carries
the shared editing ID. Import checks the source hash before mutation. The XML
parser resolves attribute namespaces, so extension recognition is based on URI,
not a literal prefix; duplicate expanded extension attributes reject.

Unbound external XFDF can address a unique raw NM alias. Ambiguous raw names,
unknown revision-scoped IDs and attempts to recreate an existing direct source
as a new annotation reject. Direct deletion is also explicit unsupported work,
not a silently successful deletion. Update/reopen checks bind the persisted ID,
original NM, output reference and destination membership. Appearance generation
uses the same IDs and persists the IDs of the objects it actually updates.

Appearance preservation now checks the selected normal state (including AS),
finite nondegenerate BBox/Matrix mapping and the existing stream-size bound,
rather than treating mere AP/N presence or an unrelated valid state as enough.
These are structural checks, not decoded paint-program or pixel validation.

These private extensions are **not** an XFDF interoperability guarantee. A
third-party consumer that ignores `pdf-name` sees the editing ID as the standard
name. Source-bound exports cannot simply be applied to another PDF. Deliberate
foreign-document import and persisted-ID remapping still need implementation.
The subsequent bounded popup/reply lifecycle implementation is documented in
`annotation_relationship_transactions.md`.

## Native move and resize

The previous standalone geometry route exported a record and imported it again.
That importer intentionally strips actions and rebuilds supported fields, which
is unsuitable for a geometry-only fidelity contract. `move_resize_annotation_pdf`
now delegates to the native source-object transaction shared with story anchors.
It no longer reconstructs or sanitizes the selected annotation through XFDF.

`edit_annotation_geometries_pdf(input, expected_source_sha256, changes)` adds an
atomic batch API. The document-subsystems action `annotation_geometry_batch`
requires `source_sha256` and a `changes` array, each containing `annotation_id`,
one-based `page`, and normalized `rect`. Existing generic JSON binding routes
deserialize the same action; no binding execution has been performed.

- Original action dictionaries, appearance references, field owners, contents
  and opaque entries remain in the original annotation dictionary.
- Endpoint/quad/vertex/callout/ink coordinates use the source-to-destination
  scale and translation. Rectangle differences are distances, so they scale
  without translation. Uniform resize also scales leader and caption offsets.
  Oppositely ordered source Rect corners are normalized for calculation.
- Existing appearance programs are retained. PDF appearance mapping fits the
  Matrix-transformed appearance bounding box to the annotation Rect; this is
  not FreeText reflow or appearance regeneration. The mapping and annotation
  geometry are described in the [Adobe PDF Reference, section 8.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).
- Every related popup/reply member must be explicitly selected. The batch must
  use one common affine transform and target page for the component. No implicit
  movement of other annotations is inferred from a single-object request.
- Original relative annotation order, destination membership and supported
  OBJR/appearance-MCR page ownership move in the same incremental transaction.
- The writer stamps selected IDs and verifies each complete touched annotation
  dictionary after reopening, in addition to rectangle, name, group topology and
  page-membership checks. Dictionary verification has a 64 MiB aggregate budget.
- Both source and destination pages are reported as changed. The compatibility
  `canonical_import` report field remains, but records zero imported annotations
  and zero regenerated appearances. `writer` identifies the native transaction.

## Boundaries

The standalone geometry batch requires 1..4096 explicit changes. It uses the
20,000-entry shared inventory limit, bounded group/geometry processing and
cooperative cancellation. Page rotation and UserUnit must match, new rectangles
must fit the target crop box, and existing destination names cannot be silently
renamed. The story workflow retains its separate explicit name-repair approval.

Ordinary direct-dictionary promotion is now implemented in the subsequent
`annotation_source_promotion.md` increment, with unique field/tag ownership in
`annotation_owner_promotion.md` and direct field-ancestor normalization in
`annotation_field_materialization.md`. Ambiguous/direct-structure owner mapping,
extended Path/Measure/ExData semantics,
nonuniform leader/caption geometry, shared tagged appearances needing cloning,
arbitrary tag subtrees and general annotation appearance typography are still
limited. Retaining AP does not fix a missing/malformed AP. Existing actions are
preserved, not executed or made safe. Signature/permission preservation is not
certified, encrypted incremental writing follows the existing writer refusal,
and historical revisions remain: this is not sanitizing redaction.

Standalone edits do not silently update another saved story's layout intent;
changed saved-anchor receipts must be reviewed/rebound through that workflow.
The generic XFDF importer still has intentionally different sanitizing semantics.
The subsequent shared relationship graph and transactional lifecycle work are
documented in `annotation_relationship_transactions.md`; native geometry and
XFDF remain distinct editing contracts.

## Unexecuted regression source and checks

Eleven regression functions in `annotation_xfdf_identity_tests.rs` cover shared IDs,
duplicate/anonymous names, popup round trips, stale exports, namespace aliases,
ambiguous raw names, direct-source protection, appearance IDs, native action and
shared-AP preservation, repeated geometry editing, common-transform groups,
document-subsystems routing, field-owner/inset preservation and names belonging
to direct or geometry-less residents, plus selected-state and appearance-mapping
validation. Existing story name-planning tests and
the standalone move operation assertion were updated for the shared route.

Only rustfmt formatting/parser checks and `git diff --check` ran. Those do not
establish Rust type correctness, test success, output fidelity, browser behavior
or interoperability. The full implementation and qualification roadmap in
`universal_editor_roadmap_tracking.md` remains active and incomplete.
