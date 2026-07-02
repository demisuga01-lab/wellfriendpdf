//! A **Python-implemented OCR backend** for the Oxide seam.
//!
//! This is the integration that matters most for the local-AI and cloud-AI
//! audiences: their model code is Python, and this lets that code *be* Oxide's
//! OCR engine. A caller passes any Python object exposing a
//!
//! ```python
//! def recognize(self, image_bytes: bytes, info: dict) -> list[dict]: ...
//! ```
//!
//! method to a parse call (`ocr=...`), and Oxide drives it exactly like the
//! native Tesseract backend: detect scanned pages → render + preprocess →
//! call `recognize` → merge the returned word boxes into the document model.
//!
//! # The Python contract
//!
//! `recognize(image_bytes, info)` receives:
//! - `image_bytes`: the preprocessed page as **raw 8-bit grayscale**,
//!   `width * height` bytes, row-major, top-left origin (y-down). Reshape with
//!   e.g. `numpy.frombuffer(image_bytes, dtype=uint8).reshape(height, width)`.
//! - `info`: a dict with `width`, `height` (pixels), `dpi`, `languages`
//!   (list[str]), and `psm` (int or None) — the [`OcrOptions`] passed through.
//!
//! and returns recognized words as either a `list` of word dicts, or a dict with
//! a `"words"` key holding that list. Each word dict has:
//! - `text`: str (required)
//! - `bbox`: `[x0, y0, x1, y1]` in **image-pixel space** (y-down, same frame as
//!   `image_bytes`) (required)
//! - `confidence`: float in 0..1 (optional; defaults to 1.0)
//! - `line_id`: int grouping words into text lines (optional but recommended —
//!   it improves line reassembly)
//!
//! # Concurrency and the GIL
//!
//! [`OcrEngine::max_concurrency`] returns **1**: the Python GIL serializes calls
//! into the interpreter, so Oxide OCRs pages one at a time when this backend is
//! used, rather than spawning a bounded parallel window that would only contend
//! on the GIL. A backend whose `recognize` releases the GIL for the heavy work
//! (e.g. a numpy/torch model that drops the GIL in native code) still runs
//! correctly here; raising the concurrency would require a pool of interpreters,
//! which is out of scope for this binding.
//!
//! # Robustness
//!
//! A Python exception in `recognize` is converted to a clean [`OxideError`] and
//! contained by the engine's seam ([`oxide_engine::ocr::dispatch`]): the page
//! degrades to the placeholder, the run continues, and no exception or panic
//! crosses back into Python as a crash.

use oxide_engine::{OcrEngine, OcrImage, OcrOptions, OcrPage, OcrWord, OxideError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

/// An [`OcrEngine`] backed by a user-supplied Python object with a
/// `recognize(image_bytes, info)` method. Constructed from any Python callable
/// object; see the module docs for the data contract.
pub struct PyOcrEngine {
    /// The Python backend object. `Py<PyAny>` is `Send + Sync` (GIL-guarded), so
    /// this satisfies the `OcrEngine: Send + Sync` bound and can be shared with
    /// the engine's parse pipeline.
    callback: Py<PyAny>,
    /// A short label recorded in OCR provenance. Taken from the object's
    /// `name` attribute if present, else `"python"`.
    name: String,
    /// Optional version string, from the object's `version` attribute.
    version: Option<String>,
}

impl PyOcrEngine {
    /// Wrap a Python object as an OCR backend. Validates up front that the object
    /// exposes a callable `recognize` attribute, so a misconfigured backend fails
    /// at parse setup with a clear message rather than once per page.
    pub fn new(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let recognize = obj.getattr("recognize").map_err(|_| {
            pyo3::exceptions::PyTypeError::new_err(
                "ocr backend must be an object with a `recognize(image_bytes, info)` method",
            )
        })?;
        if !recognize.is_callable() {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "the ocr backend's `recognize` attribute must be callable",
            ));
        }
        // Optional identity metadata for provenance.
        let name = obj
            .getattr("name")
            .ok()
            .and_then(|v| v.extract::<String>().ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "python".to_string());
        let version = obj
            .getattr("version")
            .ok()
            .and_then(|v| v.extract::<String>().ok())
            .filter(|s| !s.is_empty());
        Ok(PyOcrEngine {
            callback: obj.clone().unbind(),
            name,
            version,
        })
    }
}

impl OcrEngine for PyOcrEngine {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        Python::attach(|py| self.recognize_gil(py, image, opts)).map_err(|e: PyErr| {
            // Surface the Python traceback text in the error; the seam contains
            // this per page and falls back to the placeholder.
            OxideError::UnsupportedFeature(format!(
                "python OCR backend '{}' raised: {}",
                self.name, e
            ))
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> Option<String> {
        self.version.clone()
    }

    fn max_concurrency(&self) -> usize {
        // The GIL serializes calls into the interpreter; one at a time.
        1
    }
}

impl PyOcrEngine {
    /// The GIL-held body of `recognize`: build the `info` dict, hand the raw
    /// grayscale bytes to the Python object, and parse the returned words. All
    /// errors are `PyErr` (mapped to `OxideError` by the caller).
    fn recognize_gil(
        &self,
        py: Python<'_>,
        image: &OcrImage,
        opts: &OcrOptions,
    ) -> PyResult<OcrPage> {
        let info = PyDict::new(py);
        info.set_item("width", image.width)?;
        info.set_item("height", image.height)?;
        info.set_item("dpi", opts.dpi)?;
        info.set_item("languages", opts.languages.clone())?;
        info.set_item("psm", opts.psm)?;
        // Raw single-channel grayscale, row-major, y-down (see module docs).
        let image_bytes = PyBytes::new(py, &image.gray);

        let obj = self.callback.bind(py);
        let result = obj.call_method1("recognize", (image_bytes, info))?;

        parse_words(&result)
    }
}

/// Parse the Python return value into an [`OcrPage`]. Accepts either a list of
/// word dicts or a dict with a `"words"` list (see module docs).
fn parse_words(result: &Bound<'_, PyAny>) -> PyResult<OcrPage> {
    // Unwrap a `{"words": [...]}` envelope if present.
    let list_obj: Bound<'_, PyAny> = if let Ok(dict) = result.cast::<PyDict>() {
        match dict.get_item("words")? {
            Some(w) => w,
            None => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "recognize() returned a dict without a 'words' key",
                ))
            }
        }
    } else {
        result.clone()
    };

    let list = list_obj.cast::<PyList>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(
            "recognize() must return a list of word dicts (or a dict with a 'words' list)",
        )
    })?;

    let mut words = Vec::with_capacity(list.len());
    for item in list.iter() {
        words.push(parse_word(&item)?);
    }
    Ok(OcrPage::new(words))
}

/// Parse one word dict into an [`OcrWord`]. `text` and `bbox` are required;
/// `confidence` defaults to 1.0 and `line_id` is optional.
fn parse_word(item: &Bound<'_, PyAny>) -> PyResult<OcrWord> {
    let dict = item.cast::<PyDict>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err("each recognized word must be a dict")
    })?;

    let text: String = dict
        .get_item("text")?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("word dict missing 'text'"))?
        .extract()?;

    let bbox_any = dict
        .get_item("bbox")?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("word dict missing 'bbox'"))?;
    let bbox_vec: Vec<f64> = bbox_any.extract().map_err(|_| {
        pyo3::exceptions::PyValueError::new_err(
            "'bbox' must be a sequence of 4 numbers [x0,y0,x1,y1]",
        )
    })?;
    if bbox_vec.len() != 4 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "'bbox' must have exactly 4 numbers [x0,y0,x1,y1]",
        ));
    }
    let bbox = [bbox_vec[0], bbox_vec[1], bbox_vec[2], bbox_vec[3]];

    let confidence: f32 = match dict.get_item("confidence")? {
        Some(v) => v.extract().unwrap_or(1.0),
        None => 1.0,
    };
    let line_id: Option<u32> = match dict.get_item("line_id")? {
        Some(v) if !v.is_none() => v.extract().ok(),
        _ => None,
    };

    Ok(OcrWord {
        text,
        bbox,
        confidence: confidence.clamp(0.0, 1.0),
        line_id,
    })
}
