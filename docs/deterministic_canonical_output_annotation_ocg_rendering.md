# Annotation Ocg Rendering Deterministic Canonical Output

Type3 CID Rendering closed the deterministic writer foundations. Annotation Ocg Rendering audits that path from a trust perspective and exposes `canonicalize_pdf` plus the `canonicalize` CLI.

## Behavior

- Full rewrite through the deterministic classic-xref writer.
- Stable object traversal and output serialization for the same input/options.
- Stable output SHA-256 report.
- Content-defined chunk count for audit/versioning.
- Signature impact report.

## Signature Boundary

Canonical full rewrite changes the byte sequence and invalidates existing signature ByteRanges. Use append-only incremental updates for signature-preserving structural updates, and only claim preservation after DocMDP/FieldMDP permissions are evaluated.

## Tests

`annotation_ocg_rendering_security::canonicalize_is_deterministic_and_reports_signature_impact` asserts byte-for-byte identical output and identical hashes across repeated canonicalization.
