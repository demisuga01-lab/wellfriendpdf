# Prompt 09 Known Limits

Prompt 09 removes vague future buckets for annotation rendering, optional content, progressive resume, and tile/cache behavior. Remaining limits are intentionally narrow and report-visible.

## Annotation Rendering

- Widget annotation `/AP /N` streams and state dictionaries render through the native Form XObject path.
- Bounded widget appearance synthesis covers text, button, and choice basics when the document posture requires it.
- Generated non-widget annotation appearances remain `unsupported_reported_expected`: Text icons, FreeText layout, line/polyline/square/circle/polygon/ink/markup/stamp generation, and rich-media playback. These are assigned to the later exact annotation-generation phase, not to CJK/RTL, advanced CMM, or fuzz close-out.
- Dynamic XFA remains out of scope.

## Optional Content

- Default view configuration is parsed and applied to marked content, XObjects, annotations, patterns, and shadings where the current resource/object dictionaries expose `/OC`.
- Supported visibility inputs: `/OCProperties`, `/OCGs`, default `/D`, `/BaseState`, `/ON`, `/OFF`, `/Intent`, `/Usage /View`, `/RBGroups`, `/Order`, `/Locked`, and OCMD policies `AnyOn`, `AllOn`, `AnyOff`, and `AllOff`.
- Alternate configuration selection and active Print/Export mode selection are parsed/reportable scope but not a public render option yet.
- Malformed or cyclic optional-content references fail open with diagnostics to avoid hiding content unexpectedly.

## Progressive Resume

- The implemented model is an in-process tile checkpoint job. Completed tile surfaces are retained by the job and are not re-rendered on resume.
- The resume token records page identity, DPI, render mode, tile geometry, page dimensions, next tile, completion counts, and OCG visibility fingerprint.
- Binding-level progress callbacks and cross-process serialized pixel resumes remain later binding work.

## Tile, Band, And Cache

- Tile and band rendering remain deterministic compatibility-safe full-page-render-plus-crop paths.
- The render tile cache key includes page number, DPI, render mode, tile rectangle, and OCG visibility fingerprint.
- Global image/Form/pattern/shading/clip-mask/transparency-group surface caches are not introduced in Prompt 09. The implemented cache remains a byte-budgeted render-tile cache with deterministic eviction.
- Parallel tile rendering is not enabled by default; Prompt 09 preserves deterministic output above speed.
