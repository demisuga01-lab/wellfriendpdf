# Incremental signing

`sign_incremental` creates an append-only revision. It plans a `/Contents` capacity and
`/ByteRange`, writes the signature dictionary without changing the original prefix, obtains a
CMS value, patches the CMS boundary, reopens the result, and validates the resulting signature.

Supported intents are approval signing and certification signing. Certification emits the
catalog `/Perms` `/DocMDP` reference; the existing permission engine recognises the created
policy. `SignaturePlaceholderPlan` reports reserved and required capacity before a write.

If a CMS is too large, the configured retry grows the placeholder within its cap. A final
capacity failure returns an error and no success report. Results include prefix-preservation,
ByteRange, structural-open, cryptographic post-sign, and timestamp status fields. Older
`sign_document` remains available and is separately exercised by the engine suite.
