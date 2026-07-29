# text reflow Validation

## Direct source-rewrite unaffected-content proof

Every supported direct GeometricBlock or single-region SemanticDocument source
rewrite now reopens both documents and verifies: one exact source occurrence,
expected extraction after that one substitution under the documented layout
whitespace policy, unchanged page count, and for every untouched page unchanged
extraction hash, page box, canonical content-stream reference list, and
decoded-stream hashes. On the edited page it records every pre-existing stream,
permits only the bounded source transaction's declared changed-stream count,
requires every other pre-existing stream to hash identically, and records newly
generated stream references. It also proves annotations outside the selected
flow are unchanged while allowing only caller-declared `/Link` rectangles to
move. A failed proof aborts the transaction instead of emitting output. This
bounded proof does not yet replace the separate reference/tag proof required
for broad multi-page pagination.

The final transferred snapshot has separate serial VPS stages for workspace
format/check/clippy/test, focused text reflow behavior, canonical writer impact,
and source editing/32 impact filters. It also has real runtime stages for CLI,
fresh-wheel Python, C ABI, WASM build and Node runtime, .NET test/package,
Maven test/package, and Gradle test/build.

The differential boundary is intentionally precise: qpdf checks the produced
PDF, Poppler extracts it, and the SDK reopens, validates, and replay-undos it
to byte-identical source bytes on the supported fixture. Tools unavailable on
the VPS are reported as unavailable rather than counted as passes. The evidence
does not expand a typed unsupported boundary into a successful edit.
