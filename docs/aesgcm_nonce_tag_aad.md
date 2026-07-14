# AES-GCM Nonce, Tag, and AAD Policy

Status: `implemented_with_limits`

AESV4 object protection uses the ISO/TS 32003:2023 layout recorded in the Prompt 23B clause matrix:

- IV length: 12 bytes.
- Tag length: 16 bytes.
- AAD: nil.
- Serialized object bytes: IV, then ciphertext, then tag.

Production encryption uses a fresh random IV for each encrypted string or stream and records IVs in a per-write set. A repeated IV in the same output is rejected. Fixed IVs are exposed only through test-vector helpers and are not used by production writer paths.

Decryption verifies the authentication tag before returning plaintext. Truncated payloads, changed IVs, changed ciphertext, and changed tags fail without returning decrypted bytes.
