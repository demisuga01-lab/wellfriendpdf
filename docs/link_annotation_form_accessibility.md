# Link, Annotation, and Form Accessibility

document security preserves accessibility relationships around link annotations,
ordinary annotations, form widgets, and document subsystems form edits.

Supported operations:

- repair accessible link and annotation relationships after movement or
  redaction;
- redact resolved annotation geometry through the canonical redaction path;
- redact resolved form fields through the document subsystems form mutation path;
- retain widget/source evidence and validation notes in the document security operation
  report.

Unsupported action types, unresolved field names, and signature-protected edits
return exact typed failures.
