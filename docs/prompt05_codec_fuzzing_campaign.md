# Prompt 05 Codec Fuzzing Campaign

Prompt 05 adds a campaign harness over the existing `fuzz/` cargo-fuzz crate:

```powershell
python scripts\prompt05_codec_fuzz_campaign.py --mode smoke --dry-run
python scripts\prompt05_codec_fuzz_campaign.py --mode local-long
python scripts\prompt05_codec_fuzz_campaign.py --mode release-long
```

Artifacts:

- Target inventory: `target/prompt05-codec-closeout/fuzz-target-inventory.json`
- Smoke report: `target/prompt05-codec-closeout/fuzz-smoke-report.json`
- Dictionary: `target/prompt05-codec-closeout/fuzz-dictionary.txt`
- Crash artifacts: `target/prompt05-codec-closeout/fuzz-artifacts/`

Logical Prompt 05 targets map to existing cargo-fuzz bins: filter chains,
image inventory, DCT, JPX, JBIG2, CCITT, predictors, PDF wrappers, inline
images, worker-protocol-compatible payloads, and scheduler-admission payloads.

Smoke mode compiles all fuzz bins with:

```powershell
cargo check --manifest-path fuzz\Cargo.toml --bins --jobs 1
```

If `cargo fuzz` and nightly are available, smoke mode can also run bounded
`-runs=1` libFuzzer executions. If unavailable, the report records the precise
reason and still emits the long-run commands, artifact layout, minimization
commands, reproduction commands, and regression-promotion path.

