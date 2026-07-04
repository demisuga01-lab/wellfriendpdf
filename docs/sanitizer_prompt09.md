# Prompt 09 Sanitizer

The sanitizer detects and removes active or risky content without executing it.

## Policies

- `strict`: remove JavaScript, Launch, SubmitForm, URI, GoToR, Named actions, embedded files, file-attachment annotations, rich media, OpenAction, AA, metadata streams, and XFA.
- `balanced`: remove high-risk active content and payloads, preserve URI/named links and metadata.
- `preserve-visual`: remove active payloads while preserving visual file-attachment annotations where possible.

## Removed Content

Prompt 09 removes or nulls:

- `/S /JavaScript`, `/JS`, and `/JavaScript` entries.
- `/S /Launch`, `/S /SubmitForm`, `/S /URI`, `/S /GoToR`, `/S /Named`, and `/S /Rendition` according to policy.
- `/OpenAction` and `/AA`.
- `/EmbeddedFiles`, `/Type /EmbeddedFile`, `/Type /Filespec`, and `/EF`.
- `/Subtype /FileAttachment`.
- `/Subtype /RichMedia`, `/Movie`, `/Sound`, and `/3D`.
- `/XFA`.
- Metadata streams/references when metadata scrubbing is enabled.

After rewriting, Oxide reopens and rescans the output. Strict mode reports failure if risky content remains reachable.

## Limits

The sanitizer is a structural PDF sanitizer, not a malware scanner. It does not execute scripts, fetch URLs, inspect embedded binary payload internals, or certify that a third-party viewer will never interpret an unknown extension dictionary.
