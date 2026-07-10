# Prompt 15 Known Limits

- No TableFormer, Table Transformer, ONNX, Torch, Docling, or LayoutParser model
  runtime or weights are bundled.
- Real model quality requires application-supplied licensed weights, a runtime
  adapter, and external-corpus validation.
- The cloud table backend is a policy boundary, not a provider implementation,
  and remains disabled by default.
- Production CJK coverage depends on user-supplied licensed dictionary packs;
  the bundled dictionary is a small synthetic fixture.
- Bibliography/reference linking is limited to structure available in the PDF;
  source page/block/bbox/MCID citations are the stable contract.
- Figure/caption association uses deterministic document semantics and is not a
  general vision claim.
- Atomic tables and figures can exceed a requested token target; they are
  retained whole and marked oversized.
- The generated merged-header table currently has a `6/7` cell match. The
  merged heading is not detected by the deterministic PDF table detector.
- Docling is only availability-probed unless an explicit offline model path is
  configured; no Docling parity is claimed by package presence.
- LayoutParser and Camelot comparisons are absent when those tools are not
  installed. The availability artifact records this explicitly.
- The Prompt 15 corpus is deterministic fixture truth, not a broad production
  document corpus.
- Runtime is observational. Peak memory is unavailable on runners without
  `psutil`.
- Full all-page semantic JSON materializes detailed text, structure, table,
  token, and chunk layers together. Large files can produce large managed
  strings; callers that do not need characters should use typed Rust options or
  page-scoped CLI exports.
- A separate executable at baseline commit `9521ede` is not run by the default
  benchmark, so no before/after performance claim is made.

These limits are non-blocking because the public schema, deterministic behavior,
privacy posture, diagnostics, and availability reports represent each case
precisely.
