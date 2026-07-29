# crypto writer closeout Interoperability

Status: `implemented_with_limits`

Evidence root: `target/crypto_writer-writer-crypto/`

AES-GCM:

- Internal known-answer style vectors use fixed nonce helper only in tests.
- Runtime CLI smoke created an AESV4 PDF and decrypted it through WellfriendPdf.
- qpdf 12.3.2 was executed against the Wellfriend AESV4 encrypted output and
  returned an explicit unsupported encryption-dictionary result for `/R 7` and
  `/V 6`; this is recorded as unsupported, not as a pass.
- qpdf successfully checked the Wellfriend-decrypted output.
- Poppler `pdftoppm` 26.02.0 rendered the Wellfriend-decrypted output.
- Java JCA 25.0.2 matched independent public vectors for AES-GCM, AES Key
  Wrap, HMAC-SHA256, and HKDF-SHA256.

PubSec:

- A local scoped `/adbe.pkcs7.s5` fixture is generated in engine tests with a synthetic RSA identity and CMS EnvelopedData recipient.
- The fixture proves Wellfriend can recover the file key, decrypt the PDF, reject the wrong key, and extract visible text.
- Writer fixtures create multi-recipient PubSec PDFs, reopen with each intended recipient, reject a non-recipient key, and rotate recipients by full rewrite so removed recipients fail on the new output.
- No local independent PDF implementation with demonstrated `/Adobe.PubSec`
  `/adbe.pkcs7.s5` create/open support and compatible fixture was available in
  this closure pass. PubSec PDF-level external interoperability is therefore
  unclaimed; lower-layer primitive/provider evidence is recorded separately.

PDF-MAC:

- Wellfriend creates and verifies the standalone AESV4 PDF-MAC fixture.
- qpdf 12.3.2 was executed against the PDF-MAC output and returned the same
  unsupported `/R 7` and `/V 6` encryption-dictionary result, so qpdf is not a
  compatible ISO/TS 32004 validator in this environment.
- Java JCA primitive vectors cover the AES-KW/HMAC/HKDF layers used by the
  mapped PDF-MAC profile.

CMS/provider:

- OpenSSL CMS was not available on `PATH` in this closure pass.
- No local Bouncy Castle JAR or dependency was discovered outside the managed
  Java binding build.
- Missing OpenSSL/Bouncy Castle/PDFBox/iText/MuPDF/PDFium support is reported
  as unavailable, never as pass evidence.

Tool policy:

- qpdf and other tools are recorded only for features they actually support.
- Unsupported external behavior is not a pass.

Machine-readable artifacts:

- `crypto_writer_closeout-independent-tool-support-matrix.json`
- `crypto_writer_closeout-pubsec-interoperability.json`
- `crypto_writer_closeout-aesv4-interoperability.json`
- `crypto_writer_closeout-pdfmac-interoperability.json`
- `crypto_writer_closeout-cms-provider-interoperability.json`
