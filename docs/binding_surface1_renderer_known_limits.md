# Renderer Fuzz CMM Renderer Known Limits

Renderer Fuzz CMM closes the renderer parity campaign for the covered Native Renderer through
Porterduff Radial Color Glyph corpus and starts the advanced CMM/prepress bridge. The following
limits remain exact and bounded:

- Release-duration renderer fuzzing remains a later release-hardening run over
  the Renderer Fuzz CMM targets and promoted corpus.
- Native LittleCMS is not linked until a separate audited native boundary and
  package policy exist.
- Output-intent destination proofing remains a later CMM/prepress owner.
- Device-link ICC and multicolor ICC are not implemented.
- True black-point compensation is reported as unavailable in the current
  default backend.
- Spot/DeviceN plates, separation framebuffers, and overprint proofing remain
  later prompts.
- qcms/default ICCBased transforms are preview transforms, not full prepress
  parity.
