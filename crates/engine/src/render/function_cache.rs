//! Scoped retained function graphs. Standalone callers use the reader's cache;
//! render callers provide a worker-owned cache with their own retention policy.
//! Exact bounded keys are valid only in the bound immutable reader namespace.
//! Large keys bypass caching without rejecting otherwise valid functions.
use super::{FunctionLease, FunctionResources, PreparedFunction};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

/// Counters describe retained function entries, not total renderer/process RSS.
/// Hits count graph lookups, including hits rejected by a stricter live-use
/// policy. Large-key bypasses and poisoned-cache bypasses are not lookups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FunctionCacheMetrics {
    pub available: bool,
    pub hits: usize,
    pub misses: usize,
    pub admissions: usize,
    pub evictions: usize,
    pub skipped_oversized: usize,
    pub reader_rebinds: usize,
    pub entries: usize,
    pub bytes: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}

const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 256 * 1024;
const MAX_KEY_VISITS: usize = 4096;
const MAX_KEY_DEPTH: usize = 64;

struct Entry {
    key: Vec<u8>,
    function: Arc<PreparedFunction>,
    bytes: usize,
}

pub(crate) struct FunctionCache {
    entries: VecDeque<Entry>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    /// Weak allocation identity prevents ABA reuse without retaining a reader
    /// or its standalone cache data. Object IDs never cross reader namespaces.
    reader: Option<Weak<Mutex<FunctionCache>>>,
    metrics: FunctionCacheMetrics,
}

impl std::fmt::Debug for FunctionCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionCache")
            .field("metrics", &self.metrics())
            .finish()
    }
}

impl Default for FunctionCache {
    fn default() -> Self {
        Self::new(MAX_ENTRIES, MAX_BYTES)
    }
}

impl FunctionCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            max_entries,
            max_bytes,
            reader: None,
            metrics: FunctionCacheMetrics::default(),
        }
    }

    pub(crate) fn metrics(&self) -> FunctionCacheMetrics {
        FunctionCacheMetrics {
            available: true,
            entries: self.entries.len(),
            bytes: self.bytes,
            max_entries: self.max_entries,
            max_bytes: self.max_bytes,
            ..self.metrics
        }
    }

    fn bind_reader(&mut self, reader: &PdfReader) {
        let identity = Arc::downgrade(&reader.function_cache);
        if self
            .reader
            .as_ref()
            .is_some_and(|old| Weak::ptr_eq(old, &identity))
        {
            return;
        }
        if self.reader.is_some() {
            self.metrics.reader_rebinds = self.metrics.reader_rebinds.saturating_add(1);
            while self.evict_one() {}
        }
        self.reader = Some(identity);
    }

    pub(crate) fn set_byte_limit(&mut self, bytes: usize) {
        self.max_bytes = bytes.min(MAX_BYTES);
        while self.bytes > self.max_bytes {
            if !self.evict_one() {
                break;
            }
        }
    }

    pub(crate) fn oldest_entry_bytes(&self) -> Option<usize> {
        self.entries.front().map(|entry| entry.bytes)
    }

    pub(crate) fn evict_one(&mut self) -> bool {
        let Some(entry) = self.entries.pop_front() else {
            return false;
        };
        self.bytes -= entry.bytes;
        self.metrics.evictions = self.metrics.evictions.saturating_add(1);
        true
    }

    fn get(&mut self, key: &[u8]) -> Option<Arc<PreparedFunction>> {
        let result = self.find(key);
        if result.is_some() {
            self.metrics.hits = self.metrics.hits.saturating_add(1);
        } else {
            self.metrics.misses = self.metrics.misses.saturating_add(1);
        }
        result
    }

    fn find(&mut self, key: &[u8]) -> Option<Arc<PreparedFunction>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.key.as_slice() == key)?;
        let entry = self.entries.remove(index)?;
        let function = Arc::clone(&entry.function);
        self.entries.push_back(entry);
        Some(function)
    }

    fn insert(&mut self, key: Vec<u8>, function: Arc<PreparedFunction>) -> Arc<PreparedFunction> {
        // Concurrent misses may prepare outside the lock. Keep one canonical
        // identity after admission; no decoder or evaluator runs under the lock.
        if let Some(existing) = self.find(&key) {
            return existing;
        }
        let bytes = function
            .retained_bytes()
            .checked_add(key.capacity())
            .and_then(|value| value.checked_add(std::mem::size_of::<Entry>()));
        let Some(bytes) = bytes.filter(|&bytes| bytes <= self.max_bytes) else {
            self.metrics.skipped_oversized = self.metrics.skipped_oversized.saturating_add(1);
            return function;
        };
        if self.max_entries == 0 {
            self.metrics.skipped_oversized = self.metrics.skipped_oversized.saturating_add(1);
            return function;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            if !self.evict_one() {
                break;
            }
        }
        self.bytes += bytes;
        self.entries.push_back(Entry {
            key,
            function: Arc::clone(&function),
            bytes,
        });
        self.metrics.admissions = self.metrics.admissions.saturating_add(1);
        function
    }
}

#[allow(dead_code)] // Compatibility entry point; runtime calls the resource-aware cache.
pub(super) fn prepare(
    object: &PdfObject,
    inputs: usize,
    allow_array: bool,
    reader: &PdfReader,
) -> Option<Arc<PreparedFunction>> {
    prepare_with_resources(
        object,
        inputs,
        allow_array,
        reader,
        FunctionResources::default(),
    )
    .map(|lease| lease.into_parts().0)
}

pub(super) fn prepare_with_resources(
    object: &PdfObject,
    inputs: usize,
    allow_array: bool,
    reader: &PdfReader,
    resources: FunctionResources<'_>,
) -> Option<FunctionLease> {
    crate::cancel::check_current_cancel("PDF retained function lookup").ok()?;
    if inputs == 0 {
        return None;
    }
    let key = key(object, inputs, allow_array);
    let cache_owner = resources.cache.unwrap_or(reader.function_cache.as_ref());
    let cached = key.as_ref().and_then(|key| {
        cache_owner.lock().ok().and_then(|mut cache| {
            cache.bind_reader(reader);
            cache.get(key)
        })
    });
    if let Some(function) = cached {
        if !function.accepts_resources(resources) {
            return None;
        }
        // A hit bypasses decoding, not the current consumer's live-memory
        // allowance. Reserve outside the cache lock to avoid lock inversion.
        let memory = resources
            .memory
            .map(|budget| budget.try_acquire(function.retained_bytes() as u64))
            .transpose()
            .ok()?;
        return Some(FunctionLease { function, memory });
    }
    let (function, mut memory) =
        PreparedFunction::prepare_with_resources(object, inputs, allow_array, reader, resources)?;
    let mut function = Arc::new(function);
    crate::cancel::check_current_cancel("PDF retained function admission").ok()?;
    if let Some(key) = key {
        // A poisoned optional cache is bypassed, not a reason to return invalid
        // pixels or panic. The freshly prepared graph remains fully validated.
        if let Ok(mut cache) = cache_owner.lock() {
            // Another caller may have used this cache with another reader
            // while we prepared outside the lock. Rebind before admission too.
            cache.bind_reader(reader);
            function = cache.insert(key, function);
        }
    }
    // A racing preparation may have admitted a canonical graph with a different
    // allocation capacity. Retarget the lease without dropping its reservation.
    if !function.accepts_resources(resources) {
        return None;
    }
    if let Some(memory) = &mut memory {
        memory.resize(function.retained_bytes() as u64).ok()?;
    }
    Some(FunctionLease { function, memory })
}

fn key(object: &PdfObject, inputs: usize, allow_array: bool) -> Option<Vec<u8>> {
    let mut encoder = KeyEncoder {
        bytes: Vec::new(),
        visits: MAX_KEY_VISITS,
    };
    encoder.write(&[u8::from(allow_array)])?;
    encoder.length(inputs)?;
    encoder.object(object, 0)?;
    Some(encoder.bytes)
}

struct KeyEncoder {
    bytes: Vec<u8>,
    visits: usize,
}

impl KeyEncoder {
    fn write(&mut self, bytes: &[u8]) -> Option<()> {
        let size = self.bytes.len().checked_add(bytes.len())?;
        if size > MAX_KEY_BYTES {
            return None;
        }
        crate::cancel::check_current_cancel("PDF function cache key").ok()?;
        self.bytes.extend_from_slice(bytes);
        Some(())
    }

    fn length(&mut self, size: usize) -> Option<()> {
        self.write(&u64::try_from(size).ok()?.to_le_bytes())
    }

    fn blob(&mut self, bytes: &[u8]) -> Option<()> {
        self.length(bytes.len())?;
        self.write(bytes)
    }

    fn dictionary(&mut self, dict: &PdfDictionary, depth: usize) -> Option<()> {
        self.length(dict.len())?;
        for (name, value) in dict.entries() {
            self.blob(name.as_bytes())?;
            self.object(value, depth + 1)?;
        }
        Some(())
    }

    fn object(&mut self, object: &PdfObject, depth: usize) -> Option<()> {
        if depth > MAX_KEY_DEPTH {
            return None;
        }
        self.visits = self.visits.checked_sub(1)?;
        match object {
            PdfObject::Null => self.write(&[0]),
            PdfObject::Boolean(value) => self.write(&[1, u8::from(*value)]),
            PdfObject::Integer(value) => {
                self.write(&[2])?;
                self.write(&value.to_le_bytes())
            }
            PdfObject::Real(value) => {
                self.write(&[3])?;
                self.write(&value.to_bits().to_le_bytes())
            }
            PdfObject::String(value) => {
                self.write(&[4])?;
                self.blob(value)
            }
            PdfObject::Name(value) => {
                self.write(&[5])?;
                self.blob(value.as_bytes())
            }
            PdfObject::Array(items) => {
                self.write(&[6])?;
                self.length(items.len())?;
                for item in items {
                    self.object(item, depth + 1)?;
                }
                Some(())
            }
            PdfObject::Dictionary(dict) => {
                self.write(&[7])?;
                self.dictionary(dict, depth)
            }
            PdfObject::Stream { dict, raw } => {
                self.write(&[8])?;
                self.dictionary(dict, depth)?;
                self.blob(raw)
            }
            PdfObject::Reference { number, generation } => {
                self.write(&[9])?;
                self.write(&number.to_le_bytes())?;
                self.write(&generation.to_le_bytes())
            }
        }
    }
}

#[cfg(test)]
#[path = "function_cache_tests.rs"]
mod tests;
