# Prompt 09 Encryption

Oxide's standard security-handler implementation supports legacy RC4, AES-128, and AES-256 document encryption through the pure-Rust crypto path. Prompt 09 does not weaken the existing behavior; it adds enterprise-facing reporting and tests so integrators can see exactly which handler is in use.

## Supported

- Standard security handler detection.
- AES-256 open/decrypt/encrypt behavior for the implemented revisions.
- AES-128 and legacy RC4 for compatibility.
- Encrypted strings, streams, object streams, and xref stream handling through the reader/decode layer.
- Permission-bit reporting for print, modify, copy, annotate, fill, accessibility, assemble, and high-resolution print policy.
- Wrong-password and unsupported-revision structured errors.

## Permission Model

PDF owner-password permissions are viewer-enforced restrictions after the document is opened. They are not a cryptographic secrecy guarantee against a processor that has the opening key. Prompt 09 reports this explicitly in `security_report`.

## Public-Key Handlers

Public-key security handlers such as `/Filter /Adobe.PubSec` are detected and reported. Certificate-based decryption is not implemented in the default pure-Rust engine and is not claimed.

## Tests

- AES-256 encryption/reporting is covered by `prompt09_security::aes256_security_report_is_explicit_about_permissions`.
- Existing structural encryption tests still cover AES-256, AES-128, and RC4 round trips.

## Limits

- Public-key decryption is detection-only.
- AES-GCM/integrity-extension support is roadmap-only until compact legal fixtures and spec vectors are available.
