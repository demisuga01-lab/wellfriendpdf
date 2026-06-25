# Oxide CLI

Oxide's CLI is a scriptable interface over the Rust engine. Human-readable output remains the default where it already existed; machine-readable output is opt-in through `--json` or `--format json`.

## Exit Codes

| code | name | meaning |
| ---: | --- | --- |
| 0 | success | Command completed successfully. |
| 1 | internal error | Unexpected internal failure or caught panic. This is a bug report candidate. |
| 2 | usage error | Invalid flags, unknown format/profile/type, invalid page range, or incompatible options. Clap argument errors also use code 2. |
| 3 | I/O error | Input/output path could not be read or written. |
| 4 | parse/format error | The file is malformed, encrypted without the right password, resource-limited, or otherwise rejected as input. |
| 5 | unsupported feature | The request needs a feature this build or command does not support, such as OCR in a non-OCR build. |

Malformed PDFs should return a clean non-zero exit code and an `oxide: <category>: <message>` stderr line. Raw Rust panic text should not reach users.

## Machine Output

Use JSON for scripts:

```powershell
oxide info input.pdf --json
oxide fonts input.pdf --json
oxide detach input.pdf --list --json
oxide verify-sig signed.pdf --json
oxide extract-text input.pdf --structured --format json
oxide extract-tables input.pdf --format json --structure
oxide parse input.pdf --format json
oxide extract-fields input.pdf --format json
oxide chunk input.pdf --format json
```

File-writing commands that naturally write their primary artifact to disk expose JSON result summaries:

```powershell
oxide render input.pdf --format png --output pages.zip --json
oxide extract-images input.pdf --output images.zip --json
oxide merge a.pdf b.pdf --output merged.pdf --json
oxide split input.pdf --output page-%d.pdf --json
oxide extract-pages input.pdf 1,3-5 --output subset.pdf --json
oxide linearize input.pdf --output linearized.pdf --json
oxide encrypt input.pdf --user-pw secret --output encrypted.pdf --json
oxide rotate input.pdf --angle 90 --output rotated.pdf --json
oxide optimize input.pdf --output optimized.pdf --json
oxide repair damaged.pdf --output repaired.pdf --json
```

These summaries use stable top-level fields:

| field | type | meaning |
| --- | --- | --- |
| `op` | string | Command operation name. |
| `output` | string | Output path when the command writes one primary artifact. |
| `bytes` | number | Output byte length when known. |
| `pages`, `pages_requested`, `pages_rendered` | number/array | Page counts or selected pages, depending on command. |
| `images`, `inputs`, `files` | number | Command-specific counts. |

Command-specific extraction JSON schemas are documented by the command outputs themselves and covered by integration tests. They are treated as compatibility surfaces for scripts.

## OCR Honesty

Default builds report OCR as unavailable in `oxide --version`. Commands that need OCR return exit code 5 with an actionable message unless the CLI is rebuilt with `--features ocr` and the external `tesseract` binary plus language data are installed. `extract-tables --ocr` is intentionally unsupported today because reconstructing table grids from OCR word boxes is a known gap; use `extract-fields --ocr` or `extract-text --ocr` for scanned documents.

## Help

Top-level help groups commands by purpose:

```powershell
oxide --help
oxide extract-text --help
oxide render --help
```

Region coordinates are PDF user-space points with the origin at the bottom-left, matching the region extraction docs.

