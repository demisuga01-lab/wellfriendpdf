# Combined form action policy Interactive / DOCX Audit

## Starting state

form action policy began from the exact required checkpoint.

- HEAD: `6d07aa35695236647c0f918e14ff65798707b313`
- subject: `Close roadmap closure 18B advanced secure mutation gaps`
- worktree: clean
- secure mutation parent: `261968c8e70012d563f2282200159e51779b0e0c`
- classification: `exact_expected_start`

The machine-readable record is
`target/form_action_policy-interactive-docx/form_action_policy-starting-state.json`.

## Canonical repository paths audited before implementation

form action policy extends existing shared architectures; it does not introduce a
CLI-only action scanner or a second OOXML package writer.

| Domain | Canonical path(s) reused |
|---|---|
| AcroForm fields, widgets, inheritance, `/CO` | `crates/engine/src/interactive.rs`, `crates/engine/src/editing.rs`, `crates/engine/src/form_exchange.rs` |
| annotation actions and page provenance | `crates/engine/src/interactive.rs`, `crates/engine/src/annotation_media_redaction.rs` |
| active-content report and full-rewrite sanitizer | `crates/engine/src/security.rs`, `crates/engine/src/writer.rs` |
| XFA script/event inventory and bounded runtime policy | `crates/engine/src/xfa/mod.rs`, `crates/engine/src/xfa/script.rs` |
| FDF/XFDF | `crates/engine/src/form_exchange.rs`, `crates/engine/src/annotation_media_redaction.rs` |
| associated-file ownership | `crates/engine/src/secure_mutation.rs`, `crates/engine/src/attachments.rs` |
| DocMDP/FieldMDP and incremental mutation | `crates/engine/src/secure_mutation.rs`, `crates/engine/src/signature.rs` |
| optional content and page transforms | `crates/engine/src/optional_content.rs`, `crates/engine/src/render/transform.rs` |
| semantic/editable model | `crates/engine/src/parse.rs`, `crates/engine/src/editable.rs` |
| paragraph/table/image reconstruction | `crates/engine/src/parse.rs`, `crates/engine/src/analysis/tables.rs`, `crates/engine/src/images` |
| DOCX OOXML package and readback | `crates/engine/src/office.rs` |
| stable report facade | `crates/engine/src/sdk.rs` |
| CLI and bindings | `crates/cli/src/main.rs`, `crates/wellfriendpdf-py`, `crates/wellfriendpdf-capi`, `crates/wellfriendpdf-wasm`, `bindings/dotnet`, `bindings/java` |
| roadmap task evidence conventions | `scripts/secure_mutation_secure_mutation_audit.py`, `scripts/secure_mutation_closeout_advanced_secure_mutation_audit.py` |

## Implemented architecture

`crates/engine/src/form_action_policy.rs` is the shared form action policy policy/report layer.
It inventories actions from document JavaScript name trees, catalog/page/field/
widget/annotation action slots, inherited field owners, and `/Next` chains. The
inventory records stable IDs, owner/path provenance, event, decoded size,
SHA-256, secret-safe preview, API indicators, field dependencies, sanitizer
disposition, execution policy, and signature posture.

The default remains non-executing. The optional calculation evaluator accepts
only bounded pure scalar expressions and `AFSimple_Calculate` over a static
field list. It rejects loops, `eval`, functions, dynamic property traversal,
network, filesystem, process, UI, clipboard, and timer APIs. This is not an
Acrobat JavaScript implementation.

The form action policy sanitizer runs through the shared deterministic full-rewrite
writer, removes owner slots and action objects according to policy, reopens the
saved PDF, and rescans it. Full-rewrite mutations are governed by the Roadmap task
18B DocMDP/FieldMDP decision and explicit override requirement.

The native DOCX writer remains `crates/engine/src/office.rs`. form action policy adds:

- one exact-size Word section per source page, including landscape/mixed sizes;
- deterministic page/section breaks and layout-mode margins;
- stable deduplicated media names and relationship IDs;
- real external hyperlink relationships and hyperlink runs;
- preserved paragraph/run styling inside positioned text boxes;
- zero-inset page-relative text boxes with deterministic z-order;
- repeated table headers, row no-split posture, grid spans, and vertical merges;
- deterministic `settings.xml` and fixed core/application metadata.

## Security and fidelity boundaries

- JavaScript execution is disabled by default and arbitrary Acrobat DOM
  emulation is not claimed.
- Inventory never performs network, filesystem, process, clipboard, UI, timer,
  or cloud operations.
- Undecodable, cyclic, malformed, oversized, and unsupported action/script
  inputs are reported or removed fail-closed under mutation policies.
- Page-faithful means measured OOXML/page geometry fidelity, not perfect Word
  reconstruction.
- Word and LibreOffice observations are recorded separately from structural
  OOXML readback. Missing editor automation is reported as unavailable.
- Running headers/footers/page numbers are preserved as positioned page
  furniture when detected. Dedicated Word header/footer parts are not inferred
  without repeat-confidence, and zero part counts make that limit observable.
- Generic PDF annotations/widgets are not silently promoted to Word comments
  or content controls. Static/positioned fallback and exact unsupported rows are
  reported.

## Evidence and acceptance

The authoritative generator is
`scripts/form_action_policy_interactive_docx_audit.py`. It emits the form action policy feature
matrix, action inventory/graph/policy/rescan artifacts, interactive scorecard,
pagination taxonomy and corpus results, DOCX component/readback results,
metamorphic/differential results, performance/limit results, and the HTML
report under `target/form_action_policy-interactive-docx/`.

The feature matrix uses only the form action policy status vocabulary and is accepted
only when `blocked == 0` and `unclassified_failures == 0`.
