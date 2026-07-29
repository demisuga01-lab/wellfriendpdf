# secure mutation secure mutation audit

secure mutation starts from verified commit `d0842aae76b536f8ccc82d26f1a5a8054889ad49` with a clean worktree. The canonical implementation paths are `crates/engine/src/editing.rs` for page/image content mutation, `crates/engine/src/secure_mutation.rs` for associated files and signature policy, `crates/engine/src/writer.rs` for full and incremental serialization, and `crates/engine/src/sdk.rs` for binding parity.

## Security model

- An overlay is never counted as redaction.
- Supported 8-bit Gray/RGB/CMYK Image XObjects are cloned and rewritten in sample space. The affected clone omits `Mask` and `SMask`, so hidden alpha and mask data are not reachable from that invocation. An unaffected shared invocation may retain the original resource.
- Supported inline images are parsed by the stateful BI/ID/EI tokenizer, decoded, rewritten, and deterministically Flate encoded. No raw `EI` substring search is used.
- Unsupported image paths remove the complete affected invocation or return an explicit fail-closed error.
- Associated files are decoded with scheduler and byte caps. Extraction returns owned bytes and a sanitized single-component name; it never executes or fetches a target.
- Secure associated-file removal uses a full rewrite. Existing file specs have `EF` and `RF` removed before the canonical EmbeddedFiles tree is rebuilt.
- Signature reports keep ByteRange coverage, cryptographic validity, trust, revision coverage, DocMDP, FieldMDP, signature value preservation, semantic change, and viewer posture separate.

## Canonical paths inspected

- Inline parser: `crates/engine/src/content/tokenizer.rs` and `content/parser.rs`.
- Image decode/re-encode: `crates/engine/src/images/decoder.rs`, `images/encoder.rs`, and `images/smask.rs`.
- Existing redaction: `crates/engine/src/editing.rs`.
- Attachment compatibility API: `crates/engine/src/attachments.rs`.
- Signature crypto: `crates/engine/src/signature.rs`.
- Deterministic full/incremental writer: `crates/engine/src/writer.rs`.
- Public bindings: `crates/wellfriendpdf-py`, `crates/wellfriendpdf-capi`, `crates/wellfriendpdf-wasm`, `bindings/dotnet`, and `bindings/java`.

## Executable proof

`crates/engine/tests/secure_mutation_secure_mutation.rs` reopens output, checks affected mask references, decodes rewritten inline samples, compares deterministic output bytes, exercises add/extract/dedup/remove/rescan, and verifies an incremental output begins with the exact original byte sequence. `scripts/secure_mutation_secure_mutation_audit.py` runs that suite serially and creates the required target-local audit bundle.

The stable feature matrix is `target/secure_mutation-mask-inline-associated-signatures/secure_mutation-feature-matrix.json`. No secure mutation row is blocked. Exact bounded limits are documented in `docs/secure_mutation_known_limits.md`.
