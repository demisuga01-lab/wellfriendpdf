# Prompt 06 Compatibility Fallback Policy

Compatibility fallback is still allowed, but it is no longer silent.

Rules:

- Every compatibility fallback increments `compatibility_runs`,
  `compatibility_ops`, `compatibility_bytes`, and
  `compatibility_fallback_reasons`.
- A page may visually pass while still reporting fallback.
- Native replay is the default for covered text, Image XObject, inline image,
  and Form XObject operations.
- Simple covered fixtures are regression-tested to keep fallback at zero.
- Unsupported states must use precise reasons and closure owners.

Current measured fallback reasons:

- `unsupported_operator_pattern`: owner Prompt 07+ pattern renderer parity.
- `unsupported_operator_shading`: owner Prompt 07+ shading renderer parity.
- `unsupported_graphics_state`: owner Prompt 07 transparency and soft-mask
  renderer parity.
- `unsupported_xobject_subtype`: owner Form/image resource subtype closure if a
  future corpus page reveals unknown XObject subtype use.
- `malformed_content`: owner malformed renderable recovery prompts.
- `safety_limit_exceeded`: owner resource-limit tuning prompts.

The Prompt 06 audit currently reports three compatibility fallback pages in the
13-page corpus: tiling pattern, shading, and transparency. These are intentional
later-roadmap categories, not hidden success cases.
