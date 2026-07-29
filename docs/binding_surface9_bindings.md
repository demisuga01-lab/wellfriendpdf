# form action policy Bindings

The shared Rust SDK facade exposes versioned JSON for form-JavaScript inventory,
the action graph, sanitizer/flatten reports, interactive-data close-out, Word
pagination audit, and the combined form action policy report.

CLI commands are `form-js-report`, `form-js-sanitize`,
`form-js-flatten-values`, `interactive-data-report`,
`word-pagination-audit`, `pdf-to-docx --layout ...`, and `form_action_policy-report`.

Python, C ABI, WASM, .NET, Java Maven, and Java Gradle call the same `sdk`
functions. Output operations return owned PDF bytes plus a versioned JSON
report. C callers free returned strings/buffers with the standard Wellfriend free
functions. .NET/Java preserve disposal rules. Existing flowing DOCX methods
remain source-compatible; explicit-layout overloads are additive.
