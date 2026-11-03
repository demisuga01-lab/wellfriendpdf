//! Ordered page-parallel maps for the conversion pipeline.
//!
//! The conversions (Markdown, JSON, DOCX, XLSX, EPUB) are a map over pages
//! followed by document-wide passes. These helpers run the per-page map on
//! the current rayon pool and hand the results back in input order, so the
//! document-wide passes and the writers see exactly what the serial loop
//! produced: the output is byte-identical whatever the thread count.
//!
//! Errors keep serial semantics: the error returned is the one the first
//! failing page (in input order) produced. Cancellation scopes are carried
//! into the rayon workers, which would otherwise lose the thread-local token
//! at the parallel boundary.
//!
//! The thread count is the size of the rayon pool the caller runs in. The CLI
//! sizes the global pool from `--threads`; library callers can install their
//! own pool with [`rayon::ThreadPool::install`].

use rayon::prelude::*;

use crate::error::Result;

/// Run `work` serially when the pool has one thread or there is at most one
/// item; there is nothing to gain from the fan-out then.
fn serial(len: usize) -> bool {
    len < 2 || rayon::current_num_threads() < 2
}

/// Map `items` in parallel, keeping input order. The first error in input
/// order is returned, as the equivalent serial loop with `?` would.
pub fn try_map_ordered<I, T, F>(items: &[I], work: F) -> Result<Vec<T>>
where
    I: Sync,
    T: Send,
    F: Fn(&I) -> Result<T> + Sync,
{
    if serial(items.len()) {
        return items.iter().map(&work).collect();
    }
    let cancel = crate::cancel::current_cancel_token();
    let results: Vec<Result<T>> = items
        .par_iter()
        .with_max_len(1)
        .map(|item| cancel.scope(|| work(item)))
        .collect();
    results.into_iter().collect()
}

/// Map `items` in parallel with an infallible function, keeping input order.
pub fn map_ordered<I, T, F>(items: &[I], work: F) -> Vec<T>
where
    I: Sync,
    T: Send,
    F: Fn(&I) -> T + Sync,
{
    if serial(items.len()) {
        return items.iter().map(&work).collect();
    }
    let cancel = crate::cancel::current_cancel_token();
    items
        .par_iter()
        .with_max_len(1)
        .map(|item| cancel.scope(|| work(item)))
        .collect()
}

/// [`map_ordered`] with a memory budget: an item is started only while the
/// weights of the items in flight (its own included) stay within `budget`.
/// An item heavier than the whole budget runs once nothing else is in
/// flight. Weights are an estimate of each item's peak memory (for pages,
/// their stored content size), so a few very large pages do not all expand
/// at once.
pub fn map_ordered_weighted<I, T, F, W>(items: &[I], weight: W, budget: u64, work: F) -> Vec<T>
where
    I: Sync,
    T: Send,
    F: Fn(&I) -> T + Sync,
    W: Fn(&I) -> u64 + Sync,
{
    if serial(items.len()) {
        return items.iter().map(&work).collect();
    }
    let gate = WeightGate::new(budget);
    map_ordered(items, |item| {
        let _permit = gate.acquire(weight(item));
        work(item)
    })
}

/// A counting gate over item weights.
struct WeightGate {
    in_flight: std::sync::Mutex<u64>,
    released: std::sync::Condvar,
    budget: u64,
}

struct WeightPermit<'a> {
    gate: &'a WeightGate,
    weight: u64,
}

impl WeightGate {
    fn new(budget: u64) -> Self {
        Self {
            in_flight: std::sync::Mutex::new(0),
            released: std::sync::Condvar::new(),
            budget: budget.max(1),
        }
    }

    fn acquire(&self, weight: u64) -> WeightPermit<'_> {
        let weight = weight.min(self.budget);
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        while *in_flight > 0 && *in_flight + weight > self.budget {
            in_flight = self
                .released
                .wait(in_flight)
                .unwrap_or_else(|e| e.into_inner());
        }
        *in_flight += weight;
        WeightPermit { gate: self, weight }
    }
}

impl Drop for WeightPermit<'_> {
    fn drop(&mut self) {
        let mut in_flight = self
            .gate
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *in_flight -= self.weight;
        drop(in_flight);
        self.gate.released.notify_all();
    }
}

/// Run `work(threads)` on a dedicated pool of `threads` workers, or on the
/// current pool when `threads` is `None`. `Some(0)` means every available
/// core.
pub fn with_threads<T: Send>(threads: Option<usize>, work: impl FnOnce() -> T + Send) -> Result<T> {
    let Some(requested) = threads else {
        return Ok(work());
    };
    let threads = if requested == 0 {
        available_threads()
    } else {
        requested
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|err| {
            crate::error::WellfriendError::ResourceLimit(format!(
                "cannot start {threads} worker thread(s): {err}"
            ))
        })?;
    Ok(pool.install(work))
}

/// Cores this process may use.
pub fn available_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::WellfriendError;

    #[test]
    fn map_keeps_input_order_on_any_pool_size() {
        let items: Vec<usize> = (0..257).collect();
        for threads in [1, 2, 3, 8] {
            let out = with_threads(Some(threads), || map_ordered(&items, |&i| i * 3)).unwrap();
            assert_eq!(out, items.iter().map(|i| i * 3).collect::<Vec<_>>());
        }
    }

    #[test]
    fn first_error_in_input_order_wins() {
        let items: Vec<usize> = (0..64).collect();
        for threads in [1, 4] {
            let err = with_threads(Some(threads), || {
                try_map_ordered(&items, |&i| {
                    if i == 7 || i == 40 {
                        Err(WellfriendError::MalformedPdf(format!("page {i}")))
                    } else {
                        Ok(i)
                    }
                })
            })
            .unwrap()
            .unwrap_err();
            assert!(err.to_string().contains("page 7"), "{err}");
        }
    }

    #[test]
    fn cancellation_reaches_the_workers() {
        let token = crate::cancel::CancelToken::new();
        token.cancel();
        let items: Vec<usize> = (0..16).collect();
        let result = with_threads(Some(4), || {
            token.scope(|| {
                try_map_ordered(&items, |_| {
                    crate::cancel::current_cancel_token().check("test")?;
                    Ok(())
                })
            })
        })
        .unwrap();
        assert!(matches!(result, Err(WellfriendError::Cancelled(_))));
    }

    #[test]
    fn weighted_map_keeps_order_and_respects_the_budget() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let items: Vec<u64> = (0..200).map(|i| [1, 5, 40, 2, 9][i % 5]).collect();
        let in_flight = AtomicU64::new(0);
        let peak = AtomicU64::new(0);
        let out = with_threads(Some(8), || {
            map_ordered_weighted(
                &items,
                |&w| w,
                16,
                |&w| {
                    let now = in_flight.fetch_add(w.min(16), Ordering::SeqCst) + w.min(16);
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_micros(200));
                    in_flight.fetch_sub(w.min(16), Ordering::SeqCst);
                    w * 2
                },
            )
        })
        .unwrap();
        assert_eq!(out, items.iter().map(|w| w * 2).collect::<Vec<_>>());
        assert!(
            peak.load(Ordering::SeqCst) <= 16,
            "{}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn zero_threads_means_every_core() {
        let threads = with_threads(Some(0), rayon::current_num_threads).unwrap();
        assert_eq!(threads, available_threads());
    }
}
