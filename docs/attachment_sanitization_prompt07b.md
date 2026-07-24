# Prompt 07B Attachment Sanitization

Prompt 07B adds explicit attachment policy for redaction and sanitization.

## Policies

- `keep`: preserve attachments.
- `remove-all`: remove catalog embedded-file name-tree entries and page
  FileAttachment annotations.
- `remove-overlapping`: remove FileAttachment annotations whose rect overlaps a
  redaction region.

The full-rewrite garbage collector then drops unreachable embedded-file streams.

CLI:

```powershell
wellfriendpdf redact input.pdf --text SECRET --attachments remove-all --out redacted.pdf --strict
```

## Limits

- Full document privacy sanitization, associated-files policy, and signature
  validation belong to Prompt 09.
- FileAttachment icon painting is basic when annotations are flattened.
