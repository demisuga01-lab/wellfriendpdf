# AES-GCM PDF 2.0 Roadmap

Prompt 09 does not implement AES-GCM. It detects crypt-filter names that look like AES-GCM/integrity-extension work and reports them as unsupported.

Implementation requires:

- PDF 2.0 extension specification text identifying the exact crypt-filter names, IV layout, authentication tag handling, metadata rules, and object key derivation.
- Compact test vectors covering strings, streams, object streams, metadata encryption, wrong passwords, and tampered authentication tags.
- A pure-Rust AEAD implementation already acceptable under `#![forbid(unsafe_code)]`.
- Writer tests proving deterministic structure where possible while retaining required random encryption material.

Until those prerequisites exist, Wellfriend must fail closed or report unsupported. It must not silently treat AES-GCM as AES-CBC.
