# Font Final Parity Final Font Parity Audit

Font Final Parity started from `4c1c0d8 Complete Font Rendering Failure font visual closure` with
a clean worktree. The goal was not another broad font rewrite; it was a final
audit of the concrete font/text semantics that could still explain the
remaining 24-file Poppler font-slice failures.

Poppler was used only as a behavioral oracle through command-line rendering and
benchmark output. No Poppler/GPL implementation code was copied or transcribed.
The Rust changes describe PDF, CFF, and text-state behavior in spec terms and
through local fixtures.

## Benchmark Anchor

Font Final Parity reran the exact original 24-file font/text slice:

```powershell
python renderer-benchmark\scripts\renderer_benchmark.py --manifest renderer-benchmark\corpus\manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --category real-cjk-text,real-font-edge,real-rtl-text --output-dir target\font_final_parity-font-after-audit --dpi 72 --max-pages-per-file 1 --timeout-sec 120 --max-memory-mb 2048 --determinism-sample 5 --threshold-profile renderer
```

| Metric | Font Rendering Failure anchor | Font Final Parity after audit |
| --- | ---: | ---: |
| Files | 24 | 24 |
| Visual pages compared | 24 | 24 |
| Visual pages passed | 12 | 12 |
| Visual pass | 50.0% | 50.0% |
| Weighted score | 47.5 | 47.5 |
| Determinism | 5/5 stable | 5/5 stable |
| Peak Wellfriend memory | 11.99 MB | 11.55 MB |
| Poppler | 26.02.0 | 26.02.0 |
| PDFium | unavailable | unavailable |

The score did not move above the Font Rendering Failure anchor. The blocker list also did
not change: CJK and RTL large-region/raster drift, two blank-reference
mismatches, and `font_ascent_descent.pdf` remain. The targeted audit found no
evidence that those remaining failures are caused by untested TJ sign handling,
Tr invisible painting, partial ToUnicode fallback, Standard14 provider routing,
or the bounded bare-CFF fallback used for `glyph_accent`.

## Final Checklist

| Requested item | Current implementation status | Tests found before Font Final Parity | Missing test before Font Final Parity | Bug found / no bug found | Fix applied / no fix needed | Benchmark or fixture evidence | Remaining bounded limit |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Type1C/CFF charstring width handling. | Primary sfnt/CFF path is delegated to `ttf-parser`; the bounded bare-CFF fallback parses simple Type2 outlines and treats optional width operands as non-geometry. | `glyph_accent` benchmark and existing bare-CFF outline tests covered seac composition and simple extraction. | No direct test proved optional width operands were dropped from geometry. | No benchmark regression from width geometry was found, but coverage was incomplete. | Added `type2_width_operand_is_not_outline_geometry` and stem/hintmask width tests. | `pdfjs_full_glyph_accent` remains a 100.0% pass after the audit. | The local bare-CFF fallback still rejects subroutines and does not become a full CFF engine. |
| CFF defaultWidthX / nominalWidthX. | Full CFF width resolution is handled by the primary parser/renderer path and PDF `/Widths`/`DW` data where available; local bare-CFF fallback is outline-only. | Existing rendering covered the failing Type1C accent case. | No focused local fallback test documented the default/nominal-width boundary. | No active benchmark failure was traced to defaultWidthX/nominalWidthX. | Added tests proving explicit width operands do not distort outlines; kept width advancement separated from bbox/outline geometry. | `font_ascent_descent.pdf` remains a large-region mismatch, but the audit did not identify default/nominal width as the proven cause. | A future full pure-Rust CFF interpreter would need private-dict width arithmetic; current production path relies on `ttf-parser` and PDF widths. |
| CFF FontMatrix application. | The bare-CFF fallback normalizes the standard 0.001 Type1C coordinate scale into PDF's 1000-unit text space; Type3 FontMatrix behavior is separate. | Font Rendering Failure covered default Type1C seac rendering through the benchmark. | No deterministic non-default CFF FontMatrix fixture was available in the 24-file slice. | No double-application or missing default-matrix bug was found. | No code change beyond documenting the evidence; the audit keeps non-default bare-CFF FontMatrix as bounded. | The default Type1C accent fixture remains pixel-identical. | Non-default bare-CFF FontMatrix fixtures are not yet supported by the local fallback; no current slice file proves it is the blocker. |
| local/global subr bias. | The authoritative CFF path delegates subroutine execution to `ttf-parser`; the local fallback refuses subroutines instead of guessing bias. | Existing tests did not explicitly assert fallback rejection. | Safe subroutine rejection was not directly tested. | No subr bias bug was found in the primary path. | Added `type2_fallback_rejects_subroutines_instead_of_guessing_bias`. | The benchmark has no changed subr-heavy page after Font Final Parity. | Implementing a full local Type2 subr executor remains outside the fallback scope unless `ttf-parser` cannot cover a required fixture. |
| FDArray / FDSelect for CID-keyed CFF. | CID-keyed CFF support is reachable through the primary parser path; the local bare-CFF fallback does not parse FDArray/FDSelect. | Font Rendering Failure failure analysis identified CJK/RTL drift but not a specific FDSelect failure. | No compact CID-keyed CFF FDArray fixture exists in the committed tests. | No FDArray/FDSelect bug was proven in the 24-file slice. | No code change; documented exact delegation and limit. | Remaining CJK/RTL mismatches are still categorized as raster/fallback metrics or reference issues, not confirmed FDSelect defects. | A dedicated CID-keyed CFF fixture pack is still needed before extending the local fallback. |
| PDF text-state positioning math. | `GraphicsState` models BT/ET, Tm, Td, TD, T*, Tj, TJ, quote operators, Tc, Tw, Tz, TL, Ts, and Tr for trace/analysis; renderer text uses decoded glyph widths. | Existing tests covered basic text matrix movement. | Quote operators, text rise, spacing, and TJ sign/scaling were not covered together. | Found an approximation gap: string advance in the content-state trace path ignored word spacing and TJ string spacing. | Added `approx_text_advance_for_bytes` and deterministic glyph-position tests for Tm/Td, quote operators, spacing, rise, and rendering mode. | No 24-file benchmark page changed, which indicates the remaining anchor failures are not due to this trace-path approximation. | The content-state helper remains approximate because it does not resolve real font widths. |
| TJ array displacement. | TJ numbers move the text position by the negative PDF text-space adjustment, scaled by font size and horizontal scaling. | Some content-state advance tests existed. | No focused positive/negative TJ test with `Tz` existed. | No sign bug was found; the test locks the existing semantics. | Added `tj_displacement_sign_and_horizontal_scaling_match_pdf_semantics`. | Benchmark unchanged after rebuild. | Vertical TJ behavior remains governed by the renderer's vertical text path and was not identified as an active 04E blocker. |
| Tr invisible/clipping text modes. | Rasterizer does not paint mode 3 or mode 7; modes 4/5/6 paint according to fill/stroke but the engine does not yet accumulate glyph outlines into the clipping path for later drawing. | Invisible mode had coverage. | Clipping text mode no-paint/paint split was not covered. | No evidence tied the blank-reference mismatches to Tr. | Added tests that mode 7 does not paint and mode 4 still paints fill. | Blank-reference blockers are unchanged and remain classified as reference/environment or non-font visibility artifacts pending another reference renderer. | Full text clipping path accumulation is a renderer clipping feature, not closed by Font Final Parity. |
| CMap and ToUnicode fallback behavior. | ToUnicode remains first; missing entries fall back through encoding/Differences and AGL/glyph names; Identity-H/V and predefined UTF16 CMaps were improved in Font Subsystem. | Tests covered ToUnicode first and Identity/predefined paths. | Partial ToUnicode plus glyph-name fallback was not directly asserted. | No fallback-order bug was found, but coverage was incomplete. | Added `partial_to_unicode_falls_back_to_glyph_names_for_missing_codes`. | Extraction gates remain unchanged. | Legacy Adobe CMap packs beyond the supported set remain a bounded later-corpus item. |
| Splash/FreeType glyph raster reference posture. | Default engine remains pure Rust and WASM-safe; Poppler/Splash/FreeType is a behavioral reference only. | Font Rendering Failure docs compared Poppler outputs and warnings. | No final explicit posture note tied the unchanged score to raster/hinting evidence. | No evidence justified adding native FreeType before Decode Scheduler. | Documented the final Codec Boundary decision: no native raster backend in the default engine; use future optional backend only if a dedicated raster benchmark proves it. | Remaining drift is mostly CJK/RTL glyph/raster/reference mismatch after semantics tests passed. | Native FreeType-style hinting remains an optional future backend, not a Codec Boundary blocker. |
| Font substitution and Standard14 handling. | Bundled deterministic provider maps Standard14 families to Liberation/DejaVu fallbacks, with symbolic fonts routed away from Latin fallback. | Existing Standard14 and symbolic render tests covered parts of this path. | No single provider test covered all Standard14 families and style variants. | No new substitution bug was found. | Added `standard_14_families_resolve_to_deterministic_bundled_faces`. | `pdfjs_full_standard_fonts` remains an edge/text-shift mismatch, but the provider route is deterministic and tested. | Exact Poppler Standard14 metric/raster parity remains a measured fidelity gap, not a missing provider route. |

## Blank-Reference Mismatches

The two blank-reference mismatches remain the same as Font Rendering Failure:

- `renderer-benchmark/corpus/real-world/pdfjs-full/arial_unicode_en_cidfont.pdf`.
- `tests/corpus/pdfs/pdfjs/issue5801.pdf`.

The Font Final Parity Tr tests reduce the likelihood that Wellfriend is simply painting
`Tr 3` or `Tr 7` invisible text. Font Rendering Failure had already recorded Poppler
warnings for missing language/display fonts. With PDFium unavailable in this
environment and no MuPDF result captured for this slice, these remain classified
as reference/environment or non-font visibility artifacts, not safe targets for
font-code changes.

## Gate Decision

Font Rendering Failure moved the visual gate from weighted score `45.21` to `47.5`. Roadmap task
04E did not move it further, but it closed the requested fundamentals audit with
tests for previously untested behavior. The remaining anchor blockers are now
bounded as:

- CJK/RTL visual drift that is likely raster/hinting/fallback-metric fidelity
  rather than missing CMap/TJ/Tr fundamentals.
- `font_ascent_descent.pdf`, still a Type1/CFF metric/raster parity gap without
  evidence of CFF width or FontMatrix mishandling in the covered paths.
- Two blank-reference mismatches that need a second reference renderer or
  benchmark policy decision before they can be used as font defects.
- Full text clipping path accumulation, which belongs to renderer clipping work,
  not the final font audit.

The font phase is therefore closed enough to move to Decode Scheduler color management
without hiding known font fundamentals. Future font work should be driven by a
dedicated raster/hinting benchmark or a new CID-keyed CFF fixture pack, not by
another broad Codec Boundary loop.
