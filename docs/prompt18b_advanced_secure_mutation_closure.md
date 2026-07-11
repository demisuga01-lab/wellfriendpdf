# Prompt 18B advanced secure mutation closure

Prompt 18B starts from verified commit `261968c8e70012d563f2282200159e51779b0e0c` with a clean worktree. It extends Prompt 18 without replacing its fail-closed redaction, attachment security, signature taxonomy, deterministic writer, or exact-prefix incremental guarantees.

The canonical paths are `editing.rs` for packed, ICCBased, and inline mutation; `content/parser.rs` for distinct inline dictionaries; `prompt18.rs` for owner-specific associated files and enforced signature policy; and `sdk.rs` for the shared public surface. `crates/engine/tests/prompt18b_advanced_secure_mutation.rs` is the executable closure proof.

| Row | Status | Boundary |
|---|---|---|
| packed 1-bit stencil | implemented | lossless filters, invertible placement, exact rows |
| Indexed 1/2/4/8-bit | implemented | DeviceGray/RGB/CMYK lookup bases |
| ICCBased Gray/RGB/CMYK | implemented with limits | profile `/N` is 1, 3, or 4 |
| explicit mask and soft mask | implemented | supported masks are cloned and rewritten |
| inline PNG/TIFF predictor | implemented | matching bounded layout parameters |
| inline ImageMask and promotion | implemented | supported unambiguous images |
| catalog/page/annotation/structure/Form/XObject AF | implemented | indirect supported owners |
| incremental form/annotation/page property | implemented | prefix, reopen, visible-state proof |
| DocMDP/FieldMDP enforcement | implemented with limits | structural, not trust validation |

Unsupported codecs, malformed rows, high-channel ICC profiles, ambiguous inline dictionaries, external file specifications, and unsupported owner families are exact fail-closed cases. Secure redaction and destructive cleanup remain full rewrites with signature invalidation risk.
