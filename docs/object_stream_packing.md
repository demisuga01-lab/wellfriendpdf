# Object Stream Packing

writer history exposes deterministic PDF 1.5+ object-stream packing through the existing writer mode `WriterMode::XrefStreamWithObjStm`.

Eligible objects are non-stream indirect objects. Ineligible classes include stream objects, signature dictionaries, old xref/object-stream containers, encryption dictionaries, and objects excluded by compatibility policy. Packed output is a full rewrite and is opt-in.

Generated evidence for `form_160f.pdf`:

| Metric | Result |
| --- | --- |
| Input objects | 541 |
| Eligible packed objects | 441 |
| Object streams | 16 |
| Xref streams | 1 |
| Packed size ratio | 0.8470031790338839 |

Artifacts: `object-stream-eligibility-writer_history.json`, `object-stream-xref-results-writer_history.json`, and `object-stream-determinism-writer_history.json`.
