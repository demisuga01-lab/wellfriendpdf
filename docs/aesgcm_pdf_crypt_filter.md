# AES-GCM PDF Crypt Filter

Status: `implemented_with_limits`

Normative source: ISO/TS 32003:2023, clauses 5.1 and 5.2, Tables 2-4. The local specification PDF is recorded by filename and SHA-256 in `target/crypto_writer-writer-crypto/normative-source-manifest-crypto_writer_closeout.json`; the PDF itself is not committed.

Implemented mapping:

- Standard security handler `V=6`, `R=7`.
- Crypt filter method `/AESV4`.
- Crypt-filter key length 32 bytes.
- Per-object representation `12-byte IV || ciphertext || 16-byte tag`.
- Associated authenticated data is nil.
- No pre-encryption padding.
- Production writer IVs come from OS CSPRNG and are collision-tracked per output.

Limits:

- Public-key `Adobe.PubSec` recipient processing is separate; scoped KeyTrans decrypt/full-rewrite write/re-encrypt is implemented, but non-KeyTrans recipient classes remain unsupported.
- Encrypted incremental update is not enabled for AESV4 until cross-revision nonce uniqueness is proven.
- ISO/TS 32004 PDF-MAC validation is not implied by successful AES-GCM tag verification.
