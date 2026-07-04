# PDF Text Editing In Prompt 08

Prompt 08 added a verified high-level text replacement path:

```text
semantic search quads -> full-rewrite redaction of matched source content
                       -> replacement text overlay
                       -> reopen/search verification
```

This is intentionally conservative. A PDF content stream is not a word
processor paragraph. Replacing arbitrary text safely means removing the old
recoverable text first, then writing a replacement. Incremental save is not used
for redaction-backed replacement because old revision bytes would remain
recoverable.

Supported:

- search-based text replacement through `replace_text_pdf`.
- matching uses Prompt 06B semantic search and glyph quads.
- old source text is removed through the Prompt 07B full-rewrite redaction path.
- replacement text is drawn in the matched region with an `EditTextStyle`.
- output reopens in Oxide and can be extracted/searched.
- CLI: `oxide edit-text --query OLD --replacement NEW --out edited.pdf`.

Prompt 08B closure:

- `edit_paragraph_reflow_pdf` edits the reconstructed editable paragraph model.
- replace/insert/delete operations are represented as paragraph operations.
- rewritten text is line-broken inside the source/configured region.
- old paragraph content is removed from reachable content before rewritten lines
  are serialized.
- `edit-text` defaults to `--mode paragraph-reflow`; the older
  `overlay-fallback` mode is explicit.
- outputs are reopened and searched/extracted to prove old text is absent and
  new text is present.

Unsupported or bounded:

- same-width in-place stream patching is not claimed yet.
- arbitrary Word-like page reflow is not claimed.
- replacement text uses the existing authoring font path rather than trying to
  reconstruct an original embedded font program.
- if replacement contains the query, absence verification is not meaningful and
  is reported.

Incremental save:

- `PdfEditor::save_to_bytes(EditMode::Incremental)` remains available for
  additive overlays.
- CLI: `oxide save-incremental --text NOTE --out incremental.pdf`.
- Tests verify that incremental output preserves the original byte prefix and
  reopens.

Signature note:

Prompt 08 reports and preserves writer behavior where possible, but full
cryptographic signature preservation/validation belongs to Prompt 09.
