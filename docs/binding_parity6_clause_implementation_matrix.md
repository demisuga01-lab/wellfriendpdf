# Incremental Signing Standards clause implementation matrix

The canonical implementation is `crates/engine/src/standards_engine.rs`. It emits stable rule
IDs, derived clause references, object/page/resource evidence, deterministic ordering, and one
of eight statuses: `pass`, `fail`, `warning`, `indeterminate`, `not_applicable`,
`unsupported_reported_exact`, `deferred_crypto_standards_fuzz_corpus_parity`, or
`blocked_normative_dependency`.

| Family | Implemented direction | Explicit limit |
| --- | --- | --- |
| PDF/A | identifiers, output intent/ICC posture, encryption, fonts, risky content and supported A-1/A-2/A-3 profiles | PDF/A-4 and full corpus parity are deferred rows |
| PDF/UA | identifier, MarkInfo, structure tree, language, title, role/alt/accessibility posture | human reading order is an exact unsupported/deferred judgement |
| PDF/X | identifiers, GTS output intent, page boxes, fonts, risky content, core color posture | deep DeviceN/overprint and older-profile transparency corpus parity are deferred rows |

Clause references are identifiers and derived behaviour only; this document does not reproduce
restricted standard text. A deferred or unsupported row makes the enclosing report
non-conformant or indeterminate rather than conformant.
