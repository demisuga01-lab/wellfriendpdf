# Signature preserving edits

Schema: `prompt25.tsa-dss-ltv-mdp-signature-edits.v1`

Supported signature-preserving form-fill edits are planned, written as append-only incremental
updates, reopened, and revalidated. Prefix preservation is byte-for-byte; invalid fixture
signatures are not promoted.

Prompt 26 uses the same append-only invariant for signing: existing bytes are preserved, only a
new revision is written, and post-sign validation records exact ByteRange/prefix/reopen state.
Unknown post-sign changes fail closed; DocMDP and FieldMDP remain the authority for whether a
proposed change can preserve a signature.
