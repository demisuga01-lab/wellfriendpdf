# Type3 CID Rendering Editing/Conversion Closure Audit

| Feature | Advanced Rendering status | Type3 CID Rendering target | Implementation result | Tests | Remaining limit |
| --- | --- | --- | --- | --- | --- |
| paragraph reflow edit | redaction plus replacement overlay | model-backed paragraph rewrite | implemented via `edit_paragraph_reflow_pdf` | `paragraph_reflow_replace_reopens_with_new_text_and_old_absent` | left-to-right reflow only |
| insert text | in-memory model only | serialized paragraph insert | implemented through `ParagraphEditOperation::Insert` | `paragraph_reflow_insert_and_delete_are_undoable_in_model` | CLI targets first matched paragraph |
| delete text | in-memory model only | serialized paragraph delete | implemented through `ParagraphEditOperation::Delete` | `paragraph_reflow_insert_and_delete_are_undoable_in_model` | no vertical/RTL reflow claim |
| replace text | safe replacement overlay | true paragraph reflow default | implemented; overlay fallback is explicit | Advanced Rendering/08B edit tests | original font reconstruction not claimed |
| local paragraph rewrite | bounded future item | remove old paragraph region and serialize rewritten lines | implemented with full-rewrite redaction plus deterministic text serialization | reflow edit tests | region overflow returns diagnostic |
| line breaking | not implemented | bounded line breaker | implemented word/CJK-aware approximate metrics | overflow and wrap tests | advanced hyphenation not implemented |
| style-run preservation | summarized runs | preserve style where possible | paragraph transaction API rebuilds runs proportionally | undo/redo model test | PDF serialization currently uses Helvetica authoring path |
| font resource reuse/embedding | authoring path | deterministic Standard14 reuse | uses stable edit resources and existing writer | deterministic save test | original embedded font matching is later polish |
| page bounds overflow handling | not explicit | fail safely | overflow returns `paragraph reflow overflow` | overflow test | no automatic page reflow |
| page-faithful DOCX mode | missing | positioned OOXML mode | implemented `DocxLayout::PageFaithful` | DOCX anchor/XML test | Word rendering varies by consumer |
| flowing DOCX mode | implemented | keep unchanged | default remains `Flowing` | Advanced Rendering DOCX tests | same Advanced Rendering limits |
| text boxes/frames in DOCX | missing | preserve geometry | anchored `wp:anchor` + `wps:txbx` | DOCX XML test | python-docx cannot fully inspect textbox text |
| positioned images in DOCX | inline only | anchored image mode | implemented for page-faithful/hybrid | package/XML smoke | exact crop/mask not reconstructed |
| DOCX tables/spans | implemented | preserve native tables | unchanged, used in page-faithful mode | existing DOCX/table tests | complex table styling approximate |
| persistent undo/redo | simple in-memory transaction log | durable patch/checkpoint model | transaction log now records patches and bounded checkpoints | undo/redo test | no external HAMT/RRB store |
| transaction log durability | not serialized as patches | deterministic JSON | implemented `EditPatch`/`EditCheckpoint` | JSON assertion | import/replay API remains future SDK polish |
| snapshot/patch model | missing | compact snapshots | bounded digest checkpoints implemented | undo/redo test | checkpoint stores digest, not full structural clone |
| deterministic full save | digest smoke | explicit report/options | `save_to_bytes_with_options` reports deterministic policy | deterministic save test | object stream packing remains existing writer mode |
| deterministic incremental save | prefix smoke | stable repeated output | repeated incremental digest equality tested | deterministic save test | signatures are warned, not preserved |
| stable object ordering | writer-owned | document guarantee | existing BTree/object ordering documented | writer docs | no new object-packing optimizer |
| stable resource naming | implicit | explicit guarantee | edit resources use deterministic prefixes and next-name scan | deterministic save test | no semantic cross-object rename pass |
| metadata clock injection | not exposed | deterministic option/report | fixed PDF date option is reported; edit writer emits no wall-clock metadata | deterministic save test | full metadata authoring clock belongs writer polish |
| resource dedup | digest helper | duplicate grouping | `resource_dedup_report` groups identical bytes | resource dedup tests | writer-global dedup not enabled automatically |
| content-defined chunking | implemented | keep stable | unchanged; documented in 08B | existing/versioning tests | Rabin-compatible CDC later |
| SimHash/MinHash | SimHash only | prove scope | SimHash remains implemented; MinHash documented as not required | existing tests | MinHash later if needed |
| object-stream packing | writer mode only | deterministic posture | existing mode documented; no compression phase work | writer tests | advanced packing later |
| compression determinism | stable defaults | report | deterministic save report includes compression policy | deterministic save test | Zopfli-class Deflate not in scope |
| signature invalidation diagnostics | bounded | improve reporting | paragraph and deterministic save reports warn when signatures exist | report fields | full crypto preservation is Annotation Ocg Rendering |
