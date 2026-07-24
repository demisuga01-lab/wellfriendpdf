# Prompt 09 Annotation Appearance Parity

Prompt 09B makes annotation coverage explicit. The authoritative matrix is `target/prompt09-annotation-ocg-progressive-cache/annotation-appearance-matrix-prompt09b.json`.

## Implemented And Proven

- Widget `/AP /N` streams render through the native Form XObject path.
- Stateful widget `/AP /N` dictionaries use `/AS` selection for the default render state.
- Missing widget `/AP` is bounded to generated text/button/choice basics where the document posture requires synthesis.
- AP streams honor Form resources, `/BBox`, placement into `/Rect`, page-space mapping, and the existing native replay path for ExtGState, transparency, patterns, and shadings.
- Hidden/no-view annotation flags suppress rendering.
- `/OC`-gated annotations follow the active default-view OCG configuration.
- Malformed appearance streams fail closed without panic, OOM, or incorrect success claims.

## Matrix Posture

Prompt 09B classifies 25 subtype/style rows:

- `appearance_stream_rendered`: widget AP stream, `/AP /N`, opacity/ExtGState posture, Form XObject AP stream
- `generated_appearance_rendered`: bounded widget missing-AP fallback and widget border basics
- `native_rendered`: OCG-gated annotation visibility
- `policy_reported_not_rendered`: Text icon posture, Link border/navigation posture, FileAttachment posture, Sound/Movie/Screen/RichMedia active-media posture
- `unsupported_reported`: non-widget generated FreeText, Line, Square, Circle, Polygon, PolyLine, Highlight, Underline, Squiggly, StrikeOut, Stamp, and Ink appearances
- `deferred_with_owner`: rollover/down viewer interaction state outside default page rendering

## Reference Evidence

Prompt 09B annotation visual fixtures are rendered with Wellfriend, Poppler, PDFium, and MuPDF. Artifacts:

- `annotation-reference-results-prompt09b.json`
- `annotation-diff-metrics-prompt09b.json`
- `annotation-reference-disagreements-prompt09b.json`

Known reference disagreement is classified: PDFium does not paint the focused widget AP stream in the same cluster as Poppler/MuPDF/WellfriendPdf. Wellfriend is inside the Poppler/MuPDF cluster and is not an outlier.

## Remaining Limits

Non-widget generated annotation drawings remain precise unsupported reports unless an author AP stream exists. Dynamic XFA and active rich-media playback remain outside this renderer block.
