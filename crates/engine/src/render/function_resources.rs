//! Per-use function accounting and explicit retained-cache ownership.
use super::{FunctionCache, PreparedFunction};
use crate::decode_scheduler::{DecodeMemoryBudget, DecodeMemoryToken};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionResources<'a> {
    pub memory: Option<&'a Arc<DecodeMemoryBudget>>,
    pub max_graph_bytes: usize,
    pub max_stream_bytes: usize,
    /// Render-owned cache override. None selects the standalone reader cache.
    pub cache: Option<&'a Mutex<FunctionCache>>,
}

impl Default for FunctionResources<'_> {
    fn default() -> Self {
        Self {
            memory: None,
            max_graph_bytes: 64 * 1024 * 1024,
            max_stream_bytes: 16 * 1024 * 1024,
            cache: None,
        }
    }
}

/// A live consumer owns its reservation even when the selected retained cache
/// evicts the graph. Cache ownership itself is separately byte bounded.
pub(crate) struct FunctionLease {
    pub(super) function: Arc<PreparedFunction>,
    pub(super) memory: Option<DecodeMemoryToken>,
}

impl std::ops::Deref for FunctionLease {
    type Target = PreparedFunction;
    fn deref(&self) -> &Self::Target {
        &self.function
    }
}

impl FunctionLease {
    pub(crate) fn graph(&self) -> &Arc<PreparedFunction> {
        &self.function
    }

    pub(super) fn into_parts(self) -> (Arc<PreparedFunction>, Option<DecodeMemoryToken>) {
        (self.function, self.memory)
    }
}

#[cfg(test)]
#[path = "function_resources_tests.rs"]
mod tests;
