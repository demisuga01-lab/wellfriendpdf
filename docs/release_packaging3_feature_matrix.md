# text reflow Feature Matrix

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual implementation status

This document describes the intended text reflow boundary. It is not evidence
that every named capability is implemented. The current executable support is
limited to a provenance-resolved, bounded, single-region `GeometricBlock`
source rewrite. One narrow source-linked exception moves up to eight
pairwise-disjoint same-page vector paths through the existing advanced editing mutator when the caller
 provides its stable identity and an approved dependency edge. Unknown
 neighboring objects remain locked and a refusal leaves the input bytes
 unchanged.

The default rewrite builds a shaped Type0 subset. A second, narrower
`font_policy=preserve_original_per_run` route uses the existing advanced editing
multi-run source selector/CMap encoder and serializer. It has an executable
two-font/two-size/two-color fixture and replays supported source text state for
changed-length horizontal replacements by assigning each complete replacement
grapheme to a deterministic proportional source-style owner. It is not a general mixed-style engine:
 a fully selected text-state-only MCID BDC is relocated with its exact property
 list while the emptied source wrapper becomes an artifact, avoiding duplicate
MCID ownership. Inline link-semantic remapping, nested or partial tags, clipping, vertical text,
and cross-style justification refuse exactly.

Up to eight explicit source-associated same-page `/Link` annotations may move with the
reflowed text when the request provides the annotation index, exact expected
source rectangle, approved dependency edge, and finite delta. The canonical
transaction updates `/Rect` and existing `/QuadPoints`, retains `/A` or `/Dest`
unchanged, records the change, and participates in session undo. Widgets,
replies, generic annotations, stale rectangles, and cross-page annotation
movement remain exact refusals.

Semantic reconstruction and reading-order analysis are implemented with the
documented inference limits. A local SemanticDocument edit resolves one exact,
page-local semantic paragraph by its deterministic source-text identity and
source editing provenance; unrelated paragraphs elsewhere in the same document do
not invalidate that selection, while duplicate or partial selections return a
typed `paragraph_not_resolved` result. A narrow source-linked flow exception
can split that paragraph into an explicit, disjoint,
below-source, semantic/scene-proven-empty `next_region` on the same untagged
page; its two fragments are serialized in one positioned canonical source
stream and exact session undo is tested. A similarly narrow existing-next-page
flow, a narrow explicit horizontal same-page `next_column` continuation (LTR
rightward or RTL leftward), and a catalog-preserving append-only
one-page policy are documented separately. The append writer preserves existing
forms, annotations, outlines, named destinations, labels, and attachments but
does not infer their relation to new text. Inferred multi-column
flow, broad dependency movement beyond the explicit eight-path exception, reference repair, general page creation,
and full binding parity remain unavailable unless a document states a narrower
tested exception.

The runtime region graph validates deterministic node and edge identities,
page ownership, finite bounds, dangling-edge absence, an edge-count limit, and
explicit local versus document-scope invalidation. An owned annotated
two-column/footnote fixture executes precedence-cycle removal and scores exact
order, Kendall-style pair agreement, column order, and footnote placement.
Those are deterministic fixture measurements only; wider corpus accuracy and
cross-column source-flow application remain open closure gates.

The non-mutating overflow, hard/soft-constraint, and confidence/review query
reports are now available through Rust, CLI, Python, C ABI, WASM, .NET, and
Java and call the same canonical SDK functions. Output validation and replay-
verified `undo_reflow` are also available through those same binding adapters.
The public undo rejects stale output before executing the canonical inverse;
this does not claim complete package/runtime parity.

## Evidence status

The final transferred snapshot has separate VPS evidence for serial workspace
format/check/clippy/test, focused text reflow behavior, canonical writer impact,
source editing/32 impact filters, C ABI, CLI, fresh-wheel Python, WASM, .NET,
Maven, Gradle, qpdf/Poppler differential checks, bounded fuzz build/smoke,
performance, and repository hygiene. Each stage records a real exit code,
elapsed time, peak RSS, log path, and hash. Evidence does not expand any
documented typed refusal into an unsupported success.
## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
