# Key and Certificate Provider

Status: `implemented_with_limits`

crypto writer closeout source acquisition cleared the normative blocker for PDF public-key security handlers. The engine now has explicit in-memory providers for scoped PubSec decrypt/open plus certificate-only recipient inputs for PubSec writer/re-encryption.

Implemented provider properties:

- Explicit caller-supplied certificate and private-key candidates.
- PEM/DER certificate support.
- PKCS#8 and PKCS#1 RSA private-key support.
- Encrypted PKCS#8 DER/PEM RSA private-key support with an explicit operation-scoped password.
- Deterministic recipient candidate ordering.
- Issuer/serial and subject-key-identifier matching.
- Public certificate parsing for recipient generation.
- No trust-chain, revocation, or signer-identity claims.
- Zeroization for recovered CMS content keys, seed payloads, and file keys where practical.

Current API posture:

- Standard password-based encryption and AESV4 are available.
- `Adobe.PubSec` private-key operations are enabled only when a caller supplies an explicit provider.
- PubSec add/remove/replace recipient workflows are implemented as full rewrite with fresh seed/file-key material.
- PKCS#12/PFX RSA certificate/private-key bundles are supported on non-WASM builds with explicit password bytes and bounded extraction.
- OS certificate stores, HSM/PKCS#11 adapters, ambiguous/non-RSA PFX bundles, and non-RSA keys remain exact unsupported limits.
- CLI, Python, C ABI, .NET, and Java expose explicit byte/path wrapper surfaces; password-callback and hardware-backed provider lifecycles remain exact unsupported limits.

Encrypted PKCS#8 notes:

- Wrong passwords return an encrypted-key failure and do not echo the password.
- Decrypted private-key material is kept operation-scoped and is not serialized into JSON reports.
- Tests cover valid encrypted PKCS#8 loading and wrong-password rejection.

PKCS#12/PFX loading uses the bounded `p12` dependency behind the PubSec provider boundary. The supported profile requires exactly one unambiguous RSA certificate/private-key identity after MAC verification and bag extraction; wrong passwords, malformed containers, unsupported bag algorithms, duplicate matches, and non-RSA keys return exact errors without serializing private-key or password material.
