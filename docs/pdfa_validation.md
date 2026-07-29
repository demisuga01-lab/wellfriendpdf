# PDF/A validation

The Incremental Signing Standards PDF/A API produces a clause-mapped `StandardsValidationReport`, not a boolean.
Supported-profile checks include XMP `pdfaid` detection, metadata posture, output intent/ICC
presence, forbidden encryption, embedded-font posture, and risky-content evidence.

Supported current engine profiles are the implemented PDF/A-1, PDF/A-2, and PDF/A-3 levels.
Missing identifiers, contradictory metadata, missing output intent, encryption, and unembedded
fonts produce deterministic failure rows. PDF/A-4 is an explicit
`deferred_crypto_standards_fuzz_corpus_parity` result and never a conformant result.

This is fixture-backed, clause-mapped validation direction, not an accredited claim or a
statement of complete veraPDF parity. External veraPDF comparison is recorded separately when
the optional validator is available.
