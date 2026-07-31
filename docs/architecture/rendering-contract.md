# Rendering contract

Wellfriend rendering is a source-linked document-execution pipeline. A render
result is valid only when the engine can connect pixels back to the canonical
PDF bytes, COS objects, page program, display list, resources, optional-content
state, transaction revision, and selected runtime configuration.

The public contract is:

- exact supported execution for implemented operators and resource families;
- bounded recovery when malformed structure can be repaired without ambiguity;
- deterministic compatibility fallback when a normalized fast path is not yet
  sufficient;
- typed refusal when output would be unsafe, unbounded, or semantically
  misleading.

The renderer must not win benchmarks by disabling annotations, forms, colour
management, transparency, font handling, source provenance, or validation. A
faster render path is accepted only when it preserves document meaning and the
same supported-boundary semantics.

Cache identity includes page number, DPI, render mode, tile rectangle,
optional-content visibility, prepress colour state, source revision, and the
effective Standard/Research configuration. Omitting any render-affecting input
is a correctness bug, not a cache optimization.
