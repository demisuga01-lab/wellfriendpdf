# Prompt 08 Editing And Conversion Inventory

Prompt 08 starts from the Prompt 07B checkpoint and adds a shared editable
document model, conversion surfaces that consume that model, deterministic
writer helpers, and a verified text replacement path. It does not reopen parser,
decode, rendering, font, color, semantic extraction, forms, annotations, page
operations, or redaction except through narrow integration points.

| Feature | Current state | Prompt 08 target | Tests | Risk | Later boundary |
| --- | --- | --- | --- | --- | --- |
| logical document model | `parse::Document` and Prompt 06B semantic model exist | feed a Prompt 08 editable model | `prompt08_editing_conversion` | medium | deeper ML classification later |
| editable model | new `editable::EditableDocument` | stable IDs, blocks, paragraphs, runs, tables, images, diagnostics, transactions | unit and integration tests | medium | persistent HAMT/RRB structures later |
| paragraph reconstruction | parse blocks to paragraphs/runs | conservative paragraph/run model | integration model test | medium | advanced reflow/layout scoring later |
| style inference | parse spans expose bold/italic/link | editable runs preserve bold/italic/link placeholders | model JSON test | medium | full font family/color inference later |
| font/style runs | `InlineSpan` | editable run list | model JSON test | low | per-char font/color merging later |
| lists and numbering | parse list blocks | editable list paragraphs and Markdown/HTML export | model export test | medium | complex nested numbering later |
| headers/footers | semantic roles exist | represented and omitted from Markdown body by default | model export test | low | Word section headers later |
| footnotes/endnotes | heuristic from Prompt 06 | represented only when parse roles expose them | docs | medium | full note linkage later |
| section hierarchy | headings from parse roles | conservative sections split at headings | unit test | medium | TOC/outline integration later |
| reading order | Prompt 06B model | preserved from parse body order | existing gates | medium | no perfect untagged order claim |
| images with placement/cropping | image locator exists | page image IDs and figure image placeholders | model JSON test | medium | exact crop/mask export polish later |
| real table grid model | Prompt 07 table cells/spans | editable table cells export to Markdown/HTML/Office | Office package test | low | complex table style recreation later |
| absolute-position mode | existing PPTX positioned shapes | route through editable parse model | package readback | medium | richer shape grouping later |
| flowing DOCX mode | existing native DOCX writer | consumes editable-model derived parse document | package readback | medium | full Word layout parity later |
| page-faithful DOCX mode | Prompt 08B implements `DocxLayout::PageFaithful` | positioned OOXML anchors/text boxes | `prompt08b_editing_conversion` | medium | exact Word pagination later |
| PPTX slide mapping | existing native PPTX writer | consumes editable-model derived parse document | package readback | medium | vector shape fidelity later |
| XLSX table/sheet mapping | existing native XLSX writer | consumes editable-model derived parse document | package readback | low | numeric/date inference later |
| HTML export | existing pdftohtml-style exporter | semantic `pdf-to-html` via editable model | CLI smoke planned | low | CSS/layout modes later |
| Markdown export | parse Markdown exists | `pdf-to-markdown` via editable model | integration test | low | advanced tables/footnotes later |
| JSON export | parse/model JSON exists | editable JSON schema | integration test | low | schema versioning later |
| RAG/chunk export | Prompt 06B chunks exist | model keeps provenance/sections for chunk input | docs | low | chunking tune later |
| PDF rewrite/save | writer and editor exist | use full rewrite for text replacement | edit/reopen test | medium | direct stream patch for same-width edits later |
| incremental save | editor supports append-only overlays | expose CLI and test prefix preservation | integration test | medium | signature-aware preservation in Prompt 09 |
| deterministic output | writer deterministic modes exist | resource digests and repeat-edit hash proof | integration test | medium | object-stream packing optimization later |
| undo/redo model | Prompt 08B patch/checkpoint transaction log | model text edit undo/redo | unit test | low | external transaction replay later |
| rope/segmented editing | not present | segmented run replacement | unit test | medium | full rope later |
| curve fitting for ink | annotation points preserved | documented roadmap | docs | medium | higher quality curve fitting later |
| raster-to-vector roadmap | not implemented | documented research boundary | docs | high | optional later |
| font reconstruction roadmap | not implemented | documented research boundary | docs | high | optional later |
| content-defined chunking | not present | small deterministic prototype | versioning tests | low | writer-level dedup later |
| MinHash/SimHash dedup | not present | SimHash text sketch prototype | versioning tests | low | corpus-level dedup later |
| Zopfli/object-stream optimization | writer modes exist | bounded as optimization, not Prompt 08 core | docs | medium | compression phase later |

Baseline artifacts were recorded under `target/prompt08-baseline/` before code
changes. Prompt 08 artifacts are written under `target/prompt08-artifacts/`.
