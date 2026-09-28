# Governed native scoped text - source implementation

Status: source changes only. No current build or runtime qualification. The
complete roadmap remains active; base `main` is
`27e62db3a1b84804339e65b6025273fd003b3736`, with uncommitted accumulated work.

## Discovery through existing bindings

Universal document analysis accepts `include_scoped_text_sources: true` for the
selected page window. `scoped_text_sources.pages` contains the native Form and
selected normal appearance inventories, including exact revision, source path,
annotation/state identities and source-local logical ranges. It does not pretend
these coordinates are page hit-testing or merge them into page-logical offsets.
Discovery is optional, cancellation-aware, and has aggregate occurrence and
serialized-report limits. Request a smaller page window when those limits apply.

This uses the existing Rust JSON SDK, HTTP, C, Java, .NET, Python and WASM
universal-analysis routes; no new FFI symbol or platform-specific edit engine is
needed. This statement describes inspected source routing, not binding execution.

## Operation contract

The new universal operation is `scoped_text`:

```json
{
  "operation": {
    "kind": "scoped_text",
    "request": {
      "source": {
        "scope": "appearance",
        "request": {
          "target": "use the exact appearance target object from discovery",
          "edit": "use the native MultiRunTextRangeRequest object",
          "metadata_policy": "preserve_annotation_metadata"
        }
      }
    }
  }
}
```

The two strings above are explanatory placeholders, not valid target/edit
payloads. `scope: form` instead carries the existing `FormTextEditRequest`,
including explicit clone-one/edit-all policy. Appearance requests retain their
metadata policy and optional tagged-clone/structural ActualText decisions.
Logical offsets and edit regions belong to the selected source scope.

Optional `approved_font_asset` carries `lookup_name` and exact font `bytes`.
When absent, the planner pins the bundled font already used by the native writer
into the execution request. The request's `planned_output_sha256` is
planner-owned and must be omitted in a new request. Output receipt and pinned
program bytes enter the canonical plan and approval digest.

## Planning, approval and publication

Planning calls the native source writer into private memory. Its normal source,
tag, metadata, extraction, embedding and signature checks still apply. Nested
signature override and determinism options are normalized to the universal
policy; a caller cannot independently bypass the governing mutation mode.

The plan discloses the actual candidate output hash, native report, rewritten
object definitions, new definitions, generated font objects, metadata/tag choices
and affected-page invalidation. A writer refusal becomes a non-applicable plan,
not a ready candidate that merely defers the same refusal until apply.

New font definitions are a conservative typography-change gate. They require
`allow_font_substitution` and approval of the exact candidate lookup name. This
also gates re-encoding/subsetting a family rather than claiming identical
typography from its name. An unused pinned asset is disclosed as unused, not
reported as an actual font change. Native shaping and embedding checks remain
authoritative for the generated program.

Approval must select exactly the planned occurrence and acknowledge its visual
or structural changes. Missing, duplicate, different-scope and unrelated font
choices are rejected. Generic signature, input-recovery, output-security,
conformance and edit-contract gates still surround this route.

Apply recomputes the canonical plan and retains that private candidate. After
the complete plan and approval match, it publishes those same bytes through
the existing final-output gates; it does not perform a third source mutation.
The plan's embedded preview remains a source/output report. A separate bounded
before/candidate rendering API and shared-session browser panel now consume this
canonical plan; see `scoped_candidate_preview_implementation.md`. No independent
renderer or visual-fidelity certification is claimed.

## Write inventory and invalidation

The reader exposes a crate-private append-only plaintext definition comparison.
It verifies the original prefix and retained source identities, compares xref
definitions, and includes compressed objects when their ObjStm is rewritten.
This avoids decoding every unchanged object merely to discover the write set.
The report calls these written definitions: a dictionary can be rewritten with
an equivalent value. Newly created Form resources are listed separately from
other newly generated objects; a new font or ParentTree node is not falsely
labelled an original-resource clone.

Clone-one untagged edits invalidate the selected page. Edit-all and tagged
appearance edits conservatively invalidate document pages, including possible
cross-page logical-owner effects. They do not claim an exact minimal dirty-page
set. Final normalization or output encryption can change revision identities;
native `target_after` belongs to the candidate bytes and must be rediscovered
after those transformations.

## Validation status

21 unexecuted regression functions cover the complete native plan/approve/apply
route, Form clone-one/edit-all, shared contexts, appearance metadata/tags,
generated-font denial and exact approval, caller font bytes, stale/forged
revisions/receipts, JSON facade routing, page-window discovery, signature-option
normalization, cancellation, and xref/prefix/ObjStm write inventory.

Rustfmt syntax/formatting and Git whitespace checks passed. No compiler,
typecheck, Cargo, build, tests, PDF workloads, rendering, benchmark, binding
execution, deployment, commit or push was run. No test is reported as passing.

## Remaining roadmap work

Source-local editing retains the native boundaries: caller-owned ActualText,
broader tagged Form/vector migration, specialized/stateful widget coordination, direct owner
materialization and cross-program selection need further transactions.
Page-coordinate hit-testing, richer managed request types and actual cross-binding
qualification remain. The rendered candidate-preview UI now has source wiring,
not executed evidence. Performance still needs evidence: native staging occurs
once during planning, again for each candidate preview, and once during canonical
apply, and source discovery currently reopens inputs per native page
inventory. This increment does not establish universal or Adobe-beating editing.
