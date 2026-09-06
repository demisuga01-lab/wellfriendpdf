//! Cooperative cancellation for CPU-bound engine work.
//!
//! Rust has no safe thread cancellation: a CPU-bound loop cannot be killed
//! from outside, it can only *check* whether it should stop and bail. This
//! module provides the shared flag those loops poll.
//!
//! The server (or any caller) creates a [`CancelToken`], hands a clone to the
//! engine call, and arms a timer thread/task that calls [`CancelToken::cancel`]
//! when a deadline elapses. The engine's hot loops call
//! [`CancelToken::check`] every N iterations; once the flag is set, `check`
//! returns `Err(WellfriendError::Cancelled)` which propagates up and frees the
//! worker thread promptly.
//!
//! The flag is a single `Arc<AtomicBool>`, so when N pages render in parallel
//! across rayon workers they all observe the same cancellation and stop
//! together.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::{Result, WellfriendError};

/// A shared, cheap-to-poll cancellation flag.
///
/// Cloning is a pointer bump (the inner `Arc` is shared). A default token is
/// never cancelled, so engine entry points that don't need cancellation can
/// pass `CancelToken::none()` with zero overhead beyond an atomic load.
#[derive(Debug)]
struct CancelState {
    flag: AtomicBool,
    linked: Vec<CancelToken>,
}

#[derive(Clone, Debug)]
pub struct CancelToken {
    state: Arc<CancelState>,
}

impl CancelToken {
    /// Create a fresh, un-cancelled token.
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancelState {
                flag: AtomicBool::new(false),
                linked: Vec::new(),
            }),
        }
    }

    /// Create a token that observes cancellation from either parent.
    pub(crate) fn linked_pair(first: &Self, second: &Self) -> Self {
        Self {
            state: Arc::new(CancelState {
                flag: AtomicBool::new(false),
                linked: vec![first.clone(), second.clone()],
            }),
        }
    }

    /// A token that is never cancelled. Use for callers that don't impose a
    /// deadline (CLI, tests, internal calls).
    pub fn none() -> Self {
        Self::new()
    }

    /// Signal cancellation. Idempotent; safe to call from a timer thread while
    /// engine loops are polling.
    pub fn cancel(&self) {
        // Relaxed is sufficient: we only need eventual visibility of a single
        // boolean flip, not ordering relative to other memory. The polling
        // loops re-load it regularly.
        self.state.flag.store(true, Ordering::Relaxed);
    }

    /// Clear a prior cancellation request on this token.
    ///
    /// Progressive sessions use this only after the render step that observed
    /// cancellation has released its mutable job borrow. Existing clones then
    /// remain the current session cancellation source instead of becoming stale.
    pub(crate) fn reset(&self) {
        self.state.flag.store(false, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.state.flag.load(Ordering::Relaxed)
            || self.state.linked.iter().any(CancelToken::is_cancelled)
    }

    /// Return `Err(Cancelled)` if cancellation has been requested, else `Ok`.
    ///
    /// `context` names the loop that observed the cancellation so logs/errors
    /// can point at where the work was stopped.
    #[inline]
    pub fn check(&self, context: &str) -> Result<()> {
        if self.is_cancelled() {
            Err(WellfriendError::Cancelled(context.to_string()))
        } else {
            Ok(())
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_token_is_not_cancelled() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        assert!(t.check("x").is_ok());
    }

    #[test]
    fn cancel_is_observed() {
        let t = CancelToken::new();
        t.cancel();
        assert!(t.is_cancelled());
        let err = t.check("render-loop").unwrap_err();
        assert!(matches!(err, WellfriendError::Cancelled(ctx) if ctx == "render-loop"));
    }

    #[test]
    fn clones_share_one_flag() {
        let a = CancelToken::new();
        let b = a.clone();
        a.cancel();
        assert!(b.is_cancelled(), "clone must observe cancellation");
    }

    #[test]
    fn linked_pair_observes_either_parent() {
        let external = CancelToken::new();
        let session = CancelToken::new();
        let linked = CancelToken::linked_pair(&external, &session);
        assert!(!linked.is_cancelled());

        external.cancel();
        assert!(linked.is_cancelled(), "external parent cancellation");

        let external = CancelToken::new();
        let session = CancelToken::new();
        let linked = CancelToken::linked_pair(&external, &session);
        session.cancel();
        assert!(linked.is_cancelled(), "session parent cancellation");
    }

    #[test]
    fn reset_refreshes_existing_clones() {
        let token = CancelToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
        token.reset();
        assert!(!clone.is_cancelled());
    }
}
