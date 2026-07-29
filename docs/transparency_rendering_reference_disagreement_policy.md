# Transparency Rendering Reference Disagreement Policy

Transparency Rendering keeps Reference Renderer's rule: renderer claims must be compared against
Poppler, PDFium, and MuPDF. Poppler-only passing output is not enough.

## Rules

- If all three references agree and Wellfriend differs, treat it as an Wellfriend bug or
  unsupported edge case.
- If references disagree, record the disagreement and classify which reference
  Wellfriend matches, if any.
- If a reference renderer fails to execute, fix the reference bootstrap before
  claiming Transparency Rendering parity.
- If a fixture includes patterns or shadings, classify the paint-source gap as
  Advanced Rendering while still testing that transparency state stays bounded.
- Do not relax thresholds to hide transparency errors. Keep thresholds stable
  and record the category.

The generated disagreement summary is
`target/transparency_rendering-transparency-compositing/reference-disagreement-summary.json`.
