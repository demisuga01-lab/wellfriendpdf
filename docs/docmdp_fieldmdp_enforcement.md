# DocMDP and FieldMDP enforcement

Oxide structurally parses signature references, DocMDP `P`, FieldMDP `Action`, bounded field lists, ByteRange/revision reports, and malformed policy structures before mutation.

- P=1 blocks represented mutations.
- P=2 permits supported form filling but blocks annotation and page-property changes.
- P=3 permits supported form filling and annotations; page/content/property changes remain blocked.
- FieldMDP `All`, `Include`, and `Exclude` are evaluated against the target field. Malformed actions fail closed.

This is mutation-policy enforcement, not trust-chain validation. Viewer certification may be stricter, cryptographic verification is separate, and override is explicit rather than silent.
