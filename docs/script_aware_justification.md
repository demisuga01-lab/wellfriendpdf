# Script-Aware Justification

## Scope

Prompt 33 extends the Prompt 31 provenance/operator-editing and Prompt 32 scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

The bounded source-linked `GeometricBlock` path now drives horizontal
alignment through the canonical Prompt 20 generated-Type0 serializer. It
supports `left`, `right`, `center`, `start`, `end`, and `justify` in the public
request. `right`/`start`/`end` use resolved RTL evidence; `justify` emits
bounded PDF text-state `Tw` and then `Tc` adjustments, never glyph-outline
scaling or synthetic paths. The output report records natural width, target
width, residual, spacing adjustments, alignment, and last-line policy for
every emitted line.

The default does not justify the final line. A request may explicitly set
`justify_last_line`; any line whose remaining slack exceeds the configured
`Tw`/`Tc` bounds fails closed before mutation. Arabic kashida feature emission
and script-specific CJK punctuation rules are not yet serializable by this
writer and remain exact unsupported results. Vertical full justification is
also refused rather than using unsafe per-glyph transforms.
## Evidence status

Focused Rust tests prove that a chosen final layout emits bounded output-driving
spacing and that the Prompt 33 source-rewrite report carries the actual line
adjustments. This is not yet full script-aware justification or a release
gate.
## Known limits

Prompt 34 owns full table/formula/OCR edit engines. Prompt 35 owns final
tagged-PDF/accessibility repair and forensic redaction closure. Prompt 33 still
reports low-confidence semantic reconstruction and broad page-flow limitations
instead of treating inference as exact fact.
