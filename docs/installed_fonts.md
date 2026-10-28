# Installed fonts

The engine can use fonts installed on the host in addition to its bundled
faces (Liberation and DejaVu). Installed fonts serve three purposes:

1. **Rendering non-embedded fonts.** A PDF that names a font without
   embedding it (`/BaseFont /Arial-BoldMT`, `/Calibri`, `/MS-Mincho`) is drawn
   with the real installed face of that name instead of a metric-compatible
   stand-in.
2. **Growing an embedded subset during editing.** When replacement text needs
   characters the embedded subset does not contain, the edit uses the
   complete installed face of the same family and style, so the edited text
   keeps the source typeface.
3. **Covering scripts the bundled faces cannot draw.** Generated text in CJK,
   Indic, Thai, Lao and other scripts falls back to the installed face that
   draws every character, chosen to resemble the source design.

The implementation lives in `crates/engine/src/fonts/installed.rs` and is
exported from `wellfriendpdf_engine::fonts`.

## When installed fonts are used, and when they are not

| Situation | Behaviour |
|---|---|
| Embedded font program present | Always used; installed fonts are never consulted. |
| Non-embedded Standard 14 name (`Helvetica`, `Times-Roman`, `Courier`, `Symbol`, ...) | Bundled metric-compatible face, as before. Installed fonts are not consulted, so the common case pays nothing. |
| Non-embedded other name, Compatibility rendering | Registered caller fonts first, then an installed face matching the name, then the bundled substitute. |
| Non-embedded other name, `HighQualityExact` rendering | Installed fonts are never used. The exact render contract must not depend on the host. |
| Editing: replacement text the embedded subset cannot draw | Embedded program, then the caller-supplied program, then the installed face of the source family (`matching_full_font`), then an installed face of a similar design (`similar_installed_font`), then bundled DejaVu, then any installed face that covers the text (`generated_text_font`). |
| Editing: generated text with no source font | Bundled DejaVu Sans when it draws every character; otherwise the installed embeddable face that does. |
| Engine unit tests (`cfg(test)`) | Off unless a test opts in for its thread, so the suite is host-independent. |

## Turning it off

Installed fonts make output depend on the machine. For reproducible output:

- set `WELLFRIENDPDF_SYSTEM_FONTS=0` (also `false`, `off`, `no`) in the
  environment, or
- call `wellfriendpdf_engine::fonts::set_installed_fonts_enabled(false)` once
  at startup (process-wide).

`installed_fonts_enabled()` reports the current state. The setting is read
lazily on first use and cached.

## Discovery

Faces are discovered with [`fontdb`](https://crates.io/crates/fontdb) 0.23
(already a dependency for PDF/A font embedding):

- Linux: the directories named in the fontconfig configuration
  (`/etc/fonts/fonts.conf`), which normally include `/usr/share/fonts`,
  `/usr/local/share/fonts` and `~/.local/share/fonts`;
- macOS: `/Library/Fonts`, `/System/Library/Fonts`, `~/Library/Fonts`;
- Windows: `%WINDIR%\Fonts` and the per-user font folder;
- plus every directory in `WELLFRIENDPDF_FONT_DIRS` (platform path-list
  syntax: `:` on Unix, `;` on Windows).

TrueType, OpenType (CFF and CFF2), and collections (`.ttc`, `.otc`) are
indexed. Type 1 (`.pfb`) and bitmap formats are not.

Discovery is lazy. Nothing is scanned until a caller first needs an installed
face, so a document whose fonts are all embedded never pays for it. The
database is built once per process (`OnceLock`) and shared by all callers.

On wasm32 targets the whole facility compiles to stubs that return no fonts.

### Index cache

Parsing every installed font file is expensive: on a host with 7,446 faces
(all Debian/Ubuntu `fonts-*` packages plus the Microsoft core fonts) it takes
about 100 ms. Because the CLI is usually one process per command, that cost
would recur on every render that meets a non-embedded font. The index is
therefore cached on disk:

- location: `$XDG_CACHE_HOME/wellfriendpdf/installed-fonts.idx`, else
  `%LOCALAPPDATA%\wellfriendpdf\…` on Windows, else
  `~/.cache/wellfriendpdf/installed-fonts.idx`;
- disable with `WELLFRIENDPDF_FONT_INDEX=0`.

The first line is a signature: an XXH64 hash of the cache format version and
the path and modification time of every directory under the font roots
(walked recursively, depth-limited to 12) and of `/etc/fonts`. Installing or
removing a font changes its directory's modification time, so any change
invalidates the cache and the next process rebuilds it.

Every following line describes one face, tab-separated:

```
path  face-index  postscript-name  weight  style(0 normal,1 italic,2 oblique)  monospaced(0/1)  width-class(1-9)  family  [family ...]
```

A cache hit rebuilds the fontdb database with `push_face_info` without
opening any font file (about 10 ms for 7,446 faces). The file is written to a
temporary name and renamed into place, so a concurrent reader never sees a
partial index. Faces whose fields would contain a tab or newline are left out
of the cache.

## Name matching (`installed_font_named`)

Input: a PDF font name, possibly with a subset tag (`ABCDEF+Garamond-Bold`),
plus optional bold/italic hints.

1. **Normalize.** Strip the subset tag (everything up to the last `+`), keep
   ASCII alphanumerics, lowercase, and remove the style-neutral words producers
   append: `regular`, `roman`, `mt`, `ps`. `TimesNewRomanPS-BoldMT` becomes
   `timesnewbold`; the installed PostScript name `TimesNewRomanPS-BoldMT`
   normalizes identically.
2. **Exact PostScript match.** Every face whose normalized PostScript name
   equals the normalized request, in deterministic order (PostScript name,
   then face index). The first that loads is returned.
3. **Family match.** The family key is the part of the name before the first
   `-` or `,`, normalized, with weight and slope words removed (`semibold`,
   `bold`, `black`, `heavy`, `medium`, `light`, `thin`, `italic`, `oblique`,
   `book`, ...). Faces whose family (any language) or PostScript family key
   equals it are ranked by style distance and the closest loadable face is
   returned.

Style distance between a face and the wanted style:

```
|weight − (bold ? 700 : 400)| / 100        weight classes apart
+ |width class − 5| × 2                    condensed/expanded faces lose to normal width
+ (slope differs ? 4 : 0)                  italic vs upright
```

Bold and italic are wanted when hinted by the caller or when the name itself
contains `bold`/`black`/`heavy`/`semibold`/`demi` or
`italic`/`oblique`/`-it`.

Results are memoized per process by `(normalized name, bold, italic,
require_embeddable)`.

## Coverage fallback (`installed_font_covering`)

Input: text and an `InstalledFontStyle` (family hint, bold, italic, monospace,
serif, require-embeddable). Output: the installed face that draws every
visible character, or, when none does, the face that draws the most.

Characters considered are the non-whitespace scalars of the text,
deduplicated. Every face is parsed (memory-mapped) and counted; ties are broken
by this rank, lowest first:

1. **Family**: 0 if the face belongs to the hinted family, else 1.
2. **Design**: one point each for a monospace mismatch and a serif/sans
   mismatch. Serif-ness of the hint and of the face is judged from the name
   (`serif`, `times`, `roman`, `georgia`, `garamond`, `mincho`, `song`, `ming`,
   `batang`, `myeongjo`, ... are serif; `sans`, `gothic`, `arial` are not).
3. **Style**: style distance (above) plus a preference for widely available
   broad-coverage families (`Noto` 0, `DejaVu`/`Liberation` 1, others 2), so
   equally good matches resolve to the same design on every host that has
   them.
4. PostScript name, then face index, for determinism.

Faces without outlines are skipped (see below). With `require_embeddable`,
faces whose OS/2 `fsType` forbids subset embedding are skipped too.

Results are memoized per process by `(characters, style)`.

## Safety checks on every installed face

- **Outlines required.** A face must have a `glyf`, `CFF ` or `CFF2` table.
  Bitmap-only faces (some X11 and terminal fonts) map characters but cannot be
  drawn or embedded as outlines. Without this check, `Courier` resolved to a
  bitmap face named `courier` and edits failed later with "lacks replacement
  glyph coverage".
- **Embedding permission.** For editing (`require_embeddable`), OS/2
  `fsType` must be installable (0), preview & print (4) or editable (8), with
  neither the no-subsetting (0x0100) nor the bitmap-only (0x0200) bit set.
- **Shaped coverage.** The editing helpers (`generated_text_font`,
  `similar_installed_font`) accept a face only when every visible character
  maps to a non-zero glyph *and* shaping the text with rustybuzz leaves no
  missing-glyph cluster. Bidi controls, joiners, variation selectors, soft
  hyphen and BOM are not required to have glyphs.
- **Standalone program bytes.** A face inside a collection is extracted to a
  single-face sfnt with the engine's font container code, so every consumer
  can parse the result at index 0. Loaded programs are kept for the process
  lifetime (one copy per face) and shared.

## Integration points

| Caller | Function | Purpose |
|---|---|---|
| Renderer, `resolve_replacement_font_bytes` | `InstalledFontProvider` (implements `FontProvider`) | Real face for a non-embedded, non-Standard-14 name in Compatibility mode. Reported in font-substitution reports with resolution source `installed_system_font`. |
| Editing, `pdfa_fonts::matching_full_font` | `installed_font_named(base, …, require_embeddable = true)` | Complete program of the source family for subset growing. |
| Advanced editing, source-style insertion and inline semantic replacement | `similar_installed_font(base, text)` | Same-design substitute when the source family is not installed. |
| Advanced editing analysis and generated text | `generated_text_font(text)` | DejaVu when it covers the text, else an installed face that does. |
| PDF/A font embedding | `fonts::installed::database()` | Shares the same lazily-built database and toggle. |

## Public API

```rust
use wellfriendpdf_engine::fonts::{
    installed_font_faces, installed_font_named, installed_font_covering,
    installed_fonts_enabled, set_installed_fonts_enabled,
    InstalledFontFace, InstalledFontProgram, InstalledFontProvider, InstalledFontStyle,
};

// Every installed face, sorted by family, weight, slope and name.
let faces: Vec<InstalledFontFace> = installed_font_faces();

// The face a PDF name resolves to (subset tags allowed).
if let Some(program) = installed_font_named("ABCDEF+TimesNewRomanPS-BoldMT", false, false, true) {
    println!("{} from {:?}", program.face.post_script_name, program.face.path);
    let bytes: &'static [u8] = program.bytes; // standalone sfnt
}

// The best face for text the source font cannot draw.
let style = InstalledFontStyle {
    family_hint: Some("Garamond-Italic".into()),
    require_embeddable: true,
    ..Default::default()
};
let program = installed_font_covering("漢字テスト", &style);
```

`InstalledFontProvider` implements the engine's `FontProvider` trait and can
be chained with `RegisteredFontProvider` and `BundledFontProvider` by callers
that build their own resolution order.

## CLI

```
wellfriendpdf installed-fonts [--filter TEXT] [--resolve NAME] [--covering TEXT] [--json]
```

- `--filter` lists faces whose family or PostScript name contains the text.
- `--resolve` prints the face a PDF font name resolves to.
- `--covering` prints the face chosen for the given text.
- `--json` adds `faces_total`, `scan_seconds` and `enabled`.

Example on the test VM:

```
$ wellfriendpdf installed-fonts --filter zzz --resolve ABCDEF+TimesNewRomanPS-BoldMT --covering 漢字テスト
resolve ABCDEF+TimesNewRomanPS-BoldMT: TimesNewRomanPS-BoldMT
covering: NotoSansCJKhk-DemiLight
0 of 7446 installed faces (0.010s scan)
```

## Tests

- `fonts::installed::tests::family_key_strips_subset_tag_and_style`
- `fonts::installed::tests::design_heuristics`
- `fonts::installed::tests::installed_faces_resolve_by_name_and_cover_text_when_present`
  opts in for its thread; it checks that up to 40 installed faces resolve to
  themselves by PostScript name, that resolved programs parse standalone and
  have outlines, and that a coverage match draws its text. On a host without
  fonts it has nothing to check and passes.

Every other unit test runs with installed fonts off, so results do not depend
on the machine. Integration tests (separate crates, no `cfg(test)`) see the
host's fonts.

## Known limits

- Type 1 (`.pfb`/`.pfa`) and bitmap fonts are not indexed.
- A non-embedded CID font is matched by name only. There is no automatic
  Registry/Ordering → Noto CJK mapping for a non-embedded CID font whose name
  is not installed.
- The first coverage query parses every installed face (memory-mapped). With
  thousands of faces this costs tens of milliseconds once per process; results
  are memoized.
- Installed fonts are never part of the exact render contract.
