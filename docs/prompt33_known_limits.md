# Prompt 33 Known Limits

## Scope

Prompt 33 extends the Prompt 31 provenance/operator-editing and Prompt 32 scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Executable boundary

The supported implementation is provenance-resolved and bounded. It provides
real source rewriting, transaction replay undo, and exact typed refusal rather
than silently reducing an edit mode. One narrow exception moves up to eight pairwise-disjoint
same-page source-resolved vector paths through the canonical Prompt 20 mutator when a
 caller provides an approved dependency edge. Unknown neighboring objects
 remain locked and a refusal leaves the input bytes unchanged.

`preserve_original_per_run` adds a tested horizontal multi-run
exception. It replays existing source CMaps, font resources/sizes, text state,
 and DeviceGray/RGB/CMYK paint state. A single, fully selected,
 text-state-only MCID-bearing BDC is relocated with its original property list
 and the emptied source wrapper becomes an artifact, preserving one active
MCID sequence. Changed-length text assigns complete replacement graphemes to a
deterministic proportional source-style owner, retaining style order without
flattening or splitting a grapheme. It deliberately still refuses inline link-semantic remapping,
nested or partial tagged content, clipping text, vertical writing, inserted dictionary hyphens,
and arbitrary source color spaces.

A narrow Link-geometry exception moves up to eight caller-associated same-page `/Link`
whose exact preimage rectangle overlaps the selected source region. It preserves
the action/destination and moves `/Rect` plus existing `/QuadPoints`. It does
not infer associations and refuses widgets, replies, non-Link annotations,
appearance regeneration, page changes, and cross-page targets.

Semantic reconstruction, broad source-linked downstream movement, inferred
cross-column/cross-page flow, broad page creation, destination repair, and
full binding parity remain unavailable unless a document states a narrower
tested exception. Existing executable exceptions are an explicit same-page
next region, explicit LTR/RTL next column, an existing semantically proven
empty next-page region, and catalog-preserving explicit page append; append
keeps existing source forms, annotations, destinations, outlines, labels, and
attachments without inferring a relationship to newly authored text. It does
not repair general insertion/retargeting or tagged structures. Session undo
restores the exact stored preimage for those narrow transactions, and the
public replay-verified undo API rejects stale output first. The movement
exception is restricted to eight path objects on the
edited page: it refuses images, text objects, generic scene movement,
ambiguous or unknown neighbors, annotations/forms requiring rectangle repair,
clipping/marked/optional-content paths, and collisions.
## Evidence status

The final transferred snapshot is exercised through serial VPS workspace,
binding, differential, fuzz, performance, hygiene, and historical-impact
stages. The evidence proves the documented supported boundary and its typed
refusals. It does not imply support for any limitation listed above, and a
caller must not treat a refusal as a partial mutation or a mode downgrade.
## Known limits

Prompt 34 owns full table/formula/OCR edit engines. Prompt 35 owns final tagged-PDF/accessibility repair and forensic redaction closure. Prompt 33 reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
