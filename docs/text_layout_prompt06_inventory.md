# Prompt 06 Text/Layout Inventory

Starting checkpoint: `69ceef5 Complete Prompt 05B color prepress closure`.

Baseline before edits:

- `cargo build --release -p oxide-cli`: passed.
- 200-file text slice: `target\competitive-benchmark\prompt06-text-before`.
- `char_similarity`: 0.92743.
- `word_f1`: 1.0.
- `line_recall`: 1.0.
- `reading_order`: 0.96019.

Prompt 06 target: keep the flat text gates unchanged while adding a geometry-backed semantic model for search, redaction preview, JSON diagnostics, and downstream RAG/conversion use.

| capability | current state before Prompt 06 | missing capability | Prompt 06 target | tests | metric impact |
| --- | --- | --- | --- | --- | --- |
| character/glyph extraction | `TextCollector` emits positioned `TextChunk` runs | no public character-level quads | add page/block/line/word/span/char model | semantic unit tests and fixture test | no flat text drift |
| ToUnicode | handled in font resolver path | source counters not surfaced per char | preserve behavior; expose counters where known | fallback tests in font stack remain | unchanged |
| CMap fallback | font resolver and predefined CMap support exist | model-level provenance coarse | expose fallback counters and diagnostics surface | existing CMap tests plus docs | unchanged |
| glyph-name / AGL fallback | font resolver support exists | model-level source not exact per glyph | keep fallback order documented | existing font tests | unchanged |
| ActualText | collected in marked text path | not exposed in geometry model | expose `actual_text` provenance and counters | semantic model unit test | unchanged |
| marked content / MCID | tagged extraction uses `MarkedTextChunk` | Prompt 06 model does not yet attach MCID path | keep tagged extractor; bounded limit for text model | tagged semantic tests remain | unchanged |
| tagged PDF structure tree | `semantic.rs` supports StructTreeRoot/MCID fallback | confidence fields limited in old semantic JSON | document relationship; keep old semantic path stable | semantic extraction tests | unchanged |
| per-character quads | not public | redaction/search cannot use exact char geometry | approximate char quads from text runs | unit and integration tests | improved API only |
| word grouping | line-box proportional words in `extract_page_words` | loose word bboxes | derive word boxes from chunk/char quads | integration test | no text gate drift |
| line grouping | XY-cut and reading-order modules | model not unified with word/char quads | line objects with roles/confidence | unit tests | unchanged |
| paragraph grouping | document model heuristics | not exposed in text model | line-gap/indent paragraph objects | unit test coverage through model | unchanged |
| column detection | XY-cut layout analyzer | no strategy flag in text model | strategy/confidence fields | fixture smoke | unchanged |
| reading order | tagged path and XY-cut path | no unified reporting in text model | per-page `strategy` and low-confidence diagnostics | benchmark unchanged | unchanged |
| headers/footers | document model heuristic | not in text model | conservative role candidates | unit/fixture checks | unchanged |
| footnotes | document model heuristic | not in text model | low-font/bottom-page candidate role | docs and model fields | unchanged |
| captions | document model heuristic | not in text model | prefix-based caption candidates | docs and model fields | unchanged |
| lists/headings | document model heuristic | not in text model | list/heading role candidates | docs and model fields | unchanged |
| math/superscript/subscript | partial heuristic in docmodel | no math model | bounded: detect only as role/confidence later | documented limit | unchanged |
| bidirectional text | `unicode-bidi` reading-order support exists | model-level direction not public | direction per span/line/char | unit coverage for RTL flags indirectly | unchanged |
| vertical writing | Prompt 04 font writing mode exists; reading-order path handles vertical | no vertical flag in text model | vertical direction and counters | unit coverage through chunk flags | unchanged |
| OCR/native merge | OCR seam feeds `TextChunk`s | provenance was implicit | invisible OCR-layer chunks marked in model | unit test | unchanged |
| confidence scoring | document model has confidence | text extraction has little model confidence | add confidence fields for page/block/line/word/span/char | unit tests serialize | unchanged |
| search indexing | limited text search surfaces | no quad-backed search model | `ContentEngine::search_text` with quads | integration test | new API |
| redaction matching | editing can redact regions | text match-to-quad prep incomplete | expose stable match quads; no apply in Prompt 06 | integration test | new API |
| Markdown/HTML/JSON output | parse/document model supports outputs | Prompt 06 text model absent | add `model-json`; keep parse outputs canonical | CLI smoke | unchanged |
| RAG chunking | `chunk` command exists over parse model | text model provenance not documented | document source mapping and limits | existing chunk tests remain | unchanged |

Bounded limits:

- The new semantic text model uses `TextChunk`-level provenance. Exact per-glyph source classification such as `ToUnicode` versus predefined CMap will need a lower-level extraction event stream in a later refinement.
- Tagged PDF role paths remain in `semantic.rs`; Prompt 06 does not merge every StructTree element into the new char-level model.
- OCR backend implementation is unchanged; OCR text is represented when existing OCR policy feeds chunks.
