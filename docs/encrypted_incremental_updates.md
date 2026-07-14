# Encrypted Incremental Updates

Status: `implemented_with_limits`

Existing unencrypted incremental updates preserve the original byte prefix and append deterministic xref/trailer data.

AESV4 encrypted incremental updates are not enabled in this closure. The safe policy is:

- Full rewrite may create AESV4 encrypted output.
- Existing AESV4 documents can be opened with the correct password and written as decrypted or re-encrypted full rewrites.
- Incremental AESV4 writes return exact unsupported status until nonce uniqueness across revisions, encryption-dictionary reuse, and signature impact are proven.
- PubSec full-rewrite re-encryption and recipient mutation are implemented for scoped KeyTrans recipients, but PubSec incremental updates remain unsupported until recipient/encryption-dictionary reuse, new-object encryption, and signature impact are proven.

Linearization is invalidated by full rewrite and is not preserved by encrypted incremental updates.
