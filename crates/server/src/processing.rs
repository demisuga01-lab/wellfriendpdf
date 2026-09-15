//! Per-request resource safety: cooperative processing timeout and limit checks.
//!
//! ## Why cooperative cancellation, not a tower timeout
//!
//! The heavy work (render / extract) runs CPU-bound inside
//! `tokio::task::spawn_blocking` + rayon. A `tower::TimeoutLayer` only times
//! out the *async future* — it returns an error to the client but the blocking
//! thread keeps running, still pegging a CPU core. That leaks workers under
//! attack, which is exactly the DoS we're defending against.
//!
//! Instead we hand the engine a [`CancelToken`] and enforce one absolute
//! deadline across queueing and execution. The engine polls the flag in its hot
//! loops and bails out. Non-cooperative work keeps a bounded semaphore permit
//! until it really exits, preventing timed-out requests from flooding the
//! blocking pool.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::Semaphore;
use wellfriendpdf_engine::CancelToken;

use crate::config::ServerConfig;
use crate::error::ServerError;

struct CancelOnDrop(CancelToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Live progress counters for a running job. The worker hands a shared handle
/// to the processing core, which bumps `done` as each page/image completes; the
/// status endpoint reads these to report `pages_done`/`pages_total`. Atomics so
/// the parallel render loop can update without locking.
#[derive(Debug, Default)]
pub struct JobProgress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

impl JobProgress {
    pub fn set_total(&self, total: usize) {
        self.total.store(total, Ordering::Relaxed);
    }
    pub fn inc(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> (usize, usize) {
        (
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

/// A fully-processed result ready to be returned to a client (sync path) or
/// written to a job's result file (async path). Both the synchronous handlers
/// and the background job worker produce this via the SAME core processing
/// functions, guaranteeing byte-identical output regardless of path.
pub struct ProcessedOutput {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub filename: &'static str,
    /// Extra response headers (e.g. x-page-count). Stored alongside a job's
    /// result so the result endpoint can replay them.
    pub extra_headers: Vec<(&'static str, String)>,
}

/// Run CPU-bound `work` on the blocking pool under a cooperative deadline of
/// `timeout_secs` seconds (0 disables). Shared core for both the per-request
/// timeout and the larger per-job timeout.
///
/// `work` receives a [`CancelToken`] it must thread into the engine call so the
/// engine's hot loops can observe cancellation. Queueing for the bounded
/// blocking-work permit consumes the same deadline as execution.
pub async fn run_with_deadline_secs<F, T>(timeout_secs: u64, work: F) -> Result<T, ServerError>
where
    F: FnOnce(CancelToken) -> T + Send + 'static,
    T: Send + 'static,
{
    let timeout = (timeout_secs > 0).then(|| Duration::from_secs(timeout_secs));
    let deadline = timeout.map(|duration| tokio::time::Instant::now() + duration);
    let token = CancelToken::new();
    let semaphore = blocking_work_semaphore();
    let permit = if let Some(deadline) = deadline {
        tokio::time::timeout_at(deadline, semaphore.acquire_owned())
            .await
            .map_err(|_| ServerError::Timeout)?
    } else {
        semaphore.acquire_owned().await
    }
    .map_err(|_| ServerError::Internal("blocking work semaphore closed".to_string()))?;
    let work_token = token.clone();
    let mut join = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(work_token)
    });
    // Client disconnects and outer middleware cancellation drop this future.
    // Keep cancellation independent of the inline deadline future so detached
    // blocking work still receives the stop signal in that path.
    let _cancel_on_drop = CancelOnDrop(token.clone());

    let Some(deadline) = deadline else {
        return join.await.map_err(|join_err| {
            ServerError::Internal(format!("processing task failed: {join_err}"))
        });
    };

    tokio::select! {
        join_result = &mut join => {
            join_result.map_err(|join_err| {
                ServerError::Internal(format!("processing task failed: {join_err}"))
            })
        }
        _ = tokio::time::sleep_until(deadline) => {
            token.cancel();
            // Dropping the join handle detaches the task. Its permit remains
            // captured by the task and is released only when the work exits.
            Err(ServerError::Timeout)
        }
    }
}

fn blocking_work_semaphore() -> Arc<Semaphore> {
    static SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    Arc::clone(SEMAPHORE.get_or_init(|| {
        let permits = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(4)
            .max(2);
        Arc::new(Semaphore::new(permits))
    }))
}

/// Run CPU-bound `work` on the blocking pool under a cooperative deadline.
///
/// `work` receives a [`CancelToken`] it must thread into the engine call so the
/// engine's hot loops can observe cancellation at
/// `config.request_timeout_secs`.
///
/// Returns `Err(ServerError::Timeout)` if the deadline tripped, otherwise the
/// result of `work`.
pub async fn run_with_timeout<C, F, T>(config: C, work: F) -> Result<T, ServerError>
where
    C: AsRef<ServerConfig>,
    F: FnOnce(CancelToken) -> T + Send + 'static,
    T: Send + 'static,
{
    run_with_deadline_secs(config.as_ref().request_timeout_secs, work).await
}

/// Await `fut`, returning [`ServerError::Timeout`] if it doesn't resolve within
/// the configured budget. Used to bound the whole async handler (including
/// multipart body reads) as a coarse backstop on top of the cooperative engine
/// cancellation, so even non-engine stalls can't hang a request forever.
pub async fn with_deadline<C, F, T>(config: C, fut: F) -> Result<T, ServerError>
where
    C: AsRef<ServerConfig>,
    F: Future<Output = Result<T, ServerError>>,
{
    let config = config.as_ref();
    if config.request_timeout_secs == 0 {
        return fut.await;
    }
    let dur = Duration::from_secs(config.request_timeout_secs);
    match tokio::time::timeout(dur, fut).await {
        Ok(result) => result,
        Err(_elapsed) => Err(ServerError::Timeout),
    }
}

/// Reject a render whose pixel count would exceed the configured cap, BEFORE
/// any pixel buffer is allocated. `width_px`/`height_px` come from the page
/// viewport (MediaBox * DPI), so a giant MediaBox is caught here rather than at
/// allocation time.
pub fn check_render_pixels<C: AsRef<ServerConfig>>(
    config: C,
    page_number: usize,
    width_px: u32,
    height_px: u32,
) -> Result<(), ServerError> {
    let config = config.as_ref();
    let pixels = width_px as u64 * height_px as u64;
    if pixels > config.max_render_pixels {
        return Err(ServerError::ResourceLimit(format!(
            "page {} would render {} pixels ({}x{}), exceeding the limit of {} pixels; \
             lower the DPI or the page is abusively large",
            page_number, pixels, width_px, height_px, config.max_render_pixels
        )));
    }
    Ok(())
}

/// Enforce the running output-size cap as a ZIP (or other payload) is built.
/// Call after each chunk is appended; errors the moment the accumulated size
/// crosses the cap so the whole oversized payload is never buffered.
pub fn check_output_size<C: AsRef<ServerConfig>>(
    config: C,
    accumulated: usize,
) -> Result<(), ServerError> {
    check_output_size_limit(config.as_ref().max_output_bytes, accumulated)
}

/// Variant taking an explicit byte cap, so the same check works from a job
/// worker (which has the config's value but not the global config handle).
pub fn check_output_size_limit(
    max_output_bytes: u64,
    accumulated: usize,
) -> Result<(), ServerError> {
    if accumulated as u64 > max_output_bytes {
        return Err(ServerError::ResourceLimit(format!(
            "response output exceeded the limit of {} bytes",
            max_output_bytes
        )));
    }
    Ok(())
}

/// Reject an extract-images request that found more images than the cap.
pub fn check_image_count<C: AsRef<ServerConfig>>(
    config: C,
    count: usize,
) -> Result<(), ServerError> {
    check_image_count_limit(config.as_ref().max_image_count, count)
}

/// Variant taking an explicit cap, callable from a job worker without the
/// global config handle.
pub fn check_image_count_limit(max_image_count: usize, count: usize) -> Result<(), ServerError> {
    if count > max_image_count {
        return Err(ServerError::ResourceLimit(format!(
            "request would extract {} images, exceeding the limit of {}",
            count, max_image_count
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn dropping_deadline_future_cancels_detached_work() {
        let observed = Arc::new(AtomicBool::new(false));
        let worker_observed = Arc::clone(&observed);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();

        let task = tokio::spawn(run_with_deadline_secs(60, move |cancel| {
            let _ = started_tx.send(());
            while !cancel.is_cancelled() {
                std::thread::yield_now();
            }
            worker_observed.store(true, Ordering::Release);
        }));

        started_rx.await.expect("blocking worker started");
        task.abort();
        let _ = task.await;

        tokio::time::timeout(Duration::from_secs(1), async {
            while !observed.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached worker must observe cancellation after caller drop");
    }
}
