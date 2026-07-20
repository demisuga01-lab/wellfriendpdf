# Prompt 24 Clause Implementation Matrix

Rows summarize derived implementation behavior and cite clause families without reproducing restricted text.

- `pdf.signature.discovery` (ISO 32000-2): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `pdf.byterange` (ISO 32000-2): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `cms.signeddata` (RFC 5652): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `cms.signer_resolution` (RFC 5652): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `cms.signed_attrs` (RFC 5652 / ETSI EN 319 122-1): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `alg.rsa_pkcs1v15` (RFC 8017): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `alg.rsa_pss` (RFC 8017): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `alg.ecdsa` (RFC 5480 / FIPS 186 posture): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `pkix.path_build` (RFC 5280): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `pkix.path_validate` (RFC 5280): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `retrieval.shared` (RFC 5280 / RFC 6960): `implemented` in `crates/engine/src/signature_evidence.rs`; evidence `prompt24-signature-evidence-network-after-clippy-fixes-4gib`.
- `aia.ca_issuers` (RFC 5280): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `ocsp.request_response` (RFC 6960): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `ocsp.authorization` (RFC 6960): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `ocsp.freshness_nonce` (RFC 6960 / RFC 5019): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `crl.base_delta_indirect` (RFC 5280): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `revocation.policy` (RFC 5280 / RFC 6960 / ETSI TS 119 102-1): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `pades.baseline_b` (ETSI EN 319 142-1): `implemented` in `crates/engine/src/signature.rs`; evidence `prompt24-signatures-after-secret-fixture-removal-final-4gib`.
- `pades.bt_lt_lta` (ETSI EN 319 142-1/-2): `deferred_to_prompt25` in `crates/engine/src/signature.rs`; evidence `classified only`.
- `docmdp.fieldmdp` (ISO 32000-2): `deferred_to_prompt25` in `crates/engine/src/signature.rs`; evidence `reported only`.
- `ldap.retrieval` (RFC 5280 URI forms): `unsupported_exact_algorithm` in `crates/engine/src/signature_evidence.rs`; evidence `HTTP/HTTPS implemented; LDAP rejected explicitly`.
