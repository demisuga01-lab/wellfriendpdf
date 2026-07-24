# Prompt 09B Validation Closure Audit

Prompt 09B closes the proof gaps left after Combined Prompt 09. It does not add a new renderer feature block; it adds auditable evidence for annotation appearance rendering, OCG/layer visibility, progressive tile resume, tile/band/cache equivalence, public report parity, and multi-reference validation.

## Starting Checkpoint

- Expected starting HEAD: `df9fa7d Complete combined prompt 09 annotation ocg progressive cache parity`
- Observed starting HEAD: `df9fa7d`
- Starting worktree status: clean (`git status --short` produced no entries before edits)
- Artifact root: `target/prompt09-annotation-ocg-progressive-cache/`
- Closure script: `scripts/prompt09b_validation_closure_audit.py`
- HTML report: `target/prompt09-annotation-ocg-progressive-cache/prompt09b-html-report/index.html`

## Prompt 09 Result Audited

Prompt 09 changed the renderer in these areas: `/Properties` parsing, default-view OCG/OCMD evaluation, marked-content visibility stack, OCG checks for XObjects/annotations/patterns/shadings, OCG visibility fingerprints in tile cache keys, in-process progressive tile checkpoint/resume, feature report fields, docs, and artifact generation.

Prompt 09B classified the remaining closure gaps:

- Annotation subtype/style matrix: `implemented_and_proven`
- OCG cache fingerprint stale-reuse guard: `implemented_and_proven`
- Progressive invalid token rejection: `implemented_and_proven`
- Tile/band/cache equivalence metrics: `implemented_and_proven`
- Public report/binding feature-report surface: `implemented_and_proven`
- Alternate OCG configuration selection: `unsupported_reported`
- Non-widget generated annotation shapes: `unsupported_reported`
- CJK/RTL/color glyph: `not_in_prompt09_scope`

## Corpus

Prompt 09B generates eight deterministic PDFs:

- `prompt09b_tile_band_progressive_vector.pdf`
- `prompt09b_widget_ap_stream.pdf`
- `prompt09b_widget_missing_ap_generated.pdf`
- `prompt09b_ocg_marked_content_hidden.pdf`
- `prompt09b_ocmd_allon_hidden.pdf`
- `prompt09b_xobject_ocg_hidden.pdf`
- `prompt09b_annotation_ocg_hidden.pdf`
- `prompt09b_pattern_shading_ocg_hidden.pdf`

The corpus covers annotation AP streams, generated widget missing-AP policy, OCG marked content, OCMD `AllOn`, OCG-gated Form XObjects, OCG-gated annotations, and pattern/shading operations hidden by marked-content OCG state.

## Multi-Reference Result

Prompt 09B uses the Prompt 06B reference manifest and requires Poppler, PDFium, and MuPDF to be available. The closure run produced:

- Pages: 8
- Classification counts: 5 `all_references_agree_wellfriendpdf_pass`, 2 `reference_disagreement_wellfriendpdf_inside_cluster`, 1 `unsupported_reported_expected`
- Wellfriend outlier failures: 0
- Unclassified failures: 0

Artifacts:

- `target/prompt09-annotation-ocg-progressive-cache/multi-reference-render-results-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/multi-reference-diff-metrics-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/reference-disagreement-summary-prompt09b.json`

## Closure Artifacts

- `target/prompt09-annotation-ocg-progressive-cache/prompt09b-closure-audit.json`
- `target/prompt09-annotation-ocg-progressive-cache/annotation-appearance-matrix-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/ocg-layer-matrix-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/progressive-resume-equivalence-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/tile-full-equivalence-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/band-full-equivalence-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/cache-equivalence-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-performance-prompt09b.json`
- `target/prompt09-annotation-ocg-progressive-cache/tile-band-cache-memory-prompt09b.json`

## Public Report Surface

`feature_report_json()` now includes the additive section `prompt09b_annotation_progressive_cache_validation`. The report envelope version is unchanged. The section points to Prompt 09B artifacts and exposes annotation coverage counts, OCG validation, cache fingerprint status, progressive equivalence, tile/full equivalence, band/full equivalence, cache/no-cache equivalence, multi-reference status, outlier count, unclassified count, and bounded limits.
