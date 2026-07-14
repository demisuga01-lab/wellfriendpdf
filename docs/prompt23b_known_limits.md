# Prompt 23B Known Limits

These are exact supported-boundary limits after the final Prompt 23B closure
pass. They are not hidden blockers for Prompt 23B.

- RecipientInfo classes outside the supported `/Adobe.PubSec`
  `/adbe.pkcs7.s5` KeyTrans profile are classified in
  `pubsec-recipient-profile-matrix-prompt23b.json` as not applicable or exact
  unsupported profiles rather than silently downgraded.
- RSAES-OAEP non-empty labels and unsupported hash/MGF/key-wrap combinations
  return precise unsupported-algorithm diagnostics.
- OS certificate store and HSM/PKCS#11 support are provider extension hooks,
  not bundled portable providers.
- WASM does not assume host filesystem, OS keystore, or unconstrained private-key
  lifecycle. Unsafe private-key/PFX workflows are reported as constrained or
  unsupported.
- PKCS #12/PFX provider loading is limited to bounded, unambiguous RSA
  certificate/private-key identities; ambiguous bundles and unsupported bag or
  key algorithms fail closed.
- External PDF ecosystem support for ISO/TS 32003 AESV4 and ISO/TS 32004
  PDF-MAC was not available from qpdf 12.3.2 on this host. That unsupported
  state is recorded and not counted as a pass.
- Certificate-chain trust validation, PAdES, OCSP/CRL, TSA/DSS/LTV validation,
  and signature-preserving edit semantics are deferred to later prompts.
