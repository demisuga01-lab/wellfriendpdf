use std::cell::RefCell;
use std::sync::Arc;

use sha2::{Digest, Sha256};

struct InputIdentity {
    address: usize,
    length: usize,
    comparable_bytes: Option<Arc<[u8]>>,
    sha256: Option<String>,
    document_id: Option<String>,
    revision_id: Option<String>,
    engine: Option<crate::ContentEngine>,
}

const MAX_COMPARABLE_INPUT_BYTES: usize = 64 * 1024 * 1024;

thread_local! {
    static INPUT_IDENTITIES: RefCell<Vec<InputIdentity>> = const { RefCell::new(Vec::new()) };
}

struct InputIdentityGuard;

impl Drop for InputIdentityGuard {
    fn drop(&mut self) {
        INPUT_IDENTITIES.with(|identities| {
            identities.borrow_mut().pop();
        });
    }
}

fn stable_id(kind: &str, values: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    for value in values {
        digest.update(value);
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("{kind}-{}", &encoded[..24])
}

fn compute(bytes: &[u8]) -> InputIdentity {
    let length = bytes.len();
    InputIdentity {
        address: bytes.as_ptr() as usize,
        length,
        comparable_bytes: (length <= MAX_COMPARABLE_INPUT_BYTES).then(|| Arc::<[u8]>::from(bytes)),
        // A scope can need only one of these values. Computing all three
        // eagerly made a request for one revision identity hash the complete
        // PDF three times, and unscoped output checks repeated that work.
        // Preserve the exact existing byte-derived contracts, but materialize
        // each digest only when it is actually requested.
        sha256: None,
        document_id: None,
        revision_id: None,
        engine: None,
    }
}

fn matches(identity: &InputIdentity, bytes: &[u8]) -> bool {
    if identity.length != bytes.len() {
        return false;
    }
    identity.address == bytes.as_ptr() as usize
        || identity
            .comparable_bytes
            .as_deref()
            .is_some_and(|retained| retained == bytes)
}

fn current_position(bytes: &[u8]) -> Option<usize> {
    let address = bytes.as_ptr() as usize;
    let length = bytes.len();
    INPUT_IDENTITIES.with(|identities| {
        identities.borrow().iter().rposition(|identity| {
            identity.length == length
                && (identity.address == address
                    || identity
                        .comparable_bytes
                        .as_deref()
                        .is_some_and(|retained| retained == bytes))
        })
    })
}

/// Scope exact immutable-input identities across nested parser/editing layers.
///
/// PDF planning and apply historically recomputed the same SHA-256-derived
/// document and revision identities in each subsystem. A live immutable slice
/// is an exact process-local identity key: its allocation cannot be reused
/// while this scope holds the borrow. The digest remains content-derived and
/// byte-for-byte compatible with the public IDs; pointer identity only avoids
/// recomputing it within this synchronous call tree.
pub(crate) fn with_input_identity<T>(bytes: &[u8], operation: impl FnOnce() -> T) -> T {
    if current_position(bytes).is_some() {
        return operation();
    }
    INPUT_IDENTITIES.with(|identities| identities.borrow_mut().push(compute(bytes)));
    let _guard = InputIdentityGuard;
    operation()
}

pub(crate) fn document_id(bytes: &[u8]) -> String {
    INPUT_IDENTITIES
        .with(|identities| {
            let mut identities = identities.borrow_mut();
            let position = identities
                .iter()
                .rposition(|identity| matches(identity, bytes))?;
            if identities[position].document_id.is_none() {
                identities[position].document_id = Some(stable_id("document", &[bytes]));
            }
            identities[position].document_id.clone()
        })
        .unwrap_or_else(|| stable_id("document", &[bytes]))
}

pub(crate) fn revision_id(bytes: &[u8]) -> String {
    INPUT_IDENTITIES
        .with(|identities| {
            let mut identities = identities.borrow_mut();
            let position = identities
                .iter()
                .rposition(|identity| matches(identity, bytes))?;
            if identities[position].revision_id.is_none() {
                let length_bytes = bytes.len().to_le_bytes();
                identities[position].revision_id =
                    Some(stable_id("revision", &[bytes, &length_bytes]));
            }
            identities[position].revision_id.clone()
        })
        .unwrap_or_else(|| {
            let length_bytes = bytes.len().to_le_bytes();
            stable_id("revision", &[bytes, &length_bytes])
        })
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    INPUT_IDENTITIES
        .with(|identities| {
            let mut identities = identities.borrow_mut();
            let position = identities
                .iter()
                .rposition(|identity| matches(identity, bytes))?;
            if identities[position].sha256.is_none() {
                identities[position].sha256 = Some(format!("{:x}", Sha256::digest(bytes)));
            }
            identities[position].sha256.clone()
        })
        .unwrap_or_else(|| format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn scoped_engine(bytes: &[u8]) -> Option<(usize, Option<crate::ContentEngine>)> {
    INPUT_IDENTITIES.with(|identities| {
        let identities = identities.borrow();
        identities
            .iter()
            .rposition(|identity| matches(identity, bytes))
            .map(|position| (position, identities[position].engine.clone()))
    })
}

pub(crate) fn retain_engine(position: usize, engine: &crate::ContentEngine) {
    INPUT_IDENTITIES.with(|identities| {
        if let Some(identity) = identities.borrow_mut().get_mut(position) {
            identity.engine = Some(engine.clone());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_identity_matches_the_existing_content_derived_contract() {
        let bytes = b"immutable PDF bytes";
        let expected_document = stable_id("document", &[bytes]);
        let expected_revision = stable_id("revision", &[bytes, &bytes.len().to_le_bytes()]);
        with_input_identity(bytes, || {
            assert_eq!(document_id(bytes), expected_document);
            assert_eq!(revision_id(bytes), expected_revision);
            assert_eq!(sha256(bytes), format!("{:x}", Sha256::digest(bytes)));
            with_input_identity(bytes, || {
                assert_eq!(document_id(bytes), expected_document);
                assert_eq!(revision_id(bytes), expected_revision);
            });
        });
    }

    #[test]
    fn nested_distinct_inputs_restore_the_outer_identity() {
        let outer = b"outer";
        let inner = b"inner";
        with_input_identity(outer, || {
            let outer_revision = revision_id(outer);
            with_input_identity(inner, || {
                assert_ne!(revision_id(inner), outer_revision);
            });
            assert_eq!(revision_id(outer), outer_revision);
        });
    }
}
