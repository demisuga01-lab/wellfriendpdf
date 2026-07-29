# XFA Runtime known limits

Exact bounded limits remaining after XFA Runtime:

- No full Adobe LiveCycle/AEM Forms compatibility claim.
- JavaScript and proprietary XFA scripts are inventory-only and never execute.
- FormCalc is a pure expression subset for calculate/validate; no loops, side effects, broad SOM, locale pictures, or host functions.
- Dynamic layout is one bounded deterministic pass; complex keep/overflow leader/trailer graphs, cycles, arbitrary DOM mutation, and proprietary extensions are unsupported exact.
- Static image fields are inventoried; broad XFA image decoding/rendering and external image loading are not claimed.
- Barcode generation and dynamic signature semantics require external engines and are not implemented.
- Flattening uses page overlays on existing PDF pages; it does not synthesize missing source pages for dynamic output.
- Mutation is a full rewrite and does not preserve existing signature ByteRanges or promise DocMDP/FieldMDP compliance.
- Secure redaction is not claimed while unflattened XFA capable of regeneration remains.
- Reference evidence proves flattened output opens/renders in installed Poppler; it does not establish dynamic XFA parity. PDFium, MuPDF, and Adobe observations are reported unavailable unless actually supplied/run.
