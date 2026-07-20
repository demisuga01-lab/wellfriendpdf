# CMS Signer Resolution

SignerInfo resolution must produce exactly one certificate. Zero matches and multiple matches are distinct failures, and the validator never falls back to the first certificate in SignedData.
