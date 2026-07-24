# Parser Differential Testing

Use `scripts/parser_differential.py` to compare Wellfriend parser facts against
external engines without making those tools mandatory for normal development.

Example:

```text
python scripts/parser_differential.py tests --wellfriendpdf target/debug/wellfriendpdf.exe --limit 25 --json-out target/parser-diff/results.jsonl --markdown-out target/parser-diff/results.md
```

The script detects qpdf, Poppler `pdfinfo`, and MuPDF `mutool` when available.
It compares shallow stable facts: open success, page count where exposed,
linearization flag where exposed, object/xref counts from Wellfriend, and warning or
error presence. It does not compare raw bytes or full object semantics.

Use `--require-tools qpdf,pdfinfo` for stricter local runs.
