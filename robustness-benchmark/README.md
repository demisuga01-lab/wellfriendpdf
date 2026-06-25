# Robustness Benchmark

This benchmark measures PDF parser survival on a small, deterministic wild-PDF
subset. It is intentionally separate from the synthetic extraction benchmark:
there is no ground truth here, so the score is robustness only.

The fast loop is about 200 PDFs selected deterministically from:

- committed parity PDFs under `tests/corpus/`,
- ignored local renderer/public benchmark corpora when present,
- generated malformed probes under `robustness-benchmark/corpus/generated/`.

Bulk PDF data is not committed. Regenerate local malformed probes and the tracked
manifest with:

```powershell
python robustness-benchmark\scripts\build_corpus_manifest.py
```

Run the text robustness benchmark with bounded workers, subprocess isolation,
timeout, memory polling, and per-tool/file JSONL checkpointing:

```powershell
cargo build --release -p oxide-cli
python robustness-benchmark\scripts\robustness_benchmark.py `
  --manifest robustness-benchmark\manifest.json `
  --oxide-bin target\release\oxide.exe `
  --output-dir target\robustness-benchmark\latest `
  --report docs\robustness_benchmark.md `
  --max-workers 4 `
  --timeout 60 `
  --max-memory-mb 2048
```

Numbers from this fast loop must be labeled `indicative (approx 200-file
subset)`. Prompt 10 is the full-scale validation pass.
