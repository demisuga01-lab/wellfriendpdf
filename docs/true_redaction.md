# True Redaction

Prompt 35 redaction edits real source content. It does not complete redaction by
painting a cover rectangle, hiding content, clipping content, or adding a visual
overlay over searchable text.

Supported redaction actions include:

- text-term redaction;
- page-region redaction;
- semantic-node redaction when the node resolves to source geometry;
- annotation-region redaction;
- form-field redaction;
- metadata and attachment removal through sanitization/canonicalization;
- post-redaction residual verification.

Destructive redaction paths require explicit approval and full-rewrite
acknowledgement when residual history removal is part of the requested policy.
