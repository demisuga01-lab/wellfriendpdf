# GeometricBlock Reflow

`apply_reflow_region` requires `requested_mode=geometric_block`; it never
upgrades to SemanticDocument. It removes exactly one resolved source token,
generates shaped Type0 source content through the canonical incremental writer,
reopens and extracts the output, records dirty/provenance evidence, and can be
undone by `ReflowMutationSession`.

It supports final-line output, visible dictionary hyphens with logical
extraction preservation, bounded text-state justification, and explicit local
expansion. With `font_policy=preserve_original_per_run`, it also has a
conservative executable multi-run path that replays original CMap/font,
size/spacing/scaling/rise/rendering-mode, and DeviceGray/RGB/CMYK paint state
for horizontal replacements. Changed-length text assigns each complete
replacement grapheme to a deterministic proportional source-style owner, so
style order is preserved without flattening fonts or splitting a grapheme. One fully selected,
text-state-only MCID BDC is relocated with its original property list while the
emptied source wrapper becomes an artifact. Inline link-semantic remapping,
nested/partial tags, clipping text, vertical writing, and arbitrary
style semantics are typed refusals; they are never flattened or
silently downgraded.

An explicit `downstream_vector_moves` request can also move up to eight same-page,
source-linked path object through the canonical Prompt 20 vector mutator. The
request must name its stable vector ID, an approved relationship, and a
user-approved dependency-edge ID. Prompt 33 validates the matching Prompt 32
scene occurrence, page bounds, no-overlap target, clipping/marked-content/OCG
context, Form ownership policy, and the absence of forms or annotations that
would require reference repair. Unknown objects, images, text objects,
annotations, and ambiguous scene matches refuse without
mutation. The vector mutation is included in the reflow preimage, report, and
atomic session undo.

An explicit `downstream_link_moves` request may move up to eight same-page `/Link`
annotation associated with the selected source region. It must carry the
annotation index, exact preimage rectangle, approved relationship/dependency
edge, and finite delta. Prompt 33 proves that the current link rectangle still
matches and overlaps the source region; the canonical transaction moves `/Rect`
and existing `/QuadPoints` while preserving `/A` or `/Dest`. Widgets, replies,
generic annotations, stale association, page changes, and inferred proximity
remain locked/refused. The Link change is included in the transaction evidence
and byte-exact session undo.
