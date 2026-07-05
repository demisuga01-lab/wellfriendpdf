# Prompt 05 Extraction Scheduler Adoption

Prompt 05 extends scheduler admission beyond rendering:

- Text extraction page-content streams are admitted before the decoded reader is consumed.
- Image extraction calls `ContentEngine::decode_image_with_limits` or `decode_inline_image_with_limits`.
- Embedded-file extraction calls `extract_attachment_with_limits`.
- Parser-report decode probes call `decode_stream_report_from_dict_scheduled`.
- Direct lossless stream helpers use scheduler admission through `DecodeLimits`.

The public output order remains deterministic. Page content streams are decoded
in page content order, image extraction keeps caller enumeration order, and
parser-report stream diagnostics are emitted in stable object-id order.

Cancellation posture is cooperative. Prompt 05 observes a `CancelToken` before
decode admission; existing OS subprocess worker timeout/output caps remain the
Prompt 03 isolation boundary. No fake OCR support was added. When OCR is
compiled and configured, OCR-prep page images use renderer decode scheduling
from Prompt 04.

Evidence artifacts:

- `target/prompt05-codec-closeout/decode-callsite-inventory.json`
- `target/prompt05-codec-closeout/codec-coverage-matrix.json`
- `target/prompt05-codec-closeout/hostile-corpus-run.json`

