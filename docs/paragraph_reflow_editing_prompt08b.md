# Paragraph Reflow Editing In Prompt 08B

Prompt 08B adds a true paragraph edit path:

```text
EditableDocument paragraph/run -> replace/insert/delete -> bounded line reflow
                              -> full-rewrite removal of old reachable content
                              -> serialize rewritten paragraph lines
                              -> reopen/search verification
```

Supported edit operations:

- `ParagraphEditOperation::Replace`
- `ParagraphEditOperation::Insert`
- `ParagraphEditOperation::Delete`

The default CLI mode is now:

```powershell
oxide edit-text input.pdf --query OLD --replacement NEW --mode paragraph-reflow --out edited.pdf --json
```

`overlay-fallback` remains available only when explicitly requested.

Reflow behavior:

- uses the matched editable paragraph and block geometry.
- accepts an optional bounded region.
- uses approximate deterministic text metrics for line breaking.
- groups by words, with a CJK character fallback.
- preserves model run styles proportionally in the transaction model.
- writes the edited paragraph through the existing Standard14 authoring path.

Safety behavior:

- old paragraph content is removed with the full-rewrite redaction path before new text is serialized.
- output is reopened and searched/extracted.
- old text absence and new text presence are reported.
- overflow fails with a structured `paragraph reflow overflow` error.
- signed inputs receive invalidation diagnostics; cryptographic preservation belongs to Prompt 09.

Limits:

- left-to-right horizontal reflow is implemented.
- advanced vertical/RTL reflow, font-program reconstruction, full page reflow, and Word-like pagination are not claimed.
