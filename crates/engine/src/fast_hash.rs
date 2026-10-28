//! A fast hasher for maps keyed by small integers such as `(object,
//! generation)` pairs, where SipHash dominates xref and identity-map
//! building. The state starts from a per-process random seed and `finish`
//! applies a full 64-bit avalanche, so object numbers chosen by a hostile
//! file cannot be precomputed into colliding buckets.

use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;

const MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

fn process_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        let mut bytes = [0u8; 8];
        if getrandom::getrandom(&mut bytes).is_err() {
            // Fall back to the standard library's randomized SipHash keys.
            return std::collections::hash_map::RandomState::new().hash_one(0x5EED_u64);
        }
        u64::from_le_bytes(bytes)
    })
}

#[derive(Clone, Copy)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    #[inline]
    fn mix(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(26) ^ value).wrapping_mul(MULTIPLIER);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        // MurmurHash3 fmix64.
        let mut hash = self.0;
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        hash ^ (hash >> 33)
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.mix(u64::from_le_bytes(word));
        }
        self.mix(bytes.len() as u64);
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.mix(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.mix(value as u64);
    }
}

/// [`BuildHasher`] for [`FastHasher`], seeded once per process.
#[derive(Clone, Copy)]
pub(crate) struct FastHashBuilder(u64);

impl Default for FastHashBuilder {
    fn default() -> Self {
        Self(process_seed())
    }
}

impl BuildHasher for FastHashBuilder {
    type Hasher = FastHasher;

    #[inline]
    fn build_hasher(&self) -> FastHasher {
        FastHasher(self.0)
    }
}

/// Map keyed by `(object number, generation)`.
pub(crate) type ObjectMap<V> = std::collections::HashMap<(u32, u16), V, FastHashBuilder>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_map_behaves_like_a_hash_map() {
        let mut map = ObjectMap::default();
        for number in 0..10_000u32 {
            map.insert((number, (number % 3) as u16), number);
        }
        assert_eq!(map.len(), 10_000);
        assert_eq!(map.get(&(4242, 0)), Some(&4242));
        assert_eq!(map.get(&(4242, 1)), None);
        let builder = FastHashBuilder::default();
        assert_ne!(
            builder.hash_one((1u32, 0u16)),
            builder.hash_one((0u32, 1u16))
        );
    }
}
