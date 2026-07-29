# README known limits

The current release posture is `release_ready_with_limits`.

## Product limits

- Unsupported, ambiguous, policy-blocked, low-confidence, or unsafe edits return typed refusals.
- Semantic and OCR reconstruction may require review before destructive application.
- Dynamic XFA is not claimed as universally convertible or lossless.
- Viewer/render appearance parity is not universal.
- Accessibility repair does not replace human review.
- Signature results distinguish cryptographic integrity, certificate trust, and coverage/modification state.
- Standards validation distinguishes internal rule support from external validator availability.

## Validation limits

- Prompt 36 qpdf and Poppler evidence was current; MuPDF, PDFium, and PDFBox gaps were narrowed during the README task through wrappers or focused smoke where practical.
- veraPDF was unavailable on both Prompt 36 and README VPS attempts.
- pdfcpu was unavailable because Go was unavailable on the README VPS.
- OCRmyPDF was unavailable on the README VPS.
- Docling was documented but not provisioned to avoid heavy model/workflow setup for a README smoke.
- Commercial SDKs were not benchmarked because no legitimate benchmark license was used.
- Gradle remained limited by the VPS Gradle 4.4.1 package; Maven validated the Java runtime/package path.
