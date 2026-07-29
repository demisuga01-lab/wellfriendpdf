# Crypto Standards Fuzz veraPDF parity

Crypto Standards Fuzz introduces a reproducible veraPDF comparison runner:

```bash
python scripts/crypto_standards_fuzz_verapdf_parity.py \
  --corpus /home/demisuga01/wellpdf/corpus/verapdf/veraPDF-corpus \
  --verapdf-bin verapdf \
  --wellfriendpdf-bin target/release/wellfriendpdf \
  --artifact-root target/crypto_standards_fuzz-verapdf-crypto-fuzz
```

The runner executes veraPDF and `wellfriendpdf pdfa-validate` on the same PDF files,
normalizes PDF/A profile labels, compares pass/fail/exact-unsupported outcomes, and
writes:

- `verapdf-tool-manifest.json`
- `verapdf-corpus-manifest.json`
- `verapdf-parity-results.json`
- `verapdf-mismatch-classification.json`
- `pdfa4-parity-results.json`

Supported Wellfriend executable PDF/A profiles for this roadmap task are PDF/A-1b, 2b, 2a,
3b, and 3a. PDF/A-1a, 2u, 3u, 4, 4e, and 4f are not counted as conformant unless the
engine implements them; they must be reported as exact unsupported with evidence.
If an unsupported-profile corpus file is rejected earlier by the parser because the
PDF container itself is outside the engine's accepted syntax/version envelope, the runner
classifies that separately as a safe parse-level rejection rather than a conformance pass.

veraPDF is the authoritative external comparison tool for this Crypto Standards Fuzz PDF/A unit, but
the Wellfriend report remains a clause-mapped diagnostic engine rather than an accredited
certification statement. qpdf and pyHanko may provide structural/signature evidence, but
they are not substitutes for veraPDF PDF/A conformance.
