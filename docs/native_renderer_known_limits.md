# Native Renderer Known Limits

Native Renderer and Reference Renderer close the native replay foundation and the
multi-reference audit bootstrap. They do not claim full renderer fidelity.

Bounded limits that remain for later renderer prompts:

- transparency group compositing and soft masks;
- full blend-mode parity;
- mesh, patch, and function shading fidelity;
- tiling pattern cell fidelity and pattern-form promotion;
- advanced annotation appearance synthesis;
- complex image masks and ICC/color-management nuance;
- CJK/RTL shaping, hinting, and raster parity;
- progressive rendering/resume behavior;
- full optional-content visibility behavior.

Reference Renderer changes the evidence quality, not these renderer limits. The same
Native Renderer corpus is now compared against Poppler, PDFium, and MuPDF, and
later-owned pages are classified instead of treated as missing reference
coverage.
