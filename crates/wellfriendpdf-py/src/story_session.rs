//! Retained Python editor using the same bounded command transport as C/WASM.
//! No Python objects/callbacks are accessed while detached from the interpreter.
use super::{PyRenderCancellation, WellfriendError};
use pyo3::exceptions::PyValueError;
use pyo3::marker::Ungil;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, TryLockError};
use wellfriendpdf_engine::{
    linked_stories::LinkedStorySession, story_session_protocol as protocol, CancelToken,
};

enum State {
    Open(Box<LinkedStorySession>),
    Closed,
    Poisoned,
}

/// Session calls never wait for another call: concurrent access fails as busy.
/// Rust-only state may move across Python threads; a Mutex protects it even on
/// a free-threaded interpreter. This is not a claim of free-threaded wheel QA.
#[pyclass(name = "StoryEditSession", module = "wellfriendpdf", frozen)]
pub(crate) struct PyStoryEditSession {
    state: Mutex<State>,
}

fn invalid(message: &str) -> wellfriendpdf_engine::WellfriendError {
    wellfriendpdf_engine::WellfriendError::invalid_input(message)
}
fn cancellation(value: Option<PyRef<'_, PyRenderCancellation>>) -> CancelToken {
    value.map_or_else(CancelToken::none, |v| v.token.clone())
}

impl PyStoryEditSession {
    // The guard is acquired and released while detached. There is no lock held
    // while waiting to reattach and no GIL/mutex lock-order inversion.
    fn run<T: Send + Ungil>(
        &self,
        py: Python<'_>,
        operation: impl FnOnce(&mut LinkedStorySession) -> wellfriendpdf_engine::Result<T>
            + Send
            + Ungil,
    ) -> PyResult<T> {
        py.detach(|| self.run_native(operation))
            .map_err(|e| WellfriendError::new_err(e.to_string()))
    }

    fn run_native<T>(
        &self,
        operation: impl FnOnce(&mut LinkedStorySession) -> wellfriendpdf_engine::Result<T>,
    ) -> wellfriendpdf_engine::Result<T> {
        let mut state = self.state.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => {
                invalid("story session is busy; retry after the active call")
            }
            TryLockError::Poisoned(_) => invalid("story session is poisoned; close and reopen"),
        })?;
        match catch_unwind(AssertUnwindSafe(|| match &mut *state {
            State::Open(session) => operation(session),
            State::Closed => Err(invalid("story session is closed")),
            State::Poisoned => Err(invalid("story session is poisoned; close and reopen")),
        })) {
            Ok(result) => result,
            Err(_) => {
                *state = State::Poisoned;
                Err(invalid(
                    "panic in story session; close and reopen known bytes",
                ))
            }
        }
    }

    fn close_native(&self) -> wellfriendpdf_engine::Result<()> {
        let mut state = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Err(invalid(
                    "story session is busy; close after the active call",
                ));
            }
            // Closing remains possible after a poisoned mutex, but never
            // restores access to possibly inconsistent editing state.
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        *state = State::Closed;
        Ok(())
    }
}

#[pymethods]
impl PyStoryEditSession {
    #[new]
    #[pyo3(signature = (pdf_bytes, cancellation=None, *, password=None))]
    fn new(
        py: Python<'_>,
        pdf_bytes: &Bound<'_, PyBytes>,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
        password: Option<&Bound<'_, PyBytes>>,
    ) -> PyResult<Self> {
        let bytes = pdf_bytes.as_bytes();
        if bytes.is_empty() || bytes.len() > protocol::MAX_INPUT_BYTES {
            return Err(PyValueError::new_err(
                "story input must be 1..=256 MiB bytes",
            ));
        }
        let cancel = self::cancellation(cancellation);
        let password = wellfriendpdf_engine::crypto::secret_bytes(
            password.map_or_else(Vec::new, |value| value.as_bytes().to_vec()),
        );
        // Only an immutable Python bytes slice is borrowed across this
        // synchronous detach; the engine copies it before retaining a session.
        let session = super::run_wellfriendpdf_detached(py, || {
            protocol::open_with_password(bytes, password.as_slice(), &cancel)
        })?;
        Ok(Self {
            state: Mutex::new(State::Open(Box::new(session))),
        })
    }

    /// UTF-8 JSON in/out. All operations use the shared strict command enum.
    /// Cancellation is cooperative; success is never reclassified after commit.
    #[pyo3(signature = (command_json, cancellation=None))]
    fn command_json(
        &self,
        py: Python<'_>,
        command_json: &str,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<String> {
        if command_json.len() > protocol::MAX_COMMAND_BYTES {
            return Err(PyValueError::new_err("story command exceeds 32 MiB"));
        }
        let cancel = self::cancellation(cancellation);
        self.run(py, |session| {
            let result = protocol::execute_json(session, command_json.as_bytes(), &cancel)?;
            String::from_utf8(result).map_err(|_| invalid("story response is not UTF-8 JSON"))
        })
    }

    fn status_json(&self, py: Python<'_>) -> PyResult<String> {
        self.command_json(py, r#"{"op":"status"}"#, None)
    }

    #[pyo3(signature = (request_json, cancellation=None))]
    fn preview_json(
        &self,
        py: Python<'_>,
        request_json: &str,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<String> {
        let command = envelope("preview", request_json, None)?;
        self.command_json(py, &command, cancellation)
    }

    #[pyo3(signature = (request_json, receipt_json, cancellation=None))]
    fn checkpoint_json(
        &self,
        py: Python<'_>,
        request_json: &str,
        receipt_json: &str,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<String> {
        let command = envelope("checkpoint", request_json, Some(receipt_json))?;
        self.command_json(py, &command, cancellation)
    }

    #[pyo3(signature = (cancellation=None))]
    fn undo(
        &self,
        py: Python<'_>,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<bool> {
        let result = self.command_json(py, r#"{"op":"undo"}"#, cancellation)?;
        serde_json::from_str(&result).map_err(|_| WellfriendError::new_err("invalid undo response"))
    }

    #[pyo3(signature = (cancellation=None))]
    fn redo(
        &self,
        py: Python<'_>,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<bool> {
        let result = self.command_json(py, r#"{"op":"redo"}"#, cancellation)?;
        serde_json::from_str(&result).map_err(|_| WellfriendError::new_err("invalid redo response"))
    }

    /// Returns a copied Python bytes object; never a view into mutable history.
    fn bytes(&self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        let bytes = self.run(py, |session| Ok(session.bytes().to_vec()))?;
        Ok(PyBytes::new(py, &bytes).unbind())
    }

    /// Explicit rendering only. Opening, discovery and preview do not rasterize.
    #[pyo3(signature = (page, dpi=96, cancellation=None))]
    fn render_page_png(
        &self,
        py: Python<'_>,
        page: usize,
        dpi: u32,
        cancellation: Option<PyRef<'_, PyRenderCancellation>>,
    ) -> PyResult<Py<PyBytes>> {
        let cancel = self::cancellation(cancellation);
        let png = self.run(py, |session| {
            protocol::render_page_png(session, page, dpi, &cancel)
        })?;
        Ok(PyBytes::new(py, &png).unbind())
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.close_native())
            .map_err(|e| WellfriendError::new_err(e.to_string()))
    }

    fn __enter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<PyRef<'py, Self>> {
        slf.run(py, |_| Ok(()))?;
        Ok(slf)
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        self.close(py)?;
        Ok(false) // do not suppress exceptions raised by the with-body
    }
}

// Preserve raw request JSON (including duplicate keys) for the shared serde
// parser to reject; converting to Value here would silently normalize it.
fn envelope(op: &str, request: &str, receipt: Option<&str>) -> PyResult<String> {
    let size = request
        .len()
        .saturating_add(receipt.map_or(0, str::len))
        .saturating_add(128);
    if size > protocol::MAX_COMMAND_BYTES {
        return Err(PyValueError::new_err("story command exceeds 32 MiB"));
    }
    // Validate one complete JSON value, without changing its bytes. Otherwise
    // a trailing comma/field could escape the wrapper's request value.
    let validate = |raw: &str| -> PyResult<()> {
        let mut decoder = serde_json::Deserializer::from_str(raw);
        let _: serde::de::IgnoredAny = serde::Deserialize::deserialize(&mut decoder)
            .map_err(|e| PyValueError::new_err(format!("story JSON: {e}")))?;
        decoder
            .end()
            .map_err(|e| PyValueError::new_err(format!("story JSON: {e}")))
    };
    validate(request)?;
    if let Some(receipt) = receipt {
        validate(receipt)?;
        Ok(format!(
            r#"{{"op":"{op}","request":{request},"receipt":{receipt}}}"#
        ))
    } else {
        Ok(format!(r#"{{"op":"{op}","request":{request}}}"#))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<u8>, PyStoryEditSession) {
        use wellfriendpdf_engine::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
        let mut builder = PdfBuilder::new();
        builder
            .add_page(PageSize::custom(200.0, 200.0))
            .draw_text(
                "OLD",
                10.0,
                150.0,
                &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
            )
            .unwrap();
        let bytes = builder.to_bytes().unwrap();
        let session = protocol::open(&bytes, &CancelToken::none()).unwrap();
        (
            bytes,
            PyStoryEditSession {
                state: Mutex::new(State::Open(Box::new(session))),
            },
        )
    }

    #[test]
    fn python_session_busy_panic_and_close_are_fail_closed() {
        let (_, session) = fixture();
        let guard = session.state.try_lock().unwrap();
        assert!(session
            .run_native(|_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("busy"));
        assert!(session
            .close_native()
            .unwrap_err()
            .to_string()
            .contains("busy"));
        drop(guard);
        assert!(session
            .run_native::<()>(|_| panic!("injected unwind"))
            .unwrap_err()
            .to_string()
            .contains("panic"));
        assert!(session
            .run_native(|_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("poisoned"));
        session.close_native().unwrap();
        session.close_native().unwrap();
        assert!(session
            .run_native(|_| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("closed"));
    }

    #[test]
    fn python_session_cancellation_is_not_poison_and_state_can_move_threads() {
        let (bytes, session) = fixture();
        let token = CancelToken::new();
        token.cancel();
        assert!(session
            .run_native(|s| protocol::execute_json(s, br#"{"op":"undo"}"#, &token))
            .is_err());
        assert_eq!(
            session.run_native(|s| Ok(s.bytes().to_vec())).unwrap(),
            bytes
        );
        let session = std::thread::spawn(move || {
            assert!(session
                .run_native(|s| protocol::execute_json(
                    s,
                    br#"{"op":"status"}"#,
                    &CancelToken::none()
                ))
                .is_ok());
            session
        })
        .join()
        .unwrap();
        assert_eq!(
            session.run_native(|s| Ok(s.bytes().to_vec())).unwrap(),
            bytes
        );
        session.close_native().unwrap();
    }
}
