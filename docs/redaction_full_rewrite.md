# Redaction Full Rewrite

document security full rewrite history removal uses the canonical security
canonicalizer/writer path. It is intended for destructive history-removal
policies where incremental update history, prior object revisions, stale
metadata, or attachment references must not remain in the output.

The report records signature impact, history impact, source hashes, validation
results, and residual-verification status. If the caller does not acknowledge
the destructive full rewrite boundary, the operation is refused without changing
the input.
