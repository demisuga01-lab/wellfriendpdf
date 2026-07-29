# document security Implementation

document security closes accessibility repair, true redaction, full rewrite history
removal, sanitization, residual-data verification, and undo exposure.

## Runtime entry points

The core module is `wellfriendpdf_engine::document_security`. It exposes:

- `document_security_feature_matrix`
- `analyze_document_security`
- `plan_document_security`
- `apply_document_security`
- `undo_document_security`
- `verify_residual_data`

The SDK facade exposes JSON-compatible helpers, and Rust, CLI, Python, C ABI,
WASM, .NET, and Java wrappers call those same core helpers.

## Implemented operations

Supported document security actions include structure inspection and repair, document
language and metadata edits, ParentTree rebuild, repair-after-mutation,
text/region/semantic/annotation/form/metadata/attachment redaction, sanitizer
presets, residual verification, full rewrite history removal, and preimage undo.

Mutating actions serialize through the canonical writer path and reopen the
result before reporting success. Destructive actions require explicit approval
or full-rewrite acknowledgement, depending on the operation.

## Transaction and provenance model

Reports include preconditions, input/output hashes, read/write sets,
affected-page/object summaries, validation notes, accessibility effects,
signature/history impact, and an inverse operation description. Source evidence
is reported from object references, semantic extraction, security sanitizer
reports, and redaction verification.

## Minimum-verification posture

document security reports `implementation complete_validation_deferred` only after the
minimum document security gates pass. Exhaustive validation remains a release validation
responsibility.
