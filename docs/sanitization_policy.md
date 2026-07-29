# Sanitization Policy

Prompt 35 sanitizer presets are explicit. A request must identify the preset,
approval, and full-rewrite acknowledgement where the operation can remove data
or alter executable behavior.

Supported sanitizer families include metadata, attachments, JavaScript/actions,
embedded files, optional hidden content, and conservative all-supported cleanup
through the canonical sanitizer. Unsupported or ambiguous active content is
reported instead of silently retained as a pass.

Every sanitizer result includes removed feature families, refused features,
active-content evidence, output hash, and reopen validation.
