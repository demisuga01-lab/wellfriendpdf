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

- `unsupported_operator_pattern`: owner Prompt 08 pattern renderer parity.
- `unsupported_operator_shading`: owner Prompt 08 shading renderer parity.
- `unsupported_graphics_state`: owner Prompt 07 transparency and soft-mask
  renderer parity.
- `unsupported_xobject_subtype`: owner Form/image resource subtype closure if a
  future corpus page reveals unknown XObject subtype use.
- `malformed_content`: owner malformed renderable recovery prompts.
- `safety_limit_exceeded`: owner resource-limit tuning prompts.

The Prompt 06 audit currently reports three compatibility fallback pages in the
13-page corpus: tiling pattern, shading, and transparency. These are intentional
later-roadmap categories, not hidden success cases.

Prompt 06B keeps that policy while adding Poppler/PDFium/MuPDF comparison. A
page may now be visually compared against three reference engines and still
report compatibility fallback. That is acceptable only when the fallback reason
is explicit in the Oxide render report and the page category is owned by a
later renderer prompt.

Prompt 06B must not use fallback status to excuse reference-tool failures.
Missing or failing PDFium/MuPDF commands are closure failures. Pattern, shading,
and transparency pixel mismatches are renderer-roadmap findings only after all
required reference tools produced artifacts and the multi-reference report
classified the page.

Prompt 07 narrows the transparency bucket. Common transparency groups, alpha
state, blend modes, soft masks, isolated groups, and knockout flags now have a
native bounded-surface path and a dedicated Poppler/PDFium/MuPDF corpus under
`target/prompt07-transparency-compositing/`.

Prompt 07B closes the Prompt 07-owned transparency fallbacks for image alpha,
common image `/SMask /Matte`, ExtGState soft-mask `/BC`, DeviceGray/RGB/CMYK
luminosity masks, common DeviceGray/RGB/CMYK group color-space fixtures, and
interior knockout overlap for supported vector/Form groups. Remaining
transparency fallback reasons must now name later-owned work precisely:
advanced ICC/device-link/multicolor CMM, text clipping, or Prompt 08
pattern/shading paints. Patterns and shadings remain Prompt 08 even when they
are painted inside a transparency group.
