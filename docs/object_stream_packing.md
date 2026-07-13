# Object Stream Packing

Prompt 21 exposes deterministic PDF 1.5+ object-stream packing through the existing writer mode `WriterMode::XrefStreamWithObjStm`.

Eligible objects are non-stream indirect objects. Ineligible classes include stream objects, signature dictionaries, old xref/object-stream containers, encryption dictionaries, and objects excluded by compatibility policy. Packed output is a full rewrite and is opt-in.

Generated evidence for `form_160f.pdf`:

| Metric | Result |
| --- | --- |
| Input objects | 541 |
| Eligible packed objects | 441 |
| Object streams | 16 |
| Xref streams | 1 |
| Packed size ratio | 0.8470031790338839 |

Artifacts: `object-stream-eligibility-prompt21.json`, `object-stream-xref-results-prompt21.json`, and `object-stream-determinism-prompt21.json`.
