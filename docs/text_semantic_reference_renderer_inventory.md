# Reference Renderer Semantic Inventory

Reference Renderer is an additive closure pass on the Native Renderer semantic text model. It
does not change flat text extraction, table detection, form handling, rendering,
or redaction application.

| Feature | Native Renderer status | Gap | Reference Renderer target | Tests needed | Memory/cap concern |
| --- | --- | --- | --- | --- | --- |
| StructTree traversal | `semantic.rs` parsed tagged trees for the old semantic JSON | not attached to char/quad model | flatten tagged MCID roles into `TextStructureContext` | tagged RoleMap/MCID fixture | `max_structure_nodes` |
| RoleMap handling | older tagged path used raw roles | custom roles lost their normalized source | normalize roles, preserve original role | RoleMap fixture | bounded root RoleMap dictionary |
| MCID-to-content mapping | `TextCollector::collect_marked` existed | Native Renderer model used unmarked chunks | build model from marked chunks when structure is enabled | MCID char/search assertions | `max_mcid_entries` |
| ActualText provenance | chunk-level only | char/search summaries did not expose it | per-char ActualText source and summary counts | collector + model tests | long ActualText stays under char cap |
| ToUnicode provenance | font resolver used it | not visible below `TextChunk` | `FontDecodeSource::ToUnicode` per decoded char | synthetic mapping-source test | vector size equals logical chars |
| CMap/predefined CMap provenance | resolver supported predefined maps | not visible in model summaries | enum slots and counters for embedded/predefined CMap | unit coverage through source vectors | no new CMap cache |
| Encoding/Differences provenance | resolver used glyph names | not visible in char model | per-char `EncodingDifferences` / glyph-name source | synthetic mapping-source test | none |
| glyph-name/AGL provenance | font phase supported AGL | not visible in search matches | char/span/block summaries include glyph-name count | synthetic mapping-source test | none |
| font cmap fallback provenance | bounded enum slot | not fully emitted by current resolver | explicit source/summary slot for future resolver events | documented bounded limit | none |
| OCR provenance placeholder | invisible OCR-layer chunks | only run-level | per-char/search hidden+OCR flags | existing OCR/hidden tests | no new OCR backend |
| unknown/unmapped provenance | replacement char counted coarsely | not attached to chars | `UnknownUnmapped` flag and summary | source-vector test path | none |
| char-level quad/provenance | char quads existed | mapping source was chunk-level | chars carry source, MCID, role, role source | semantic model tests | `max_chars_per_page` |
| span-level provenance | spans had flags | no MCIDs or summary counts | span MCID list and provenance summary | semantic model tests | compact by default |
| block role confidence | heuristic roles only | tagged roles disconnected | tagged roles override heuristics with confidence | RoleMap fixture | none |
| CJK segmentation | character tokenization only | no bounded run segmentation | `char` and `simple`; `dictionary` aliases simple | CJK simple fixture | `max_cjk_run_chars` |
| RTL/vertical semantic fields | direction flags existed | unchanged | preserve existing fields | existing Native Renderer tests | none |
| semantic JSON schema | model-json existed | structure/provenance absent | add compact structure/provenance fields | CLI smoke + serde tests | detailed chars remain optional by API |
| RAG citation spans | docs only | no provenance guidance | document using search/model spans | docs | no new chunker |
| search match provenance | quads existed | no MCID/role/source summary | matches expose MCIDs, role, provenance summary, hidden flag | search fixture | bounded by `max_matches` |
| redaction-planning provenance | search quads existed | match source unclear | search results identify ActualText/hidden/fallback/tag source | search fixture | redaction apply remains Transparency Rendering |

Baseline before edits:

- text slice: char-sim `0.927`, word-F1 `1.000`, order `0.960`.
- field slice: strict field-F1 `0.725`.
- table slice: cell-F1 `0.987` in the current scorer; the roadmap gate remains table shape-F1 `0.96232`.

