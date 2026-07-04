# Prompt 07 True Redaction

True redaction means sensitive content is removed or made unrecoverable. A
black rectangle alone is not redaction.

## Inputs

- Semantic search matches from Prompt 06B.
- Explicit `page:x,y,w,h` rectangles.
- Existing editor redaction rectangles.

The CLI command is:

```powershell
oxide redact input.pdf --text SECRET --pages 1-2 --out redacted.pdf --json --strict
oxide redact input.pdf --rect 1:72,700,120,30 --out redacted.pdf
```

## Text

Text redaction rewrites page content streams. For `Tj`, `TJ`, single-quote, and
double-quote text-showing paths, glyphs intersecting the redaction box are
removed and surviving text is repositioned using `TJ` advances. If the font
cannot be resolved, the path fails closed by removing the whole intersecting
string.

## Images and Vectors

Intersecting image invocations and simple vector paths are removed
conservatively. Partial pixel-level redaction inside shared image XObjects is
not claimed as complete; the current policy avoids leaving an intersecting
image invocation recoverable in the redacted page content.

## Metadata, Alternate Text, Annotations, Links

By default redaction scrubs removed text from metadata-like streams and inline
marked-content `/ActualText` and `/Alt` strings. Overlapping annotations and
links are removed through the page annotation edit path. The CLI exposes
`--no-metadata-scrub` for callers that need to preserve metadata, but strict
verification should normally keep scrubbing enabled.

## Verification

`redaction_verification_report` reopens the output, searches with hidden text
included, and checks raw bytes for requested terms. Strict CLI mode fails if any
term remains extractable or directly present in output bytes.

Prompt 07 fixed a safety gap in the full-rewrite writer: superseded old content
streams were no longer referenced but were still copied into the output file.
Full-rewrite editing now retains only objects reachable from the updated root
and info dictionaries.

Known bounded limits:

- Encoded secrets that are not recoverable as text by Oxide and are not direct
  raw byte matches require broader Prompt 09 sanitization policy.
- Pixel-level partial redaction of shared images is bounded follow-up work.
- Redaction apply intentionally uses full rewrite; incremental redaction is
  rejected because old revisions preserve sensitive bytes.
