# XFA Runtime release verdict

Verdict: complete as a bounded XFA runtime and sandbox foundation, with zero `blocked` XFA Runtime-scope audit rows.

The release supports ordered packet inventory, hardened XML, static extraction and dataset binding, ordinary-PDF preview/flatten/reopen, a capped minimal dynamic runtime, default-disabled active content, opt-in pure FormCalc calculate/validate, JavaScript security blocking, sanitizer rescan, and shared reports across all bindings.

This verdict explicitly excludes full Adobe parity, JavaScript execution, unrestricted FormCalc, complex dynamic layout, external connections/resources, broad image/barcode engines, signature preservation after mutation, and secure-redaction claims for unflattened unsupported XFA. See `target/xfa_runtime-xfa-runtime/` for generated evidence and `xfa_runtime_known_limits.md` for the bounded remainder. Combined annotation/media redaction may begin after the required clean commit and full validation matrix are green.
