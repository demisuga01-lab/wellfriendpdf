# Prompt 17 interactive-document and redaction audit

## Starting checkpoint

Prompt 17 started from clean commit `f063ab00d9afa9f9bc258d85ebb24d0db6833ab9` (`Complete combined prompt 16 xfa runtime sandbox foundation`). `git status --short` was empty. The Prompt 03-16 gate scripts and Prompt 16 tests/docs/artifacts were present. The machine-readable checkpoint is generated at `target/prompt17-annotation-xfdf-media-redaction/prompt17-starting-state.json`.

## Canonical owners inspected and extended

| Concern | Canonical owner used by Prompt 17 |
| --- | --- |
| bounded XML | `crates/engine/src/xfa/xml.rs` |
| AcroForm FDF/XFDF values | `crates/engine/src/form_exchange.rs` (left field-only) |
| annotation/form/page inventory | `crates/engine/src/interactive.rs` |
| annotation mutation, flattening, redaction, image cloning | `crates/engine/src/editing.rs` |
| annotation AP selection/rendering | `crates/engine/src/render/page_renderer.rs` |
| active-content scan/sanitizer | `crates/engine/src/security.rs` |
| deterministic object writing | `crates/engine/src/writer.rs` |
| stable public envelopes | `crates/engine/src/sdk.rs` |
| Prompt 17 typed orchestration | `crates/engine/src/prompt17.rs` |

`form_exchange.rs` remains the canonical scalar AcroForm field exchange path. Annotation XFDF was added separately because its geometry, page mapping, relationships, policy, appearance, and transaction semantics are not field-value semantics. Prompt 17 does not introduce another PDF parser, writer, renderer, sanitizer, or image decoder.

## Implemented closure

- Annotation XFDF uses the existing bounded XML parser, exact XFDF namespace validation, UTF-8 validation, DTD/entity rejection, capped nodes/attributes/depth/text, stable IDs, deterministic order/escaping, page mapping, popup/reply relationships, scalar safe extensions, action inventory by kind/digest, create/update, explicit-ID delete, conflict policy, AP regeneration policy, reopen verification, hashes, and signature-impact reporting.
- Appearance generation writes deterministic Form XObject `/AP` streams with `/N`, `/R`, `/D`, `/BBox`, rotation-aware `/Matrix`, resources, ExtGState opacity/blend, fonts, stable state/resource naming, border dash/cloud effects, line endings, and policy decisions. Supported generation covers FreeText, Line, Square, Circle, Polygon, PolyLine, text markup, Stamp, Caret, Ink, Text/FileAttachment icons, common Widgets, and repeated or single-text Redact previews. The page renderer now renders valid AP for every annotation subtype rather than Widgets alone.
- Rich-media policy inventories RichMedia, Sound, Movie, Screen, Rendition/MediaClip, 3D, assets, embedded streams, URLs, JavaScript associations, activation data, AP posters, MIME/hash/byte data, and provenance without decoding or execution. Six explicit modes share the canonical sanitizer. Static-poster flattening is subtype-selective and keeps unrelated annotations/widgets intact.
- Non-axis image redaction accepts page polygons, maps rotated CropBox coordinates, inverts arbitrary finite affine image CTMs, conservatively rasterizes the real polygon in sample space, supports 8-bit Gray/RGB/CMYK decoder output, clones the affected XObject invocation with a deterministic resource name, omits Mask/SMask reachability from rewritten clones, and removes only the affected invocation when secure partial rewrite is unavailable. Inline-image groups and unsupported/nested Form invocations use secure removal; strict policy fails closed. Overlay marks are never counted as secure redaction.
- Rust, CLI, Python, versioned C ABI, WASM, .NET, and Java call the same SDK facade. The additive feature key is `prompt17_annotation_xfdf_media_nonaxis_redaction`.

## Security invariants

No XML external lookup, network request, filesystem resource, media player, JavaScript, Flash/SWF, 3D JavaScript, launch action, Rendition action, or media codec is executed by Prompt 17 policy code. Imported XFDF action metadata never creates active PDF actions. Full rewrite is required for mutations. Unsupported redaction never falls back to an overlay-only success claim.

## Evidence and validation

`scripts/prompt17_interactive_redaction_audit.py` generates the feature matrix, corpus manifest, reference/metamorphic/security/performance evidence, HTML report, and every named Prompt 17 JSON artifact under `target/prompt17-annotation-xfdf-media-redaction/`. Focused engine coverage is in `crates/engine/tests/prompt17_interactive_redaction.rs`.

Final executed validation:

- all local compilation, tests, package gates, and reference audits were run serially with `CARGO_BUILD_JOBS=1`, Cargo `--jobs 1` where accepted, and `RAYON_NUM_THREADS=1` under the repository's 4096 MiB scheduler/report ceiling;
- strict format and all-feature/all-target Clippy passed;
- the complete default-feature workspace suite passed, including integration, server, and doc tests;
- all 25 out-of-workspace fuzz bins compiled;
- the Prompt 03 release/package manifest recorded 16/16 passed steps;
- the wheel was installed into a new isolated venv and its 15-test Python suite passed;
- .NET tests/pack, Maven tests/package/runtime smoke, Gradle tests/build/runtime smoke, C ABI tests/header syntax, WASM target, and wasm-pack Node/web smokes passed;
- the Prompt 02B C ABI/.NET/Java memory-lifecycle stress gate passed;
- all retained Prompt 04-16 audit entrypoints passed;
- the Prompt 17 focused suite passed 7/7;
- qpdf reopened all mutated outputs, all deterministic/metamorphic checks passed, and Poppler/PDFium/MuPDF/Wellfriend all rendered the generated static appearances with zero supported-row Wellfriend outliers.

The generated Prompt 17 verdict records zero blocked rows, unclassified failures, security-proof failures, and overlay-only success claims. The Windows harness records the configured 4 GiB cap and serial validation posture; it does not claim a portable measured subprocess peak RSS. Unavailable Valgrind/Windows-local ASan/TSan commands are not counted as passed; the repository's Linux sanitizer workflow remains the recorded external gate. The required commit and clean post-commit status are the final closure steps.
