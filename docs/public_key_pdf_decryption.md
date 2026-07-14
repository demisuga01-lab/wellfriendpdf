# Public-Key PDF Decryption

Status: `implemented_with_limits`

Normative source acquisition is complete for ISO 32000-2:2020 public-key security-handler clauses. The engine implements a scoped runtime path for:

- `/Filter /Adobe.PubSec`.
- `/SubFilter /adbe.pkcs7.s5` crypt-filter recipient arrays.
- CMS EnvelopedData with KeyTransRecipientInfo.
- Issuer/serial and subject-key-identifier recipient matching.
- RSAES-PKCS1-v1_5, default-parameter RSAES-OAEP, and explicit RSAES-OAEP SHA-1/SHA-256/SHA-384/SHA-512 MGF1 parameters with absent/default or empty pSource labels.
- AES-128-CBC, AES-192-CBC, and AES-256-CBC CMS content decryption.
- PubSec seed/permissions parsing and file-key derivation.
- Decryption of PDF strings and streams through the existing crypt-filter reader path.
- Full-rewrite PubSec writing/re-encryption to one or more KeyTrans recipients with fresh seed/file-key material.
- Recipient add/remove/replace by full rewrite; unchanged recipients are preserved only when explicitly supplied to the new recipient set.

Remaining exact limits:

- `/adbe.pkcs7.s3` and `/adbe.pkcs7.s4` are parsed but do not yet have external interoperability fixtures.
- Key agreement, KEK, password, and other CMS recipient forms are rejected with exact unsupported diagnostics.
- Non-empty OAEP pSpecified labels and legacy non-AES CMS content algorithms are rejected.
- PubSec encrypted incremental updates are not enabled.
- PKCS #12/PFX password-provider wrappers are enabled through explicit byte/password inputs on non-WASM builds. Encrypted PKCS #8 DER/PEM RSA private keys are also supported with explicit password bytes; managed binding callback ergonomics remain limited to supplying the final password buffer.

Detection and reporting remain fail-closed. The parser does not silently downgrade PubSec documents to Standard-handler encryption.

Trust-chain validation, OCSP/CRL, PAdES, and TSA validation are not in Prompt 23B scope.
