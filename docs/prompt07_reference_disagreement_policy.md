# Prompt 07 Reference Disagreement Policy

Prompt 07 keeps Prompt 06B's rule: renderer claims must be compared against
Poppler, PDFium, and MuPDF. Poppler-only passing output is not enough.

## Rules

- If all three references agree and Wellfriend differs, treat it as an Wellfriend bug or
  unsupported edge case.
- If references disagree, record the disagreement and classify which reference
  Wellfriend matches, if any.
- If a reference renderer fails to execute, fix the reference bootstrap before
  claiming Prompt 07 parity.
- If a fixture includes patterns or shadings, classify the paint-source gap as
  Prompt 08 while still testing that transparency state stays bounded.
- Do not relax thresholds to hide transparency errors. Keep thresholds stable
  and record the category.

The generated disagreement summary is
`target/prompt07-transparency-compositing/reference-disagreement-summary.json`.
