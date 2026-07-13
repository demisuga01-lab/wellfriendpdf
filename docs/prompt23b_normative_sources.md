# Prompt 23B Normative Sources

Status: `blocked_normative_dependency`

No required PDF-family normative source was available in the repository or in
local ignored workspace files at Prompt 23B start. This document intentionally
does not paraphrase or reconstruct missing clauses from memory, blogs, vendor
behavior, or prior implementation assumptions.

## Required But Unavailable

| Document | Required use | Local status | Implementation status |
| --- | --- | --- | --- |
| ISO 32000-2:2020 | Public-key security-handler structures, encryption dictionary semantics, crypt filters, file-key recovery, metadata and embedded-file behavior | not found | blocked |
| ISO/TS 32003:2023 | PDF AES-GCM crypt-filter name, nonce/tag/AAD layout, object/string/stream representation, writer rules | not found | blocked |
| ISO/TS 32004:2024 | Integrity protection interactions for encrypted PDF 2.0 documents if applicable | not found | blocked |
| ISO/TS 32002:2022 | CMS terminology/signature interaction only where normatively relevant | not found | blocked |
| Applicable ISO 32000-2 errata/resolutions | Current correction set for clauses used by implementation | not found | blocked |

Cryptographic RFCs and NIST documents are not sufficient to implement Prompt
23B without the PDF-family clauses above, because the missing information is the
PDF mapping: recipient payload layout, crypt-filter selection, PDF object
context, nonce/tag/AAD placement, and document writer behavior.

## Storage Policy

Do not commit copyrighted standards PDFs unless redistribution is explicitly
allowed. A valid future setup can use an ignored local directory with:

- local filename/path
- SHA-256
- acquisition source
- license/access status
- redistribution status
- clause mapping

After that, regenerate `target/prompt23-writer-crypto/normative-source-manifest-prompt23b.json`
and `target/prompt23-writer-crypto/normative-dependency-gate-prompt23b.json`
before any code changes.
