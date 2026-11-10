# Bundled fonts

These font programs are embedded into `wellfriendpdf-engine` (via `include_bytes!` in
`src/render/font_rasterizer.rs`) and used as **fallback / substitution fonts**
when a PDF references a non-embedded font (e.g. the standard-14 families, or a
symbolic font Wellfriend cannot otherwise resolve). They are *not* redistributed as
standalone fonts — they ship as substitution data inside the library.

Both families are open fonts under permissive licenses. Their full license
texts are included in this directory as the licenses require.

| File(s) | Family | License | Text |
|---|---|---|---|
| `DejaVuSans.ttf` | DejaVu Sans | DejaVu / Bitstream Vera (permissive, MIT-like) + Arev; DejaVu changes public domain | [`LICENSE-DejaVu.txt`](LICENSE-DejaVu.txt) |
| `LiberationSans-*.ttf`, `LiberationSerif-*.ttf`, `LiberationMono-*.ttf` | Liberation Sans / Serif / Mono | SIL Open Font License (OFL) 1.1 | [`LICENSE-Liberation.txt`](LICENSE-Liberation.txt) |
| `FoxitSymbol.cff`, `FoxitDingbats.cff` | PDFium's Symbol / ZapfDingbats faces (bare CFF) | PDFium BSD-3-Clause | [`LICENSE-PDFium.txt`](LICENSE-PDFium.txt) |

## Provenance

- **DejaVu Sans** — <https://dejavu-fonts.github.io/> — derives from Bitstream
  Vera (© 2003 Bitstream, Inc.) and Arev (© 2006 Tavmjong Bah); the DejaVu
  modifications are released into the public domain. The bundled
  `LICENSE-DejaVu.txt` is reproduced verbatim from the font's own embedded
  license (OpenType `name` table, ID 13).
- **Liberation** — <https://github.com/liberationfonts/liberation-fonts> —
  metric-compatible substitutes for Arial / Times New Roman / Courier New.
  Digitized data © 2010 Google Corporation; © 2012 Red Hat, Inc. Licensed
  under the SIL OFL 1.1 (Reserved Font Name: "Liberation").

- **Foxit Symbol / Dingbats** — the font data PDFium bundles in
  `core/fxge/fontdata/chromefontdata` (© 2014 The PDFium Authors, original
  code © 2014 Foxit Software Inc.), extracted byte for byte. The renderer
  draws non-embedded `Symbol` and `ZapfDingbats` text with them, so those
  glyphs have the exact shapes PDFium draws; other symbolic fonts still use
  DejaVu Sans for its Unicode coverage.

## License-compliance notes

- The OFL permits bundling/embedding and redistribution (including in
  commercial software) provided the copyright notice + license travel with the
  fonts and the Reserved Font Name is not used for modified versions. We ship
  the unmodified fonts with their license, so this is satisfied.
- The Bitstream Vera / DejaVu terms are MIT-like (use/copy/merge/distribute
  freely) with a rename-on-modification clause and a "not sold by itself"
  clause; we ship the unmodified font with its license, so this is satisfied.
- Neither family is copyleft; both are compatible with this project's permissive
  (MIT-or-Apache-class) positioning. See the repository `NOTICE` /
  `docs/licenses.md` for the consolidated third-party attribution.

## Scripts the bundled faces do not cover (CJK, Indic, Thai, ...)

The bundled faces cover Latin, Greek, Cyrillic, symbols and dingbats. A PDF
that does not embed its CJK, Indic or other non-Latin fonts needs faces from
somewhere else. No CJK face is bundled: one Noto Sans CJK face is 15-20 MB,
too much for the WebAssembly package and for every native build.

Glyphs the selected font cannot draw are taken, one at a time, from:

1. faces the host registered with `ContentEngine::register_font_bytes`
   (`registerFontBytes(name, bytes)` in the WebAssembly binding). A
   registered face replaces fonts whose `/BaseFont` has its name, and also
   serves as a coverage fallback for every font, whatever its name;
2. fonts installed on the host (native builds only, see
   `src/fonts/installed.rs`);
3. the bundled faces above.

For a composite font whose `CIDSystemInfo` ordering is Japan1, GB1, CNS1 or
Korea1 the registered and installed faces come first, and faces made for that
language (OS/2 code pages, localized family names) win, so Japanese text is
not drawn with Chinese glyph shapes. The WebAssembly build has no installed
fonts: a browser host that must draw non-embedded CJK or Indic text registers
a covering face (for example a Noto Sans CJK subset) with `registerFontBytes`;
without one those glyphs are not drawn.
