use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

/// Stable key for decoded stream cache entries.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DecodeCacheKey {
    pub object: Option<(u32, u16)>,
    pub stream_identity: String,
    pub filter_chain_hash: u64,
    pub decode_mode: String,
}

impl DecodeCacheKey {
    pub fn new(
        object: Option<(u32, u16)>,
        stream_identity: impl Into<String>,
        filter_chain_hash: u64,
        decode_mode: impl Into<String>,
    ) -> Self {
        Self {
            object,
            stream_identity: stream_identity.into(),
            filter_chain_hash,
            decode_mode: decode_mode.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DecodeCacheMetrics {
    pub budget_bytes: usize,
    pub max_entry_bytes: usize,
    pub current_bytes: usize,
    pub entries: usize,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub skipped_oversized: usize,
}

/// Per-document decoded stream LRU cache with exact byte accounting.
///
/// The cache stores owned decoded bytes only. It does not cache borrowed source
/// spans, failed decode output, or entries larger than the configured per-entry
/// limit. It is intentionally a small utility rather than a global cache so
/// callers can place it behind a document/session boundary.
#[derive(Clone, Debug)]
pub struct DecodeCache {
    entries: HashMap<DecodeCacheKey, Vec<u8>>,
    order: VecDeque<DecodeCacheKey>,
    budget_bytes: usize,
    max_entry_bytes: usize,
    current_bytes: usize,
    hits: usize,
    misses: usize,
    evictions: usize,
    skipped_oversized: usize,
}

impl DecodeCache {
    pub fn new(budget_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            budget_bytes,
            max_entry_bytes,
            current_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            skipped_oversized: 0,
        }
    }

    pub fn disabled() -> Self {
        Self::new(0, 0)
    }

    pub fn get(&mut self, key: &DecodeCacheKey) -> Option<Vec<u8>> {
        let Some(value) = self.entries.get(key).cloned() else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        self.touch(key);
        Some(value)
    }

    pub fn insert(&mut self, key: DecodeCacheKey, data: Vec<u8>) -> bool {
        if self.budget_bytes == 0 || self.max_entry_bytes == 0 || data.len() > self.max_entry_bytes
        {
            self.skipped_oversized += 1;
            return false;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(previous.len());
            self.order.retain(|candidate| candidate != &key);
        }

        while self.current_bytes + data.len() > self.budget_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.current_bytes = self.current_bytes.saturating_sub(old.len());
                self.evictions += 1;
            }
        }

        if data.len() > self.budget_bytes {
            self.skipped_oversized += 1;
            return false;
        }

        self.current_bytes += data.len();
        self.order.push_back(key.clone());
        self.entries.insert(key, data);
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.current_bytes = 0;
    }

    pub fn metrics(&self) -> DecodeCacheMetrics {
        DecodeCacheMetrics {
            budget_bytes: self.budget_bytes,
            max_entry_bytes: self.max_entry_bytes,
            current_bytes: self.current_bytes,
            entries: self.entries.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            skipped_oversized: self.skipped_oversized,
        }
    }

    fn touch(&mut self, key: &DecodeCacheKey) {
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str) -> DecodeCacheKey {
        DecodeCacheKey::new(Some((1, 0)), id, 42, "strict")
    }

    #[test]
    fn repeated_decode_hits_cache() {
        let mut cache = DecodeCache::new(16, 16);
        assert!(cache.insert(key("a"), b"abc".to_vec()));
        assert_eq!(cache.get(&key("a")).unwrap(), b"abc");
        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.current_bytes, 3);
    }

    #[test]
    fn large_stream_is_not_cached() {
        let mut cache = DecodeCache::new(16, 4);
        assert!(!cache.insert(key("large"), b"12345".to_vec()));
        assert!(cache.get(&key("large")).is_none());
        assert_eq!(cache.metrics().skipped_oversized, 1);
    }

    #[test]
    fn eviction_keeps_budget() {
        let mut cache = DecodeCache::new(6, 6);
        assert!(cache.insert(key("a"), b"aaa".to_vec()));
        assert!(cache.insert(key("b"), b"bbb".to_vec()));
        assert!(cache.insert(key("c"), b"ccc".to_vec()));
        let metrics = cache.metrics();
        assert!(metrics.current_bytes <= 6);
        assert_eq!(metrics.evictions, 1);
        assert!(cache.get(&key("a")).is_none());
    }

    #[test]
    fn disabled_cache_produces_no_entries() {
        let mut cache = DecodeCache::disabled();
        assert!(!cache.insert(key("a"), b"abc".to_vec()));
        assert!(cache.get(&key("a")).is_none());
        assert_eq!(cache.metrics().entries, 0);
    }
}
