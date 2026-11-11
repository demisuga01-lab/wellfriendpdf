//! Interactive XFA form sessions: thin wrappers over
//! `wellfriendpdf_engine::XfaFormSession` (JSON in/out, bytes for PDFs and
//! PNGs). The session holds single-threaded script engines, so the Python
//! object is bound to the thread that created it.
use super::{parse_json_str, run_wellfriendpdf, WellfriendError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes};
use wellfriendpdf_engine::{XfaFormSession, XfaSessionOptions};

#[pyclass(name = "XfaFormSession", module = "wellfriendpdf", unsendable)]
pub(crate) struct PyXfaFormSession {
    // `None` after a panic: the session is poisoned.
    inner: Option<XfaFormSession>,
}

impl PyXfaFormSession {
    pub(crate) fn open(
        engine: &wellfriendpdf_engine::ContentEngine,
        run_scripts: bool,
    ) -> PyResult<Self> {
        let options = XfaSessionOptions { run_scripts };
        let inner = run_wellfriendpdf(|| engine.xfa_session_with_options(&options))?;
        Ok(Self { inner: Some(inner) })
    }

    fn run<T>(
        &mut self,
        operation: impl FnOnce(&mut XfaFormSession) -> wellfriendpdf_engine::Result<T>,
    ) -> PyResult<T> {
        let session = self
            .inner
            .as_mut()
            .ok_or_else(|| WellfriendError::new_err("XFA session is poisoned; reopen it"))?;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(session))) {
            Ok(result) => result.map_err(|err| WellfriendError::new_err(err.to_string())),
            Err(_) => {
                self.inner = None;
                Err(WellfriendError::new_err(
                    "panic in XFA session; the session is poisoned",
                ))
            }
        }
    }
}

#[pymethods]
impl PyXfaFormSession {
    /// Runs a JSON command (`{"op": "fields" | "value" | "set_value" |
    /// "click" | "dispatch" | "validations" | "script_log" |
    /// "export_data_xml" | "page_count" | "form_type", ...}`) and returns the
    /// JSON result string.
    fn command_json(&mut self, command_json: &str) -> PyResult<String> {
        self.run(|session| session.command_json(command_json))
    }

    /// Fields and exclusion groups (SOM path, type, value, presence...).
    fn fields(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let json = self.run(|session| session.command_json(r#"{"op":"fields"}"#))?;
        parse_json_str(py, &json)
    }

    /// Edits a field (enter, change, exit, then calculate and validate).
    fn set_value(&mut self, py: Python<'_>, som: &str, value: &str) -> PyResult<Py<PyAny>> {
        let outcome = self.run(|session| session.set_field_value(som, value))?;
        outcome_json(py, &outcome)
    }

    /// Clicks a field (buttons, check boxes, radio buttons).
    fn click(&mut self, py: Python<'_>, som: &str) -> PyResult<Py<PyAny>> {
        let outcome = self.run(|session| session.click(som))?;
        outcome_json(py, &outcome)
    }

    /// Dispatches one event activity to a node.
    fn dispatch_event(&mut self, py: Python<'_>, som: &str, activity: &str) -> PyResult<Py<PyAny>> {
        let outcome = self.run(|session| session.dispatch_event(som, activity))?;
        outcome_json(py, &outcome)
    }

    /// The datasets packet XML of the current data.
    fn export_data_xml(&mut self) -> PyResult<String> {
        self.run(|session| session.export_data_xml())
    }

    /// PNG of XFA page `page` (1-based) with the session's state.
    #[pyo3(signature = (page, dpi=72))]
    fn render_page_png(&mut self, py: Python<'_>, page: usize, dpi: u32) -> PyResult<Py<PyBytes>> {
        let png = self.run(|session| session.render_page_png(page, dpi))?;
        Ok(PyBytes::new(py, &png).unbind())
    }

    /// The PDF with the session's data saved into its datasets packet.
    fn save_pdf(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        let pdf = self.run(|session| session.save_to_pdf())?;
        Ok(PyBytes::new(py, &pdf).unbind())
    }

    /// The current XFA layout as a standalone PDF.
    fn layout_pdf(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        let pdf = self.run(|session| session.layout_pdf())?;
        Ok(PyBytes::new(py, &pdf).unbind())
    }
}

fn outcome_json(
    py: Python<'_>,
    outcome: &wellfriendpdf_engine::XfaSessionOutcome,
) -> PyResult<Py<PyAny>> {
    let json = serde_json::to_string(outcome)
        .map_err(|err| WellfriendError::new_err(format!("XFA session output: {err}")))?;
    parse_json_str(py, &json)
}
