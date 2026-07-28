# Reflow Constraint Solver

## Scope

Prompt 33 extends the Prompt 31 provenance/operator-editing and Prompt 32 scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

Prompt 33 uses `cassowary` 0.3.0 (MIT/Apache-2.0) for the supported
single-region feasibility check. Required constraints pin the region bounds,
require non-negative dimensions, and require the final shaped line-stack height
to fit the region. A weak baseline-grid preference is included in the same
solve. For the narrow explicit flow adapters, the same solve additionally pins
the requested target rectangle inside the page box and enforces either a
below-source `next_region` relation or an explicit same-reading-band
`next_column` relation (rightward LTR or leftward RTL). Infeasibility is returned in the reflow report before
source mutation.

The runtime report separates `hard_constraints`, `soft_constraints`, and
`unsatisfied_soft_constraints`, including the fixed bounded constraint count.
Callers may supply at most 64 constraints over the resolved metrics
`region_left`, `region_right`, `region_top`, `region_bottom`,
`content_height`, `region_width`, `region_height`, `line_count`, and
`line_height`. Relations are `eq`, `le`, and `ge`; priorities are `required`,
`strong`, `medium`, and `weak`. The exact metric is pinned into the solver,
so an incompatible required constraint refuses before source mutation, while
an incompatible soft constraint is retained with its residual in
`unsatisfied_soft_constraints`. Unknown variables, bad priorities/relations,
non-finite values, and oversize sets are typed infeasibilities rather than
being ignored.

This is not yet a general pagination solver: movable objects, no-overlap with
arbitrary scene objects, caption/footnote zones, inferred column flow,
general page flow, and broad source-position updates remain unsupported and
refuse rather than being inferred.
## Evidence status

The retained VPS program exercises feasible, hard-conflict, soft-priority,
locked-neighbor, bounded-flow, and invalid-coordinate cases, then carries the
same request through workspace, binding, differential, fuzz, performance,
hygiene, and historical-impact validation. The evidence covers this bounded
solver only; it does not claim an unbounded document-wide optimizer.
## Known limits

Prompt 34 owns full table/formula/OCR edit engines. Prompt 35 owns final tagged-PDF/accessibility repair and forensic redaction closure. Prompt 33 reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
