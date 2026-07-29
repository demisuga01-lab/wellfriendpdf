# Semantic Intelligence ParentTree Recovery

Semantic Intelligence adds a conservative recovery layer for tagged PDFs whose authored
`/StructTreeRoot` is incomplete or broken but whose `/ParentTree` and page
marked-content IDs still carry useful evidence.

The recovery path is additive. It does not claim repaired output is the original
author hierarchy. Each recovered node records:

- source page and MCID
- source structure object when available
- role and original role
- bounding box recovered from marked text chunks
- evidence kind: spec-derived, repaired, inferred, orphan, conflicting, or ignored malformed
- confidence and diagnostics

Supported Semantic Intelligence cases:

- ParentTree number trees and arrays
- page `/StructParents` mapping to ParentTree arrays
- structure nodes with missing or unknown roles
- role-map gaps repaired to `Span`
- orphan marked content when no clean parent chain exists
- duplicate ParentTree key and MCID diagnostics
- malformed `/Limits`, null entries, missing refs, and non-dictionary nodes reported without panic

Merge policy:

- deterministic visible content remains primary
- no cross-page merge is created unless page `/StructParents` evidence links the page to that ParentTree entry
- malformed recursion is capped
- conflicts are diagnostics, not hidden rewrites

Exact limits:

- recovery does not certify PDF/UA correctness
- repaired hierarchy is not asserted to be author-original
- object references without visible text are reported but not converted into invented text
- resource-heavy malformed recursion remains bounded and fail-closed
