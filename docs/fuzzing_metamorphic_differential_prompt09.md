# Prompt 09 Fuzzing, Metamorphic, and Differential Testing

## Coverage-Guided Fuzzing

Existing fuzz bins cover parser, filters, crypto, writer, parser-report, color-report, PDF/A, editing, signature validation, structured PDFs, and display lists. Prompt 09 keeps the standard smoke:

```powershell
cargo check --manifest-path fuzz/Cargo.toml --bins
```

Long coverage-guided runs remain availability-aware:

```powershell
cargo fuzz run parse_pdf
cargo fuzz run signature_validation
cargo fuzz run structured_pdf
```

## Structure-Aware Mutation

`scripts/prompt09_structure_mutator.py` creates deterministic mutations for:

- stream `/Length` corruption
- signature `/ByteRange` overlap
- JavaScript and Launch injection
- OutputIntent corruption
- StructTree corruption
- duplicate object headers

Outputs are written under `target/prompt09-structure-mutations/`.

## Metamorphic Tests

Prompt 09 adds tests for:

- sanitize preserves safe text
- canonical rewrite is deterministic
- validation reports remain machine-readable after deterministic rewrite

Existing phases also cover redact-save-reopen, split/merge, deterministic writer, and table/text stability.

## Differential Tools

The recommended differential smoke is availability-aware:

- `qpdf --check`
- Poppler `pdfinfo` and `pdftotext`
- MuPDF `mutool`
- PDFium harness when configured
- PDFBox when available
- veraPDF when available

Missing tools are skipped in normal development and should be required only in strict CI profiles.
