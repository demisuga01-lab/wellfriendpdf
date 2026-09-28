# Tagged appearance cloning - source implementation

Status: source changes only, not compiled or executed. The complete roadmap is
still active. Base is `main` at `27e62db3a1b84804339e65b6025273fd003b3736`;
the accumulated candidate is uncommitted.

## Implemented transaction

`advanced_editing::form_text::appearance::AppearanceTextEditRequest` now accepts
an optional `tagged_clone` decision. Missing options retain the reject policy,
now also checking whole-annotation OBJR ownership that the old stream-only guard
could miss. The native writer records the source-to-clone mapping for the leaf and
all copied ancestors. The selected normal appearance is staged privately, then
`tagged_structure::stream_clones` reconciles its logical ownership before the
API returns any bytes. No intermediate candidate is externally published.

Policies:

- `reject`: retain the existing stream-owned tag guard.
- `move_exclusive_namespaces`: move MCR/OBJR carriers only when the original
  namespace no longer appears in page or normal/rollover/down appearance calls.
- `split_shared_namespaces_after_source`: explicitly approve keeping the old
  carrier and inserting the new carrier immediately after it under the same
  logical StructElem. This decision does not infer a new role or reading order.

The original structural graph must already pass the canonical ParentTree
validator. Direct-owner normalization and malformed-graph repair remain separate
transactions. Clone IDs must be fresh and unique, all new programs must belong
to the selected appearance graph, original programs must remain unchanged, and
every cloned tagged namespace must retain exactly its source MCID set.

All mapped MCR `/Stm` and stream OBJR `/Obj` carriers update together. New root
appearance carriers bind `/StmOwn` to the annotation. Nested ordinary Forms do
not acquire a fabricated annotation owner when the source omitted `/StmOwn`.
Explicit incompatible owners are not silently reassigned. StructElem identity,
role, ID, unrelated children and attributes are retained. The complete canonical
ParentTree/IDTree writer assigns collision-free owner keys and validates the
reopened result. The edit report includes `tagged_ownership`; its validation is
not PDF/UA conformance certification.

The actual-program scanner does not mistake old MCR references for evidence that
a stream is still painted. It inspects all AP N/R/D state programs, including
inactive states. For patterns, Type 3 font programs and soft masks, a bounded
resource-reference traversal conservatively retains a source carrier even when
the resource may be unused. Those program kinds do not gain an execution-level
liveness claim from this change. A retained explicit StmOwn must still name a
real owner; moving between annotation and non-annotation ownership is not an
implicit side effect of splitting. Unknown top-level AP metadata is preserved and
is not interpreted as a stream or state dictionary.

The scanner also now applies the original page's Resources when a Form omits
Resources or sets it to null; it no longer substitutes the immediately enclosing
Form's resource dictionary. Explicit local resources remain authoritative.

## Logical replacement text

Changing glyphs does not justify retaining an affected logical `/ActualText`.
Every affected StructElem and its ancestors is inspected. Whole-annotation OBJR
owners participate even when the appearance itself contains no MCIDs.

For each existing structural ActualText override, supply an exact update:

```json
{
  "tagged_clone": {
    "policy": "move_exclusive_namespaces",
    "actual_text_updates": [
      {
        "element": [13, 0],
        "expected_text": "old complete logical wording",
        "replacement_text": "new complete logical wording"
      }
    ]
  }
}
```

The writer compares expected text with the current PDF string, writes the new
Unicode value, and rejects missing, stale, duplicate, unrelated or unmatched
updates. An ancestor may cover more than the selected appearance, so its complete
replacement wording is a user decision, not a guessed substring replacement.
Source content-stream ActualText cleanup continues to use the native text writer.
Caller-owned content-stream ActualText remains a separate complete-owner edit.

## Integration and regression source

The option and resulting report pass through the existing Rust JSON SDK and
WASM appearance-edit method. TypeScript defines the policy/options payload.
Existing untagged requests remain unchanged; default tagged edits cannot bypass
logical-text protection through an annotation-only OBJR.

14 unexecuted regression functions cover exclusive and shared-state migration,
distinct ParentTree keys, nested repeated Forms, structural text compare-and-swap,
ancestor overrides, whole-annotation OBJR text, repeated editing, source/ID
preservation, JSON/default-option behavior, stale clone IDs, cancellation,
resource fallback, conservative soft-mask sharing and changed-MCID refusal.

Rustfmt syntax/formatting and Git whitespace checks passed for this increment.
No compiler, Cargo, build, test, PDF workload, rendering, benchmark, binding
execution, deployment, commit or push was run. These source assertions are not
runtime results.

## Remaining scope

This is not general tagged-object cloning: page Form/vector targets, arbitrary
non-annotation stream owners, separate semantic roles for shared occurrences,
MCID renumbering/splitting, direct appearance materialization, widgets/field-value
coordination and cross-program selection still need their own transactions.
Universal plan/approval now carries this route through the existing JSON
bindings; see `governed_scoped_text_implementation.md`. Richer managed request
models and actual native/binding execution remain unqualified.
Incremental history remains and is not sanitizing redaction. No universal-editor
or Acrobat-superiority claim is established.

Ownership interpretation follows the MCR entry definitions in
[ISO 32000-2, section 14.7.4.2](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf).
The standard defines the ownership fields; it does not certify this implementation.
