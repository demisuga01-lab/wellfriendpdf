//! Server-wide OCR backend hook.
//!
//! A deployment can register **one** OCR backend at startup; every parser
//! endpoint then routes scanned pages through it, so a `POST /api/v1/parse` of a
//! scanned PDF returns recovered text instead of the placeholder. With no backend
//! registered (the default), the server stays digital-born-only exactly as
//! before — no OCR dependency is pulled in and scanned pages degrade to
//! placeholders.
//!
//! # Why a global
//!
//! The backend is process-wide configuration, set once before serving and read
//! by every request handler — the same shape as [`crate::config`]. It is stored
//! in a [`OnceLock`] so registration is a startup step and reads are lock-free.
//!
//! # Enabling a backend
//!
//! The server crate deliberately has **no** OCR dependency of its own. Two ways a
//! deployment supplies one:
//!
//! - Build with `--features ocr`: [`init_from_env`] then auto-registers the
//!   Tesseract backend when `OXIDE_OCR=1` (or `auto`/`force`) is set in the
//!   environment, discovering the `tesseract` binary on `PATH`.
//! - Embed the server crate and call [`set_backend`] directly with any
//!   [`OcrEngine`] before starting the router.
//!
//! Either way the seam is identical; only where the backend comes from differs.

use std::sync::Arc;
use std::sync::OnceLock;

use oxide_engine::{OcrPolicy, ParseOptions};

/// A registered backend + the policy to apply. `None` inside the `OnceLock`
/// means "explicitly no OCR" (registration ran but found nothing); an unset
/// `OnceLock` means registration has not run yet — both read as "no OCR".
struct OcrHook {
    engine: Arc<dyn oxide_engine::OcrEngine>,
    policy: OcrPolicy,
}

static OCR_HOOK: OnceLock<Option<OcrHook>> = OnceLock::new();

/// Register a server-wide OCR backend with the given policy. Idempotent: the
/// first registration wins (later calls are ignored), matching the `OnceLock`
/// config pattern. Returns `true` if this call performed the registration.
///
/// Call once, before building the router. Safe to skip entirely — the server
/// then serves digital-born-only.
pub fn set_backend(engine: Arc<dyn oxide_engine::OcrEngine>, policy: OcrPolicy) -> bool {
    OCR_HOOK.set(Some(OcrHook { engine, policy })).is_ok()
}

/// Whether a backend is registered and active (policy is not `Off`).
pub fn is_enabled() -> bool {
    matches!(OCR_HOOK.get(), Some(Some(h)) if h.policy != OcrPolicy::Off)
}

/// A short label for the registered backend (for `/health` / logs), or `None`.
pub fn backend_name() -> Option<String> {
    match OCR_HOOK.get() {
        Some(Some(h)) => Some(h.engine.name().to_string()),
        _ => None,
    }
}

/// Apply the registered OCR backend (if any) to a [`ParseOptions`]. Called by
/// every parser endpoint just before parsing. A generous engine-enforced
/// per-page timeout is set so a wedged backend fails a page rather than pinning
/// the request's cooperative deadline. No-op when no backend is registered.
pub fn apply_to(opts: &mut ParseOptions) {
    if let Some(Some(hook)) = OCR_HOOK.get() {
        opts.ocr = Some(Arc::clone(&hook.engine));
        opts.ocr_policy = hook.policy;
        opts.ocr_timeout = Some(std::time::Duration::from_secs(60));
    }
}

/// Apply the registered OCR backend (if any) to an [`ExtractOptions`], so
/// field extraction over a scanned PDF recovers text first. Same contract as
/// [`apply_to`]; no-op when no backend is registered. (`ExtractOptions` has no
/// per-page timeout field — it forwards `ocr`/`ocr_policy` into `ParseOptions`
/// internally, where the parse step's own containment still applies.)
pub fn apply_to_extract(opts: &mut oxide_engine::ExtractOptions) {
    if let Some(Some(hook)) = OCR_HOOK.get() {
        opts.ocr = Some(Arc::clone(&hook.engine));
        opts.ocr_policy = hook.policy;
    }
}

/// Initialize the OCR hook from the environment at startup. With the `ocr`
/// feature, reads `OXIDE_OCR` (`off`/`auto`/`force`; `1`/`on`/`true` ⇒ `auto`)
/// and, when not `off`, discovers and registers the Tesseract backend. Without
/// the feature this is a no-op (any backend must be registered via
/// [`set_backend`]). Logs the outcome. Never panics; a discovery failure leaves
/// the server digital-born-only with a warning.
pub fn init_from_env() {
    let raw = std::env::var("OXIDE_OCR").unwrap_or_default();
    let policy = match OcrPolicy::parse(&raw) {
        Some(p) => p,
        None if raw.trim().is_empty() => OcrPolicy::Off,
        None => {
            tracing::warn!("OXIDE_OCR='{raw}' is not off/auto/force; OCR stays disabled");
            OcrPolicy::Off
        }
    };
    if policy == OcrPolicy::Off {
        return;
    }

    #[cfg(feature = "ocr")]
    {
        use oxide_engine::OcrEngine as _;
        match oxide_ocr_tesseract::TesseractEngine::new() {
            Ok(engine) => {
                let name = engine.name().to_string();
                set_backend(Arc::new(engine), policy);
                tracing::info!("OCR enabled server-wide: backend='{name}', policy={policy:?}");
            }
            Err(e) => {
                tracing::warn!(
                    "OXIDE_OCR requested but the Tesseract backend could not start ({e}); \
                     scanned pages will degrade to placeholders"
                );
            }
        }
    }
    #[cfg(not(feature = "ocr"))]
    {
        tracing::warn!(
            "OXIDE_OCR requested but this server was built without the `ocr` feature; \
             rebuild with `--features ocr` or register a backend via set_backend()"
        );
    }
}
