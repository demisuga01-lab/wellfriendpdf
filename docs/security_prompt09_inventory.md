# Prompt 09 Security Inventory

Prompt 09 starts from `2625870` with a clean tree. The audit found mature foundations for standard encryption, CMS signatures, PDF/A/UA checks, deterministic writing, redaction, and fuzz targets. Prompt 09 adds the unified security report, sanitizer, validation-profile reports, signature check separation, canonical output audit, and structure-aware mutation harness.

| Feature | Current state | Prompt 09 target | Tests | External tools | Remaining limit |
|---|---|---|---|---|---|
| AES-128/256 standard security handler | RC4, AES-128, AES-256 read/write paths exist | Report and test AES-256 explicitly | `prompt09_security::aes256_security_report_is_explicit_about_permissions` | None | Public-key handlers remain detection-only |
| public-key security handler | Detected as unsupported in crypto path | Security report exposes detection and unsupported status | Synthetic report path | None | Certificate decryption later |
| permissions model | Permission bits reported in `DocumentInfo` | Add explicit owner-password policy note | Prompt 09 AES-256 report test | None | Viewer policy is not enforcement against processors |
| AES-GCM roadmap | Not implemented | Detect/report as unsupported PDF 2.0 extension work | Security report fields | None | Needs spec vectors before implementation |
| ByteRange parser | Implemented in signature verifier | Add explicit check bits | Prompt 09 signature check test | None | DocMDP/FieldMDP permission checks bounded |
| CMS/PKCS#7 validation | RSA/CMS detached verification exists | Keep exact status separation | Existing signature tests plus Prompt 09 | None | ECDSA/RSA-PSS remain unsupported |
| PAdES profile checks | Subfilters/timestamp/DSS material reported | Keep profile level in signature report | Existing LTV tests | None | No full PAdES certification |
| DSS/LTV | DSS/VRI/certs/OCSP/CRL reported | Expose LTV material and conservative verified bit | Existing LTV tests | None | Live revocation not fetched |
| timestamp validation | Timestamp token presence/parse count reported | Report timestamp present, verified=false unless policy verifies | Existing timestamp test | None | TSA trust/imprint validation later |
| incremental signing | Append signing path exists | Keep report and docs precise | Existing signing tests | qpdf optional | Non-RSA algorithms later |
| signature-preserving incremental update | LTV append preserves byte prefix | Security docs and signature impact notes | Existing LTV tests | qpdf optional | DocMDP/FieldMDP not fully evaluated |
| sanitizer active-content removal | Redaction had removal pieces | Add security policy sanitizer and strict rescan | Prompt 09 sanitizer test | None | Deep referenced action graph is best effort |
| sanitizer attachment removal | Prompt 07B policy exists | Security sanitizer removes EmbeddedFiles/FileAttachment | Prompt 09 sanitizer test | None | Attachment object GC is full-rewrite bounded |
| sanitizer metadata/XMP cleanup | Redaction scrub exists | Security sanitizer removes metadata streams/refs | Prompt 09 sanitizer test | None | Semantic metadata consistency later |
| PDF/A validation profile | PDF/A subset exists | veraPDF-style profile report wrapper | Prompt 09 standards test | veraPDF optional | No certification claim |
| PDF/A conversion/output intent | Basic PDF/A conversion exists | Document supported conversion scope | Compliance tests | veraPDF optional | Full archival conversion later |
| PDF/UA validation profile | Tag/MCID subset exists | Profile wrapper and docs | Prompt 09 standards test | veraPDF optional | Full accessibility audit later |
| PDF/X validation profile | Color/prepress report exists | OutputIntent, active content, TrimBox subset | Prompt 09 standards test | veraPDF/qpdf optional | Full PDF/X rules later |
| Arlington validation integration | Generated tables from Prompt 01 | Include Arlington status in standards report | Prompt 09 standards test | None | Full predicate evaluation bounded |
| veraPDF-style profile framework | Partial compliance structs | Add profile/rule/status JSON report | Prompt 09 standards test | veraPDF optional | Rule coverage incremental |
| fuzzing targets | Many cargo-fuzz targets exist | Compile and document Prompt 09 coverage | `cargo check --manifest-path fuzz/Cargo.toml --bins` | cargo-fuzz optional | Long fuzz runs are CI/user work |
| structure-aware mutators | Structured PDF fuzz target exists | Add deterministic mutation script | `scripts/prompt09_structure_mutator.py` smoke | None | Not coverage-guided by itself |
| metamorphic tests | Some property tests exist | Add sanitize-preserves-text and canonical determinism | Prompt 09 tests | None | Larger corpus runs in Prompt 10 |
| differential tests | Availability-aware scripts exist | Document qpdf/Poppler/MuPDF/PDFium/PDFBox/veraPDF behavior | Smoke commands | Optional tools | Skips absent tools |
| deterministic canonical output | Prompt 08B writer determinism exists | Add canonicalize API/CLI/report | Prompt 09 canonical test | qpdf optional | Full rewrite invalidates signatures |
| security threat model | Scattered docs | Add threat model and policy docs | Docs | None | Formal audit later |
