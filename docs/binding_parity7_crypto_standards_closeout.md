# Crypto Standards Fuzz crypto and standards close-out

Crypto Standards Fuzz does not add new cryptographic formats by default. It closes the release state
across crypto writer closeout through Incremental Signing Standards by recording current support, exact limits, binding
parity, interoperability evidence, fuzz coverage, and VPS validation status.

The close-out matrix is generated with:

```bash
python scripts/crypto_standards_fuzz_closeout_reports.py
```

Primary output:

- `target/crypto_standards_fuzz-verapdf-crypto-fuzz/crypto-standards-closeout-matrix.json`

Covered areas:

- password encryption/decryption
- public-key security handler and PubSec recipients
- AES-GCM / AESV4 posture
- PDF-MAC posture
- CMS SignedData and PAdES baseline validation
- OCSP/CRL/TSA/DSS/VRI/LTV evidence handling
- DocMDP/FieldMDP enforcement
- signature-preserving edits and incremental signing
- PDF/A, PDF/UA, PDF/X validation and cross-profile conflicts
- WebAssembly, OS trust-store, and external signer constraints

Release-critical rows must have real VPS evidence. Unavailable optional external tools are
recorded as unavailable and are not converted into passes.
