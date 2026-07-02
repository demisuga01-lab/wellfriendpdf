# Parser Repair

Parser repair is fail-soft and diagnostic. It does not silently declare damaged
files valid.

Implemented report fields include xref-known object count, scan-carved object
count, duplicate object numbers, truncated objects, stream length mismatches,
missing `endstream` cases, trailer reconstruction signals, recovered page
dictionaries, parse failures, skipped byte ranges, and a coarse confidence
label.

Forensic page-tree recovery is audit-only. If page dictionaries are recoverable
but `/Root` or `/Pages` is not, `parser-report` records recovered page object
numbers and marks the page tree as reconstructed for reporting. It does not
rewrite or replace the parsed document model.

Strict mode still fails on unrecoverable structure. Repair mode opens when the
bounded fallbacks can build a safe object index. Audit mode records both paths.
