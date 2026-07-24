# Prompt 09 Security Threat Model

## Threats

- Malicious PDFs with broken xrefs, duplicate objects, cyclic page/StructTree graphs, and malformed dictionaries.
- Decompression bombs and codec bombs.
- Object graph bombs and huge page boxes.
- JavaScript, Launch actions, SubmitForm, remote GoTo, external URLs, XFA, rich media, 3D, movie, sound, embedded files, and external file specs.
- Malformed encryption dictionaries and wrong-password denial cases.
- Malformed signatures, overlapping ByteRanges, padded Contents, unsupported CMS algorithms, and misleading PAdES/DSS metadata.
- Malformed fonts, images, color profiles, functions, annotations, forms, tables, and semantic structure.
- Untrusted cloud/self-hosted uploads and batch processing of hostile customer documents.

## Controls

- Rust memory safety with `#![forbid(unsafe_code)]`.
- Decode and render caps.
- Parser-report and Arlington diagnostics.
- Sanitizer policies with strict rescan.
- Signature status separation.
- Standards-profile rule reports.
- Fuzz, structure-aware mutation, metamorphic checks, and differential smokes.
- Deterministic canonical output for auditability.
- Prompt 11B native CMM keeps LittleCMS/lcms2 behind the explicit
  `native-cmm-lcms2` feature, preserves `wellfriendpdf-engine` `forbid(unsafe_code)`,
  applies ICC size/channel validation, and reports fallback/native backend state
  across bindings.

## Non-Goals

- Executing document JavaScript or XFA.
- Full malware scanning of embedded payloads.
- Full legal PDF/A/UA/X certification.
- Full PKI, TSA, OCSP, CRL, and PAdES policy validation.
- Certification-grade PDF/X proofing, device-link ICC, multicolor ICC,
  separation framebuffers, spot/DeviceN plates, and bounded Prompt 13 overprint
  simulation.
# Prompt 04 Codec Boundary Note

Combined Prompt 04 keeps the engine's codec default pure Rust and adds a central native codec registry/allowlist policy. Native/C codec dependencies remain denied by default, native in-process decode is forbidden by default, and future native backends must be feature-gated, worker-required, allowlisted, and report-visible.

RLBox/WASM codec sandboxing is explicitly hard-blocked for this repository state by `target/prompt04-codec-boundary-scheduler/rlbox-wasm-feasibility.json`; Wellfriend must not claim RLBox production support until a reproducible cross-platform prototype exists.

Prompt 05 extends the codec threat model from renderer decode into extraction,
image extraction, parser-report decode probes, and attachment extraction. These
paths now inherit scheduler memory-token admission and fail-closed diagnostics
through the shared decode/report layer. Hostile codec corpus and fuzz campaign
artifacts are generated under `target/prompt05-codec-closeout/`; long fuzz
campaigns remain release hardening work, not a prerequisite for starting the
renderer parity phase.

Renderer decode paths now acquire scheduler memory tokens before image/stream decode and observe render cancellation before decoder entry. This is a bounded-memory and deterministic-order improvement, not a renderer parity phase.
