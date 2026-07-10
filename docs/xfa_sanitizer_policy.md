# XFA sanitizer policy

Modes are:

- `remove_scripts_events_connections`: rewrite validated XML without script/event/calculate/validate/connect nodes and drop connectionSet/sourceSet packet pairs.
- `preserve_static_data`: the same active-content neutralization while retaining template/datasets.
- `remove_all_xfa`: remove AcroForm `/XFA` references.
- `flatten_then_remove`: statically flatten only when eligible, then remove XFA.

Every mode rescans the output. A neutralization pass succeeds only when scripts, events, and external connections are absent; a remove pass succeeds only when XFA is absent. Malformed XML is never rewritten as if sanitized. All modes are full rewrites with explicit signature impact.
