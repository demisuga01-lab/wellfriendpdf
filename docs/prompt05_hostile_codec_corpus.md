# Prompt 05 Hostile Codec Corpus

The hostile codec corpus is generated, not committed as binary blobs:

```powershell
python scripts\prompt05_hostile_codec_corpus.py generate
python scripts\prompt05_hostile_codec_corpus.py run --wellfriendpdf-bin target\debug\wellfriendpdf.exe
```

Artifacts:

- Manifest: `target/prompt05-codec-closeout/hostile-corpus-manifest.json`
- Fixtures: `target/prompt05-codec-closeout/hostile-corpus/`
- Run report: `target/prompt05-codec-closeout/hostile-corpus-run.json`

The generator currently creates 25 fixtures covering Flate bombs, predictor
bombs, truncated Flate streams, invalid PNG predictors, huge dimensions,
malformed/truncated DCT, malformed JPX, JBIG2 stress/corruption, CCITT malformed
runs/impossible dimensions, excessive filter chains, unknown filters, wrong
DecodeParms, negative Length, stream/endstream mismatch, object-stream edge
cases, inline-image EOD ambiguity, malformed image masks, malformed ICC
profiles, embedded-file bombs, metadata bombs, and incremental revision stream
traps.

Each manifest row records category, trigger type, expected result,
memory/time expectations, worker-isolation expectation, regression flag, raw
seed path, PDF wrapper path, and generator command. The runner uses
`wellfriendpdf parser-report --include-decode` with strict decode and scheduler budgets
so malformed inputs produce structured reports instead of panics or silent
fallback.

