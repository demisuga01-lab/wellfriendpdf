# CJK Dictionary Provider

The provider loads one or more dictionary packs and builds a deterministic
longest-match index for CJK tokenization. The built-in dictionary is a tiny
synthetic fixture for tests and examples. Production use is through
user-supplied packs.

Provider behavior:

- validates manifest JSON and UTF-8 TSV entries;
- verifies `sha256:` entry-file hashes when present;
- rejects entry-count mismatches, invalid UTF-8, hash mismatches, empty packs,
  and memory/entry/token cap violations;
- supports multiple packs and languages;
- supports entry priority for overlapping terms;
- deduplicates by `term + language` after deterministic priority ordering;
- preserves source text and reports token provenance instead of rewriting text.

Default limits:

- `max_entries`: `500000`
- `memory_cap_bytes`: `67108864`
- `max_token_chars`: `64`

The provider API is exposed from the Rust crate root as
`CjkDictionaryProvider`, `CjkDictionaryPackManifest`, and
`CjkDictionaryProviderLimits`.
