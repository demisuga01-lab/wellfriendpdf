# Annotation XFDF import, export, and security

Prompt 17 exports namespace-correct UTF-8 XFDF with stable ordering by page, annotation ID, and subtype. Existing `/NM` values are preserved. Missing IDs are deterministically derived from page, index, subtype, rectangle, and contents. File IDs, geometry, author/title/subject, dates, colors, opacity, blend mode, flags, intent/state, popup/reply links, OCG metadata, Widget linkage, safe scalar extensions, static-appearance metadata, attachment names, and action inventory are represented.

Import supports safe-field merge, replacement, or conflict rejection; create/update; explicit-ID deletion; 1-based engine page mapping from XFDF’s 0-based `page`; stable relationship resolution; optional AP regeneration; deterministic full rewrite; reopen verification; and signature-impact reporting. New standalone Widgets are rejected because the AcroForm field tree owns Widget semantics. File payloads and active actions are never imported.

The parser rejects invalid UTF-8, wrong namespaces, malformed structure/geometry, non-finite numbers, DTDs, entity declarations, undeclared entities, duplicate attributes, excessive bytes/nodes/attributes/depth/text, and excessive annotation/relationship/geometry counts. It performs no network or filesystem access.

Success:

```text
wellfriendpdf annotation-xfdf-export input.pdf --output annotations.xfdf --json
wellfriendpdf annotation-xfdf-import input.pdf annotations.xfdf --output imported.pdf --json
```

Failure example: an XFDF document containing `<!DOCTYPE ... SYSTEM "file:///...">` returns a malformed-input error before any annotation transaction begins.

Exact limits: rich text is sanitized to plain text; arbitrary XHTML/CSS is not reproduced. Unknown extensions are restricted to bounded scalar text. Action targets are inventoried by kind and SHA-256 rather than executed. This is not a full Acrobat XFDF compatibility claim.
