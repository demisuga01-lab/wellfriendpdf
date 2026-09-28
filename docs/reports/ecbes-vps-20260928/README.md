# WellPDF SDK 100-PDF VPS qualification - 2026-09-28

## Executive verdict

Commit `e2a83fc` contains the ECBES editing, rendering, API, and binding implementation qualified by this report.

On the fixed 100-PDF corpus, the release binary achieved:

- 100/100 verified source-text edits;
- 100/100 WellPDF semantic parses on both originals and edited outputs;
- 100/100 qpdf acceptance on both originals and edited outputs;
- 100/100 first-page WellPDF renders on both originals and edited outputs; and
- 100/100 comparable rasters against PDFium, MuPDF, and Poppler, with no reference-renderer failures.

These are 100/100 operational results for this corpus and workflow. They are not proof that every valid PDF is editable, and they are not pixel identity with the reference renderers. Median raster divergence remains material. No Acrobat build was installed or benchmarked, so this evidence does not support a universal or Adobe-beating claim.

## Reproducibility identity

| Item | Value |
|---|---|
| VPS | `2a0e:97c0:711:3bc::1` |
| Repository | `/root/wellpdfsdk-qualification/src` |
| Tested base HEAD | `27e62db3a1b84804339e65b6025273fd003b3736` plus the implementation working tree |
| Implementation commit | `e2a83fc` |
| Tested/local modified tracked files | 117, logical content matched after CRLF normalization |
| Tested/local untracked source files | 698, manifest SHA-256 `e2ff7335caba57a24c413f4653dab8db034a543da39ece68798b99463a705e18` on both systems |
| Release CLI SHA-256 | `879e2bc695790fc7523cf6a835b7149b2d6b7ee7a6dd542466930ecb18f83943` |
| Release CLI size | 66,780,304 bytes |
| Release C ABI SHA-256 | `44c67341ba197397fd4bb8988e345e722ed918994394cd76398c5436086f3401` |
| Original input manifest | `56d6ec3c6a94a08156b461132eabb1862065ce25210b721f9f4f317d352029b6` |
| Edited output manifest | `bcd373276d9aa065b9891b3bbfe31effb6d854e47d131b4b44291a68ced317be` |

The implementation was committed after qualification. Before commit, normalized content manifests proved that the VPS source and the local source being committed were identical. See [reproducibility.txt](reproducibility.txt), [corpus-manifest.tsv](corpus-manifest.tsv), and [validation-summary.json](validation-summary.json).

## Build, unit, and binding gates

| Gate | Result |
|---|---:|
| Focused renderer regression | 232 passed, 0 failed |
| Full Rust workspace | 5,399 passed, 0 failed |
| Optimized CLI and C ABI build | Passed |
| C ABI debug build | Passed |
| Java binding | 3 passed, 0 failed |
| .NET binding | 17 passed, 0 failed |
| Python optimized wheel and binding | 32 passed, 0 failed, 1 skipped |
| WASM/TypeScript binding | `npm ci` and typecheck passed |

The skipped Python test is recorded rather than counted as a pass. The first remote release-build shell lost its connection and recorded exit 101 without a Rust diagnostic; the detached retry completed successfully and produced the hashed binaries above. The earlier long Java debug run was deliberately terminated after its DOCX conversion exceeded the interactive window; the optimized release binding suite subsequently passed.

## Corpus and edit contract

The fixed corpus contains 81 arXiv papers, 16 IRS forms, one DARPA SafeDocs sample, one Mozilla PDF.js regression file, and one veraPDF conformance file. Originals were read-only.

For each file, the harness:

1. extracted page-one text with WellPDF and Poppler;
2. selected a word that was visible and unique to both extractors;
3. created a revision-bound operator-preserving edit plan;
4. explicitly approved an exact candidate and any reported font decision;
5. applied the edit to a new output file;
6. required the output hash and report to prove a real change;
7. required source-aware occurrence proof from the apply report;
8. reopened and extracted the result with WellPDF;
9. independently observed the replacement with Poppler; and
10. ran qpdf structural comparison against the original diagnostics.

The replacement preserves the selected word length. This is a broad source-edit qualification, not a claim that every possible replacement string, scan reconstruction, table mutation, font substitution, or document-wide reflow operation succeeds on every file.

### Editing result

| Outcome | Count |
|---|---:|
| Applied and fully verified | 100/100 |
| Verification failure | 0 |
| Typed refusal | 0 |
| Timeout | 0 |

Raw per-file operation evidence is in [editing-results.jsonl](editing-results.jsonl), with the aggregate in [editing-summary.json](editing-summary.json).

### Editing timing

The per-file `elapsed_ms` measurement covers the complete harness operation:
initial SDK and Poppler extraction, planning, approval where required, apply,
qpdf comparison, SDK reopen/extraction, and final Poppler verification.

| Measurement | P50 | P90 | P95 | P99 | Maximum |
|---|---:|---:|---:|---:|---:|
| End-to-end verified edit | 33,016.260 ms | 55,890.344 ms | 68,940.164 ms | 98,026.668 ms | 111,736.765 ms |
| Plan | 7,636.419 ms | 13,036.515 ms | 16,907.035 ms | 21,318.599 ms | 23,204.997 ms |
| Approval | 25.978 ms | 33.209 ms | 36.433 ms | 57.788 ms | 70.570 ms |
| Apply | 24,370.821 ms | 39,875.242 ms | 48,818.867 ms | 67,200.242 ms | 73,955.928 ms |

## Parsing comparison

`accepted` follows each tool's documented process result. qpdf warning exit 3 is accepted but separately reported. MuPDF was invoked through `mutool info`.

| Corpus | WellPDF | qpdf | MuPDF |
|---|---:|---:|---:|
| 100 originals | 100 accepted, 0 failed | 100 accepted: 97 clean, 3 warning | 95 accepted, 5 failed |
| 100 edited outputs | 100 accepted, 0 failed | 100 accepted: 99 clean, 1 warning | 95 accepted, 5 failed |

MuPDF's five failures are the same IRS form family before and after editing: `f1040.pdf`, `f1040sa.pdf`, `f1040sc.pdf`, `f1040sd.pdf`, and `f1040se.pdf`. Each reports `syntax error after element name`, associated with metadata parsing. WellPDF accepted all five, and the edit did not introduce the condition. See [mupdf-input-metadata-failures.json](mupdf-input-metadata-failures.json), [parse-original.json](parse-original.json), and [parse-edited.json](parse-edited.json).

### Parsing timing

These are process durations from 100 per-file observations. They are not an
equivalent-operation speed ranking: WellPDF emits a semantic document model,
qpdf performs a structural check, and MuPDF `mutool info` inventories document
resources.

| Tool and corpus | P50 | P90 | P95 | P99 | Maximum |
|---|---:|---:|---:|---:|---:|
| WellPDF, originals | 1,164.201 ms | 4,550.653 ms | 6,944.989 ms | 14,024.827 ms | 16,136.160 ms |
| WellPDF, edited | 1,109.898 ms | 4,727.693 ms | 6,484.281 ms | 15,025.047 ms | 15,919.602 ms |
| qpdf, originals | 182.814 ms | 1,331.799 ms | 2,029.282 ms | 7,946.815 ms | 11,974.864 ms |
| qpdf, edited | 191.145 ms | 1,305.994 ms | 2,138.305 ms | 8,167.515 ms | 11,679.947 ms |
| MuPDF, originals | 26.593 ms | 49.605 ms | 58.166 ms | 93.001 ms | 280.164 ms |
| MuPDF, edited | 26.404 ms | 49.009 ms | 55.323 ms | 166.890 ms | 327.036 ms |

## Rendering comparison

Every comparison rendered page one at 144 DPI. A page was comparable only when WellPDF and the reference renderer both produced an image with identical dimensions. `changed > 8` is the percentage of pixels for which at least one RGB channel differs by more than 8. Antialiasing and color-management choices contribute to this metric, so it is a diagnostic rather than a count of objectively wrong pixels.

### Operational result

| Corpus | WellPDF pages | PDFium comparable | MuPDF comparable | Poppler comparable | Failures |
|---|---:|---:|---:|---:|---:|
| Originals | 100/100 | 100/100 | 100/100 | 100/100 | 0 |
| Edited outputs | 100/100 | 100/100 | 100/100 | 100/100 | 0 |

### Pixel divergence distribution

These are fidelity percentiles across files, not render-time percentiles.

| Corpus | Reference | P50 | P90 | P95 | P99 | Maximum |
|---|---|---:|---:|---:|---:|---:|
| Originals | MuPDF | 8.379496% | 11.293026% | 11.791062% | 15.770450% | 31.288504% |
| Originals | Poppler | 9.161623% | 12.973691% | 13.444442% | 17.801277% | 31.454685% |
| Originals | PDFium | 10.087342% | 14.145231% | 14.624750% | 18.511040% | 32.420260% |
| Edited | MuPDF | 8.366498% | 11.277501% | 11.794105% | 15.751366% | 31.297379% |
| Edited | Poppler | 9.155820% | 12.949965% | 13.439491% | 17.781729% | 31.462513% |
| Edited | PDFium | 10.078393% | 14.142704% | 14.618561% | 18.493297% | 32.425445% |

The edited-output distribution closely tracks the original distribution, which is evidence against a broad renderer regression from this edit workflow. It does not mean the renderer is pixel-identical to any reference engine. Raw page-level results are in [render-original.json](render-original.json) and [render-edited.json](render-edited.json).

### Rendering campaign time

| Corpus | Workers | Pages | Wall time | Completed comparison-set pages/s | Mean campaign wall time/page |
|---|---:|---:|---:|---:|---:|
| Originals | 4 | 100 | 302.035 s | 0.331087 | 3,020.350 ms |
| Edited outputs | 4 | 100 | 282.391 s | 0.354119 | 2,823.910 ms |

Each comparison-set page includes WellPDF, PDFium, MuPDF, and Poppler renders
plus pixel comparison. The harness retained total campaign duration but not an
individual duration for each page. Therefore P90/P95/P99 render latency is not
recoverable from this evidence and is not fabricated from aggregate throughput.

## What changed relative to the 2026-09-27 run

The prior run observed 99/100 semantic parses, 99/100 compatibility renders, and only 6/100 verified edits. The new tested source state produced 100/100 in all three corresponding WellPDF operational gates. The implementation work that closed those observed failures includes deterministic source/provenance handling, strict no-op rejection, broader text/font routing, page-selection error handling, simple-font fallback when malformed ToUnicode data is recoverable, image/color decoding fixes, renderer fallback corrections, and adaptive nonlinear gradient approximation with explicit fail-closed behavior where exact preservation is unavailable.

| Campaign date | Source state | Semantic parse | Compatibility render | Verified source edit |
|---|---|---:|---:|---:|
| 2026-09-27 | Research snapshot over `27e62db` | 99/100 | 99/100 | 6/100 glyph-preserving |
| 2026-09-28 | Implementation `e2a83fc` | 100/100 originals and edited | 100/100 originals and edited | 100/100 |

The implementation commit timestamp is `2026-09-28T22:50:39+05:30`, and the
evidence commit timestamp is `2026-09-28T22:55:36+05:30`. The original harness
did not retain wall-clock execution start and finish timestamps; filesystem
copy times are not substituted for missing execution timestamps. Derived
latency values and their calculation rule are in
[timing-summary.json](timing-summary.json).

## Remaining claim boundaries

- The corpus is finite and biased toward research papers and US tax forms.
- Only page one was rendered for the cross-renderer visual comparison.
- The qualified edit is a visible, unique page-one word replacement; this is not exhaustive coverage of OCR reconstruction, vertical text, every embedded font program, shared-form occurrence editing, arbitrary color spaces, tables, annotations, signatures, encryption, or tagged-document repagination.
- Incremental edits are not sanitizing redaction; historical revisions may retain prior bytes.
- Visual rendering succeeds on all sampled pages, but divergence versus the three reference engines is still substantial on some pages.
- Acrobat was not part of the comparator set. “Better than Adobe” requires the same edit tasks against a named Acrobat version and measurable criteria.
- No finite test campaign proves “universal PDF editing.” Unsupported or ambiguous cases must continue to fail with typed, non-mutating outcomes.

## Defensible claim

The evidence supports:

> WellPDF provides native, revision-bound PDF source editing with a 100/100 verified success rate on this published 100-file workflow, plus 100/100 sampled parsing and first-page rendering in the same corpus, with independent qpdf, Poppler, PDFium, and MuPDF evidence.

It does not support an unrestricted “edit any object in any PDF” or “better than Adobe” claim.

## Evidence index

- [aggregate-summary.json](aggregate-summary.json): compact corpus results.
- [validation-summary.json](validation-summary.json): build, unit, release, and binding gates.
- [editing-summary.json](editing-summary.json) and [editing-results.jsonl](editing-results.jsonl): edit outcomes and per-file proofs.
- [parse-original.json](parse-original.json) and [parse-edited.json](parse-edited.json): WellPDF/qpdf/MuPDF parser records.
- [render-original.json](render-original.json) and [render-edited.json](render-edited.json): page-level renderer comparisons.
- [timing-summary.json](timing-summary.json): derived P50/P90/P95/P99 latency and campaign-throughput summary.
- [corpus-manifest.tsv](corpus-manifest.tsv): exact corpus hashes, sizes, and names.
- [reproducibility.txt](reproducibility.txt): toolchain and binary identity.
