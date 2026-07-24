# Standards validation report envelope

`StandardsValidationReport` is the binding-neutral envelope for PDF/A, PDF/UA, and PDF/X.
It contains a schema version, family/profile, detected identifiers, conformance status, stable
rule results, counts, clause references, evidence, implementation state, and diagnostics.

`CrossProfileConflictReport` contains all three family reports, deterministic conflict IDs,
involved profiles/rules, severity, evidence, and an aggregate result. A pass in one family never
hides a failure, unsupported status, or deferral in another family.

Rust serializes this envelope directly; CLI JSON, Python, C ABI JSON, WASM JSON, .NET, and Java
all delegate to the same engine rather than synthesizing report-only rows.
