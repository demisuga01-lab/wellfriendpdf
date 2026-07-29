# Repository Professionalization Final Report

## Verdict

`professionalization_complete_evidence_backed`

## Scope

- README was replaced with an evidence-driven public README focused on product capabilities, measured benchmark numbers, fair comparator framing, exact limits, and MIT licensing.
- Internal numbered roadmap naming was migrated to product-domain names across production modules, tests, scripts, fuzz targets, and documentation filenames.
- Tracked generated `target/` artifacts were removed from the Git index while preserving local files.
- Package metadata was consolidated under MIT licensing while third-party attribution was preserved in `NOTICE` and `docs/licensing.md`.

## Benchmark corpus

The committed benchmark corpus is documented in `benchmarks/corpus/manifest.json`. It is compact and repository-owned, with synthetic fixtures only.

## Wellfriend measurements



## Same-host comparator measurements

- qpdf structural_check: median 1.6891839914023876 ms, p95 1.7953829956240952 ms, successes 10/10
- qpdf structural_rewrite: median 1.7910670139826834 ms, p95 1.8224340165033937 ms, successes 10/10
- Poppler pdfinfo page_count: median 3.8583370042033494 ms, p95 3.966359014157206 ms, successes 10/10
- Poppler pdftotext text_extraction: median 3.9752229931764305 ms, p95 4.171967972069979 ms, successes 10/10
- pikepdf (qpdf wrapper) open_save: median 0.34725997829809785 ms, p95 0.37985999369993806 ms, successes 10/10
- pypdfium2 (PDFium wrapper) page_count: median 0.03756699152290821 ms, p95 0.05104701267555356 ms, successes 10/10
- PyMuPDF (MuPDF wrapper) text_and_render: median 0.5129690398462117 ms, p95 0.5897340015508235 ms, successes 10/10
- pdfplumber text_extraction: median 0.668391992803663 ms, p95 0.7241659914143384 ms, successes 10/10

## Validation

Result folder: `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z`

Maximum observed RSS: 3021960 KiB, below the 32 GiB cap.

- final-cargo-fmt: exit 0, peak RSS 176928 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cargo-fmt.log`
- final-git-diff-check: exit 0, peak RSS 8480 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-git-diff-check.log`
- final-cargo-check-tmpdir: exit 0, peak RSS 2268404 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cargo-check-tmpdir.log`
- final-cargo-clippy: exit 0, peak RSS 2317612 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cargo-clippy.log`
- final-cargo-test-2: exit 0, peak RSS 433420 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cargo-test-2.log`
- final-readme-source-example: exit 0, peak RSS 2949720 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-readme-source-example.log`
- final-cli-help: exit 0, peak RSS 3021960 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cli-help.log`
- final-cli-smoke: exit 0, peak RSS 86176 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-cli-smoke.log`
- final-capi-check: exit 0, peak RSS 2047716 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-capi-check.log`
- final-wasm-check: exit 0, peak RSS 2014764 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-wasm-check.log`
- final-python-binding-check: exit 0, peak RSS 17588 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-python-binding-check.log`
- final-capi-native-build: exit 0, peak RSS 2663560 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-capi-native-build.log`
- final-dotnet-build-noserver: exit 0, peak RSS 178080 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-dotnet-build-noserver.log`
- final-dotnet-test-native-2: exit 0, peak RSS 138204 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-dotnet-test-native-2.log`
- final-maven-test-native: exit 0, peak RSS 287332 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-maven-test-native.log`
- final-gradle-provision: exit 0, peak RSS 124152 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-gradle-provision.log`
- final-gradle-test-8c: exit 0, peak RSS 413400 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-gradle-test-8c.log`
- final-naming-audit-vps: exit 0, peak RSS 31588 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/final-naming-audit-vps.log`
- benchmark-run-2: exit 0, peak RSS 374564 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/benchmark-run-2.log`
- comparator-benchmark: exit 0, peak RSS 101460 KiB, artifact `/home/demisuga01/wellpdf/results/repository-professionalization-20260729T192340Z/comparator-benchmark.log`

## Naming and license closure

- Naming audit verdict: `zero_internal_roadmap_names`.
- Stale brand audit verdict: `no_legacy_branding`.
- License consistency verdict: `mit_license_consistent`.

## Remaining product boundaries

The README and linked docs keep the public boundary explicit: unsupported or ambiguous edits return typed refusal, low-confidence semantic/OCR reconstruction requires review, dynamic XFA conversion is not universal, appearance/rendering parity is viewer-dependent in some edge cases, accessibility still requires human review for semantic correctness, and comparator results are task-specific rather than a universal ranking.
