# Prepress Proofing Prepress Proofing Benchmark

The benchmark entrypoint is:

```powershell
python scripts/prepress_proofing_prepress_benchmark.py
```

It writes artifacts under `target/prepress_proofing-prepress-closeout/` and covers 18
fixture categories: process overprint, spot overprint, DeviceN, OPM 0/1,
fill/stroke distinction, text, vector, image, shading, tiling pattern,
transparency, soft masks, output intents, BPC, rendering intents, device-link,
multicolor ICC context, and malformed fail-closed cases.

Recorded dimensions include input PDF hash, preview hash, plate hash, backend
posture, rendering intent, BPC, output-intent hash, profile hashes, plate names,
channel counts, tile/band/progressive/cache status, memory, diagnostics,
unsupported exact rows, and reference renderer status.

Poppler, PDFium, and MuPDF are executed when target-local tools are available.
Missing reference tools are reported as `unavailable_exact`; they are not counted
as passed.
