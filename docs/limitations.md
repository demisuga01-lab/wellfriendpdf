# Current boundaries

Wellfriend is implementation-complete for the supported true-editing architecture, but it is not a claim of universal PDF or viewer parity.

## Practical boundaries

- Unsupported or ambiguous edits return typed refusals; the engine does not silently downgrade edit modes.
- Low-confidence semantic reconstruction, OCR output, mathematical inference, and destructive replacement require review or explicit approval.
- Dynamic XFA is preserved and inventoried; universal lossless dynamic-XFA conversion is not claimed.
- Appearance generation is viewer-independent for supported annotations and AcroForm widgets, but all-viewer visual parity is not claimed.
- Accessibility repair can rebuild and validate supported structures, but human accessibility review remains necessary for meaning and alternate-text quality.
- Signature handling distinguishes byte-range integrity, modification coverage, and certificate trust; unanchored signatures are not described as trusted.
- External comparator and standards tools are reported only when actually available and run for the relevant task.

## Release posture

The repository is suitable for supported engineering evaluation and controlled embedding. A public release tag or certification requires the normal release process and any external audit chosen by the owner.
