# DocMDP and FieldMDP policy

Wellfriend parses signature `Reference` arrays, `TransformMethod`, `TransformParams`, DocMDP `P`, FieldMDP `Action` and `Fields`, and conflicting or malformed entries. `P=1` blocks changes; `P=2` is limited to form-oriented changes; `P=3` may permit annotation-oriented incremental changes. Field locks are applied structurally to form edits.

This layer does not convert structural permission into a cryptographic or viewer-acceptance claim. Cryptographic verification is run separately, and viewer enforcement remains implementation dependent.
