# Interactive and Final Line Layout

Preview uses deterministic greedy UAX #14 filling. Final layout uses bounded
dynamic programming over the same grapheme-safe candidates and advanced editing
shaped advances. The final range list, including supported visual dictionary
hyphens and mandatory separators, is passed to the canonical source writer.

Both algorithms carry the configured maximum consecutive dictionary-hyphen
count as execution state (currently two), rather than merely reporting it.
When no legal sequence remains within that bound, the final optimizer returns
an unresolved overflow plan and no source rewrite is attempted.

Candidate spans are capped at 2,048. The optimizer currently costs raggedness
and break penalties; widow/orphan, keep-with-next, and baseline-grid penalties
remain unavailable rather than being implied by a report field.
