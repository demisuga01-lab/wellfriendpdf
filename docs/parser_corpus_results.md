# Parser Corpus Results

Prompt 01B adds the expanded corpus runner but does not vendor SafeDocs or other
large corpora into the repository.

Run a bounded local corpus pass with:

```text
python scripts/parser_corpus_runner.py --input PATH_TO_CORPUS --wellfriendpdf-bin target/debug/wellfriendpdf.exe --output target/parser-corpus/audit.jsonl --summary docs/parser_corpus_results.md --mode audit --limit 200 --max-total-bytes 1073741824 --timeout 30
```

The runner records category, size, mode, open status, diagnostic counts by
severity, recovered object count, timeout/crash status, elapsed time, and notes.
If SafeDocs is absent, use repository fixtures for smoke coverage and keep the
result label honest: fixture smoke is not an all-SafeDocs pass.
