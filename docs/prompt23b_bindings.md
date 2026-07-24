# Prompt 23B Binding Surface

Status: `implemented_with_limits`

Rust:

- `EncryptAlgorithm::Aes256Gcm`.
- `aes256_gcm_encrypt_pdf_object`, `aes256_gcm_decrypt_pdf_object`, and test-vector nonce helper.
- Standard writer/reader AESV4 support.
- `PubSecIdentity`, `PubSecKeyProvider`, `PubSecRecipientCertificate`, `PubSecEncryptOptions`, `parse_pubsec_encryption_info`, `recover_pubsec_file_key`, `open_bytes_with_pubsec_provider`, `encrypt_pdf_pubsec`, and `reencrypt_pdf_pubsec`.
- `pdf_mac_report`, `pdf_mac_report_bytes`, and `pdf_mac_verify_report_bytes` expose ISO/TS 32004 structure discovery and supported standalone verification. `write_standalone_pdf_mac` and `pdf_mac_create_json` create AESV4 full-rewrite output with a standalone PDF-MAC token.

CLI:

- `encrypt --algo aesgcm`.
- `aes-gcm-encrypt --pdf-output ...`.
- `aes-gcm-decrypt --pdf-output ...`.
- `pubsec-report` reports scoped PubSec support and exact unsupported recipient profiles.
- `pubsec-decrypt`, `pubsec-encrypt`, and `pubsec-reencrypt` use explicit certificate/private-key or recipient-certificate paths. CLI secrets are file-backed; private-key bytes are not serialized into reports.
- `pubsec-add-recipient`, `pubsec-remove-recipient`, `pubsec-replace-recipient`, and `pubsec-decrypt-edit-reencrypt` route through the same full-rewrite re-encryption policy with file-key rotation.
- `pdf-mac-report` and `pdf-mac-verify` report/verify supported standalone tokens and never return `valid` from structure-only inspection.
- `pdf-mac-create --pdf-output ...` writes AESV4 encrypted full-rewrite output with a standalone `/AuthCode` PDF-MAC token and a secret-free JSON report.

Python:

- Existing `encrypt_pdf(..., algo="aesgcm")` path accepts AES-GCM.
- `pubsec_decrypt_pdf`, `pubsec_encrypt_pdf`, and `pubsec_reencrypt_pdf` expose byte-output runtime paths with explicit certificate/key paths.
- `pubsec_decrypt_pdf_pfx` and `pubsec_reencrypt_pdf_pfx` accept bounded PKCS #12/PFX providers with explicit password bytes.
- `PdfDocument.pdf_mac_report()`, `PdfDocument.pdf_mac_verify()`, and `PdfDocument.pdf_mac_create()` expose report dictionaries and owned output bytes with no secret fields.

C ABI, WASM, .NET, Java:

- C ABI exposes `wellfriendpdf_document_open_pubsec_from_bytes`, `wellfriendpdf_document_open_pubsec_pfx_from_bytes`, and `wellfriendpdf_document_pubsec_encrypt_pdf` with explicit byte lengths and owned output buffers.
- C ABI exposes `wellfriendpdf_document_pdf_mac_report_json`, `wellfriendpdf_document_pdf_mac_verify_json`, and `wellfriendpdf_document_pdf_mac_create_pdf` for PDF-MAC posture reporting, verification, and owned output creation.
- .NET exposes `WellfriendDocument.OpenPubSec(...)`, `OpenPubSecPfx(...)`, `PubSecEncryptPdf(...)`, `PdfMacReportJson()`, `PdfMacVerifyJson()`, and `PdfMacCreate()` on the existing `SafeHandle` lifecycle.
- Java exposes `WellfriendPdf.Document.openPubSec(...)`, `openPubSecPfx(...)`, `pubsecEncryptPdf(...)`, `pdfMacReportJson()`, `pdfMacVerifyJson()`, and `pdfMacCreate()` on the existing `AutoCloseable` lifecycle.
- WASM keeps file/keystore-backed private-key operations out of scope; report surfaces remain available and browser-host assumptions are not added.

Remaining binding limits:

- PKCS #12/PFX providers are exposed through byte-buffer APIs with explicit password bytes. Callback-style password providers remain limited to host-language code that supplies the final password buffer.
- Multi-recipient PubSec writing is available in Rust/CLI/Python; the C ABI/.NET/Java thin wrappers expose a single-recipient call to keep buffer ownership explicit.
- AES-GCM object authentication is separate from PDF-MAC document integrity. Standalone AESV4 PDF-MAC creation/verification is implemented; AttachedToSig remains unsupported exact.
