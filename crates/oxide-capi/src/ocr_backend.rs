//! A **C-function-pointer OCR backend** for the Oxide seam.
//!
//! This lets a non-Rust integrator supply OCR to Oxide through the C ABI: they
//! provide a `recognize` function pointer (plus opaque `userdata`), Oxide renders
//! and preprocesses scanned pages and calls back into it, and the recognized
//! words flow into the same document model as native text — exactly like the
//! Rust and Python backends.
//!
//! # The C contract
//!
//! ```c
//! // Oxide hands the callee this sink to report each recognized word. Call it
//! // once per word. `text` is a NUL-terminated UTF-8 string owned by the caller
//! // for the duration of the call. bbox is [x0,y0,x1,y1] in image-pixel space
//! // (y-down, the same frame as `gray`). `line_id` groups words into lines; pass
//! // a negative value if unknown.
//! typedef void (*oxide_ocr_emit_word_fn)(
//!     void* sink, const char* text,
//!     double x0, double y0, double x1, double y1,
//!     float confidence, int32_t line_id);
//!
//! // The integrator implements this. Return 0 on success, non-zero to signal a
//! // recognition failure (Oxide degrades that page to the placeholder). `gray`
//! // is width*height 8-bit grayscale, row-major, top-left origin.
//! typedef int (*oxide_ocr_recognize_fn)(
//!     void* userdata,
//!     const uint8_t* gray, uint32_t width, uint32_t height,
//!     uint32_t dpi,
//!     void* sink, oxide_ocr_emit_word_fn emit);
//!
//! typedef struct {
//!     void* userdata;                    // opaque, passed back to recognize
//!     oxide_ocr_recognize_fn recognize;  // required
//!     uint32_t max_concurrency;          // 0 => 1; pages OCR'd in parallel up to this
//!     const char* name;                  // optional label for provenance; may be NULL
//! } OxideOcrBackend;
//! ```
//!
//! # Safety / threading
//!
//! The backend struct is copied into a Rust adapter that implements
//! [`OcrEngine`]. `recognize` may be invoked from multiple threads concurrently
//! when `max_concurrency > 1`, so a callee that is not thread-safe must leave it
//! at `1` (the default). Panics/hangs on the Rust side are contained by the
//! engine seam; a non-zero return degrades the page cleanly.

use std::ffi::{c_void, CStr};
use std::os::raw::{c_char, c_int};

use oxide_engine::{OcrEngine, OcrImage, OcrOptions, OcrPage, OcrWord, OxideError};

/// Sink callback Oxide passes to the C `recognize`; called once per word.
pub type OxideOcrEmitWordFn = extern "C" fn(
    sink: *mut c_void,
    text: *const c_char,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    confidence: f32,
    line_id: i32,
);

/// The integrator-supplied recognition function. Returns 0 on success.
pub type OxideOcrRecognizeFn = extern "C" fn(
    userdata: *mut c_void,
    gray: *const u8,
    width: u32,
    height: u32,
    dpi: u32,
    sink: *mut c_void,
    emit: OxideOcrEmitWordFn,
) -> c_int;

/// The C backend descriptor (see the module docs for the ABI).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OxideOcrBackend {
    pub userdata: *mut c_void,
    pub recognize: Option<OxideOcrRecognizeFn>,
    pub max_concurrency: u32,
    pub name: *const c_char,
}

/// The Rust adapter: owns a copy of the C descriptor and implements [`OcrEngine`]
/// by invoking the function pointer with an [`emit_word`] trampoline that pushes
/// into a `Vec<OcrWord>`.
pub struct CAbiOcrEngine {
    backend: OxideOcrBackend,
    name: String,
}

// SAFETY: the adapter is `Send + Sync` iff the C integrator's `recognize` +
// `userdata` are safe to call from multiple threads. That is exactly what
// `max_concurrency` documents: the default of 1 serializes calls, and a value
// >1 is the integrator asserting thread-safety. Raw pointers are otherwise not
// auto-`Send`/`Sync`, so we assert it here on that documented contract.
unsafe impl Send for CAbiOcrEngine {}
unsafe impl Sync for CAbiOcrEngine {}

impl CAbiOcrEngine {
    /// Build the adapter from a C descriptor. Returns `None` if `recognize` is
    /// null (a backend with no recognizer is unusable).
    ///
    /// # Safety
    /// `backend.name`, if non-null, must point to a valid NUL-terminated string
    /// for the duration of this call. `backend.recognize` / `backend.userdata`
    /// must remain valid for the lifetime of the returned engine.
    pub unsafe fn from_descriptor(backend: OxideOcrBackend) -> Option<Self> {
        backend.recognize?;
        let name = if backend.name.is_null() {
            "c-abi".to_string()
        } else {
            // SAFETY: caller guarantees a valid NUL-terminated string.
            unsafe { CStr::from_ptr(backend.name) }
                .to_str()
                .unwrap_or("c-abi")
                .to_string()
        };
        Some(CAbiOcrEngine { backend, name })
    }
}

/// Trampoline handed to the C side as the word sink. `sink` is a
/// `*mut Vec<OcrWord>`; each call pushes one word.
extern "C" fn emit_word(
    sink: *mut c_void,
    text: *const c_char,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    confidence: f32,
    line_id: i32,
) {
    if sink.is_null() || text.is_null() {
        return;
    }
    // SAFETY: `sink` is the `&mut Vec<OcrWord>` we passed into `recognize`; it is
    // valid for the duration of that call and only touched from the callee's
    // (synchronous) invocations of this trampoline.
    let words = unsafe { &mut *(sink as *mut Vec<OcrWord>) };
    // SAFETY: the C contract requires `text` to be a valid NUL-terminated UTF-8
    // string for the duration of the call. Lossy-decode to never panic on bad
    // bytes.
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    words.push(OcrWord {
        text,
        bbox: [x0, y0, x1, y1],
        confidence: confidence.clamp(0.0, 1.0),
        line_id: if line_id < 0 {
            None
        } else {
            Some(line_id as u32)
        },
    });
}

impl OcrEngine for CAbiOcrEngine {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        let Some(recognize) = self.backend.recognize else {
            return Err(OxideError::UnsupportedFeature(
                "C-ABI OCR backend has a null recognize function".to_string(),
            ));
        };
        let mut words: Vec<OcrWord> = Vec::new();
        let sink = &mut words as *mut Vec<OcrWord> as *mut c_void;
        // Invoke the integrator's recognizer. It reports words synchronously via
        // `emit_word` and returns a status code.
        let status = recognize(
            self.backend.userdata,
            image.gray.as_ptr(),
            image.width,
            image.height,
            opts.dpi,
            sink,
            emit_word,
        );
        if status != 0 {
            return Err(OxideError::UnsupportedFeature(format!(
                "C-ABI OCR backend '{}' returned failure status {status}",
                self.name
            )));
        }
        Ok(OcrPage::new(words))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn max_concurrency(&self) -> usize {
        (self.backend.max_concurrency as usize).max(1)
    }
}
