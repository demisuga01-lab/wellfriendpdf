//! Containment for the OCR seam: run an untrusted backend so that a **panic**,
//! a **hang**, or an **error** inside it can never crash Wellfriend, corrupt the
//! document model, or cross the seam as an unwound panic.
//!
//! An OCR backend is, by design, third-party code — a subprocess (Tesseract), a
//! Python object, a C function pointer, a network client. Wellfriend makes three
//! guarantees about calling into it, all enforced *here*, on Wellfriend's side of the
//! trait, so every backend inherits them for free:
//!
//! 1. **No panic escapes.** [`recognize_contained`] wraps the backend call in
//!    [`std::panic::catch_unwind`]; a panicking backend becomes a clean
//!    [`WellfriendError`], and the page falls back to the placeholder.
//! 2. **The engine owns the timeout.** A backend is never trusted to return. When
//!    a positive timeout is given, the call runs on a scratch thread and is
//!    bounded by [`std::sync::mpsc::Receiver::recv_timeout`]; on expiry the
//!    caller gets a `Cancelled` error immediately. (The backend thread is
//!    detached — it cannot be force-killed in safe Rust — but it holds only its
//!    own `Arc` + image clone and its result is dropped when it finally lands, so
//!    it leaks nothing into the document model. Backends that own a killable
//!    resource, like the Tesseract subprocess, *also* enforce their own internal
//!    timeout; this is the outer backstop for the ones that don't.)
//! 3. **Errors are contained per call.** Any `Err` from the backend is returned
//!    as-is for the caller to handle per page/region — the run continues.
//!
//! The one-page-at-a-time discipline of the parse pipeline is unchanged: this
//! module bounds a *single* `recognize` call. Bounded-window parallelism across
//! pages is layered on top by the caller using [`OcrEngine::max_concurrency`].
//!
//! # Why the timeout path takes an `Arc`
//!
//! The engine crate is `#![forbid(unsafe_code)]`, so the scratch thread cannot
//! borrow a non-`'static` `&dyn OcrEngine`. An `Arc<dyn OcrEngine>` is
//! `'static + Send + Sync` (the trait requires `Send + Sync`), so it clones
//! cheaply into the worker thread with no unsafe. The parse pipeline already
//! holds the engine as an `Arc`, so this is free at the call site.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Result, WellfriendError};
use crate::ocr::{OcrEngine, OcrImage, OcrOptions, OcrPage};

/// Recognize one page image through `engine`, containing panics and (when
/// `timeout` is `Some` and positive) enforcing an engine-side deadline.
///
/// - `timeout == None` or `Some(0)` → call the backend directly on this thread,
///   with panic containment but no deadline (the backend's own timeout, if any,
///   still applies). This is the zero-overhead path used when the caller does not
///   want the extra thread.
/// - `timeout == Some(d)` with `d > 0` → clone the `Arc` onto a scratch thread
///   and wait at most `d`; on expiry return [`WellfriendError::Cancelled`].
///
/// Never panics. Never blocks longer than `timeout` (when set). The returned
/// `Err` is the backend's own error, a timeout, or a captured panic message.
pub fn recognize_contained(
    engine: &Arc<dyn OcrEngine>,
    image: &OcrImage,
    opts: &OcrOptions,
    timeout: Option<Duration>,
) -> Result<OcrPage> {
    match timeout {
        Some(d) if !d.is_zero() => recognize_with_timeout(engine, image, opts, d),
        _ => recognize_catching(engine.as_ref(), image, opts),
    }
}

/// Call the backend on the current thread, converting any panic into an error.
fn recognize_catching(
    engine: &dyn OcrEngine,
    image: &OcrImage,
    opts: &OcrOptions,
) -> Result<OcrPage> {
    // `AssertUnwindSafe`: `&dyn OcrEngine` / `&OcrImage` / `&OcrOptions` are
    // shared references we only read; a backend that panics mid-`recognize`
    // leaves nothing of ours observably broken (we drop the borrow and fall back).
    let call = std::panic::AssertUnwindSafe(|| engine.recognize(image, opts));
    match std::panic::catch_unwind(call) {
        Ok(result) => result,
        Err(payload) => Err(WellfriendError::UnsupportedFeature(format!(
            "OCR backend '{}' panicked: {}",
            engine.name(),
            panic_message(&payload)
        ))),
    }
}

/// Run the backend on a scratch thread and bound the wait by `timeout`. On
/// expiry the backend thread is detached (left to finish and discard its result)
/// and a `Cancelled` error is returned so the caller degrades the page.
fn recognize_with_timeout(
    engine: &Arc<dyn OcrEngine>,
    image: &OcrImage,
    opts: &OcrOptions,
    timeout: Duration,
) -> Result<OcrPage> {
    // The backend runs on another thread, so it needs owned data. The image is
    // the heavy piece; a single page clone is bounded and short-lived (dropped
    // when the OCR call returns), keeping the memory discipline intact.
    let image_owned = image.clone();
    let opts_owned = opts.clone();
    let name = engine.name().to_string();
    let engine_owned = Arc::clone(engine);

    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name(format!("ocr-{name}"))
        .spawn(move || {
            let result = recognize_catching(engine_owned.as_ref(), &image_owned, &opts_owned);
            // If the receiver already timed out and went away, this send fails;
            // that is fine — the result is simply dropped.
            let _ = tx.send(result);
        });

    let handle = match handle {
        Ok(h) => h,
        // Could not spawn a thread (resource pressure): fall back to a direct,
        // still-panic-contained call rather than failing the page outright.
        Err(_) => return recognize_catching(engine.as_ref(), image, opts),
    };

    match rx.recv_timeout(timeout) {
        Ok(result) => {
            // The backend finished in time; join to reclaim the thread promptly.
            let _ = handle.join();
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Detach: we cannot safely kill the thread, but it only holds its own
            // clones and its result will be discarded. The page is failed cleanly.
            Err(WellfriendError::Cancelled(format!(
                "OCR backend '{name}' exceeded the {}ms per-page timeout",
                timeout.as_millis()
            )))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // The thread died without sending (should be impossible given
            // `catch_unwind`, but handle it rather than hang).
            let _ = handle.join();
            Err(WellfriendError::UnsupportedFeature(format!(
                "OCR backend '{name}' terminated without returning a result"
            )))
        }
    }
}

/// Best-effort extraction of a human string from a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrPage, OcrWord};

    struct PanicEngine;
    impl OcrEngine for PanicEngine {
        fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> Result<OcrPage> {
            panic!("boom from the backend");
        }
        fn name(&self) -> &str {
            "panic"
        }
    }

    struct SlowEngine;
    impl OcrEngine for SlowEngine {
        fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> Result<OcrPage> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(OcrPage::new(Vec::new()))
        }
        fn name(&self) -> &str {
            "slow"
        }
    }

    struct OkEngine;
    impl OcrEngine for OkEngine {
        fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> Result<OcrPage> {
            Ok(OcrPage::new(vec![OcrWord {
                text: "ok".into(),
                bbox: [0.0, 0.0, 1.0, 1.0],
                confidence: 0.9,
                line_id: Some(0),
            }]))
        }
        fn name(&self) -> &str {
            "ok"
        }
    }

    fn arc(e: impl OcrEngine + 'static) -> Arc<dyn OcrEngine> {
        Arc::new(e)
    }

    #[test]
    fn panic_is_contained_as_error_not_unwind() {
        let img = OcrImage::white(4, 4);
        let opts = OcrOptions::default();
        let engine = arc(PanicEngine);
        // Both with and without a timeout, a panic becomes a clean Err.
        let e = recognize_contained(&engine, &img, &opts, None).unwrap_err();
        assert!(e.to_string().contains("panicked"), "got: {e}");
        let e =
            recognize_contained(&engine, &img, &opts, Some(Duration::from_secs(5))).unwrap_err();
        assert!(e.to_string().contains("panicked"), "got: {e}");
    }

    #[test]
    fn hang_is_bounded_by_the_engine_timeout() {
        let img = OcrImage::white(4, 4);
        let opts = OcrOptions::default();
        let engine = arc(SlowEngine);
        let start = std::time::Instant::now();
        let e = recognize_contained(&engine, &img, &opts, Some(Duration::from_millis(150)))
            .unwrap_err();
        // Returned promptly (well under the backend's 30s sleep).
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "did not time out promptly"
        );
        assert!(matches!(e, WellfriendError::Cancelled(_)), "got: {e}");
    }

    #[test]
    fn successful_call_passes_through_on_both_paths() {
        let img = OcrImage::white(4, 4);
        let opts = OcrOptions::default();
        let engine = arc(OkEngine);
        let p = recognize_contained(&engine, &img, &opts, None).unwrap();
        assert_eq!(p.words.len(), 1);
        let p = recognize_contained(&engine, &img, &opts, Some(Duration::from_secs(5))).unwrap();
        assert_eq!(p.words.len(), 1);
    }
}
