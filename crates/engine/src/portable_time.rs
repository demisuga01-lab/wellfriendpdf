//! A monotonic clock that also works on `wasm32-unknown-unknown`, where
//! `std::time::Instant::now()` panics ("time not implemented on this
//! platform"). There, time does not advance: an instant's elapsed time is
//! zero, so time-to-live caches keep entries until their size bound evicts
//! them and timings report zero.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant;

#[cfg(target_arch = "wasm32")]
impl Instant {
    pub fn now() -> Self {
        Instant
    }

    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    pub fn duration_since(&self, _earlier: Instant) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}
