# Native Renderer Compatibility Fallback Policy

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

- `unsupported_operator_pattern`: owner Advanced Rendering pattern renderer parity.
- `unsupported_operator_shading`: owner Advanced Rendering shading renderer parity.
- `unsupported_graphics_state`: owner Transparency Rendering transparency and soft-mask
  renderer parity.
- `unsupported_xobject_subtype`: owner Form/image resource subtype closure if a
  future corpus page reveals unknown XObject subtype use.
- `malformed_content`: owner malformed renderable recovery prompts.
- `safety_limit_exceeded`: owner resource-limit tuning prompts.

The Native Renderer audit currently reports three compatibility fallback pages in the
13-page corpus: tiling pattern, shading, and transparency. These are intentional
later-roadmap categories, not hidden success cases.

Reference Renderer keeps that policy while adding Poppler/PDFium/MuPDF comparison. A
page may now be visually compared against three reference engines and still
report compatibility fallback. That is acceptable only when the fallback reason
is explicit in the Wellfriend render report and the page category is owned by a
later renderer roadmap task.

Reference Renderer must not use fallback status to excuse reference-tool failures.
Missing or failing PDFium/MuPDF commands are closure failures. Pattern, shading,
and transparency pixel mismatches are renderer-roadmap findings only after all
required reference tools produced artifacts and the multi-reference report
classified the page.

Transparency Rendering narrows the transparency bucket. Common transparency groups, alpha
state, blend modes, soft masks, isolated groups, and knockout flags now have a
native bounded-surface path and a dedicated Poppler/PDFium/MuPDF corpus under
`target/transparency_rendering-transparency-compositing/`.

Transparency Closeout closes the Transparency Rendering-owned transparency fallbacks for image alpha,
common image `/SMask /Matte`, ExtGState soft-mask `/BC`, DeviceGray/RGB/CMYK
luminosity masks, common DeviceGray/RGB/CMYK group color-space fixtures, and
interior knockout overlap for supported vector/Form groups.

Advanced Rendering removes text clipping, common shadings, and tiling patterns from the
generic compatibility fallback set. Type3 CID Rendering closes the common Type3/CID
text-clip and Type 7 tensor-interior leftovers. Remaining fallbacks must name
exact limits: advanced ICC/device-link/multicolor CMM, image/resource-only Type3
charprocs that fail closed, exotic missing glyph outlines, malformed streams,
or pattern/Type3/patch recursion and count safety caps.
