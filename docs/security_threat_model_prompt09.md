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

## Non-Goals

- Executing document JavaScript or XFA.
- Full malware scanning of embedded payloads.
- Full legal PDF/A/UA/X certification.
- Full PKI, TSA, OCSP, CRL, and PAdES policy validation.
