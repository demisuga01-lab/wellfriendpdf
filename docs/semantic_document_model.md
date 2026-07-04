# Semantic Document Model

Oxide has two related semantic surfaces:

- The canonical parse/document model for Markdown, HTML, JSON, Office conversion, and RAG chunking.
- The Prompt 06 semantic text model for geometry-backed search, redaction preview, and extraction diagnostics.

The Prompt 06 model is intentionally text-focused. It does not replace the canonical parser model.

## Prompt 06 Text Model

`TextSemanticDocument`

- `pages`
- aggregate `counters`
- aggregate `diagnostics`

`TextSemanticPage`

- `page`
- `page_box`
- `blocks`
- `strategy`
- `confidence`
- page `counters`
- page `diagnostics`

`TextSemanticBlock`

- `text`
- `role`
- `lines`
- `paragraphs`
- `quad`
- `confidence`
- `provenance`

`TextSemanticLine`

- `text`
- `role`
- `direction`
- `words`
- `spans`
- `chars`
- `quad`
- `confidence`
- `provenance`

`TextSemanticWord`, `TextSemanticSpan`, and `TextSemanticChar` all carry quads. Characters additionally carry font name, font size, direction, mapping source, provenance flags, and confidence.

## Provenance Flags

The stable flags are:

- `native_pdf_text`
- `tagged_pdf`
- `actual_text`
- `ocr`
- `fallback_cmap`
- `fallback_glyph_name`
- `synthetic_layout`
- `low_confidence_order`
- `deduplicated`
- `hidden_or_invisible`
- `artifact_header_footer_candidate`

## Roles

Roles are heuristic unless they come from the existing tagged-PDF path:

- `body_text`
- `heading`
- `list`
- `table_candidate`
- `figure_caption`
- `header`
- `footer`
- `footnote`
- `marginalia`
- `unknown`

Prompt 06 keeps roles non-destructive. A caller can request body-only behavior later, but the model itself preserves the text.

## Memory Rules

The model is page-window friendly. `TextSemanticOptions` includes caps:

- `max_chunks_per_page`
- `max_chars_per_page`

When caps are hit, the page gets a structured diagnostic rather than an unbounded allocation.
