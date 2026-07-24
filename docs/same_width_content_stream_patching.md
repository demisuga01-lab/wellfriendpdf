# Same-width content-stream patching

The Prompt 20 patcher is a minimal incremental edit for text that can remain in
the existing font, encoding, and text geometry. Its lexical layer preserves
literal versus hexadecimal representation and decoded byte ranges for `Tj`,
individual `TJ` strings, quote, and double-quote text operators. Eligibility
reports the source stream, operator and element, byte range, font type,
encoding/CMap posture, glyph counts, encoded and serialized lengths, per-glyph
advances, total advance, writing mode, text render mode, marked-content depth,
filters, encryption posture, and signature decision.

Exact mode requires equal glyph count, encoded length, serialized string
length, and total advance. Tolerance mode permits only the configured advance
delta; it does not permit a different font or encoding. Type3, ambiguous or
missing reverse mappings, shaping, RTL or vertical reorder, clipping text
modes, unsafe encryption, and undecodable streams are rejected exactly.

Apply recompresses only the selected decoded content stream, appends its
replacement object, and verifies that the original PDF is an exact prefix,
the output reopens, the replacement extracts, and the old text is absent.
Prefix preservation is structural evidence, not a claim of cryptographic
signature validity or viewer acceptance.

CLI example:

```text
wellfriendpdf edit-text input.pdf --query ABC --replacement DEF --mode same-width-patch --output patched.pdf --json
```

Failure example: substituting Arabic for an LTR WinAnsi string is rejected
because it requires shaping and bidi ordering; use `rtl-reflow` instead.
