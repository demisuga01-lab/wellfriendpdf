# Tsa Validation

Schema: `pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1`

TSA validation resolves one TSA signer certificate, requires id-kp-timeStamping EKU, validates key usage where present, builds a path through the Signature Validation Resume PKIX engine at genTime, and applies revocation policy without treating missing evidence as good.
