# Security Policy

## Reporting

Report suspected vulnerabilities through the project maintainer's private security channel before public disclosure. Include a minimal PDF or mutation script where possible.

## Unsafe Code

The engine crate uses `#![forbid(unsafe_code)]`. Native dependencies must be optional, isolated, documented, and kept out of default/WASM builds unless a later policy explicitly changes that posture.

## Dependency Updates

Security-sensitive dependencies for crypto, parsing, image decoding, compression, and XML/OOXML handling should be updated promptly after advisories and verified through the standard gates.

## Fuzzing

Fuzz targets should compile on every release candidate. Long fuzz runs are expected before major releases and may be split by target family.

## Sandboxing

Oxide does not execute PDF active content. Integrators running untrusted files at scale should still combine Oxide with process/container sandboxing, CPU/memory/time limits, and storage isolation.

## Crypto and Standards Claims

Only claim cryptographic validation when actual verification was performed. Only claim standards certification when an external certification-grade validator and applicable corpus pass.
