# Public PDF Benchmark

This directory contains the reproducible public-corpus text extraction benchmark
for WellfriendPdf. It is measurement infrastructure only; it does not change parser,
renderer, server, or CLI behavior.

## Layout

```text
public-benchmark/
  corpus/          ignored local PDFs downloaded by build_public_corpus.py
  manifests/       tracked reproducibility manifests
  results/raw/     ignored per-file benchmark output
  scripts/
    build_public_corpus.py
    run_text_benchmark.py
  capability_matrix.json
```

## Build The Corpus

```powershell
python public-benchmark\scripts\build_public_corpus.py `
  --target-count 4500 `
  --output-manifest public-benchmark\manifests\public_corpus_manifest.json
```

The script downloads public PDFs from:

- Mozilla pdf.js `test/pdfs` fixtures.
- veraPDF PDF/A, PDF/UA, and ISO PDF test corpus.
- PDF Association / DARPA SafeDocs public targeted PDF artifacts.
- arXiv PDFs across several categories for scale and real-world scholarly
  layout diversity.

Downloaded PDFs are stored under `public-benchmark/corpus/`, which is ignored by
git. The tracked manifest records source, URL, SHA-256, size, local path, and
category tags so the corpus can be recreated.

## Install Competitor Packages

Use an isolated environment if possible:

```powershell
python -m venv .venv-public-benchmark
.\.venv-public-benchmark\Scripts\python -m pip install -U pip
.\.venv-public-benchmark\Scripts\python -m pip install -r public-benchmark\requirements.txt
```

Tools that fail to install or import are skipped with an explicit note; the
benchmark never fabricates numbers.

## Run The Benchmark

```powershell
cargo build --release -p wellfriendpdf-cli
python public-benchmark\scripts\run_text_benchmark.py `
  --manifest public-benchmark\manifests\public_corpus_manifest.json `
  --wellfriendpdf-bin target\release\wellfriendpdf.exe `
  --output-dir public-benchmark\results\raw\run-full `
  --report docs\benchmark_public.md
```

The harness runs one isolated subprocess per `(tool, file)`, enforces a timeout,
records crash/timeout/error as data, measures wall time and peak RSS, and writes
overall and per-category aggregates plus a text-similarity sample.

For a quick harness smoke:

```powershell
python public-benchmark\scripts\run_text_benchmark.py --limit 25 --timeout 20
```
