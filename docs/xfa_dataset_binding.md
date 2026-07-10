# XFA dataset binding

Oxide implements bounded explicit `ref`, name, global, and none bindings. `$record`, `$data`, and common `xfa.datasets.data` prefixes are normalized; simple dotted SOM segments and zero-based indices are supported. Dynamic repeated subforms propagate the selected instance node to child name/relative-ref binding while absolute refs remain rooted at the document dataset. Repeated, missing, and duplicate nodes produce deterministic results and diagnostics. Dataset order is retained.

Raw XML values are always preserved. Numeric and ISO-like date checks add coercion diagnostics without destroying the source value. Locale-specific pictures, predicates, wildcards, parent axes, class selectors, and arbitrary SOM methods are outside the subset and must not be inferred as supported.

Dataset materialization is capped at 50,000 nodes.
