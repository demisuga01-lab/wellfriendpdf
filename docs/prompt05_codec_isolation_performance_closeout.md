# Prompt 05 Codec Isolation and Performance Close-Out

Close-out command:

```powershell
python scripts\prompt05_codec_closeout.py
```

Artifacts:

- Inventory: `target/prompt05-codec-closeout/decode-callsite-inventory.json`
- Coverage matrix: `target/prompt05-codec-closeout/codec-coverage-matrix.json`
- Hostile corpus manifest: `target/prompt05-codec-closeout/hostile-corpus-manifest.json`
- Hostile corpus run: `target/prompt05-codec-closeout/hostile-corpus-run.json`
- Fuzz inventory: `target/prompt05-codec-closeout/fuzz-target-inventory.json`
- Fuzz smoke: `target/prompt05-codec-closeout/fuzz-smoke-report.json`
- Performance report: `target/prompt05-codec-closeout/performance-report.json`
- Verdict: `target/prompt05-codec-closeout/closeout-verdict.json`

Release-grade verdict is deliberately narrow: Prompt 05 can close the
codec/performance phase for starting the renderer parity campaign when the
inventory has no partial/blocked/missing rows, the hostile corpus runner passes,
and fuzz targets compile. This is not a claim that multi-day fuzz hardening has
already completed. Release candidates still require the recorded local-long and
release-long fuzz campaigns.

Known limits:

- RLBox/WASM sandboxing remains hard-blocked; OS subprocess isolation remains the practical native boundary.
- OCR is not claimed unless the optional backend is compiled and configured.
- Worker overhead is not remeasured by Prompt 05; Prompt 03 release-gate evidence remains authoritative.
- Long fuzz campaigns are infrastructure-complete but must be run for release hardening.

