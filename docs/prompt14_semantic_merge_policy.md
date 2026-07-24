# Prompt 14 Semantic Merge Policy

Prompt 14 preserves Wellfriend's deterministic semantic model as the primary
evidence source.

ParentTree merge:

- prefer authored StructTree evidence when it is usable
- use ParentTree recovery only when useful structure is missing or broken
- use page `/StructParents` to avoid impossible cross-page mappings
- preserve visible marked content
- label repaired and orphan nodes explicitly
- report duplicates, missing refs, malformed limits, and role-map gaps

CJK merge:

- raw extracted text is unchanged
- dictionary mode adds a token layer only
- char and simple modes remain available
- offsets, quads, MCIDs, and provenance remain attached to source spans

ML layout merge:

- deterministic blocks remain primary
- high-confidence proposals can add hints
- low-confidence proposals remain suggestions
- model proposals cannot delete text
- conflicts and malformed proposals become diagnostics

Confidence defaults:

- ParentTree spec-derived nodes use higher confidence than repaired or orphan nodes
- dictionary matches use higher confidence than unknown fallback
- layout merge threshold defaults to `0.78`

Exact limits:

- repaired output is not certification evidence
- model hints are not structural truth
- unsupported malformed recursion remains fail-closed
