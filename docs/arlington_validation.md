# Arlington Validation

Oxide consumes the Arlington PDF Model through generated Rust tables. Runtime
validation does not parse TSV files.

Source:

- upstream: `https://github.com/pdf-association/arlington-pdf-model`
- pinned commit: `5a8639424495c27a30df30bb9491a346f9316014`
- generated file: `crates/engine/src/generated/arlington_tables.rs`

Current coverage: 613 TSV files, 613 object models, 3983 key rules, 924
required-key rules, 3983 type rules, 441 indirect-reference rules, 1698 link
metadata rules, 3429 unsupported predicates reported, and 0 generator parse
warnings.

Implemented checks are intentionally parser/object-model level: required keys,
basic value types, allowed name values, shallow direct/indirect policy where
representable, deprecated-key diagnostics, and unsupported predicate reporting.
Full predicate evaluation is not complete.

Regeneration:

```text
python scripts/fetch_arlington_model.py --out target/arlington-pdf-model-5a863942
python scripts/generate_arlington_tables.py --arlington-root target/arlington-pdf-model-5a863942 --commit 5a8639424495c27a30df30bb9491a346f9316014 --out crates/engine/src/generated/arlington_tables.rs --stats-json target/arlington/arlington_stats.json --complete
```

`--complete` rejects tiny mock fixtures so the real integration cannot
accidentally regress to seed data.
