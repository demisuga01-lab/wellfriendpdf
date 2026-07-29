# Semantic Privacy Policy

Semantic extraction, ParentTree recovery, dictionary segmentation, table
extraction, search, and advanced chunking are local deterministic operations.
No ML runtime is required.

## Defaults

- Cloud upload: disabled.
- Hidden telemetry: none.
- Model weights: none bundled.
- Provider credentials: never included in reports.
- External reference model downloads: disabled by the benchmark.
- Raw text: preserved; dictionary/model layers do not rewrite it.

## Cloud Proposal Boundary

Setting a backend name is not consent to upload. A cloud adapter must verify an
explicit endpoint, API-key environment-variable name, payload policy, upload
flag, and privacy acknowledgement. Missing or malformed authorization fails
closed. Endpoint and environment-variable names may be diagnostic metadata;
secret values may not be logged or serialized.

## Chunking Original Input

Chunking an original PDF reports
`document_state=original_input_not_asserted_sanitized`. It may also report
hidden-content and active-content warnings. Applications must not interpret a
successful chunk operation as sanitization.

## Chunking Redacted Output

Redaction must first produce rewritten PDF bytes. Chunk those new bytes, not an
in-memory pre-redaction model. The sanitized/redaction posture marks signatures
as potentially invalidated and states that removed content is not included.
Semantic Closeout never reconstructs removed text.

Security and signature fields are posture evidence, not a substitute for an
application's authorization, retention, or legal compliance policy.
