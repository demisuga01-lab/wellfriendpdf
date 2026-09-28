# Selected annotation appearance text — source implementation

Status: source implementation, not compiled or runtime-qualified. The complete
roadmap remains active. Base: `main` at
`27e62db3a1b84804339e65b6025273fd003b3736`; accumulated work remains uncommitted.

## Implemented route

`advanced_editing::form_text::appearance` provides `analyze_appearance_text` and
`edit_appearance_text`. Its public target binds exact input bytes, page,
annotation-array slot, annotation object, selected normal state, appearance
stream, and nested source Do path. There is no fictitious page Contents slot or
root Do operation in this target. Coordinates precede the selected source Form's
Matrix; the inventory includes source BBox and annotation Rect.

The internal source scope now distinguishes page Form and annotation appearance
roots. Both use the existing multi-run writer, font resources, ActualText
cleanup, source advance compensation and inline/generated source placement.
Appearance entry has independent initial graphics/text state. Nested Forms use
the same object-bound inherited-font/paint-resource logic and page fallback as
page Form editing. The page is neither flattened nor replaced with a surrogate.

The selected leaf and every selected ancestor are copied. Only the selected
normal AP stream/state is rebound on its unique owning annotation. Shared AP
and state dictionaries become owner-local dictionary copies. Other normal
states, rollover/down streams, source Form dictionaries, page content and other
annotation occurrences remain unchanged. Stream Matrix/BBox/Group entries are
retained. Canonical annotation identity checks reject duplicate page ownership;
tagged-stream ownership guards prevent uncoordinated tag cloning. An explicit
tagged-clone option now composes with canonical MCR/OBJR/ParentTree migration;
see `tagged_appearance_clone_implementation.md` for its policies and limits.

The code reopens the result, rediscovers the same source location, checks the
whole expected direct text where the pre-edit logical/extracted mapping agrees,
compares every original source program, and compares annotation metadata plus
the exact intended AP rebind. The report returns a new hash-bound target for
another edit. These are implemented postconditions, not evidence of execution.

## Explicit metadata policy

The required `metadata_policy` is one of:

- `preserve_annotation_metadata`: change source appearance text but retain
  Contents/comments and all other annotation metadata. Annotation comments do
  not generally mean the text painted by a stamp.
- `synchronize_free_text_plain_text`: root FreeText only. Require Contents,
  whole selected appearance text, direct extracted text and the native logical
  model to agree before mutation. Update Contents atomically and remove RC under
  this explicit plain-text choice. Report `rich_text_discarded` and
  `annotation_contents_updated`; do not imply rich-style preservation.

Widget appearances require coordinated field values/default appearance and are
not silently edited independently. Source-local text mutation is not permanent
redaction: incremental historical bytes remain.

## Integration

- JSON SDK: `advanced_editing_appearance_text_analyze_json` and
  `advanced_editing_appearance_text_edit_json`.
- WASM: `appearanceTextSourcesJson(page)` and
  `editAppearanceText(requestJson, fontBytes?)`, with TypeScript target/policy
  declarations.
- Request JSON: `target`, existing `MultiRunTextRangeRequest` as `edit`,
  `metadata_policy`, and optional `tagged_clone` (default reject). Optional font
  bytes use the existing source writer.

WASM returns a new output; it does not mutate a retained document/editor session.
Reopen accepted bytes and use `target_after`. Password-bearing facade input uses
the existing canonicalized mutation input, not a new encrypted-session contract.

## Unexecuted regression source

13 added regression functions cover direct and nested shared appearances,
repeated nested Do isolation, original page/program preservation, unchanged
normal/rollover/down states, Matrix/BBox/Group preservation, repeated editing,
stale/forged targets, partial removal, direct normal-stream slots, FreeText
synchronization and mismatch, widget/tag boundaries, duplicate annotation
ownership, JSON SDK output, generated font ownership and cancellation.

Rustfmt syntax/formatting and whitespace checks passed. No compiler, Cargo,
build, test, PDF workload, renderer, benchmark, binding execution, deployment,
commit or push was run. No runtime correctness or visual fidelity is claimed.

## Remaining requirements

- Broader tagged stream owners and caller-owned content-stream ActualText. The
  selected appearance migration route is described in the follow-up report.
- Ordinary widget field value/default appearance coordination is implemented in
  source by `widget_text_transaction_implementation.md`. Specialized/multi-state
  edits still need explicitly governed transactions.
- Direct annotation/appearance source materialization through canonical owners.
- Appearance-aware page hit-testing/search and source-aware permanent redaction.
- Governed universal source discovery/plan/approval and pinned-font disclosure
  are now connected through the shared JSON routes; see
  `governed_scoped_text_implementation.md`. Rendered candidate preview and a native
  browser panel are wired by `scoped_candidate_preview_implementation.md`;
  richer managed request models and production integration remain. Source
  routing is not cross-binding execution or a completed editing UI.
- All current-build, save/reopen, independent-render/extract, difficult-corpus
  and performance evidence. Universal editing and Acrobat superiority remain
  unestablished.
