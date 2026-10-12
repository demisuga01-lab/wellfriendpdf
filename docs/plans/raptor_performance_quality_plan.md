# RAPTOR performance and quality implementation plan

Status: proposed; implementation awaits the user's instruction to continue.
Reviewed source: `main`, `904db7bb8771a3c5e2d30c78c9830bd2bb7a17d1`, 10 October 2026.
Evidence: `docs/reports/final-benchmark-vps-20261008/`. This planning pass runs no builds, tests, or PDF workloads.

## Verdict and objective

Wellfriend does not currently lead every parser or document operation. Resident open plus page-count P50/P95/P99 is 1.15/3.49/11.58 ms; PDFium records 0.72/2.15/6.15 ms. Wellfriend's maximum is 50.90 ms versus Poppler's 22.93 ms. All five engines pass the reported 150-document opening/page-count check. That check does not establish complete object, content, text, or recovery correctness.

The larger opportunities are document writing, semantic extraction, Office/vector conversion, and raster execution. Merge P50 is 213.76 ms versus qpdf's 107.55 ms; rotate maximum is 24,874.89 ms versus qpdf's 3,062.44 ms. DOCX maximum is 625,898.94 ms. SVG first-page P50 is 2,393.40 ms versus MuPDF's 73.14 ms. These operation timings include their existing benchmark contracts and require stage attribution before assigning predicted gains.

The objective is lower time and memory for equivalent work, with preserved document semantics and improved rendering fidelity. Each change must identify the work it eliminates and pass independent output checks. Corpus filenames, producer-specific benchmark exceptions, lower DPI, skipped objects, omitted images, or cached final answers cannot determine a fast path.

## Source findings and remedies

| Finding | Current source | Proposed remedy | Expected affected work |
| --- | --- | --- | --- |
| File-backed object boundary search scans all xref entries per lookup | `reader.rs::next_object_boundary` | Build a sorted, deduplicated physical-offset index once; successor lookup by binary search. Preserve active generation and revision precedence independently of physical offsets. | File-backed traversal, extraction, repair, writing |
| Ordinary object and object-stream hits return deep clones | `reader.rs::get_object` | Internal shared object handles and immutable stream storage; owned compatibility results at public boundaries; copy only changed dictionaries. | Parse, extract, edit, render, write |
| Cached page programs and text results are cloned | `engine.rs::PageArtifactCache` | Shared operation/text slices for internal consumers; retain public owned-return APIs. | Repeated page interpretation and multi-output workflows |
| Full engine construction materializes and hashes the whole input | `render/document_view.rs::CanonicalDocument::from_document` | Separate session identity from authenticated byte identity; compute the latter once when required, or during ingestion. Keep existing external digest definitions. | Large-file first-page access and initial editing/rendering |
| First canonical page identity scans the object-identity vector | `render/document_view.rs::page_identity_for` | Build a generation-sensitive identity lookup with the xref index. | Many-page first traversal |
| Repeated requested-page traversal starts from the root | `document.rs::collect_page`, `walk_page_tree_until` | Cached subtree intervals and a sequential page iterator; validate encountered counts and widen to full traversal on inconsistency. | Sequential/random page access; flat page trees |
| ObjStm lookup decodes and parses all members eagerly | `reader.rs::parse_object_stream` | Decode once, index member spans, parse requested members lazily; prefetch/eager parse only when traversal demand warrants it. | Metadata opening and large compressed-object streams |
| Object-stream cache limits count, not retained bytes | `reader.rs::BoundedObjectStreamCache` | Account decoded buffers, member indexes, and parsed objects by bytes. | Memory pressure and latency tails |
| Untagged model building computes a semantic geometric fallback then builds geometry again | `docmodel.rs::build_document_model`, `semantic.rs::geometric_fallback` | Route using validated structure availability and share a prepared page analysis across both consumers. Preserve existing empty/malformed-tag behavior. | Markdown, HTML, JSON, tables, Office export |
| Reading-order graph compares all block pairs | `docmodel.rs::reading_order` | Sweep/interval indexes to enumerate potentially interacting pairs; keep the same precedence predicate, ties, and cycle handling. Output can still be quadratic when the true relation is dense. | Dense pages and semantic conversion |
| Small structural changes use a whole-object rewrite | `structural.rs::rotate_pages`, `crop_pages`; `writer.rs::rewrite_document_objects` | Explicit writer planning for local append updates or complete streaming rewrites. Preserve the caller's save/history/security contract. | Rotate, crop, metadata, watermark, numbering, flatten, edit |
| Incremental output copies the entire input into a Vec | `writer.rs::write_incremental_update_raw` | Base-plus-delta source representation and streaming output; materialize full bytes once only for APIs that require them. | Large-file edits and repeated checkpoints |
| Office images are converted to PNG before deduplication | `office.rs::collect_docx_images` and PPTX media collection | Deduplicate by source and full decode/output policy before work; share media bytes and separate placement records. Reuse original media only when format, masks, Decode, color policy, and orientation permit. | DOCX/PPTX time, memory, output size |
| Office conversion builds an editable projection then clones its parse document | `office.rs::parse_for_office`, `editable.rs::to_parse_document` | Reuse the approved canonical projection and borrow/share immutable content; preserve required editable-model transformations. | Every Office conversion |
| SVG/PS check and then regenerate each glyph outline | `render/svg.rs::show_text`, `emit_glyph`; equivalent PS methods | Shared prepared font/glyph output and reusable vector definitions; preserve advances, offsets, clipping, and paint order. | SVG/PS/EPS conversion |
| Dense clips and scalar affine sampling dominate rendering | `render/buffer.rs`, `clip_dag.rs`, `image_painter.rs` | Bounded/tiled clip evaluation and optimized image kernels, followed by parallel tile execution. | Raster latency and large scans |

These are static implementation observations. Their individual wall-time contributions need VPS profiles; a source complexity issue is not a measured speedup. `decode_cache.rs` also clones its returned buffers and has linear recency updates, but the inspected production code does not consume that utility: changing it alone would not accelerate these paths.

## RAPTOR extension

Retain RAPTOR's revision binding, prepared transactions, existing render invalidation, exact transform memoization, and cancellation. Extend the existing implementation rather than introduce a competing document engine.

An artifact should have the following execution record:

```text
Artifact = (kind, dependency identities, contract, shared value,
            retained bytes, measured derivation cost, demand count, evidence)
```

The proposed extension adds four mechanisms:

1. **Demand-directed preparation.** A request declares what it needs: root metadata, selected objects/pages, text, tables, visuals, or standards checks. The scheduler derives their dependency closure. Example: rotation needs page dictionaries and inherited rotation; it does not need image pixels or semantic blocks. A complete audit still requests the complete closure.
2. **Verified promotion between revisions.** The existing render dependency graph already supports narrow invalidation. Extend this to parser, semantic, shaping, and proof artifacts. A surviving artifact is promoted only after the transaction establishes unchanged dependencies, ownership, authority, and contract. Plans/approvals remain bound to their exact revision. Unknown dependencies invalidate the containing scope. Rewrite/remapping requires a verified identity mapping; numeric object IDs alone are insufficient.
3. **Cost-aware cache admission.** Trial a byte-aware frequency policy with utility proportional to expected reuse times measured recomputation cost divided by retained bytes. Use counters as hints only; they never decide correctness. Avoid admitting one-use large scans that evict hot fonts and page programs. Keep a cheap policy for tiny documents, partition memory across tenants/contracts, and compare against the existing policy by ablation.
4. **Shared derivation and batch execution.** Coalesce concurrent requests for the same artifact and fuse repeated interpretation across requested outputs. Build each page analysis once for text/HTML/tables; decode each image once for Office media; write one transaction for a batch of edits. Dependency cycles, cancellation, and recursive object streams need explicit ownership/state handling so waiting on an in-flight derivation cannot deadlock.

This composition is a research candidate, not an established new algorithm. Its falsifiable question is whether dependency-scoped reuse plus cost-aware admission reduces cold/warm latency tails and memory under mixed workloads while producing equivalent results. Measure each mechanism separately. Record invalidation misses, bytes copied, duplicate derivations, and cache eviction causes.

## Implementation sequence

### 1. Establish measurements and correct quality attribution

Add stage timing and counters for source reads, xref, object resolution, stream decode, page lookup, text/geometry/tag analysis, layout, raster, media encoding, serialization, hashing, and verification. Distinguish engine time from command startup, filesystem I/O, and independent validation. Bind binary/source hashes and build flags to every run.

Measure actual page durations. The current compactor computes document time divided by page count, then percentiles across documents; retain that metric under its accurate name and add actual page percentiles. Compare references pairwise, classify their disagreement, and select visual panels from full-resolution failures. A Wellfriend self-comparison of 1.0 cannot establish quality.

### 2. Remove parser and resource amplification

Implement the physical offset index, direct object-identity map, shared internal object/program access, byte-budgeted ObjStm storage, and page iterator/index. Introduce immutable source leases with pinned ranges or bounded windows. A file handle alone does not make externally writable bytes immutable: hash-bound operations must use a stable snapshot or verified source policy.

Defer authenticated whole-input hashing until required, without replacing a required digest with file size/mtime or an unchecked short hash. Keep source/session identity and externally verifiable byte identity distinct. Add vectorized scanning only to grammar-safe whitespace, comments, delimiter search, and repair candidates; canonical validation still handles strings, escapes, nested dictionaries, indirect lengths, and inline-image binary data. SIMD JSON techniques inspire byte classification, not a replacement PDF grammar.

### 3. Share semantic analysis and improve spatial algorithms

Create prepared page text, graphics, classification, and geometry products keyed by revision and all interpretation options. Reuse them across semantic/document/export routes. Use sweep or interval indexes for block precedence, figure overlap, table candidates, and caption lookup where equivalence can be demonstrated. Parallelize independent page analysis with deterministic reduction for reading order, document font statistics, and repeated furniture. OCR uses separate provider/model/preprocessing keys and bounded work; recognition quality remains measured independently.

### 4. Plan writing by operation contract

Provide `WriterPlan` choices: local append, reachable-subgraph export, complete rewrite, and offset-sensitive linearization. Local append is suitable only where retained history is permitted; sanitize, redaction, encryption changes, canonicalization, and standards conversion use their required full-output paths. Split/merge preserve reachable resource sharing and catalog/page-reference rules. Lock/unlock process every required encrypted byte once. Signing retains full ByteRange hashing. Repair retains malformed-input classification and bounded recovery.

Stream unchanged bytes and compressed payloads when legal. Preserve a generation-sensitive reference rewrite map. Use borrowed objects, reusable serialization buffers, and bounded parallel compression/encryption of independent payloads with ordered assembly. Do not decode/recompress a stream merely to copy it. Batch mutations, construct one output revision, and perform one coordinated validation pass over the exact published output.

### 5. Accelerate conversions and editing

Reuse prepared projections for HTML/Markdown/JSON/XLSX. Reuse media by source/policy identity before decode and PNG encoding; avoid recompressing already compressed media inside ZIP where useful. Stream Office XML/media packages and bound in-flight image bytes. Font/glyph definitions are shared for SVG/PS/EPS; any regional raster fallback preserves compositing, clipping, and stacking with unaffected vectors.

Editing sessions retain font/layout/provenance products. Apply the existing prepared-plan path consistently across Rust, server, CLI, C, Java, Python, JS/WASM, and other supported bindings. Add compatible session, batch, and streaming entry points where a byte-only API forces repeated open/copy/close. Reflow begins at the changed paragraph and stops only when downstream content, geometry, styles, anchors, and pagination state match. Validate affected content plus verified preservation conditions; retain independent reopen/render checks. Performance work does not bypass them.

### 6. Correct and accelerate raster execution

First trace CMYK JPEG samples through decoder marker/inversion handling, `/Decode`, transfer/default color policy, ICC conversion, alpha, and compositing. The eight-anchor DeviceCMYK fallback is a fidelity concern, but the precise causal contribution to the observed cover failure needs stage probes. Reuse existing CMM infrastructure; supply explicit default CMYK profiles and consistent profile-based transforms across native and portable targets. Preserve rendering intent, output intent, overprint, and masks. Avoid a second transform or premature conversion of components needed by overprint.

Implement bounded clip origins/extents, uniform interiors, edge tiles, and lazy DAG intersections. Prepare filter coefficients and transforms outside pixel loops. Use exact coverage for bitonal reduction; use separable filters for orthogonal transforms and a filtered pyramid/EWA strategy for arbitrary affine minification. Quality mode must affect its cache key and execution. Codec reduced-resolution/ROI decode is admitted only after equivalent-output or defined quality-contract verification.

Compile paint operations with state and spatial bounds. Build a tile operation index so tile workers do not replay every operation. Include stroke, antialiasing, filter, and mask halos. Preserve original order within each tile, group isolation, knockout, soft-mask, blend, and backdrop dependencies; use a wider serial/group fallback until dependencies are expressible. Share font/image/program data; allocate tile-local output and scratch buffers. Coordinate page and tile concurrency to avoid nested oversubscription. Persist prepared fonts and use adequate TrueType/CFF hinting and rendering support.

### 7. Tune scheduling and complete integration

Use a single bounded worker budget governed by both CPU and memory. Keep small requests on a cheap serial path; select page/tile/decode work by estimated cost. Reuse scratch buffers, reduce allocation and lock contention, prefetch only likely ranges, and stop superseded work. Benchmark one-core latency and equal-core throughput separately. Test SIMD/scalar and native/WASM results under their declared contracts. Public binding changes preserve existing byte APIs while offering real retained sessions and streaming output.

## Validation and success gates

All Rust processes and PDF runtime testing run on the supplied IPv6 VPS. Use the existing 150 PDFs for before/after attribution plus a separately selected holdout set covering xref/ObjStm variations, revisions, encryption, malformed recovery, tagged content, RTL/vertical text, fonts, CMYK/ICC/DeviceN, transparency, scans, dense vectors, and large files. Extend size/page distributions; do not infer generality from these 150 alone.

Freeze operation contracts, source/tool/binary versions, fonts, profiles, page boxes, DPI, output format, CPU allocation, memory, timeout, cache state, and quality settings. Randomize order and repeat trials; report failures/timeouts and unconditional timing alongside qualified-output timing. Record P50/P90/P95/P99/max, throughput, RSS, allocation/copy/decode counts, confidence intervals, and work-normalized measures. Memory-resident open and path-backed open are separate workloads.

| Workload | Correctness evidence | Performance objective |
| --- | --- | --- |
| Open, page access, xref/object traversal | Page tree, active generations, reachable references, decoded content; malformed/repaired outcomes | Match or beat PDFium across the declared open/access workloads, including tails; retain memory bounds |
| Structural, security, signing, standards | Reopen plus operation-specific page/content/resource/tag/annotation/security checks; independent qpdf/MuPDF and relevant validators | Beat the appropriate competing operation at equal output contract; document unavoidable full-file cost |
| Text, tables, semantic/Office export | Text precision/recall, reading order, table-cell/structure metrics, image/font/relationship/layout checks with reviewed truth | Remove duplicate analysis and media work; reduce cold and multi-output time without losing content |
| Editing and reflow | Selected-source change/removal, unrelated preservation, Unicode, repeated save/reopen/edit, layout/anchors/tags | Reduce planning, mutation, save and proof separately; batch benefits must appear end to end |
| Rendering | Native-resolution pairwise/reference-disagreement analysis, SSIM/FLIP where available, edges/thin lines, OCR text, color differences, reviewed failures | Close correctness failures, then seek at least 10% lower time than the fastest comparator under an equal declared contract |

Publish exactly two top-level benchmark tables: parsing/document facilities and visual rendering, with tools as columns and workloads/metrics as rows. Editing occupies rows in the first table; per-operation stage records remain linked evidence. Show current measurements, appropriate unsupported cells, the worst real quality examples, and complete failure counts.

Numerical gains are hypotheses until measured. A 35% hot-path fraction made ten times faster improves total time by only `1 / (0.65 + 0.35/10) = 1.46x`; removing clips alone cannot explain away a fivefold render gap. Encryption/signing/full-output delivery require work proportional to the required bytes. Arbitrary PDFs cannot share a universal fixed millisecond maximum. A 'best' claim requires a named workload, competitors, quality contract, and reproducible winning evidence.

## Research foundations

- [Self-adjusting computation and dependency propagation](https://www.cs.cmu.edu/~blelloch/papers/ABBHT09.pdf): reuse unchanged computation with explicit dependencies; measure bookkeeping overhead.
- [Parallel self-adjusting computation](https://arxiv.org/abs/2105.06712): sequential/parallel dependencies constrain safe change propagation.
- [TinyLFU admission](https://arxiv.org/abs/1512.00727) and [size-aware cache management](https://arxiv.org/abs/2105.08770): candidate foundations for admission under variable artifact sizes, not guaranteed gains for this workload.
- [SIMD structural parsing research](https://arxiv.org/abs/1902.08318): bounded byte classification and staged validation; PDF-specific state remains necessary.
- [Texture-filtering survey](https://graphics.stanford.edu/courses/cs248-02/heckbert_texsurv.pdf): exact rectangular integration, prefiltering, and EWA tradeoffs.
- [Little CMS tutorial](https://www.littlecms.com/LittleCMS2.17%20tutorial.pdf): profile-driven color transforms and intent handling.
- [qpdf design](https://qpdf.readthedocs.io/en/stable/design.html): lazy indirect-object access and stream/object handling.
- [MuPDF execution guidance](https://mupdf.readthedocs.io/en/latest/reference/c/overview.html): display-list execution and worker resource sharing.

The user can authorize implementation by saying continue. The sequence above defines the work and evidence gates; this plan itself makes no completion or speed claim.
