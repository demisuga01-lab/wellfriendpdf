# Prompt 09 Annotation, OCG, Progressive, and Cache Audit

## Starting checkpoint

- Starting HEAD: `d3b27eb Complete prompt 08B type3 cid tensor closure`
- Observed starting HEAD: `d3b27eb`
- Starting worktree status: clean (`git status --short` produced no entries)
- Artifact root: `target/prompt09-annotation-ocg-progressive-cache/`
- Reference renderer policy: reuse Prompt 06B Poppler/PDFium/MuPDF bootstrap and manifest discipline. A missing reference renderer makes the affected row partial unless the target-local bootstrap proves it unavailable.
- Memory cap: Prompt 09 validation is run with the user-specified 4 GB cap in mind; stress artifacts report peak reservation posture rather than allowing unbounded surfaces.

## Prior audit contracts extended here

- Prompt 06/06B established native replay, compatibility fallback taxonomy, Poppler/PDFium/MuPDF manifests, multi-reference render artifacts, diff metrics, and HTML reports.
- Prompt 07/07B closed transparency groups, blend modes, soft masks, and knockout behavior with a zero-unclassified-outlier matrix.
- Prompt 08/08B closed text clipping, axial/radial and mesh/patch shadings, tiling patterns, Type3 clipping, CID fallback policy, and tensor patch posture.
- Prompt 09 must use the same matrix vocabulary: `all_reference_pass`, `reference_disagreement_oxide_inside_cluster`, `unsupported_reported_expected`, `malformed_reference_failure`, and `oxide_outlier`.

## Existing code audit

- Annotation AP streams already render through the native Form XObject path for widget annotations. Selection supports `/AP /N` streams and state dictionaries via `/AS`, with a bounded widget appearance synthesis fallback.
- Non-widget generated annotation appearances are not complete in the current renderer; they must be classified by subtype instead of being silently claimed.
- Page and Form resources already parse fonts, XObjects, color spaces, ExtGState, patterns, and shadings. Prompt 09 adds `/Properties` so optional-content membership can affect marked content and resource dispatch.
- The content interpreter previously parsed `BMC`, `BDC`, and `EMC` but treated them as no-ops. Prompt 09 owns marked-content visibility stacks.
- XObject, shading, and pattern dispatch are already centralized, which gives Prompt 09 clear OCG visibility gates.
- Tile rendering and band rendering already use deterministic full-page render plus crop semantics. Prompt 09 adds an OCG visibility fingerprint to cache keys and reports the current compatibility-safe posture.
- Cancellation already exists in full-page and display-list render loops. Prompt 09 adds a documented tile-level progressive checkpoint model and public report fields; binding callback APIs remain a later integration surface.

## Live implementation checklist

- [x] Verify starting status and previous prompt docs.
- [x] Create Prompt 09 audit document.
- [x] Add `/Properties` resource parsing.
- [x] Add optional-content inventory/report and visibility evaluator.
- [x] Apply OCG visibility to marked content, XObjects, annotations, patterns, and shadings where current structures expose membership.
- [x] Add OCG-aware cache keys.
- [x] Add progressive render resume/checkpoint report and tests.
- [x] Generate Prompt 09 artifact matrices and HTML report.
- [x] Update public feature report surfaces with Prompt 09 status.
- [x] Run focused renderer tests, audit script, formatting, and workspace validation as feasible.
- [ ] Commit cleanly with Prompt 09 closure status.

## Known limits that must stay explicit

- Dynamic XFA and rich-media playback are outside this renderer block.
- CJK/RTL raster parity and color-glyph rendering remain assigned to the next renderer block.
- Advanced ICC/device-link/multicolor CMM remains assigned to the prepress/CMM phase.
- Fully interactive binding-level progress callbacks are distinct from the engine's tile-level progressive checkpoint model.
