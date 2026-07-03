# Prompt 04C Font Closure

Prompt 04C targeted two Prompt 04B failures: generated PDF font programs were still full embedded, and the 24-file Poppler font slice did not improve.

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
- Latin/simple text and rustybuzz-shaped Arabic output already routed through Prompt 04B shaped CID emission.
- Composite dependency closure.
- Deterministic output.

Fallback:

- CFF/CFF2/OpenType `OTTO` fonts.
- TrueType collections.
- Malformed table directories, `loca`, `glyf`, composite data, or unsupported table shape.
- Resource-limit hits such as invalid glyph count or composite-depth overflow.

Fallback is explicit in the embedded font stream under `/OxideSubset`, with code/reason fields. The main stable diagnostic codes are:

- `font.subset.fallback.unsupported_format`
- `font.subset.fallback.malformed_glyf`
- `font.subset.fallback.resource_limit`

## Benchmark Result

Prompt 04C reran the same 24-file Prompt 04B slice. See `docs/font_prompt04c_failure_analysis.md`.

| Run | Weighted score | Visual pass | Artifact |
| --- | ---: | ---: | --- |
| Prompt 04B | 45.21 | 45.83% | `target/prompt04b-font-render-benchmark/` |
| Prompt 04C v2 | 45.21 | 45.83% | `target/prompt04c-font-render-benchmark-v2/` |

The score did not move. Prompt 04C is therefore a production generated-output closure, not a visual-benchmark closure. The remaining score blockers are existing-PDF rendering fidelity gaps in Type1C/CFF positioning/rasterization, CJK/RTL raster drift, and two blank-reference mismatches where Poppler produced blank pages while Oxide rendered content.

## Native FreeType/HarfBuzz Decision

Native FreeType and native HarfBuzz remain out of the default engine for the Prompt 04 phase. The implemented closure uses the existing pure-Rust stack:

- `ttf-parser` for sfnt parsing and validation.
- Existing rustybuzz-backed shaping for generated complex-script output.
- Existing pure-Rust glyph outline/raster path.

The visual benchmark evidence shows that if Prompt 04 is judged by Poppler pixel parity alone, the next highest-impact work is a dedicated CFF/Type1C/raster-positioning pass or an optional native raster backend. That is not hidden as a completed Prompt 04C item.

## Remaining Limits

- Dense GID renumbering is not implemented; the subset writer preserves original GIDs.
- CFF/CFF2 subsetting is not implemented.
- Variable font subsetting is not implemented.
- Color-font subsetting/rendering is not implemented.
- Existing-PDF Type1C/CFF visual fidelity still trails Poppler on `font_ascent_descent` and `glyph_accent`.
- The 24-file visual benchmark score remains 45.21.
