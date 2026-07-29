# Crypto Standards Fuzz long parser fuzz campaign

Parser fuzz scope:

- COS object parser
- tokenizer and content lexer
- numeric, name, string, array, and dictionary parsing
- stream dictionary parsing
- xref table and xref stream parsing
- object stream parsing
- trailer, root, and catalog diagnostics
- incremental revision chain and rewrite interaction
- repair scanner behavior
- linearization hints
- hybrid-reference files through the end-to-end parser
- encrypted-object metadata where safe without keys
- malformed object graph traversal

Crypto Standards Fuzz parser targets:

- `parse_pdf`
- `content_tokenizer`
- `cos_object`
- `parser_report`
- `xref_stream`
- `object_stream`
- `document_rewrite`
- `linearize`
- `structured_pdf`
- `decode_scanner`
- `crypto`

VPS campaign command shape:

```bash
python scripts/release_fuzz_runner.py \
  --group parser \
  --long-high-priority \
  --high-priority-seconds 1800 \
  --smoke-runs 64 \
  --memory-mb 16384 \
  --max-len 262144 \
  --json-output target/crypto_standards_fuzz-verapdf-crypto-fuzz/long-parser-fuzz-results.json
```

The high-priority long target is `parse_pdf`, because it exercises the real end-to-end
open-bytes path over COS, xref, trailer, catalog, object stream, hybrid-reference, and
repair behavior. Specialized parser targets still build and smoke-run to keep narrower
parsers covered.

No unclassified crash, hang, timeout, or OOM can remain for Crypto Standards Fuzz closure.
