# Reading Order

The precedence graph is built from canonical semantic geometry and role
evidence. It has deterministic topological order. When a cycle appears, the
lowest-confidence edge is removed with edge-ID tie-breaking and reported with
an alternative/review flag.

This is an analysis result. Low confidence does not authorize a semantic edit,
and corpus-level reading-order accuracy metrics remain a text reflow closure
gate. The owned annotated two-column/footnote fixture executes the same
resolver, deliberately introduces a precedence cycle, verifies deterministic
lowest-confidence edge removal, and scores exact order, Kendall-style pair
agreement, column-order accuracy, and footnote placement. It is fixture
evidence only; it does not claim corpus-level accuracy.
