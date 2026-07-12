# Vertical and RTL paragraph reflow

Prompt 20 distinguishes existing PDF glyph streams from newly inserted text.
Existing strings retain their code/CID/GID provenance and are removed from the
reachable stream when replaced; they are never fed back through a shaper.
New Unicode is resolved with UAX #9 ordering and rustybuzz, then embedded as a
Type0/CIDFontType2 font with sequential CIDs, an explicit CID-to-GID map, and a
per-CID ToUnicode map.

`paragraph_reflow_rtl` supports Arabic, Hebrew, combining marks, mixed LTR/RTL
runs, numbers, punctuation, and balanced bidi controls within the selected
paragraph. `paragraph_reflow_vertical` uses Identity-V, top-to-bottom glyph
progression, right-to-left columns, vertical punctuation policy, and clockwise
orientation for bounded Latin/punctuation classes. Layout accepts a finite
region, font size, line spacing, line/column cap, and error/clip/expand overflow
policy. Serialization uses canonical six-decimal numbers and deterministic
resource names.

The true-edit path currently requires the old paragraph to occupy exactly one
decoded PDF string token. A paragraph split across independent strings must be
selected through a future multi-token provenance operation; Oxide returns an
exact error and does not silently use an overlay. The bundled DejaVu font covers
Arabic and Hebrew but not arbitrary CJK, so vertical Japanese requires a
caller-supplied font containing the requested glyphs. Missing glyphs fail
closed with cluster diagnostics.

Rust example:

```rust
let (bytes, report) = oxide_engine::edit_advanced_text_pdf(
    &input, 1, "Invoice", "فاتورة 123",
    oxide_engine::AdvancedTextMode::ParagraphReflowRtl,
    &oxide_engine::AdvancedTextEditOptions::default(), None,
)?;
assert!(report.replacement_extracts && report.old_text_absent);
```

Failure example: vertical Japanese with the bundled fallback returns missing
UTF-8 cluster diagnostics rather than emitting `.notdef` glyphs.
