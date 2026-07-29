# text reflow Performance

Layout candidate shaping is bounded to 2,048 spans; semantic graph creation is
bounded to 16,384 nodes and 32,768 edges. All text reflow Cargo work runs one
job at a time on the VPS and the recorded peak RSS stays below the 32 GiB
aggregate budget.

The final VPS evidence includes a measured five-sample runtime benchmark for
layout analysis, interactive preview, final source rewrite, and replay undo on
the repository-owned one-page multi-stream fixture. It records median, p95,
maximum, child-process peak RSS, input scope, and explicit limits in
`performance-memory-results.json` in the text reflow result directory.

No universal performance claim is made. The benchmark is deliberately scoped
to the supported bounded operation; it does not claim measurements for broad
multi-column pagination, unbounded story reconstruction, or unsupported
page-creation policies.
