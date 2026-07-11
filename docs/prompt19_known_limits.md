# Prompt 19 Known Limits

- Oxide does not implement or claim full Acrobat JavaScript.
- The safe calculation subset is static, scalar, bounded, opt-in, and rejects
  loops/dynamic/runtime APIs.
- Script streams above 8 MiB, total decoded script bytes above 64 MiB, action
  graphs above 100,000 nodes or depth 64, and dependency graphs above 100,000
  edges are denied.
- Full-rewrite sanitization may invalidate existing signature byte ranges and
  is blocked/override-gated by Prompt 18B structural policy.
- Page-faithful DOCX does not guarantee identical pagination in Word and
  LibreOffice; font metrics and line breaking differ.
- Dedicated Word headers/footers, notes, comments, content controls, arbitrary
  vector clipping/blending, vertical text, and complex floats are reported
  rather than synthesized without reliable source semantics.
- Word automation evidence is absent when Word is not installed or automation
  cannot be used; LibreOffice evidence is similarly tool-dependent.
- DOCX limits are 10,000 pages, 100,000 OOXML parts, and 2 GiB output.
