# Rust-native conversions to PDF

Five converters turn common text formats into PDF without external tools,
network access or the Typst compiler:

| Command | Engine function | HTTP route | C function |
|---|---|---|---|
| `text-to-pdf` | `text_to_pdf` / `text_to_pdf_with_report` | `POST /api/v1/text-to-pdf` | `wellfriendpdf_text_to_pdf` |
| `markdown-to-pdf` | `markdown_to_pdf` / `markdown_to_pdf_with_report` | `POST /api/v1/markdown-to-pdf` | `wellfriendpdf_markdown_to_pdf` |
| `csv-to-pdf` | `csv_to_pdf` / `csv_to_pdf_with_report` | `POST /api/v1/csv-to-pdf` | `wellfriendpdf_csv_to_pdf` |
| `json-to-pdf` | `json_to_pdf` / `json_to_pdf_with_report` | `POST /api/v1/json-to-pdf` | `wellfriendpdf_json_to_pdf` |
| `svg-to-pdf` | `svg_to_pdf` / `svg_to_pdf_with_report` | `POST /api/v1/svg-to-pdf` | `wellfriendpdf_svg_to_pdf` |

Text, Markdown, CSV and JSON are laid out with the SDK's own authoring API
(`FlowDocument`, rich paragraphs and the paginated table layout). All text is
real, selectable text. Fonts are the bundled Liberation faces with DejaVu Sans
as a fallback, which covers Latin, Greek, Cyrillic, Arabic, Hebrew, Armenian,
Georgian and many symbols. CJK and other scripts without a bundled glyph are
replaced with U+FFFD and counted in the report (`replaced_characters`); they
are never dropped silently.

SVG uses `usvg` + `svg2pdf` and stays vector.

## Calling convention

### CLI

```powershell
wellfriendpdf text-to-pdf --input notes.txt --output notes.pdf --options '{"page_size":"letter","monospace":true}'
wellfriendpdf markdown-to-pdf --input README.md --output readme.pdf
wellfriendpdf csv-to-pdf --input data.csv --output data.pdf --options '{"delimiter":";","max_columns":30}'
wellfriendpdf json-to-pdf --input payload.json --output payload.pdf
wellfriendpdf svg-to-pdf --input logo.svg --output logo.pdf --options '{"page":"fit","page_size":"a4"}'
```

On success the command prints exactly one line to stdout:

```json
{"ok":true,"pages":3,"bytes":48211}
```

On failure it prints `{"ok":false,"code":"...","message":"..."}` and exits
non-zero (the usual CLI exit classes: 2 usage, 3 I/O, 4 input, 5 unsupported).
`code` is one of the server vocabulary values: `INVALID_INPUT`,
`UNSUPPORTED_INPUT`, `RESOURCE_LIMIT`, `INVALID_JSON`, `INVALID_OPTIONS`,
`ENCRYPTED_INPUT` or `INTERNAL` (engine function `native_contract_code`). The
specific converter code below starts `message`.
Warnings (for example a remote Markdown image shown as a placeholder) go to
stderr. These commands always print the JSON line; they don't use the
older `--json` pretty summary flag.

### HTTP

Send multipart `file` (required) and `options` (optional, the same JSON
object as `--options`). The response is `application/pdf` with an
`x-page-count` header. Errors use the server's normal JSON error body, and the
message starts with the converter code. The server never reads local files,
so Markdown `base_dir` is rejected; embed images as `data:` URIs.

### C API

```c
int wellfriendpdf_csv_to_pdf(const uint8_t *data, uintptr_t len,
                             const char *options_json, /* NULL = defaults */
                             WellfriendBuffer *out_buffer, char **error_out);
```

Free `out_buffer` with `wellfriendpdf_buffer_free`. The error string starts
with the converter code. Markdown `base_dir` is rejected here too.

## Shared options

Every option is optional. Unknown keys are ignored; a known key with a bad
value fails with `invalid_options`. The server's camelCase names are accepted:
`pageSize`, `orientation`, `fontSize`, `margin`, `delimiter`, `headerRow`,
`title`, `author` (plus `lineWrap`, `maxColumns`, `tabWidth`, `lineHeight`,
`autoOrient`). Values of `pageSize`, `orientation`, `line_wrap` and `page`
are case-insensitive (`"A4"`, `"Letter"`, `"Landscape"`).

| Field | Meaning | Default |
|---|---|---|
| `page_size` | `"a4"`, `"letter"` or `"legal"` | `"a4"` |
| `margins` | points: one number, or `{"top","right","bottom","left"}` | 54 (CSV and SVG: 36) |
| `font_size` | body size in points, 7 to 22 | text 10, Markdown 10.5, CSV 8, JSON 9 |
| `orientation` | `"portrait"` or `"landscape"` (CSV also `"auto"`) | portrait (CSV: auto) |
| `title` | PDF `/Title` metadata | none (Markdown: first H1) |
| `author` | PDF `/Author` metadata | none |

`/Creator` is always `Wellfriend PDF SDK <command>`.

## text-to-pdf

- Input must be UTF-8. A BOM is accepted.
- `line_wrap` controls long lines:
  - `"word"` (default) wraps at spaces;
  - `"char"` wraps at the last character that fits;
  - `"none"` fails with `text_line_too_wide`, naming the line, instead of clipping.
- `monospace` (default `false`) selects Liberation Mono.
- `tab_width` (default 4) expands tabs to column stops.
- `line_height` (default 1.25, range 0.8 to 3) sets line spacing as a multiple of the font size.
- `landscape` (default `false`) turns the page.
- Pages break automatically. A form feed (U+000C) also starts a new page.

## markdown-to-pdf

- Parsed with `pulldown-cmark`: CommonMark plus GFM tables, task lists, strikethrough and blockquote tags.
- Supported elements:
  - headings, which are tagged `/H1` to `/H6`;
  - nested ordered, unordered and task lists;
  - block quotes, shown with a left rule;
  - fenced and indented code blocks, in monospace on a tinted background;
  - horizontal rules;
  - tables, with column alignment and a header row repeated on each page;
  - bold, italic, strikethrough and inline code.
- Links become PDF `/Link` annotations with a `/URI` action. Only `http`, `https` and `mailto` links get an annotation; other links stay as plain text and the report says so.
- Images:
  - `data:` URIs with base64 PNG or JPEG are embedded;
  - local paths are embedded only when they resolve inside `base_dir`. The CLI defaults `base_dir` to the Markdown file's folder;
  - remote URLs are **never fetched**, and neither are paths outside `base_dir`. Each one is shown as a visible "[image not embedded: alt]" placeholder and listed in the warnings.
- Raw HTML is shown as literal source and never interpreted.

## csv-to-pdf

- Parsed with the `csv` crate. Ragged rows are padded.
- `has_header` (default `true`) repeats the first record as the table header on every page. Blank header cells become `Column N`.
- `delimiter`:
  - `"auto"` (the default, same as unset) picks from `,`, `;`, tab and `|` by sampling the first records;
  - you can also give one ASCII character or `"tab"`.
- `max_columns` (default 50, at most 200). More columns fail with `csv_column_limit_exceeded`; columns are never silently truncated.
- `orientation`:
  - `"auto"` (default) switches to landscape when the measured natural width doesn't fit portrait;
  - `"portrait"` and `"landscape"` force one orientation.
- Columns that still don't fit are narrowed in proportion to their extra width, and long cells wrap. If even the minimum column width can't fit, it fails with `csv_table_too_wide`.
- At most 100,000 rows (`csv_row_limit_exceeded`).

## json-to-pdf

- Parsed with `serde_json`, with the default recursion limit of 128 levels.
- Output is a pretty-printed, colour-coded key/value tree in monospace. Each nesting level is indented by `indent` points (default 14).
- `tables` (default `true`): an array of 1 to 12-key objects that all share the same keys is drawn as a table, with nested values shown as compact JSON.
- At most 200,000 output lines (`json_too_large`).

## svg-to-pdf

- `page`:
  - `"natural"` (default) makes one page the SVG's size, at 96 CSS px per inch, capped at 200 inches;
  - `"fit"` scales the drawing onto `page_size` inside `margins` and centres it.
- `auto_orient` (default `true`) uses landscape when the drawing is wider than it is tall.
- `data:` images are resolved. `href` values that point to files or URLs are ignored, and so are scripts.
- Text uses only the bundled fonts, never system fonts, and stays selectable text.
- SVG filters are not rasterised. The `svg2pdf` `filters` feature is off to keep the dependency set small.

## Error codes

Each specific code is the prefix of the error message, and `native_error_code()` returns it. The CLI `code` field holds the contract class from `native_contract_code()`.

| Code | Contract class | Meaning |
|---|---|---|
| `invalid_options` | `INVALID_OPTIONS` | malformed options JSON, bad value, margins or delimiter |
| `font_size_out_of_range` | `INVALID_OPTIONS` | `font_size` outside 7 to 22 |
| `invalid_utf8` | `INVALID_INPUT` | text-like input is not UTF-8 |
| `empty_input` | `INVALID_INPUT` | empty CSV, JSON or SVG |
| `text_line_too_wide` | `INVALID_INPUT` | `line_wrap: "none"` and a line overflows |
| `csv_parse_error` | `INVALID_INPUT` | malformed CSV |
| `svg_parse_error` | `INVALID_INPUT` | `usvg` rejected the input |
| `json_parse_error` | `INVALID_JSON` | malformed JSON |
| `input_too_large` | `RESOURCE_LIMIT` | input over 64 MiB |
| `csv_column_limit_exceeded` | `RESOURCE_LIMIT` | more columns than `max_columns` |
| `csv_row_limit_exceeded` | `RESOURCE_LIMIT` | more than 100,000 rows |
| `csv_table_too_wide` | `RESOURCE_LIMIT` | columns can't fit even at the minimum width |
| `json_too_large` | `RESOURCE_LIMIT` | rendered tree over 200,000 lines |
| `svg_convert_error` | `UNSUPPORTED_INPUT` | `svg2pdf` failed |

Other failures map from the general engine codes: `parse`/`malformed_pdf` to
`INVALID_INPUT`, `unsupported_feature` to `UNSUPPORTED_INPUT`, `resource_limit`
to `RESOURCE_LIMIT`, `encrypted` to `ENCRYPTED_INPUT`, and anything else
(including I/O) to `INTERNAL`.

## Dependencies

| Crate | Version | Licence |
|---|---|---|
| `pulldown-cmark` | 0.13.4 | MIT |
| `csv` | 1.4.0 | Unlicense OR MIT |
| `svg2pdf` | 0.13.0 | MIT OR Apache-2.0 |
| `usvg` (through `svg2pdf`) | 0.45 | Apache-2.0 OR MIT |
| `pdf-writer` | 0.12.1 | MIT OR Apache-2.0 |
