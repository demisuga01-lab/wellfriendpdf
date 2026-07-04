# Search And Redaction Text Model

Prompt 06 adds source-quad search. It prepares redaction geometry but does not apply redactions.

## API

```rust
let matches = engine.search_text(
    &[1],
    "quick brown",
    TextSearchOptions {
        case_sensitive: false,
        ..Default::default()
    },
)?;
```

Each `TextSearchMatch` contains:

- page
- original match text
- normalized query text
- character range
- source quads
- confidence
- provenance flags

## Matching

Supported by the default normalizer:

- exact string search
- case-insensitive search
- common Unicode ligature expansion
- hyphenation-aware line-break matching
- whitespace-collapsed multi-line matching
- hidden text exclusion by default

`include_hidden` can be enabled for audit/search-all modes.

## Redaction Readiness

Prompt 06 returns conservative glyph/character quads that can drive:

- preview highlights
- candidate redaction regions
- audit output
- later redaction apply logic

Prompt 06 intentionally does not apply redactions. Applying redactions requires content removal/appearance updates and belongs to Prompt 07 or a focused redaction editing phase.

## Diagnostics

The model can emit diagnostics for:

- hidden or invisible text observed
- duplicate text removed
- ActualText replacement used
- low-confidence visual ordering
- page chunk/character cap hits

Search callers should inspect provenance when deciding whether hidden text, OCR text, or low-confidence order should be considered.
