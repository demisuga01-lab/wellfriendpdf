# Prompt 27 crypto and standards release verdict

The release verdict is generated at:

- `target/prompt27-verapdf-crypto-fuzz/crypto-standards-release-verdict.json`

The verdict can only be release-grade when:

- Prompt 23B, 24B, 25B, and 26 focused gates pass or are rerun with exact scoped
  equivalents.
- No plaintext, private-key, TSA/OCSP/CRL credential, signer callback secret, or token
  appears in source, logs, docs, or kept evidence.
- Network retrieval remains policy-controlled and fail-closed.
- Evidence bundles are treated as untrusted until validated.
- External signer callbacks cannot bypass certificate pinning or algorithm checks.
- WASM host filesystem, unrestricted network, OS trust-store, and external signer limits
  remain exact unsupported statuses.
- Public package names and messages use Wellfriend PDF SDK / `wellfriendpdf`.

If any security failure remains unclassified, Prompt 27 is not complete.
