# Click-to-edit text editor API

The engine exposes the building blocks of an interactive, click-to-edit PDF
text editor. It does not provide a user interface. The caller owns windows,
input, caret blinking and drawing; the engine owns the text, its geometry and
the PDF.

| Type | Role |
|---|---|
| `PageTextLayout` | Read-only geometry of one page's editable text: glyph quads, lines, paragraphs; hit-testing, carets, selections, word/line/paragraph ranges, caret movement, page↔pixel mapping. |
| `TextEditSession` | A document being edited: layouts per page, replace/insert/delete through the style-preserving reflow transaction, dirty regions, undo/redo. |
| `EditorGlyph`, `EditorLine`, `EditorParagraph`, `Caret`, `CaretMove`, `TextEditOutcome` | Data returned by the two above. |

Source: `crates/engine/src/text_editor.rs` (public API),
`crates/engine/src/advanced_editor_layout.rs` (glyph geometry, a private
submodule of `advanced_editing`). Re-exported from the crate root.

Bindings: the browser/wasm crate has a `TextEditor` class with the same
operations; the CLI has `wellfriendpdf text-layout`.

## Offsets: the page's logical text

Every offset in this API is a Unicode-scalar index into
`PageTextLayout::text`, the page's **logical text**. This is the same string
that the editing transactions verify edits against
(`advanced_editing::analyze_multi_run_source`):

- the decoded text of every text-showing string operand in the page's content
  streams (`Tj`, `TJ` elements, `'`, `"`), in content order;
- an `/ActualText` marked-content value replaces the strings it covers, and is
  counted once even when it spans several operands.

Because the editor and the edit transactions share one definition of the
text, an offset range selected by a click is passed to the edit unchanged
(`target_logical_scalar_range`) and the transaction can prove it changed
exactly the selected source glyphs.

Two properties follow from this definition:

- **No synthesized spaces or line breaks.** If a producer positions words
  with `TJ` gaps rather than space glyphs, the logical text has no space there
  (`"Helloworld"`). Word boundaries are therefore computed from geometry (see
  [Word, line, paragraph](#word-line-paragraph)), not from the string.
- **Form XObject text is not included.** Only page content streams are
  scanned; text inside forms and annotation appearances is edited through
  their own APIs.

## Building a layout

```rust
use wellfriendpdf_engine::{ContentEngine, PageTextLayout};

let engine = ContentEngine::open_bytes(pdf_bytes)?;
let layout = PageTextLayout::build(&engine, 1)?;   // 1-based page
// or: PageTextLayout::from_bytes(&pdf_bytes, 1)?
println!("{} glyphs, {} lines, editable: {}", layout.glyphs.len(), layout.lines.len(), layout.editable);
```

### How glyph geometry is computed

`advanced_editor_layout::page_glyph_layout` scans the page content with the
editing scanner (`scan_page_text_string_tokens_with_metrics`). For every
string operand the scanner records the font resource, size, character and
word spacing, horizontal scaling, rise, render mode, the decoded bytes, and
two matrices at the moment the string is shown:

- the **text-space position** (`Tm` advanced by previous glyphs and `TJ`
  adjustments), tracked by `inline_text::PositionState`;
- the **CTM**, tracked by the same state machine through `cm`, `q` and `Q`
  (`GeometryCtm`). The CTM's `Debug` output is deliberately constant, because
  scanner-state fingerprints compare inherited text state: the same Form
  shown at two positions must still compare equal.

The glyph pass then walks the string's character codes with the font
resolver. For a horizontal font, with `fs` the font size, `Th` the horizontal
scaling (Tz, stored in percent, divided by 100), `Tc`/`Tw` character and word
spacing and `w` the glyph width in 1/1000 em:

```
glyph box (text space)  x: [pen, pen + w/1000·fs·Th]
                        y: [rise + descent·fs, rise + ascent·fs]
pen advance             (w/1000·fs + Tc + (word space ? Tw : 0)) · Th
```

`ascent` and `descent` come from the font descriptor (`/Ascent`, `/Descent`
divided by 1000; for Type 0 fonts, the descendant's descriptor), defaulting to
0.8 and −0.2 when absent or implausible. Vertical fonts use the `W2` vertical
displacement and a box one em wide centred on the pen.

Each corner is mapped to page user space with `translate(corner) × Tm × CTM`.
The result is the glyph's **quad**: bottom-start, bottom-end, top-end,
top-start relative to the writing direction, which stays exact under rotation
and shear. `baseline` holds the pen position before and after the glyph.

Each glyph's logical range is the cumulative length of the decoded text of
the preceding codes in its string, offset by the string's start in the
logical text. When an `/ActualText` value replaced the string, the string's
range is shared out over its glyphs proportionally (the glyph-to-character
correspondence is not knowable in that case).

Glyphs whose position is unknown (a string shown with no usable text matrix,
or a font whose encoding cannot be validated) are omitted from `glyphs`.
Their text is still part of `text`.

### Lines

Glyphs are grouped in content order. A new line starts when, relative to the
previous glyph's baseline direction `d` and normal `n`, and with `s` the larger
of the two font sizes:

- the next glyph's baseline is more than `0.5·s` off the previous baseline
  (across `n`), or
- it moves backwards along `d` by more than `0.5·s`, or
- it jumps forward by more than `6·s` (a column gap), or
- the writing direction (horizontal/vertical) changes.

A line records its glyph index range, logical range (first glyph start to
last glyph end), bounding box, and baseline from the first glyph's start to
the last glyph's end.

### Paragraphs

Consecutive lines join the current paragraph when all of these hold:

- the baseline pitch (distance between line starts along the normal) is
  positive and below `1.7·s`, where `s` is the largest font size on the
  previous line;
- the pitch is at most `1.4×` the paragraph's first pitch (so a larger gap
  starts a new paragraph);
- the line starts within `1.5·s` of the previous line's start, or of the
  paragraph's first line (first-line indents and hanging indents are allowed).

### Editable and read-only layouts

`layout.editable` is `true` when `text` is the editing model's exact logical
text. Some pages are refused by the editing model, for example when a font's
code space is ambiguous ("ambiguous prefix-overlapping PDF code spaces"). The
layout is then built from a tolerant scan instead:

- strings whose font cannot decode them contribute no text;
- `/ActualText` is applied as in the editing model;
- `editable` is `false`.

A read-only layout still supports selection, hit-testing and copying text.
`TextEditSession::replace` refuses a non-editable page with a clear error,
rather than editing offsets that the transaction would not recognise.

## Queries

All coordinates are PDF user space of the page (points, origin bottom-left)
unless the method name says `device`.

| Method | Returns |
|---|---|
| `len()` / `is_empty()` | Number of scalars in `text`; the largest caret offset. |
| `text_in([start, end])` | The text of a range (clamped). |
| `glyph_at(point)` | Index of the glyph whose quad contains the point. |
| `hit_test(point)` | Caret offset for a click: nearest line, then nearest glyph edge along that line. |
| `hit_test_device(pixel, dpi)` | Same, for a pixel position in a render at `dpi`. |
| `caret(offset)` | `Caret { offset, top, bottom, line }`: the segment to draw. |
| `selection_quads([start, end])` | One quad per line covering the selected glyphs. |
| `word_range(offset)` / `line_range(offset)` / `paragraph_range(offset)` | Ranges for double-click, triple-click, and paragraph selection. |
| `move_caret(offset, CaretMove)` | New offset after a keyboard movement. |
| `device_transform(dpi)` | Page-to-pixel matrix `[a b c d e f]` of a render at `dpi`. |
| `to_device(point, dpi)` / `from_device(pixel, dpi)` | Coordinate conversion. |

### Hit-testing

1. **Nearest line.** Distance from the point to each line's bounding box, with
   horizontal distance weighted by 0.25 so a click beside a line (in the
   margin) still selects that line rather than the one above or below.
2. **Nearest edge.** Project the point and every glyph's start and end pen
   positions onto the line's baseline direction. The closest edge wins; a
   glyph's start edge gives its start offset, its end edge its end offset.
   Clicking the right half of a glyph therefore places the caret after it.

### Carets

`caret(offset)` resolves, in order:

1. a glyph whose range contains `offset`. If `offset` is the glyph's start
   and the glyph begins a line that directly continues the previous line
   (the previous line ends at the same offset), the caret is drawn at the end
   of the previous line. This is the usual "end of line" caret at a soft line
   break.
2. If `offset` is inside a multi-character glyph (a ligature), the caret is
   interpolated along the glyph's bottom and top edges.
3. Past the last glyph: the end edge of the last glyph that ends at or before
   `offset`.

Top and bottom come from the glyph quad, so carets slant with italic or
rotated text and are vertical-text aware.

### Selections

For each line intersecting the range, the selected glyphs are those whose
ranges overlap it. The quad runs from the first selected glyph's start edge to
the last selected glyph's end edge, so highlights follow the baseline under
rotation. Draw them with `to_device` for screen coordinates.

### Word, line, paragraph

- **Word.** From the glyph at the offset, expand left and right while
  adjacent glyphs are on the same line, neither is whitespace, and the gap
  between them along the baseline is below `WORD_GAP_EM × font size` (0.12 em,
  the same threshold text extraction uses). A whitespace glyph is a word of
  its own.
- **Line.** The line's logical range.
- **Paragraph.** The paragraph's logical range.

### Caret movement

| `CaretMove` | Behaviour |
|---|---|
| `Left` / `Right` | One scalar. |
| `WordLeft` | Skip whitespace to the left, then to the start of that word. |
| `WordRight` | To the end of the current word, then over following whitespace. |
| `LineStart` / `LineEnd` | Line range ends. |
| `LineUp` / `LineDown` | Take the caret's midpoint, hit-test it on the previous/next line. Past the first/last line, go to text start/end. |
| `ParagraphStart` / `ParagraphEnd` | Paragraph range ends. |
| `TextStart` / `TextEnd` | 0 and `len()`. |

`CaretMove::parse` accepts `left`, `right`, `word_left`, `word_right`,
`line_start`/`home`, `line_end`/`end`, `line_up`/`up`, `line_down`/`down`,
`paragraph_start`, `paragraph_end`, `text_start`, `text_end` (case-insensitive,
`-` or `_`).

### Device coordinates

`device_transform(dpi)` builds the same `Viewport` the renderer uses (crop box,
display `/Rotate`, `/UserUnit`), so a caret or highlight drawn with
`to_device` lands on the pixels of a render at that DPI, including rotated
pages. `from_device` is the exact inverse.

## Editing

```rust
use wellfriendpdf_engine::{CaretMove, TextEditSession};

let mut session = TextEditSession::open(pdf_bytes)?;

// User clicks at pixel (412, 230) of page 1 rendered at 96 dpi.
let offset = session.layout(1)?.hit_test_device([412.0, 230.0], 96).unwrap();
let word = session.layout(1)?.word_range(offset);

let outcome = session.replace(1, word, "replacement")?;
// outcome.caret        -> place the caret here (offset after the new text)
// outcome.dirty_region -> repaint this user-space rectangle (None: whole page)
// outcome.report       -> the reflow transaction's JSON report

session.undo();
session.redo();
std::fs::write("edited.pdf", session.bytes())?;
```

### What `replace` does

1. Takes the current layout of the page (cached per page and revision).
2. Refuses a non-editable layout.
3. Normalizes the edit so the transaction always rewrites a non-empty
   selection with non-empty text:
   - a pure insertion at `offset` becomes "replace the scalar before `offset`
     with that scalar + the new text" (or the scalar after it, at offset 0);
   - a deletion becomes "replace the selection plus one neighbouring scalar
     with that neighbouring scalar".
4. Builds a `GeometricReflowRequest` with `source_text` = the selected logical
   text and `target_logical_scalar_range` = the selection, and calls
   `text_reflow::apply_reflow_region`. That transaction measures the source
   paragraph's font, size, leading, baselines and column width, keeps the
   source font for every grapheme it can encode, grows the subset from an
   installed face of the same family when it cannot (see
   [installed fonts](installed_fonts.md)), rewrites the paragraph, verifies
   the old text is gone and the new text extracts, and reports.
5. Rebuilds the page layout from the new bytes.
6. Computes the dirty region by comparing old and new glyph lists from both
   ends. Glyphs equal in text and quad at the start and at the end are
   skipped; the region is the union of the boxes of the remaining glyphs in
   both layouts.
7. Pushes the new bytes onto the history (discarding any redo tail) and
   clears cached layouts.

`insert(page, offset, text)` and `delete(page, range)` are thin wrappers.

### Undo and redo

The session keeps the complete document bytes of every revision. `undo` and
`redo` move a cursor through that list, so they are exact and instantaneous,
at the cost of memory proportional to the number of edits times the document
size. A caller with very large documents can bound history by saving
`bytes()` and reopening a session.

## Bindings

### Browser (wasm)

`crates/wellfriendpdf-wasm/src/text_editor.rs` exports `TextEditor`. Pixel
coordinates are at the caller's DPI.

```js
const editor = new TextEditor(pdfBytes);
const layout = JSON.parse(editor.layoutJson(1));        // glyphs, lines, paragraphs (user space)
const m = editor.deviceTransform(1, 96);                 // page → pixel matrix
const offset = editor.hitTest(1, x, y, 96);              // -1 on a page without text
const caret = JSON.parse(editor.caretJson(1, offset, 96)); // {offset, line, top:[x,y], bottom:[x,y]}
const quads = JSON.parse(editor.selectionJson(1, start, end, 96));
const [ws, we] = editor.range(1, offset, "word");        // "word" | "line" | "paragraph"
const next = editor.moveCaret(1, offset, "word_right");
const outcome = JSON.parse(editor.replace(1, ws, we, "new text"));
editor.undo(); editor.redo();
const saved = editor.bytes();
```

`insert`, `delete`, `textIn`, `canUndo` and `canRedo` are also available.

### CLI

```
wellfriendpdf text-layout file.pdf -p 2 --dpi 100 [--click X,Y] [--glyphs]
```

Prints the page's text, `editable`, lines, paragraphs, page box, rotation and
device transform. `--click` resolves a pixel position to its offset, caret,
word, line and paragraph. `--glyphs` adds every glyph.

Example (IRS i1040gi, page 2, 100 dpi, click at 200,300):

```json
"click": {
  "offset": 525,
  "word": { "range": [524, 530], "text": "Filing" },
  "caret": { "offset": 525, "line": 15, "bottom": [145.56, 568.00], "top": [145.56, 579.07] }
}
```

## Tests

`crates/engine/src/text_editor_tests.rs` builds a Letter page with two lines
of Helvetica (`Hello world` at 72,700 and `Second line` 14 pt below) and
checks:

- logical text, glyph count, line and paragraph grouping, and exact glyph
  geometry (the `H` advance is 722/1000 em);
- hit-testing either half of a glyph, clicks in the left margin, the caret at
  a soft line break;
- selection quads across lines, word ranges, word/line/paragraph movement,
  line up/down;
- device mapping round-trips at 96 and 144 dpi;
- replace with undo and redo, insertion and deletion by offset.

## Known limits

- Text in Form XObjects and annotation appearances is not part of the layout.
- Line and paragraph grouping is geometric and follows content order. A page
  whose content order differs from reading order (some multi-column layouts)
  groups lines in content order.
- Glyphs inside an `/ActualText` replacement share the replacement's offsets
  proportionally.
- Glyph boxes use the font descriptor's ascent and descent, not each glyph's
  outline bounds.
- Undo history holds full document copies.
