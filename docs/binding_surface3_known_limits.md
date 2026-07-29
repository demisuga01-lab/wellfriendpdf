# Prepress Proofing Known Limits

Prepress Proofing intentionally does not claim:

- certification-grade PDF/X validation.
- full legal prepress conformance.
- hardware RIP equivalence.
- vendor-specific RIP quirks without reference evidence.
- arbitrary unsafe native color management.
- unlimited high-channel plate rendering.

Exact remaining limits:

- certification-grade PDF/X validation is owned by the later standards phase.
- resource-heavy recursive Type3 charprocs invoking nested XObjects, images,
  shadings, or patterns remain fail-closed.
- ICC profiles or images requiring high-channel pixel layouts not exposed by the
  safe native wrapper are `unsupported_reported_exact`.
- malformed recursive resource bombs fail closed under scheduler/resource caps.
- active media or dynamic content remains blocked by security policy.
