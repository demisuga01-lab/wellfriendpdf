# Reflow Overflow Policy

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

The runtime executes local stages in order: final shaped line breaking,
selected dictionary hyphenation, bounded text-state justification, and an
explicit allowed-region expansion. An expansion must contain the original
region, remain inside the page box, and fit the final layout; only then does
the canonical source writer use the expanded geometry. The transaction report
records both source and effective regions and returns
`fit_after_region_expansion`.

It never clips or silently reduces a font. Unknown or ambiguous neighbors are
locked. After local stages, separately documented bounded adapters may use an
explicit below-source, proven-empty same-page `next_region`, an explicit
direction-aware same-page `next_column` (rightward LTR or leftward RTL), an identical-box proven-empty next page, or
the plain one-page explicit creation policy. Each uses source-linked canonical
writer output and session undo; arbitrary dependency movement and all inferred
or ambiguous transitions remain exact refusals rather than global page shifts.
## Evidence status

Focused engine tests prove that an explicit expansion changes actual source
output only after the preflight succeeds. This is not the full overflow
closure or a release gate.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
