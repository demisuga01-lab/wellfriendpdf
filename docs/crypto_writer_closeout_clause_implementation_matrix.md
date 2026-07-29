# crypto writer closeout Clause Implementation Matrix

This matrix records the clause-to-code contract before implementation. It uses
clause identifiers and derived engineering notes only; it does not redistribute
standards text.

| Feature | Source clauses | Implementation target | Security boundary |
| --- | --- | --- | --- |
| PubSec encryption dictionary | ISO 32000-2:2020 7.6.5.2, Table 23 | Parse handler filter, SubFilter, recipient locations, crypt-filter references | Unsupported variants fail with exact diagnostics |
| PubSec payload and file key | ISO 32000-2:2020 7.6.5.3, Figure 4, Table 24 | Recover seed and permission payload from CMS and derive file key | Recovered key material is zeroized and not logged |
| PubSec crypt filters | ISO 32000-2:2020 7.6.6, Tables 25-27 | Select stream, string, embedded-file, and explicit Crypt filters | Unknown CFM never downgrades to RC4 |
| CMS EnvelopedData | RFC 5652 sections 3, 6.1, 6.2, 6.2.1, 10.2.4 | Parse ContentInfo, EnvelopedData, KeyTransRecipientInfo, issuer/serial, SKI | Unsupported recipient types are exact unsupported rows |
| RSA key transport | RFC 8017 plus selected CMS OIDs | Use audited RSA primitives for supported KeyTrans algorithms | Decryption failures do not expose padding details |
| Certificate matching | RFC 5280 and RFC 5652 | Match DER fingerprint, issuer/serial, and subject key identifier | Matching is not trust validation |
| AESV4 dictionary | ISO/TS 32003:2023 5.1, Tables 2-4 | Parse/write V=6, R=7, and CFM=AESV4 | Random IVs are intentional non-determinism |
| AESV4 object layout | ISO/TS 32003:2023 5.2 | Use 12-byte IV, ciphertext, 16-byte tag, nil AAD, no padding | No plaintext before tag verification |
| AESV4 IV uniqueness | ISO/TS 32003:2023 5.2 | Use OS CSPRNG and per-write collision tracking | Fixed IVs only in test-only vector helpers |
| PDF MAC reporting | ISO/TS 32004:2024 5.1, 5.2, 6.1-6.6 | Runtime report/verify posture parses trailer `AuthCode`, location shape, `ByteRange`, CMS AuthenticatedData envelope, and `KDFSalt`; verification returns `valid` only after CMS, unwrap, HMAC, ByteRange, and digest checks pass | AES-GCM object authentication is not document-level MAC validation; no `valid` state is returned from structure-only inspection |
| PDF MAC PasswordRecipientInfo | ISO/TS 32004:2024 6.3-6.4; RFC 5652 6.2.4 | Generate and verify the required PasswordRecipientInfo profile for standalone PDF-MAC tokens | Exactly one recipient is accepted; unsupported recipient classes are exact rows |
| PDF MAC HKDF/AES-KW/HMAC | ISO/TS 32004:2024 6.3-6.5; RFC 5869; RFC 3394; RFC 4231 | Derive the wrap key, unwrap the MAC key, verify HMAC-SHA256, and reject bad unwrap/MAC | Keys and unwrap buffers are zeroized where practical and never reported |
| PDF MAC ByteRange digest | ISO/TS 32004:2024 5.2.2, 6.6.2 | Validate Standalone ByteRange bounds and compare covered-byte SHA-256 digest | Bad ranges or digest mismatch never return document-integrity `valid` |
| PDF MAC writer patching | ISO/TS 32004:2024 5.2.2, 6.6.2 | AESV4 full rewrite reserves trailer `AuthCode`/`MAC` placeholders, stabilizes ByteRange, patches CMS token, and verifies covered bytes through Rust, CLI, C ABI, Python, .NET, and Java surfaces | AttachedToSig and non-SHA256 profiles remain unsupported exact |
| Provider encrypted PKCS#8 | PKCS #8 / RFC 5958 | Load DER/PEM encrypted PKCS#8 RSA keys with explicit password | Wrong-password diagnostics do not echo passwords or key bytes |
| Provider PKCS#12/PFX | RFC 7292 | Load bounded PFX bundles with private-key/certificate matching | `implemented_with_limits`; non-WASM PubSec provider accepts one unambiguous RSA identity with explicit password bytes; ambiguous bundles, unsupported algorithms, wrong passwords, malformed containers, and WASM extraction return exact errors |
