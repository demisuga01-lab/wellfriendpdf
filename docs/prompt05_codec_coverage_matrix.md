# Prompt 05 Codec Coverage Matrix

The generated matrix lives at:

- `target/prompt05-codec-closeout/codec-coverage-matrix.json`

Generate it with:

```powershell
python scripts\prompt05_codec_closeout.py
```

Current matrix shape:

| Status | Count |
| --- | ---: |
| `scheduler_covered` | 16 |
| `metadata_only` | 4 |
| `already_covered` | 3 |
| `unsupported_reported` | 1 |
| `partial` | 0 |
| `blocked` | 0 |
| `missing` | 0 |

Every `scheduler_covered` row names a module, entry point, decode behavior, and
evidence artifact. Every `metadata_only` row explains why it cannot trigger
hostile stream decode. The native backend row remains `unsupported_reported`
because Prompt 04's central native codec registry denies unsafe native backends
by default.

