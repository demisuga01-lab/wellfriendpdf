# crypto writer closeout Normative Sources

Status: `source_gate_passed_for_implementation`

crypto writer closeout originally stopped at `blocked_normative_dependency` because the
PDF-family specifications were not available in the workspace. The blocker
evidence is preserved in the crypto writer closeout audit. The roadmap task resumed from
`40556fb1f48cd1035f0767b78afbfe1c2034bb36` after the required standards were
made available as ignored local files under `E:\wellpdfsdk\PDFA\`.

The specification PDFs remain local only and are excluded by `.git/info/exclude`.
Do not commit or redistribute them. Repository documentation and artifacts may
record identifiers, editions, filenames, hashes, access status, clause mappings,
and implementation notes.

## PDF-Family Sources

| Document | Local file | SHA-256 | Redistribution | crypto writer closeout use |
| --- | --- | --- | --- | --- |
| ISO 32000-2:2020 | `PDFA/ISO_32000-2_sponsored_EC3-1.pdf` | `71157FA5021F8A80197F483B2F6B815DD690159FDA12C1EF1C2D0D9FCFB36DEC` | do not commit PDF | PubSec handler, crypt filters, recipient seed/permission payload, file-key recovery, metadata and embedded-file policy |
| ISO/TS 32001:2022 | `PDFA/ISO_TS_32001-2022_sponsored_EC3.pdf` | `6B7127107E5441D80A94D953529606C3DD031EE12102E5D61015643C4E6A4012` | do not commit PDF | available, not used for immediate PubSec/AES-GCM implementation |
| ISO/TS 32002:2022 | `PDFA/ISO_TS_32002-2022_sponsored_EC3.pdf` | `C72DE6290C3595E0B2043145A80D81F16F19D1A9F869807E686994DBE16F8F20` | do not commit PDF | CMS/signature terminology only where crypto writer closeout overlaps signature-impact reporting |
| ISO/TS 32003:2023 | `PDFA/ISO_TS_32003-2023_sponsored.pdf` | `17D8CD1715BF15A03CC54614530B978FB3F4B490FFF1F3B4525E77DD176A6816` | do not commit PDF | AES-GCM crypt filter, nonce/tag/AAD representation, V/R additions |
| ISO/TS 32004:2024 | `PDFA/ISO-TS-32004-2024_sponsored.pdf` | `321E1CD9B9571B49F4FF0D107C20360FA81B9A4061585A895C05BB94B8B034A0` | do not commit PDF | encrypted-document integrity reporting and PDF MAC interaction boundaries |
| ISO/TS 32005:2023 | `PDFA/ISO-TS-32005-2023-sponsored.pdf` | `1301D9A9A3864BDB0B58871D55CAE9935962A9938A56987F7A71BD5372B62555` | do not commit PDF | available, not required for immediate PubSec/AES-GCM closure |

## Clause Map

The implementation uses clause identifiers and derived engineering notes only.
It does not copy large passages from the standards.

| Source | Clauses / tables used | Implementation dependency |
| --- | --- | --- |
| ISO 32000-2:2020 | 7.6.5.1, 7.6.5.2, 7.6.5.3, Tables 23 and 24 | PubSec encryption dictionary, permitted SubFilter values, recipient locations, permissions payload, seed/file-key recovery |
| ISO 32000-2:2020 | 7.6.6, Tables 25, 26, and 27 | Crypt-filter method selection, AuthEvent, Length interpretation, embedded-file filters, PubSec crypt-filter recipient payloads |
| ISO/TS 32003:2023 | 5.1, Tables 2, 3, and 4 | `/V 6`, Standard handler `/R 7`, `/CFM /AESV4`, crypt-filter length behavior |
| ISO/TS 32003:2023 | 5.2 | AES-GCM PDF object representation: 12-byte IV, ciphertext, 16-byte tag, nil AAD, no padding, per-filter 32-byte key |
| ISO/TS 32004:2024 | 5.1.1, 5.1.2, 5.1.3, Tables 2, 3, and 4 | Integrity-required permission bit additions for Standard and PubSec encrypted documents |
| ISO/TS 32004:2024 | 5.2.1, 5.2.2, 5.2.3, Table 6 | Trailer `/AuthCode`, KDF salt, integrity dictionary parsing/reporting |
| ISO/TS 32004:2024 | 6.1 through 6.6, Tables 7, 8, and 9 | PDF MAC token validation is a separate document-level integrity feature; crypto writer closeout records precise unsupported/partial states unless full CMS AuthenticatedData validation is present |
| RFC 5652 | Sections 3, 6.1, 6.2, 6.2.1, 10.2.4 | CMS ContentInfo, EnvelopedData, RecipientInfo, KeyTransRecipientInfo, issuer/serial matching |
| RFC 8017 | RSAES-PKCS1-v1_5 and RSAES-OAEP sections | RSA key-transport primitive selection and failure handling |
| RFC 5280 | Certificate, issuer, serial, subject-key-identifier profile sections | X.509 parsing and recipient identity matching; no trust-chain validation claim |
| RFC 5084 | AES-GCM CMS algorithm identifier and parameters | CMS AES-GCM parameter parsing if encountered in recipient envelope content encryption |

## Gate Decision

The source gate is now passed for implementation. The remaining engineering gate
is not normative availability; it is whether each requested cryptographic profile
can be implemented, tested, and independently cross-checked without guessing or
secret leakage.
