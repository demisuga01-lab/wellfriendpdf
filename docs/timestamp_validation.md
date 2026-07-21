# Timestamp Validation

Schema: `prompt25.tsa-dss-ltv-mdp-signature-edits.v1`

RFC 3161 signature timestamp validation parses the token CMS, decodes TSTInfo, checks messageImprint against exact SignerInfo.signature bytes, verifies token CMS signature, and reports duplicate/malformed tokens explicitly.
