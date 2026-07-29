# Font Failure Analysis Font Closure

Font Failure Analysis targeted two Font Subsystem failures: generated PDF font programs were still full embedded, and the 24-file Poppler font slice did not improve.

## Implemented

| Area | Status | Notes |
| --- | --- | --- |
| sfnt/glyf subsetting | DONE WITH BOUNDED LIMIT | TrueType/OpenType sfnt fonts with `glyf`/`loca` outlines are subset for generated Type0/CIDFontType2 output. |
| GID strategy | DONE WITH BOUNDED LIMIT | Preserves original glyph IDs, prunes unused `glyf` records to empty entries, and keeps CIDToGIDMap simple and correct. This is less compact than dense GID renumbering but avoids composite rewrite risk. |
| Composite glyphs | DONE | Composite dependencies are recursively included with a depth cap; malformed composites fall back instead of corrupting output. |
| sfnt tables | DONE WITH BOUNDED LIMIT | Rebuilds `glyf`, `loca`, table directory checksums, `head.checkSumAdjustment`, and `head.indexToLocFormat`; copies other required tables intact under preserved-GID semantics; drops stale `DSIG`. |
| Generated PDF integration | DONE | Type0/CIDFontType2 `/FontFile2` embeds subset bytes, not the original full font. Subset font names use deterministic six-letter tags. |
| Extraction/copy for shaped output | DONE | Authoring emits `/ActualText`; text collection now honors inline `/ActualText` replacement once per marked-content span so shaped RTL output extracts in logical order. |
| Visual fix attempted | PARTIAL | Non-CID true `.notdef`/control glyph painting is skipped while preserving custom-named Type1C glyphs. This improved local Standard14/font-edge metrics but not the aggregate benchmark. |

## Subsetting Scope

Supported:

- sfnt TrueType/OpenType fonts with `glyf` and `loca`.
- Generated Type0/CIDFontType2 authoring path.
- Latin/simple text and rustybuzz-shaped Arabic output already routed through Font Subsystem shaped CID emission.
- Composite dependency closure.
- Deterministic output.

Fallback:

- CFF/CFF2/OpenType `OTTO` fonts.
- TrueType collections.
- Malformed table directories, `loca`, `glyf`, composite data, or unsupported table shape.
- Resource-limit hits such as invalid glyph count or composite-depth overflow.

Fallback is explicit in the embedded font stream under `/WellfriendSubset`, with code/reason fields. The main stable diagnostic codes are:

- `font.subset.fallback.unsupported_format`
- `font.subset.fallback.malformed_glyf`
- `font.subset.fallback.resource_limit`

## Benchmark Result

Font Failure Analysis reran the same 24-file Font Subsystem slice. See `docs/font_font_failure_analysis_failure_analysis.md`.

| Run | Weighted score | Visual pass | Artifact |
| --- | ---: | ---: | --- |
| Font Subsystem | 45.21 | 45.83% | `target/font_subsystem-font-render-benchmark/` |
| Font Failure Analysis v2 | 45.21 | 45.83% | `target/font_failure_analysis-font-render-benchmark-v2/` |

The score did not move. Font Failure Analysis is therefore a production generated-output closure, not a visual-benchmark closure. The remaining score blockers are existing-PDF rendering fidelity gaps in Type1C/CFF positioning/rasterization, CJK/RTL raster drift, and two blank-reference mismatches where Poppler produced blank pages while Wellfriend rendered content.

## Native FreeType/HarfBuzz Decision

Native FreeType and native HarfBuzz remain out of the default engine for the Codec Boundary phase. The implemented closure uses the existing pure-Rust stack:

- `ttf-parser` for sfnt parsing and validation.
- Existing rustybuzz-backed shaping for generated complex-script output.
- Existing pure-Rust glyph outline/raster path.

The visual benchmark evidence shows that if Codec Boundary is judged by Poppler pixel parity alone, the next highest-impact work is a dedicated CFF/Type1C/raster-positioning pass or an optional native raster backend. That is not hidden as a completed Font Failure Analysis item.

## Remaining Limits

- Dense GID renumbering is not implemented; the subset writer preserves original GIDs.
- CFF/CFF2 subsetting is not implemented.
- Variable font subsetting is not implemented.
- Color-font subsetting/rendering is not implemented.
- Existing-PDF Type1C/CFF visual fidelity still trails Poppler on `font_ascent_descent` and `glyph_accent`.
- The 24-file visual benchmark score remains 45.21.

## Font Rendering Failure Superseding Note

Font Rendering Failure closed the `glyph_accent` Type1C/CFF rendering blocker by adding a
bounded pure-Rust CFF charset/name fallback and `seac` composition fallback for
SID-keyed bare CFF simple fonts. The same 24-file font slice moved from
weighted score `45.21` to `47.5`, visual pass `45.83%` to `50.0%`, with
`pdfjs_full_glyph_accent` changing from fail to pass and no regressions.

`font_ascent_descent`, CJK/RTL drift, and blank-reference mismatches remain
separate bounded items documented in `docs/font_font_rendering_failure_failure_analysis.md`.
