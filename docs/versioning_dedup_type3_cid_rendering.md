# Versioning And Dedup In Type3 CID Rendering

Advanced Rendering already added:

- `content_defined_chunks`
- `resource_digest`
- `simhash_text`
- `hamming_distance`

Type3 CID Rendering adds:

- `resource_dedup_report(resources)`: deterministic SHA-256 grouping of byte-identical resources.

The helper reports:

- input count.
- unique count.
- duplicate count.
- canonical resource index per digest.
- duplicate indices per digest.

This is intentionally not a compression engine. It gives the writer/conversion layer stable evidence for future dedup decisions without silently merging PDF objects that may have different semantics.

Limits:

- writer-global resource dedup is not automatically enabled.
- MinHash is not implemented because SimHash covers current near-duplicate text tests.
- object packing and Zopfli-class Deflate are not Type3 CID Rendering blockers.
