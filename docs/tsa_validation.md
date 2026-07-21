# Tsa Validation

Schema: `prompt25.tsa-dss-ltv-mdp-signature-edits.v1`

TSA validation resolves one TSA signer certificate, requires id-kp-timeStamping EKU, validates key usage where present, builds a path through the Prompt 24B PKIX engine at genTime, and applies revocation policy without treating missing evidence as good.
