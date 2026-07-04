# Document Versioning, Deduplication, And Write Optimization

Prompt 08 adds small deterministic helpers for future writer/versioning work.
They are intentionally separate from compression.

Implemented:

- `content_defined_chunks(data, min, avg, max)`: bounded rolling-hash chunking.
- `resource_digest(data)`: SHA-256 resource fingerprint.
- `simhash_text(text)`: deterministic near-duplicate text sketch.
- `hamming_distance(a, b)`: sketch distance.
- `resource_dedup_report(resources)`: deterministic grouping of byte-identical
  resources by SHA-256 digest.

Use cases:

- detect unchanged streams/resources across edits.
- compare reconstructed blocks across document versions.
- support deterministic write reports and future dedup decisions.

Tests:

- chunks are deterministic and cover the input.
- resource digests are stable hex SHA-256 values.
- duplicate resources are grouped by canonical input index.
- similar text has a lower SimHash distance than unrelated text.

Bounded limits:

- Rabin fingerprint chunking can replace the current rolling boundary heuristic
  later if stronger compatibility with external CDC systems is required.
- writer-level resource dedup by digest is not yet globally enabled; Prompt 08B
  provides the auditable grouping primitive.
- object-stream packing and high-effort Deflate are later optimization work.
