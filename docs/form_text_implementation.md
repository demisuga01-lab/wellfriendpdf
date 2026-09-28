# Occurrence-bound Form text — source implementation

Status: uncompiled and unexecuted. No Cargo, compiler, build, test, PDF workload,
render, benchmark, deployment, commit or push was run. Syntax/formatting and
whitespace checks are not typechecking or runtime qualification. The complete
editor/rendering roadmap remains active and incomplete.

## Native path

`advanced_editing::form_text::analyze_form_text` discovers page Form XObject
occurrences through the existing canonical content scanner. Each target carries
the input hash, page, Contents slot and exact ordered invocation identities.
Repeated shared streams and identical Form instances remain distinct. Direct
text within a Form has its own logical scalar range; descendants are separate
targets. The inventory includes the Form-local BBox. Coordinates precede the
Form Matrix and caller transforms; callers must supply edit geometry in that
space, not unconverted page coordinates.

`edit_form_text` dispatches to the existing multi-run writer with an explicit
source context. It shares CMap-boundary checks, ActualText cleanup, original
advance compensation, styled/inline replacement, supplied or bundled shaping
fonts, font embedding and source-order placement. The context is not a surrogate
PDF: no page is flattened or temporarily replaced in the saved output. Native
font-resource updates are attached to the edited Form, not the page.

The source scanner captures caller state at each Do. An inherited selected font
is bound to its actual resource object, with a collision-checked local alias if
the Form reuses its name. Inherited named colour spaces, patterns and marked
properties are similarly bound. Form-local q/Q cannot consume a caller frame.
Already-selected graphics state and named-resource lookup are distinct: legacy
Forms with omitted Resources fall back to the page's dictionary, not an enclosing
Form's dictionary. The vector inventory and clone-chain resolver now follow this
same page fallback. This follows the PDF Association's
[resource-dictionary clarifications](https://pdf-issues.pdfa.org/32000-2-2020/clause07.html#783-resource-dictionaries).
Modern PDF requires independent Form resource dictionaries; this is a legacy
input compatibility path, not a PDF 2.0 conformance certificate.

Clone-one copies the leaf and every selected ancestor, rewrites only the selected
Do operations, then uses exact Contents-slot copy-on-write. Matrix, BBox, group
and other source dictionary entries stay with their Form. The original Form
object is compared against the source before returning bytes.

Edit-all checks inherited contexts across all page occurrences. Resource-graph
checks also reject references from unaccounted annotation appearances, patterns
or Type3 programs; those need their own owner-specific context analysis. Merely
unused references in those graphs can conservatively trigger the boundary.

Output validation extracts the selected Form's direct text, not an unrelated
duplicate elsewhere on the page. Where pre-edit extraction agrees with the
source scalar model, the whole expected post-edit string must match exactly.
`whole_direct_text_verified` explicitly indicates whether that full comparison
was available; non-isomorphic ActualText still uses the native complete-owner
selection checks. The report returns an output-hash-bound target for another
edit, plus the exact source occurrence on its selected spans.

Discovery bounds depth to 8, occurrences to 4096 per page and unique decoded
source bytes to 256 MiB per page. Edit-all additionally bounds document-wide
occurrences to 4096. Ownership traversals have node budgets and cancellation
checks. These are safety bounds, not performance claims.

## SDK surface

- Rust: `advanced_editing::form_text::{analyze_form_text, edit_form_text}`.
- JSON SDK: `advanced_editing_form_text_analyze_json` and
  `advanced_editing_form_text_edit_json` (optional supplied shaping-font bytes).
- WASM `WellfriendPdf`: `formTextSourcesJson(page)` and
  `editFormText(requestJson, fontBytes?)`; declarations are in `wellfriendpdf.d.ts`.

The edit request contains `target`, the existing `MultiRunTextRangeRequest` as
`edit`, and explicit `shared_form_policy`. WASM output does not mutate an existing
document/session. Reopen accepted bytes and use the returned target. Do not reuse
targets after unrelated edits. Password-bearing SDK input uses the facade's
existing canonicalized mutation input; these APIs do not add encrypted retained
sessions or promise encryption/profile preservation.

## Unexecuted regression source

Thirteen test functions cover inherited font shadowing, nested/shared Form cloning,
repeated Contents slots, unchanged other-page/source objects, partial deletion,
generated font placement, save/reopen/re-edit targets, stale/forged targets,
compatible/incompatible edit-all contexts, caller ActualText boundaries, exact
native offsets, inherited pattern aliases, appearance ownership, JSON reports,
the page-versus-parent fallback rule, and device-colour default-space conflicts.
None was executed.

## Remaining work

- Caller-owned ActualText migration, stream-owned tagged clone migration,
  selections across multiple Form scopes, and Do inside an open text object.
- Inherited device-colour style reconstruction when caller/Form DefaultGray,
  DefaultRGB or DefaultCMYK differ. The scanner now flags this conflict instead
  of silently replaying the command in the wrong colour space; a local colour
  selection can establish a known Form-local style.
- Annotation normal-appearance editing now has a separate source occurrence root
  over this writer; see `appearance_text_editing_implementation.md`. Widget value
  coordination and tagged appearance cloning remain open, as do pattern, Type3
  and soft-mask program editing contexts.
- General page-coordinate hit-testing for these Form-local text models.
- Universal transaction planning/approval and C/Java/.NET/Python exposure of this
  new API, plus production editor integration; WASM methods are not browser QA.
- Renderer/retained-plan/group resource-scope ownership has a subsequent source
  fix in `renderer_resource_scope_implementation.md`. Its execution evidence and
  broader paint-state/default-colour fidelity remain outstanding.
- Broader font/layout/paint correctness and all executable qualification.

Historical bytes remain in incremental output. This is genuine source editing,
not sanitizing redaction, proof of universal editing, or an Acrobat comparison.
