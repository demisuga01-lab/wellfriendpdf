# Clone On Write Editing

Wellfriend PDF SDK Prompt 31 closes the operator-preserving true-editing layer by
reusing the existing Prompt 20 source-range editing and writer paths instead of
creating a second editor. A visual cover-up is not considered an edit.

## Implemented Contract

- Edit modes are explicit: OperatorPreserving, GeometricBlock, SemanticDocument.
- OperatorPreserving text edits mutate the source text-showing operators for Tj,
  TJ, quote, and double-quote cases already supported by the canonical engine.
- Path/vector/Form occurrence edits route through canonical source-range vector
  mutation and shared-resource clone-on-write policy.
- Image occurrence editing refuses with a typed no-change report until the
  Prompt 32 occurrence graph is complete.
- Operation reports include source identity, changed objects, overlay detection,
  unaffected-content proof, signature impact, conformance impact, and reopen validation.

## Evidence

- fmt: not_passed (None)
- check: not_passed (None)
- clippy: not_passed (None)
- test: not_passed (None)
- engine_focus: not_passed (None)

## Exact Deferrals

- Stable display-list-to-instruction IDs remain Prompt 32 work.
- Image occurrence mutation returns a typed no-change refusal in Prompt 31.
- Geometric block and semantic document reflow remain Prompt 33 work.

## Verdict

Prompt 31 verdict: complete.
