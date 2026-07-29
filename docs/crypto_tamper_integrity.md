# Crypto Tamper and Integrity

Status: `implemented_with_limits`

AES-GCM object authentication and ISO/TS 32004 PDF-MAC integrity are separate checks.

Implemented AESV4 tamper handling:

- Ciphertext bit changes fail with `authentication_failure`.
- IV changes fail with `authentication_failure`.
- Tag changes fail with `authentication_failure`.
- Truncated payloads fail closed.
- Reader object access does not return unauthenticated stream bytes.

PDF-MAC status:

- ISO/TS 32004 clauses are mapped for reporting boundaries.
- Runtime PDF-MAC structure discovery is implemented for trailer `AuthCode`, location, byte range, CMS AuthenticatedData envelope, and encryption-dictionary `KDFSalt` posture.
- Standalone PDF-MAC creation and verification are implemented for AESV4 full rewrite using the mapped PasswordRecipientInfo profile, pdfMacWrapKdf, AES-256-KW, HMAC-SHA256, SHA-256 authenticated attributes, and covered ByteRange digest comparison.
- The writer reserves a deterministic trailer `/AuthCode` placeholder, iterates until ByteRange offsets stabilize, patches the final CMS token in place, and verifies that covered bytes did not change during patching.
- AttachedToSig extraction/binding and non-SHA256 profiles remain exact unsupported PDF-MAC limits. PKCS #12/PFX provider extraction is implemented for bounded non-WASM PubSec RSA identities and remains unsupported for ambiguous/non-RSA bundles and WASM extraction.
- Reports must not claim document-level integrity validity from AES-GCM object tag success alone.

Tamper handling added in crypto writer closeout:

- Bad AES-KW integrity returns `authentication_failed` and clears the unwrap buffer.
- Bad HMAC returns `authentication_failed`.
- Bad covered ByteRange digest returns `invalid`.
- Malformed AuthenticatedData, wrong algorithms, duplicate/missing authenticated attributes, or invalid ByteRange never return `valid`.
