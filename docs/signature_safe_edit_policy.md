# Signature-safe edit policy

The shared decision is one of `safe_incremental`, `incremental_with_warning`, `full_rewrite_required`, `blocked_by_signature_policy`, or `explicit_override_required`.

The report separately states ByteRange and revision coverage, append-only status, cryptographic validity, trust, modification after signing, DocMDP/FieldMDP structure, certification/approval/timestamp type, DSS/LTV presence, signature value and appearance preservation, semantic preservation, and viewer warning risk.

An append-only update can leave covered bytes and the signature dictionary untouched while still changing signed-document semantics and producing a viewer warning. Redaction, sanitizer removal, attachment removal, XFA flattening, canonicalization, and full rewrite normally require an explicit signed-semantics override.
