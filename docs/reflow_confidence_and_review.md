# Reflow Confidence and Review

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

`evaluate_reflow_confidence` centrally evaluates geometry, text mapping, and
font identity for GeometricBlock; SemanticDocument additionally requires
reading-order, semantic-type, and cross-page-flow confidence. The deterministic
thresholds are `auto_apply` 0.90, `apply_with_warning` 0.80,
`review_required` 0.70, and `refuse` below 0.70.

Semantic application and page creation require explicit low-confidence
approval. A refusal contains the dimensions and a no-change proof; it never
falls back to another edit mode.
## Evidence status

The retained VPS matrix covers confidence/refusal fixtures through the same
serial workspace, binding, differential, fuzz, performance, hygiene, and
historical-impact program used for the other supported operations. It proves
the thresholds are enforced for the documented boundary, not that low-
confidence inference may be applied without review.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
