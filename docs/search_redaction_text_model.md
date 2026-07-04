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
- provenance summary
- MCIDs and tagged role when available
- hidden-text inclusion flag

## Matching

Supported by the default normalizer:

- exact string search
- case-insensitive search
- common Unicode ligature expansion
- hyphenation-aware line-break matching
- whitespace-collapsed multi-line matching
- hidden text exclusion by default

`include_hidden` can be enabled for audit/search-all modes.

## Prompt 06B Provenance

Search matches now expose the same structure/provenance layer as the semantic
model:

- `mcids`: tagged-PDF marked-content IDs when available.
- `role` and `role_source`: tagged/RoleMap role metadata or `unknown`.
- `provenance_summary`: counts for ActualText, ToUnicode, CMap, glyph-name,
  hidden/OCR, unknown, tagged MCID, and synthetic layout sources.
- `includes_hidden`: whether the match uses hidden/invisible text.

Prompt 07 redaction apply should consume these match quads and provenance fields
instead of re-searching raw content streams.

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
