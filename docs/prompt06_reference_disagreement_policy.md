# Prompt 06 Reference Disagreement Policy

Prompt 06B compares every Prompt 06 corpus page across Wellfriend, Poppler, PDFium,
and MuPDF. The audit records all six pairwise comparisons:

- Wellfriend vs Poppler
- Wellfriend vs PDFium
- Wellfriend vs MuPDF
- Poppler vs PDFium
- Poppler vs MuPDF
- PDFium vs MuPDF

Reference disagreement is not automatically an Wellfriend bug. The report classifies
each page into one of the machine-readable categories in
`renderer-parity-taxonomy-prompt06b.json`.

Rules:

- If all references agree and Wellfriend matches them, the page is
  `all_references_agree_wellfriendpdf_pass`.
- If all references agree and Wellfriend does not match them, the page is
  `all_references_agree_wellfriendpdf_mismatch`, unless the category is explicitly
  later-owned and needs manual review.
- If references disagree and Wellfriend matches exactly one reference, the report
  names that reference.
- If references disagree and Wellfriend does not clearly match one reference, the
  report records `references_disagree_wellfriendpdf_between_references` or
  `needs_manual_review`.
- Missing reference output is `reference_tool_failure`, not visual mismatch.
- Dimension mismatch is separate from pixel mismatch because pixel metrics would
  otherwise be misleading.

Later-owned categories for this campaign are:

- `pattern/later`
- `shading/later`
- `transparency/later`

Those pages remain in the corpus. They are not deleted to make the audit look
better; they carry forward as measured renderer-roadmap work.

The Prompt 06B closure run recorded 10 `all_references_agree_wellfriendpdf_pass` pages
and 3 `references_disagree_wellfriendpdf_between_references` pages. The disagreement
examples were annotation appearance, tiling pattern, and shading. Transparency
remains later-owned even though the 06B threshold classified that page as a
pass, because full transparency and soft-mask fidelity are not Prompt 06B
features.
