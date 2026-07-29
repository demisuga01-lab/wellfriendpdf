# Codec Boundary Font Subsystem Foundation

This document records the Codec Boundary font baseline and the implemented closure
work. It is intentionally an engineering status document, not a claim that
Wellfriend now matches mature font engines such as FreeType plus HarfBuzz across
every script and PDF font corner.

## Baseline Inventory

| Font area | Before Codec Boundary | Codec Boundary status |
|---|---|---|
| Type1 / Standard 14 | Standard encodings, Symbol/Zapf mappings, Type1 outline fallback, bundled Liberation/DejaVu fallback existed | Audited and exposed through deterministic `BundledFontProvider`; font-report diagnostics now identify Standard 14 substitution |
| TrueType / OpenType | Embedded font extraction and `ttf-parser` outline path existed | Kept as primary pure-Rust raster path; public `TextShaper` can use sfnt bytes for generated output |
| CFF / Type2 | Bare CFF and Type1C/CIDFontType0C fallback existed in renderer | Audited as delegated existing backend; no native dependency added |
| Type0 / CID | Identity-H/V detection, CID widths, W2 vertical metrics, CIDToGIDMap handling existed | Report now includes descendant type and writing mode; vertical mode is explicit in diagnostics |
| Type3 | CharProc rendering support existed through content streams | Report now identifies Type3 CharProc rendering as an explicit diagnostic |
| ToUnicode / CMaps | bfchar, bfrange, multi-byte maps, cap-bounded parsing existed | Kept as first-priority extraction path; fuzz coverage retained |
| Glyph-name recovery | Generated AGL table and `uniXXXX` / `uXXXX` support existed | Documented as the Standard 14/simple-font fallback; ligature expansion remains in resolver |
| Font substitution | Renderer selected bundled fallback fonts directly | Added public provider seam with deterministic bundled provider and Standard 14 detection |
| Glyph cache | Per-render LRU cache existed with entry-count cap | Added byte budget, hit/miss/eviction/oversized-skip metrics, and tests |
| Generated shaping | Internal renderer used rustybuzz for existing Arabic/Indic runs when needed | Codec Boundary added public `TextShaper`; Font Subsystem routes generated complex-script runs into shaped Type0 CIDs with ActualText and ToUnicode clusters |
| Writing / embedding | Authoring embeds full TrueType Unicode fonts as Type0/CIDFontType2 with ToUnicode | Font Subsystem emits subset CID maps/widths/ToUnicode; true sfnt glyph-program rewriting remains a documented safe-boundary decision |

## Architecture

The font stack is centralized around these modules:

- `crates/engine/src/fonts/resolver.rs`: PDF font dictionary resolution,
  encoding selection, ToUnicode priority, Type0/CID widths, vertical metrics,
  and character-code to Unicode fallback.
- `crates/engine/src/fonts/cmap.rs`: cap-bounded ToUnicode CMap parser.
- `crates/engine/src/fonts/glyph_list.rs`: generated Adobe Glyph List style
  glyph-name lookup plus `uniXXXX` and `uXXXX` conventions.
- `crates/engine/src/fonts/predefined_cmap.rs`: bounded predefined-CMap
  metadata for Identity and common Uni*-UTF16 CMaps.
- `crates/engine/src/fonts/provider.rs`: deterministic provider seam for
  embedded, Standard 14, bundled fallback, and future user/system providers.
- `crates/engine/src/fonts/shaper.rs`: generated-output text shaping facade.
- `crates/engine/src/render/text_decode.rs`: shared renderer/SVG/display-list
  character-code to glyph decode path.
- `crates/engine/src/render/glyph_outline.rs` and
  `crates/engine/src/render/font_rasterizer.rs`: pure-Rust outline extraction
  and raster fallback paths.
- `crates/engine/src/render/glyph_cache.rs`: per-render glyph outline cache with
  LRU eviction and byte accounting.
- `crates/engine/src/fonts_report.rs`: public font inventory and diagnostics
  exposed through Rust, CLI JSON, Python JSON, and C JSON surfaces.

The raster renderer now uses the shared `render::text_decode::decode_text_bytes`
path instead of maintaining a duplicate decoder in `page_renderer.rs`. This
keeps display-list/vector text and raster text aligned on the same PDF
font-code resolution behavior.

## Encoding and Unicode Recovery Priority

Wellfriend’s extraction/render text decoding follows this practical priority:

1. `/ToUnicode` CMap when present and parseable.
2. Simple-font `/Encoding` plus `/Differences`.
3. Adobe Glyph List style glyph-name recovery, including `uniXXXX` and `uXXXX`.
4. Ligature expansion for common Unicode presentation ligatures.
5. Type0/CID CMap code-size, predefined UTF-16 CMap metadata, and CID
   width/vertical metadata.
6. Last-resort Unicode scalar fallback or replacement character with logging.

`ActualText` and full reading-order semantics are intentionally left to the
semantic extraction roadmap task. Codec Boundary does not reorder existing PDF text or
reshape already-positioned glyph streams.

## Font Provider and Substitution

`BundledFontProvider` is deterministic and host-independent. It maps Standard
14 and common family/style names to bundled Liberation Sans, Liberation Serif,
Liberation Mono, or DejaVu Sans. Symbolic names such as Symbol, ZapfDingbats,
Wingdings, and Webdings route to DejaVu Sans for broader Unicode coverage.

The provider abstraction is ready for user-registered directories and optional
native/system providers, but Codec Boundary does not scan arbitrary host font
directories by default. This avoids non-reproducible server output.

## Rasterization and Cache

Glyph outlines remain pure Rust:

- sfnt TrueType/OpenType via `ttf-parser`;
- bare CFF / Type1C through the existing CFF support;
- Type1 through the existing Type1 parser;
- Type3 through PDF CharProc content streams.

The glyph cache is per-render scratch state. Codec Boundary added approximate byte
accounting, an 8 MiB default budget, oversized-entry rejection, and stats:
hits, misses, evictions, skipped oversized entries, and bytes currently held.
The cache does not hold cross-document references.

Hinting remains limited to the existing lightweight renderer policy. Full
FreeType-style bytecode hinting/autohinting is not integrated.

## Generated Text Shaping

`TextShaper` is a public generated-output facade:

- Latin/default text uses deterministic cmap/advance fallback.
- Arabic and Indic script runs use rustybuzz when the font bytes parse as an
  sfnt face.
- The result records glyph IDs, clusters, advances, offsets, direction, and
  whether complex shaping was used.

This is not applied to existing PDF content streams. Existing PDFs already
carry positioned glyph codes. Re-shaping them would be incorrect.

Font Subsystem routes generated complex-script runs through the same shaping facade
and emits shaped CID glyph streams for Type0/CIDFontType2 output. The writer
adds `/ActualText` for the logical source text and maps each shaped CID back to
its Unicode cluster in ToUnicode. Latin/simple runs keep the scalar CID path for
stable output.

## Writing, Embedding, and Subsetting

Existing authoring has one concrete safe font output path:

- Builtin/custom Unicode text can be emitted with an embedded TrueType font as
  Type0/CIDFontType2.
- Widths, CIDToGIDMap, FontDescriptor, and ToUnicode are written.
- Extraction of generated text is preserved through ToUnicode.
- Font Subsystem writes used-CID `/W`, CIDToGIDMap, and ToUnicode entries for
  shaped glyph clusters.

Font Subsystem still does not rewrite sfnt glyph programs. The writer uses subset
maps but embeds the full configured font program. This is deliberate: partial
subsetting without robust `cmap`/`glyf`/`loca`/`hmtx`/`maxp`/`head` rewriting
would risk corrupt generated PDFs. Reports expose this boundary with
`font.subset.sfnt_deferred`.

## CJK, RTL, and Vertical Writing

- Identity-H and Identity-V are recognized.
- Common UTF-16 predefined maps are recognized:
  `UniJIS-UTF16-H/V`, `UniGB-UTF16-H/V`, `UniCNS-UTF16-H/V`, and
  `UniKS-UTF16-H/V`.
- Type0/CID widths and `/W2` vertical metrics are parsed.
- Vertical Type0 text advances through shared decoded-glyph metrics in raster,
  SVG, and PostScript output.
- Multi-byte ToUnicode/CMap cases are tested.
- Font reports expose `writing_mode` and emit `font.vertical.detected`.
- Existing PDF RTL rendering follows the supplied glyph positions; generated
  RTL shaping is available through `TextShaper`.

Full legacy predefined CJK CMap packs and full vertical glyph substitution are
not claimed here. Unsupported predefined-looking names such as `90ms-RKSJ-H`
emit `font.cmap.predefined.unsupported` unless the PDF supplies an embedded
CMap or ToUnicode map.

## Public Diagnostics

`FontInfo` now includes:

- raw base font and subtype;
- descriptor presence;
- embedded program kind;
- Type0 descendant subtype;
- writing mode;
- fallback requirement;
- structured diagnostics with stable codes.

Example diagnostic codes:

- `font.standard14.substitution`
- `font.substitution.required`
- `font.tounicode.missing`
- `font.tounicode.missing_type0`
- `font.custom_encoding.no_tounicode`
- `font.type0.descendant_missing`
- `font.type3.charprocs`
- `font.vertical.detected`
- `font.cmap.predefined.used`
- `font.cmap.predefined.unsupported`
- `font.cmap.vertical`
- `font.cmap.identity`
- `font.color_glyphs.detected`
- `font.subset.sfnt_deferred`

The human `wellfriendpdf fonts` table remains pdffonts-style. JSON surfaces expose the
extra fields.

## Fuzz and Tests

Codec Boundary adds `font_mapping`, a fuzz target for glyph-name recovery,
deterministic provider matching, and generated-text shaping. Existing targets
continue to cover raw font program parsing (`fonts`) and ToUnicode CMaps
(`cmap`).

Small reviewed seeds live under `fuzz/seeds/fonts/`, including Font Subsystem
predefined-CMap, color-font, and shaped-authoring seeds.

## Known Limits

- Native FreeType is not integrated; the current raster path is pure Rust and
  does not claim full TrueType bytecode hinting parity.
- Native HarfBuzz bindings are not added; rustybuzz backs generated complex
  shaping without adding FFI risk.
- True sfnt glyph-program subsetting is not implemented; authoring writes
  subset maps with full-font fallback and a structured diagnostic.
- Full legacy predefined CJK CMap coverage is not vendored; UTF-16 common maps
  and Identity-H/V are explicit.
- Full vertical writing fidelity is not claimed; vertical advances are used,
  but vertical alternates and punctuation substitution are not.
- Color font / emoji rendering is detected and diagnosed, not rendered in color.

See also `docs/font_subsystem_font_subsystem.md` for the Font Subsystem closure table.
