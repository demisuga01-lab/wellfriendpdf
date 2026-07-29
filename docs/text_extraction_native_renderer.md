# Native Renderer Text Extraction

Native Renderer adds an additive semantic text model. The existing flat extraction path remains unchanged so the current gates stay stable.

Pipeline:

```text
PDF text operators
-> TextCollector positioned runs
-> XY-cut / reading-order layout
-> semantic text model
-> words, spans, chars, quads, provenance, confidence
-> search/redaction matches and JSON model output
```

## Public Surface

New engine APIs:

- `ContentEngine::collect_page_text_chunks(page)`.
- `ContentEngine::extract_text_semantic_model(pages, TextSemanticOptions)`.
- `ContentEngine::search_text(pages, query, TextSearchOptions)`.

CLI:

- `wellfriendpdf extract-text input.pdf --structured --format model-json`.
- `wellfriendpdf extract-text input.pdf --semantic --format model-json`.

The older formats remain stable:

- `--structured --format text|json`.
- `--semantic --format text|json`.
- `parse --format markdown|json|html`.
- `chunk --format json`.

## Unicode Recovery

The extraction stack keeps the existing priority established by the font phase:

1. ActualText where marked content semantically replaces visible glyphs.
2. ToUnicode CMap.
3. Embedded CMap / CID mapping.
4. Predefined CMap / Identity-H/V.
5. Font Encoding / Differences.
6. Glyph-name / AGL / `uniXXXX` / `uXXXX`.
7. Font cmap fallback where safe.
8. OCR only when the selected OCR policy provides text.
9. Replacement character plus diagnostics.

Native Renderer does not change the font decoder. The new model records ActualText, hidden/OCR-like text, RTL, vertical writing, unknown characters, and coarse native/fallback provenance at text-run granularity.

## Layout

Native Renderer reuses the existing deterministic geometry stack:

- Tagged PDF semantic extraction when the existing semantic path is requested.
- XY-cut block segmentation for untagged pages.
- Reading-order reconstruction for vertical and RTL runs.
- Conservative role candidates for headers, footers, headings, lists, captions, and footnotes.

Every model object has geometry and confidence:

- page
- block
- paragraph
- line
- word
- span
- character

## Search And Redaction Readiness

`search_text` returns matches with source quads. The search normalizer supports:

- exact search
- case-insensitive search
- ligature-aware matching for common Unicode ligatures
- hyphenation-aware matching across line breaks
- whitespace-collapsed multi-line matching
- hidden text inclusion only when requested

Native Renderer does not apply redactions. It produces stable match geometry for Transparency Rendering or a redaction-specific apply phase.

## Benchmarks

Before:

- Artifact: `target\competitive-benchmark\native_renderer-text-before`.
- `char_similarity`: 0.92743.
- `word_f1`: 1.0.
- `line_recall`: 1.0.
- `reading_order`: 0.96019.

After:

- Artifact: `target\competitive-benchmark\native_renderer-text-after`.
- `char_similarity`: 0.92743.
- `word_f1`: 1.0.
- `line_recall`: 1.0.
- `reading_order`: 0.96019.
- `text pass rate`: 200/200.

Prior extraction gates after Native Renderer:

- Field slice: `target\competitive-benchmark\native_renderer-fields-after`, `field_f1`: 0.72503, `field_value_f1`: 0.81434.
- Table slice: `target\competitive-benchmark\native_renderer-tables-after`, `table_shape_f1`: 0.96232, `table_cell_f1`: 0.98737.

## Known Limits

- Exact per-character decoder source is not yet available for all glyphs; the model reports exact ActualText/hidden/native flags and coarse counters.
- Full ML page-element classification is not included.
- Full redaction application remains outside Native Renderer.
- Full table reconstruction remains Transparency Rendering work; Native Renderer exposes text geometry that the table phase can consume.
