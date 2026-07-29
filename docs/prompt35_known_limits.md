# Prompt 35 Known Limits

Prompt 35 implements production code paths for accessibility repair, redaction,
sanitization, residual verification, and undo, but does not claim exhaustive
release validation.

Known boundaries:

- inaccessible or contradictory tagged-PDF structures can require manual review;
- structure updates that cannot be expressed through current canonical APIs are
  refused;
- signature-protected edits follow the existing signature policy;
- semantic redaction requires a resolvable source-linked semantic node;
- dynamic or viewer-specific behavior is not treated as verified accessibility;
- complete corpus-level accessibility, standards, differential, and historical
  gate replay is deferred to Prompt 36.
