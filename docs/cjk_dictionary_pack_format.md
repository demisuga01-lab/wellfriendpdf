# CJK Dictionary Pack Format

A dictionary pack is a manifest JSON file plus a UTF-8 TSV entries file.

Manifest fields:

- `pack_id`
- `languages`
- `scripts`
- `source`
- `license`
- `version`
- `date`
- `hash`
- `entries_path`
- `entry_count`
- `generation_command`
- `normalization_form`
- `redistribution_allowed`
- `expected_memory_footprint_bytes`

TSV entry format:

```text
term<TAB>language<TAB>priority<TAB>source<TAB>confidence
```

`term` and `language` are required. `priority`, `source`, and `confidence` are
optional. Supported language tags are `zh`, `ja`, `ko`, `mixed`,
`mixed_latin`, and `und`.

The current normalization policy is `trim_no_unicode_rewrite`. Wellfriend trims entry
edges but does not silently normalize or rewrite Unicode content. Pack builders
must apply any desired Unicode normalization before generating the manifest
hash.
