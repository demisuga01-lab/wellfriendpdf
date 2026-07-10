use std::path::PathBuf;
use std::sync::Arc;

use oxide_engine::{
    sdk, ContentEngine, DocType, DocumentInfo, ExtractOptions, ExtractionProfile,
    ImageLocateOptions, ImageOutputFormat, OcrPolicy, PageRegion, ParseOptions, SerializeOptions,
};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyList, PyModule, PyType};
use serde::Serialize;
use serde_json::json;

mod ocr_backend;
use ocr_backend::PyOcrEngine;

create_exception!(oxide, OxideError, PyException);

#[pyclass(name = "Document", module = "oxide", unsendable)]
struct PyDocument {
    engine: Arc<ContentEngine>,
}

#[pyclass(name = "Page", module = "oxide", unsendable)]
struct PyPage {
    engine: Arc<ContentEngine>,
    number: usize,
}

#[pyclass(name = "RegionPage", module = "oxide", unsendable)]
struct PyRegionPage {
    engine: Arc<ContentEngine>,
    number: usize,
    region: PageRegion,
}

#[pyclass(name = "_PageIterator", module = "oxide", unsendable)]
struct PyPageIterator {
    engine: Arc<ContentEngine>,
    next: usize,
    total: usize,
}

#[pymethods]
impl PyDocument {
    #[new]
    #[pyo3(signature = (source, password=None))]
    fn new(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<Self> {
        open_impl(source, password)
    }

    #[classmethod]
    #[pyo3(signature = (path, password=None))]
    fn from_path(
        _cls: &Bound<'_, PyType>,
        path: PathBuf,
        password: Option<&str>,
    ) -> PyResult<Self> {
        let engine = if let Some(password) = password {
            run_oxide(|| ContentEngine::open_path_with_password(path, password.as_bytes()))?
        } else {
            run_oxide(|| ContentEngine::open_path(path))?
        };
        Ok(Self {
            engine: Arc::new(engine),
        })
    }

    #[classmethod]
    #[pyo3(signature = (data, password=None))]
    fn from_bytes(
        _cls: &Bound<'_, PyType>,
        data: Vec<u8>,
        password: Option<&str>,
    ) -> PyResult<Self> {
        let engine = if let Some(password) = password {
            run_oxide(|| ContentEngine::open_bytes_with_password(data, password.as_bytes()))?
        } else {
            run_oxide(|| ContentEngine::open_bytes(data))?
        };
        Ok(Self {
            engine: Arc::new(engine),
        })
    }

    #[getter]
    fn page_count(&self) -> PyResult<usize> {
        run_oxide(|| self.engine.page_count())
    }

    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let info = run_oxide(|| DocumentInfo::gather(self.engine.document()))?;
        json_to_py(py, &info)
    }

    fn __len__(&self) -> PyResult<usize> {
        self.page_count()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyResult<PyPageIterator> {
        Ok(PyPageIterator {
            engine: Arc::clone(&slf.engine),
            next: 1,
            total: run_oxide(|| slf.engine.page_count())?,
        })
    }

    fn __getitem__(&self, index: isize) -> PyResult<PyPage> {
        let total = self.page_count()? as isize;
        let idx = if index < 0 { total + index } else { index };
        if idx < 0 || idx >= total {
            return Err(PyIndexError::new_err("page index out of range"));
        }
        self.page((idx + 1) as usize)
    }

    fn page(&self, number: usize) -> PyResult<PyPage> {
        validate_page(&self.engine, number)?;
        Ok(PyPage {
            engine: Arc::clone(&self.engine),
            number,
        })
    }

    fn pages(&self) -> PyResult<Vec<PyPage>> {
        let total = self.page_count()?;
        Ok((1..=total)
            .map(|number| PyPage {
                engine: Arc::clone(&self.engine),
                number,
            })
            .collect())
    }

    #[pyo3(signature = (page=None, profile="fast-text"))]
    fn extract_text(&self, page: Option<usize>, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        match page {
            Some(page) => run_oxide(|| self.engine.get_page_text_with_profile(page, profile)),
            None => all_text_with_profile(&self.engine, profile),
        }
    }

    #[pyo3(signature = (page=None))]
    fn extract_tables<'py>(&self, py: Python<'py>, page: Option<usize>) -> PyResult<Py<PyAny>> {
        if let Some(page) = page {
            let tables = run_oxide(|| self.engine.extract_tables(page))?;
            return json_to_py(py, &tables);
        }

        let mut out = Vec::new();
        for number in 1..=run_oxide(|| self.engine.page_count())? {
            let tables = run_oxide(|| self.engine.extract_tables(number))?;
            for table in tables {
                out.push(json!({"page": number, "table": table}));
            }
        }
        json_to_py(py, &out)
    }

    #[pyo3(signature = (doc_type=None, min_confidence=0.0))]
    fn extract_fields<'py>(
        &self,
        py: Python<'py>,
        doc_type: Option<&str>,
        min_confidence: f32,
    ) -> PyResult<Py<PyAny>> {
        let options = ExtractOptions {
            min_confidence,
            doc_type: match doc_type {
                Some("auto") | None => None,
                Some(value) => Some(DocType::parse(value).ok_or_else(|| {
                    PyValueError::new_err(
                        "doc_type must be auto, invoice, receipt, form, or generic",
                    )
                })?),
            },
            ..Default::default()
        };
        let fields = run_oxide(|| self.engine.extract_fields(&options))?;
        json_to_py(py, &fields)
    }

    #[pyo3(signature = (profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn document_model<'py>(
        &self,
        py: Python<'py>,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<Py<PyAny>> {
        let profile = parse_profile_py(profile)?;
        let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
        let document = run_oxide(|| self.engine.parse_document_with_profile(profile, &options))?;
        let value: serde_json::Value = serde_json::from_str(&document.to_json())
            .map_err(|err| OxideError::new_err(format!("document JSON error: {err}")))?;
        json_to_py(py, &value)
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn to_markdown<'py>(
        &self,
        detect_headings: bool,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        if detect_headings {
            let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
            let document =
                run_oxide(|| self.engine.parse_document_with_profile(profile, &options))?;
            Ok(document.to_markdown(&SerializeOptions::default()))
        } else {
            all_text_with_profile(&self.engine, profile)
        }
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn markdown<'py>(
        &self,
        detect_headings: bool,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        self.to_markdown(detect_headings, profile, ocr, ocr_lang, ocr_dpi)
    }

    #[pyo3(signature = (profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn to_html<'py>(
        &self,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
        let document = run_oxide(|| self.engine.parse_document_with_profile(profile, &options))?;
        Ok(document.to_html(&SerializeOptions::default()))
    }

    #[pyo3(signature = (page, dpi=150))]
    fn render(&self, page: usize, dpi: u32) -> PyResult<Vec<u8>> {
        run_oxide(|| self.engine.render_page_png_fast(page, dpi))
    }

    // ── Report surfaces (shared oxide_engine::sdk facade) ────────────────────
    //
    // Each returns a Python dict parsed from the SDK's versioned-JSON envelope
    // `{"schema_version", "kind", "report"}`. The same facade backs the C ABI,
    // so a report requested from Python and from C is byte-identical JSON.

    /// Security report: encryption, signatures, risky active content, findings.
    fn security_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::security_report_json(bytes, None))
    }

    /// Risky active-content inventory (JavaScript, launch/URI actions, etc.).
    fn risky_content_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::risky_content_report_json(bytes, None))
    }

    /// Parser diagnostics: repair/xref/revisions/linearization/encryption.
    /// `mode` is one of "strict" | "repair" | "audit".
    #[pyo3(signature = (mode="repair"))]
    fn parser_report<'py>(&self, py: Python<'py>, mode: &str) -> PyResult<Py<PyAny>> {
        let mode = mode.to_string();
        self.report_json(py, |bytes| {
            sdk::parser_report_json(bytes, Some(&mode), None)
        })
    }

    /// Color / prepress report. `profile` is "generic" | "pdfa" | "pdfx".
    #[pyo3(signature = (profile="generic"))]
    fn color_report<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| sdk::color_report_json(bytes, Some(&profile)))
    }

    /// PDF/A validation report. `profile` in pdfa1b/pdfa2b/pdfa2a/pdfa3b/pdfa3a.
    #[pyo3(signature = (profile="pdfa2b"))]
    fn validate_pdfa<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| {
            sdk::pdfa_validation_json(bytes, Some(&profile), None)
        })
    }

    /// PDF/UA (accessibility) validation report.
    fn validate_pdfua<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::pdfua_validation_json(bytes, None))
    }

    /// Standards-profile report. `profile` in pdfa/pdfua/pdfx/security/all.
    #[pyo3(signature = (profile="all"))]
    fn validate<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| {
            sdk::standards_profile_json(bytes, Some(&profile), None)
        })
    }

    /// Combined interactive report (forms + annotations + page operations).
    fn interactive_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::interactive_report_json(bytes, None))
    }

    /// AcroForm field inventory (trees, inheritance, widgets, XFA status).
    fn forms_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::forms_report_json(bytes, None))
    }

    /// Prompt 16 bounded XFA packet inventory and XML-safety report.
    fn xfa_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_report_json(bytes, None))
    }

    /// Prompt 16 static XFA fields/datasets/layout/provenance extraction.
    fn xfa_extract<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_extract_json(bytes, None))
    }

    /// Prompt 16 script/event inventory and fail-closed default policy.
    fn xfa_script_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_script_report_json(bytes, None))
    }

    /// Prompt 16 XFA-specific security/signature/redaction posture.
    fn xfa_security_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_security_report_json(bytes, None))
    }

    /// Bounded minimal dynamic XFA runtime report.
    #[pyo3(signature = (script_policy="disabled", execute_events=false))]
    fn xfa_runtime_report<'py>(
        &self,
        py: Python<'py>,
        script_policy: &str,
        execute_events: bool,
    ) -> PyResult<Py<PyAny>> {
        let script_policy = script_policy.to_string();
        self.report_json(py, |bytes| {
            sdk::xfa_runtime_report_json(bytes, Some(&script_policy), execute_events, None)
        })
    }

    /// Annotation inventory (kinds, quads, appearance status, unsafe actions).
    fn annotations_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::annotation_report_json(bytes, None))
    }

    /// Page-operations report (boxes, labels, destinations, preservation risk).
    fn pages_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::page_operations_report_json(bytes, None))
    }

    /// Signature report (validity, trust, coverage, LTV, certificate).
    fn signature_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::signature_report_json(bytes, None))
    }

    /// Font inventory (name, type, embedding status, subsetting, encoding).
    fn font_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::font_report_json(bytes, None))
    }

    /// Text semantic model: pages → blocks → paragraphs → lines → words/spans
    /// with geometry, confidence, provenance, and reading order. `pages` empty
    /// or None means all pages.
    #[pyo3(signature = (pages=None))]
    fn text_semantic<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| sdk::text_semantic_json(bytes, &pages, None))
    }

    /// RAG-ready semantic chunk set (canonical model → chunks).
    fn chunks<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::chunk_report_json(bytes, None))
    }

    /// Prompt 15 provenance-aware RAG chunks with stable hashes, table/cell,
    /// CJK dictionary, structure/MCID, ParentTree, and security metadata.
    #[pyo3(signature = (pages=None))]
    fn advanced_chunks<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| {
            sdk::advanced_chunk_report_json(bytes, &pages, None)
        })
    }

    /// Full Prompt 15 semantic binding bundle as a versioned Python dictionary.
    #[pyo3(signature = (pages=None))]
    fn semantic_bundle<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| {
            sdk::semantic_binding_report_json(bytes, &pages, None)
        })
    }

    /// Semantic text and CJK dictionary-token search with source provenance.
    #[pyo3(signature = (query, pages=None))]
    fn semantic_search<'py>(
        &self,
        py: Python<'py>,
        query: &str,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        let query = query.to_string();
        self.report_json(py, |bytes| {
            sdk::semantic_search_report_json(bytes, &pages, &query, None)
        })
    }

    /// TableFormer/Table Transformer hook and backend availability status.
    fn table_proposal_status<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let json = run_oxide(sdk::table_proposal_status_json)?;
        parse_json_str(py, &json)
    }

    /// Tagged-structure semantic document (structure tree / MCID model).
    #[pyo3(signature = (pages=None))]
    fn semantic_document<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| sdk::semantic_document_json(bytes, &pages, None))
    }

    // ── Output-producing operations (return (bytes, report) tuples) ──────────

    /// Render a supported XFA preview as PDF overlay bytes plus a versioned
    /// report. The result can be written to `output`.
    #[pyo3(signature = (script_policy="disabled", execute_events=false, dpi=72, output=None))]
    fn xfa_render<'py>(
        &self,
        py: Python<'py>,
        script_policy: &str,
        execute_events: bool,
        dpi: u32,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| {
            sdk::xfa_render_preview_json(&bytes, Some(script_policy), execute_events, dpi, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Flatten the supported static XFA subset under an explicit Prompt 16 mode.
    #[pyo3(signature = (mode="flatten_supported_static", output=None))]
    fn xfa_flatten<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| sdk::xfa_flatten_json(&bytes, Some(mode), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Remove or neutralize XFA active content under an explicit policy.
    #[pyo3(signature = (mode="remove_scripts_events_connections", output=None))]
    fn xfa_sanitize<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| sdk::xfa_sanitize_json(&bytes, Some(mode), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Sanitize: remove active/risky content and re-scan. `policy` is one of
    /// "strict" | "balanced" | "preserve-visual". Returns `(bytes, report)` and
    /// optionally writes the sanitized PDF to `output`.
    #[pyo3(signature = (policy="balanced", output=None))]
    fn sanitize<'py>(
        &self,
        py: Python<'py>,
        policy: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| sdk::sanitize_json(&bytes, Some(policy), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Canonicalize: deterministic full-rewrite copy + audit report. Returns
    /// `(bytes, report)`; `date_epoch` fixes the source date epoch.
    #[pyo3(signature = (date_epoch=None, output=None))]
    fn canonicalize<'py>(
        &self,
        py: Python<'py>,
        date_epoch: Option<i64>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| sdk::canonicalize_json(&bytes, date_epoch, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Redact every occurrence of `terms` (case-insensitive), full-rewrite, and
    /// verify absence. Returns `(bytes, report)`. `strict=True` raises if a term
    /// survives. Writes to `output` when given.
    #[pyo3(signature = (terms, strict=false, output=None))]
    fn redact<'py>(
        &self,
        py: Python<'py>,
        terms: Vec<String>,
        strict: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_oxide(|| sdk::redact_terms_json(&bytes, &terms, strict, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }
}

impl PyDocument {
    /// The original file bytes backing this document (copied out of the reader).
    fn file_bytes(&self) -> Vec<u8> {
        self.engine.document().reader().file_bytes().to_vec()
    }

    /// Run a facade report over this document's bytes and parse it to a dict.
    fn report_json<'py, F>(&self, py: Python<'py>, f: F) -> PyResult<Py<PyAny>>
    where
        F: FnOnce(&[u8]) -> oxide_engine::Result<String>,
    {
        let bytes = self.file_bytes();
        let json = run_oxide(|| f(&bytes))?;
        parse_json_str(py, &json)
    }
}

#[pymethods]
impl PyPage {
    #[getter]
    fn number(&self) -> usize {
        self.number
    }

    #[getter]
    fn text(&self) -> PyResult<String> {
        run_oxide(|| self.engine.get_page_text(self.number))
    }

    #[getter]
    fn words<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_words(py, &self.engine, self.number)
    }

    #[getter]
    fn tables<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let tables = run_oxide(|| self.engine.extract_tables(self.number))?;
        json_to_py(py, &tables)
    }

    #[getter]
    fn images<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_images(py, &self.engine, self.number)
    }

    #[pyo3(signature = (profile="fast-text"))]
    fn text_with_profile(&self, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        run_oxide(|| self.engine.get_page_text_with_profile(self.number, profile))
    }

    #[pyo3(signature = (x0, y0, x1, y1))]
    fn region(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> PyResult<PyRegionPage> {
        let region = run_oxide(|| PageRegion::new(x0, y0, x1, y1))?;
        let region = run_oxide(|| self.engine.clamp_region_to_page(self.number, region))?;
        Ok(PyRegionPage {
            engine: Arc::clone(&self.engine),
            number: self.number,
            region,
        })
    }

    #[pyo3(signature = (x0, y0, x1, y1))]
    fn within(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> PyResult<PyRegionPage> {
        self.region(x0, y0, x1, y1)
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text"))]
    fn markdown(&self, detect_headings: bool, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        if !detect_headings {
            return run_oxide(|| self.engine.get_page_text_with_profile(self.number, profile));
        }
        let options = ParseOptions {
            pages: vec![self.number],
            ..Default::default()
        };
        let document = run_oxide(|| self.engine.parse_document_with_profile(profile, &options))?;
        Ok(document.to_markdown(&SerializeOptions::default()))
    }

    #[pyo3(signature = (dpi=150))]
    fn render(&self, dpi: u32) -> PyResult<Vec<u8>> {
        run_oxide(|| self.engine.render_page_png_fast(self.number, dpi))
    }
}

#[pymethods]
impl PyRegionPage {
    #[getter]
    fn number(&self) -> usize {
        self.number
    }

    #[getter]
    fn bbox(&self) -> Vec<f64> {
        self.region.as_array().to_vec()
    }

    #[getter]
    fn text(&self) -> PyResult<String> {
        run_oxide(|| self.engine.extract_text_in_region(self.number, self.region))
    }

    #[getter]
    fn words<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let words = run_oxide(|| {
            self.engine
                .extract_words_in_region(self.number, self.region)
        })?;
        json_to_py(py, &words)
    }

    #[getter]
    fn tables<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let tables = run_oxide(|| {
            self.engine
                .extract_tables_in_region(self.number, self.region)
        })?;
        json_to_py(py, &tables)
    }

    #[getter]
    fn images<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_region_images(py, &self.engine, self.number, self.region)
    }
}

#[pymethods]
impl PyPageIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<PyPage>> {
        if self.next > self.total {
            return Ok(None);
        }
        let page = PyPage {
            engine: Arc::clone(&self.engine),
            number: self.next,
        };
        self.next += 1;
        Ok(Some(page))
    }
}

#[pyfunction]
#[pyo3(signature = (source, password=None))]
fn open(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<PyDocument> {
    open_impl(source, password)
}

#[pyfunction]
#[pyo3(signature = (inputs, output=None, passwords=None))]
fn merge_pdfs(
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    passwords: Option<Vec<String>>,
) -> PyResult<Vec<u8>> {
    if inputs.is_empty() {
        return Err(PyValueError::new_err("inputs must not be empty"));
    }
    let mut engines = Vec::with_capacity(inputs.len());
    for (idx, path) in inputs.iter().enumerate() {
        let password = passwords
            .as_ref()
            .and_then(|values| values.get(idx))
            .map(String::as_str);
        engines.push(open_engine_path(path, password)?);
    }
    let mut specs = Vec::with_capacity(engines.len());
    for engine in &engines {
        let total = run_oxide(|| engine.page_count())?;
        specs.push((engine.document(), (1..=total).collect::<Vec<_>>()));
    }
    let bytes = run_oxide(|| oxide_engine::build_merged(&specs))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, pages, output=None, password=None))]
fn extract_pages(
    pdf: PathBuf,
    pages: &str,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let bytes = run_oxide(|| engine.extract_pages(&selected))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, angle, pages="all", relative=false, output=None, password=None))]
fn rotate_pdf(
    pdf: PathBuf,
    angle: i32,
    pages: &str,
    relative: bool,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let rotation = if relative {
        oxide_engine::Rotation::Relative(angle)
    } else {
        oxide_engine::Rotation::Absolute(angle)
    };
    let bytes = run_oxide(|| oxide_engine::rotate_pages(&engine, &selected, rotation))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn decrypt_pdf(pdf: PathBuf, output: Option<PathBuf>, password: Option<&str>) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_oxide(|| oxide_engine::decrypt_pdf(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, user_password="", owner_password=None, output=None, algo="aes256", permissions=-1, password=None))]
fn encrypt_pdf(
    pdf: PathBuf,
    user_password: &str,
    owner_password: Option<&str>,
    output: Option<PathBuf>,
    algo: &str,
    permissions: i32,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    use oxide_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};
    let engine = open_engine_path(&pdf, password)?;
    let algorithm = EncryptAlgorithm::parse(algo)
        .ok_or_else(|| PyValueError::new_err("algo must be aes256, aes128, or rc4"))?;
    let owner = owner_password.unwrap_or(user_password);
    let params = EncryptParams {
        user_password: secret_bytes(user_password.as_bytes().to_vec()),
        owner_password: secret_bytes(owner.as_bytes().to_vec()),
        permissions,
        algorithm,
        encrypt_metadata: true,
    };
    let bytes = run_oxide(|| oxide_engine::encrypt(&engine, &params))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn optimize_pdf(
    pdf: PathBuf,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let (bytes, _) = run_oxide(|| oxide_engine::optimize(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn repair_pdf(pdf: PathBuf, output: Option<PathBuf>, password: Option<&str>) -> PyResult<Vec<u8>> {
    let bytes = std::fs::read(&pdf).map_err(|err| OxideError::new_err(err.to_string()))?;
    let password = password.unwrap_or("").as_bytes().to_vec();
    let repaired = run_oxide(|| oxide_engine::repair(bytes, &password))?;
    write_optional(&output, &repaired)?;
    Ok(repaired)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn linearize_pdf(
    pdf: PathBuf,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_oxide(|| oxide_engine::linearize(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, out_dir, pages="all", dpi=150, quality=85, format="jpg", password=None))]
#[allow(clippy::too_many_arguments)]
fn pdf_to_images<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    out_dir: PathBuf,
    pages: &str,
    dpi: u32,
    quality: u8,
    format: &str,
    password: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let format = oxide_engine::RasterImageFormat::parse(format)
        .ok_or_else(|| PyValueError::new_err("format must be jpg or png"))?;
    let results = run_oxide(|| {
        oxide_engine::export_pdf_pages_to_images(
            &engine, &out_dir, &selected, dpi, format, quality, "page",
        )
    })?;
    json_to_py(py, &results)
}

#[pyfunction]
#[pyo3(signature = (images, output=None, page_size="a4", margin=0.0))]
fn images_to_pdf(
    images: Vec<PathBuf>,
    output: Option<PathBuf>,
    page_size: &str,
    margin: f64,
) -> PyResult<Vec<u8>> {
    let page_size = oxide_engine::ImagePdfPageSize::parse(page_size)
        .ok_or_else(|| PyValueError::new_err("page_size must be a4, letter, or size-to-image"))?;
    let bytes = run_oxide(|| {
        oxide_engine::images_to_pdf_from_paths(
            &images,
            oxide_engine::ImageToPdfOptions {
                page_size,
                margin_points: margin,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, layout="pages", password=None))]
fn pdf_to_xlsx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    layout: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let layout = oxide_engine::XlsxLayout::parse(layout)
        .ok_or_else(|| PyValueError::new_err("layout must be pages or tables"))?;
    let bytes =
        run_oxide(|| oxide_engine::pdf_to_xlsx(&engine, &oxide_engine::XlsxOptions { layout }))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, include_images=true, password=None))]
fn pdf_to_pptx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    include_images: bool,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_oxide(|| {
        oxide_engine::pdf_to_pptx(&engine, &oxide_engine::PptxOptions { include_images })
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, include_images=true, password=None))]
fn pdf_to_docx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    include_images: bool,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_oxide(|| {
        oxide_engine::pdf_to_docx(
            &engine,
            &oxide_engine::DocxOptions {
                include_images,
                layout: oxide_engine::DocxLayout::Flowing,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (docx, output=None))]
fn docx_to_pdf(docx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&docx)?;
    let bytes = run_oxide(|| {
        oxide_engine::docx_to_pdf(&input, &oxide_engine::OfficeToPdfOptions::default())
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (xlsx, output=None))]
fn xlsx_to_pdf(xlsx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&xlsx)?;
    let bytes = run_oxide(|| {
        oxide_engine::xlsx_to_pdf(&input, &oxide_engine::OfficeToPdfOptions::default())
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pptx, output=None))]
fn pptx_to_pdf(pptx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&pptx)?;
    let bytes = run_oxide(|| {
        oxide_engine::pptx_to_pdf(&input, &oxide_engine::OfficeToPdfOptions::default())
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, text=None, image=None, output=None, pages="all", position="center", opacity=0.28, rotation=45.0, font_size=64.0, color="#8c8c8c", scale=0.5, password=None))]
#[allow(clippy::too_many_arguments)]
fn watermark_pdf(
    pdf: PathBuf,
    text: Option<String>,
    image: Option<PathBuf>,
    output: Option<PathBuf>,
    pages: &str,
    position: &str,
    opacity: f64,
    rotation: f64,
    font_size: f64,
    color: &str,
    scale: f64,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    if text.is_some() == image.is_some() {
        return Err(PyValueError::new_err("pass exactly one of text or image"));
    }
    let input = read_edit_input_py(&pdf, password)?;
    let engine = run_oxide(|| ContentEngine::open_bytes(input.clone()))?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let position = parse_stamp_position_py(position)?;
    let bytes = if let Some(text) = text {
        let color = parse_rgb_color_py(color)?;
        run_oxide(|| {
            oxide_engine::watermark_text_pdf(
                input,
                &text,
                oxide_engine::TextWatermarkOptions {
                    pages: selected,
                    position,
                    opacity,
                    rotation_degrees: rotation,
                    font_size,
                    color,
                },
            )
        })?
    } else {
        let image_path = image.expect("checked above");
        let image =
            std::fs::read(&image_path).map_err(|err| OxideError::new_err(err.to_string()))?;
        run_oxide(|| {
            oxide_engine::watermark_image_pdf(
                input,
                &image,
                image_path.extension().and_then(|s| s.to_str()),
                oxide_engine::ImageWatermarkOptions {
                    pages: selected,
                    position,
                    opacity,
                    scale,
                },
            )
        })?
    };
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, pages="all", position="bottom-center", format="Page {n} of {total}", start=1, font_size=10.0, color="#333333", password=None))]
#[allow(clippy::too_many_arguments)]
fn add_page_numbers(
    pdf: PathBuf,
    output: Option<PathBuf>,
    pages: &str,
    position: &str,
    format: &str,
    start: isize,
    font_size: f64,
    color: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let input = read_edit_input_py(&pdf, password)?;
    let engine = run_oxide(|| ContentEngine::open_bytes(input.clone()))?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let position = parse_stamp_position_py(position)?;
    let color = parse_rgb_color_py(color)?;
    let bytes = run_oxide(|| {
        oxide_engine::add_page_numbers_pdf(
            input,
            oxide_engine::PageNumberOptions {
                pages: selected,
                position,
                format: format.to_string(),
                start,
                font_size,
                color,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, order="all", output=None, password=None))]
fn organize_pdf(
    pdf: PathBuf,
    order: &str,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_oxide(|| engine.page_count())?;
    let selected = parse_pages_spec_py(order, total)?;
    let bytes = run_oxide(|| oxide_engine::organize_pdf(&engine, &selected))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, password=None))]
fn fonts<'py>(py: Python<'py>, pdf: PathBuf, password: Option<&str>) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let fonts = run_oxide(|| engine.list_fonts())?;
    json_to_py(py, &fonts)
}

#[pyfunction]
#[pyo3(signature = (pdf, password=None))]
fn verify_signatures<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    password: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let sigs = run_oxide(|| engine.verify_signatures())?;
    json_to_py(py, &sigs)
}

/// Feature / capability report: SDK version, envelope version, and which
/// optional engine capabilities are compiled into this build. No document input.
#[pyfunction]
fn feature_report(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let json = run_oxide(sdk::feature_report_json)?;
    parse_json_str(py, &json)
}

/// Decode budget report for a hypothetical image stream: shows the decode-limit
/// / decompression-bomb policy that decoding a `filter`/`width`/`height`/
/// `components` image would trigger. No document input.
#[pyfunction]
#[pyo3(signature = (filter, width, height, components=3))]
fn decode_budget_report(
    py: Python<'_>,
    filter: &str,
    width: u32,
    height: u32,
    components: u8,
) -> PyResult<Py<PyAny>> {
    let filter = filter.to_string();
    let json = run_oxide(|| sdk::decode_budget_report_json(&filter, width, height, components))?;
    parse_json_str(py, &json)
}

/// Codec isolation diagnostic report over caller-supplied encoded stream bytes.
/// `policy` is "in_process", "isolated_preferred", "isolated_required",
/// "report_only", or "disabled".
#[pyfunction]
#[pyo3(signature = (filter, data, policy="in_process"))]
fn codec_isolation_report(
    py: Python<'_>,
    filter: &str,
    data: Vec<u8>,
    policy: &str,
) -> PyResult<Py<PyAny>> {
    let filter = filter.to_string();
    let policy = policy.to_string();
    let json = run_oxide(|| sdk::codec_isolation_report_json(&filter, &data, Some(&policy)))?;
    parse_json_str(py, &json)
}

/// Resource-dedup report over caller-supplied resource byte buffers. Groups
/// byte-identical resources by content digest (the deterministic-writer dedup
/// evidence). Pass a list of `bytes`.
#[pyfunction]
fn resource_dedup_report(py: Python<'_>, resources: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
    let json = run_oxide(|| sdk::resource_dedup_report_json(&resources))?;
    parse_json_str(py, &json)
}

#[pymodule]
fn oxide(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("OxideError", py.get_type::<OxideError>())?;
    module.add_class::<PyDocument>()?;
    module.add_class::<PyPage>()?;
    module.add_class::<PyRegionPage>()?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(merge_pdfs, module)?)?;
    module.add_function(wrap_pyfunction!(extract_pages, module)?)?;
    module.add_function(wrap_pyfunction!(rotate_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(decrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(encrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(optimize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(repair_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(linearize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_images, module)?)?;
    module.add_function(wrap_pyfunction!(images_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_xlsx, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_pptx, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_docx, module)?)?;
    module.add_function(wrap_pyfunction!(docx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(xlsx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pptx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(watermark_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(add_page_numbers, module)?)?;
    module.add_function(wrap_pyfunction!(organize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(fonts, module)?)?;
    module.add_function(wrap_pyfunction!(verify_signatures, module)?)?;
    module.add_function(wrap_pyfunction!(feature_report, module)?)?;
    module.add_function(wrap_pyfunction!(decode_budget_report, module)?)?;
    module.add_function(wrap_pyfunction!(codec_isolation_report, module)?)?;
    module.add_function(wrap_pyfunction!(resource_dedup_report, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add(
        "__report_envelope_version__",
        oxide_engine::REPORT_ENVELOPE_VERSION,
    )?;
    Ok(())
}

fn open_impl(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<PyDocument> {
    if let Ok(data) = source.extract::<Vec<u8>>() {
        let engine = if let Some(password) = password {
            run_oxide(|| ContentEngine::open_bytes_with_password(data, password.as_bytes()))?
        } else {
            run_oxide(|| ContentEngine::open_bytes(data))?
        };
        return Ok(PyDocument {
            engine: Arc::new(engine),
        });
    }

    let py = source.py();
    let os = py.import("os")?;
    let path_obj = os
        .call_method1("fspath", (source,))
        .map_err(|_| PyTypeError::new_err("source must be a filesystem path or bytes"))?;
    let path: PathBuf = path_obj.extract()?;
    let engine = if let Some(password) = password {
        run_oxide(|| ContentEngine::open_path_with_password(path, password.as_bytes()))?
    } else {
        run_oxide(|| ContentEngine::open_path(path))?
    };
    Ok(PyDocument {
        engine: Arc::new(engine),
    })
}

fn open_engine_path(path: &PathBuf, password: Option<&str>) -> PyResult<ContentEngine> {
    if let Some(password) = password {
        run_oxide(|| ContentEngine::open_path_with_password(path, password.as_bytes()))
    } else {
        run_oxide(|| ContentEngine::open_path(path))
    }
}

fn write_optional(path: &Option<PathBuf>, bytes: &[u8]) -> PyResult<()> {
    if let Some(path) = path {
        std::fs::write(path, bytes).map_err(|err| OxideError::new_err(err.to_string()))?;
    }
    Ok(())
}

fn read_edit_input_py(path: &PathBuf, password: Option<&str>) -> PyResult<Vec<u8>> {
    if password.is_some() {
        let engine = open_engine_path(path, password)?;
        run_oxide(|| oxide_engine::decrypt_pdf(&engine))
    } else {
        std::fs::read(path).map_err(|err| OxideError::new_err(err.to_string()))
    }
}

fn parse_pages_spec_py(spec: &str, total: usize) -> PyResult<Vec<usize>> {
    if total == 0 {
        return Err(PyValueError::new_err("document has no pages"));
    }
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok((1..=total).collect());
    }
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start
                .trim()
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            let end: usize = end
                .trim()
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            if start == 0 || end == 0 || start > end {
                return Err(PyValueError::new_err(format!(
                    "invalid page range '{part}'"
                )));
            }
            for page in start..=end.min(total) {
                out.push(page);
            }
        } else {
            let page: usize = part
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            if page == 0 || page > total {
                return Err(PyValueError::new_err(format!(
                    "page {page} out of range 1..={total}"
                )));
            }
            out.push(page);
        }
    }
    if out.is_empty() {
        return Err(PyValueError::new_err("page selection matched no pages"));
    }
    Ok(out)
}

fn parse_stamp_position_py(value: &str) -> PyResult<oxide_engine::StampPosition> {
    oxide_engine::StampPosition::parse(value)
        .ok_or_else(|| PyValueError::new_err(format!("unknown position '{value}'")))
}

fn parse_rgb_color_py(value: &str) -> PyResult<oxide_engine::RgbColor> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(PyValueError::new_err(format!(
            "color '{value}' must be #RRGGBB"
        )));
    }
    let r = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    Ok(oxide_engine::RgbColor {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
    })
}

fn run_oxide<T, F>(operation: F) -> PyResult<T>
where
    F: FnOnce() -> oxide_engine::Result<T>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(OxideError::new_err(err.to_string())),
        Err(_) => Err(OxideError::new_err("Rust panic while processing PDF")),
    }
}

fn validate_page(engine: &ContentEngine, page: usize) -> PyResult<()> {
    let total = run_oxide(|| engine.page_count())?;
    if page == 0 || page > total {
        Err(PyIndexError::new_err(format!(
            "page {page} out of range 1..={total}"
        )))
    } else {
        Ok(())
    }
}

fn parse_profile_py(name: &str) -> PyResult<ExtractionProfile> {
    ExtractionProfile::parse(name).ok_or_else(|| {
        PyValueError::new_err(
            "profile must be fast-text, layout-faithful, tables-focused, or rag-chunks",
        )
    })
}

/// Build [`ParseOptions`] with an optional Python OCR backend wired in. When
/// `ocr` is `None`, returns the default options (OCR off — scanned pages degrade
/// to the placeholder, matching the pre-OCR behavior). When `ocr` is a Python
/// object, wraps it as a [`PyOcrEngine`] and enables the `Auto` policy so scanned
/// pages are recognized through it. `ocr_lang` is split on `+`/`,` (falling back
/// to `eng`); `ocr_dpi` sets the rasterization DPI (~300 is the sweet spot).
fn parse_options_with_ocr(
    ocr: Option<&Bound<'_, PyAny>>,
    ocr_lang: &str,
    ocr_dpi: u32,
) -> PyResult<ParseOptions> {
    let mut options = ParseOptions::default();
    let Some(obj) = ocr else {
        return Ok(options);
    };
    let backend = PyOcrEngine::new(obj)?;
    let langs: Vec<String> = ocr_lang
        .split(['+', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    options.ocr = Some(Arc::new(backend));
    options.ocr_policy = OcrPolicy::Auto;
    options.ocr_options = oxide_engine::OcrOptions {
        languages: if langs.is_empty() {
            vec!["eng".to_string()]
        } else {
            langs
        },
        dpi: ocr_dpi.max(1),
        psm: None,
    };
    options.ocr_dpi = ocr_dpi.max(1);
    Ok(options)
}

fn all_text_with_profile(engine: &ContentEngine, profile: ExtractionProfile) -> PyResult<String> {
    let total = run_oxide(|| engine.page_count())?;
    let mut pages = Vec::new();
    for page in 1..=total {
        pages.push(run_oxide(|| {
            engine.get_page_text_with_profile(page, profile)
        })?);
    }
    Ok(pages.join("\n"))
}

fn json_to_py<'py, T: Serialize>(py: Python<'py>, value: &T) -> PyResult<Py<PyAny>> {
    let raw = serde_json::to_string(value)
        .map_err(|err| OxideError::new_err(format!("JSON serialization error: {err}")))?;
    parse_json_str(py, &raw)
}

/// Parse a JSON string (an SDK-facade envelope) into a native Python object.
fn parse_json_str<'py>(py: Python<'py>, raw: &str) -> PyResult<Py<PyAny>> {
    let json = py.import("json")?;
    Ok(json.call_method1("loads", (raw,))?.unbind())
}

fn page_words<'py>(py: Python<'py>, engine: &ContentEngine, page: usize) -> PyResult<Py<PyAny>> {
    let words = run_oxide(|| engine.extract_page_words(page))?;
    json_to_py(py, &words)
}

fn page_images<'py>(py: Python<'py>, engine: &ContentEngine, page: usize) -> PyResult<Py<PyAny>> {
    let options = ImageLocateOptions {
        pages: Some(vec![page]),
        ..Default::default()
    };
    let images = run_oxide(|| engine.find_all_images(&options))?;
    let list = PyList::empty(py);
    for image in images {
        let dict = PyDict::new(py);
        dict.set_item("page", image.page_number)?;
        dict.set_item("name", image.xobject_name.clone())?;
        dict.set_item("width", image.width)?;
        dict.set_item("height", image.height)?;
        dict.set_item("bits_per_component", image.bits_per_component)?;
        dict.set_item("color_space", image.color_space.clone())?;
        dict.set_item("filters", image.filter.clone())?;
        dict.set_item("inline", image.is_inline)?;
        dict.set_item("mask", image.is_mask)?;
        dict.set_item("soft_mask", image.is_smask)?;
        match run_oxide(|| engine.extract_image_bytes(&image, ImageOutputFormat::Png, None)) {
            Ok(bytes) => {
                dict.set_item("format", "png")?;
                dict.set_item("data", PyBytes::new(py, &bytes))?;
            }
            Err(err) => {
                dict.set_item("format", py.None())?;
                dict.set_item("data", py.None())?;
                dict.set_item("error", err.to_string())?;
            }
        }
        list.append(dict)?;
    }
    Ok(list.into_any().unbind())
}

fn page_region_images<'py>(
    py: Python<'py>,
    engine: &ContentEngine,
    page: usize,
    region: PageRegion,
) -> PyResult<Py<PyAny>> {
    let images = run_oxide(|| engine.find_page_images_in_region(page, region))?;
    let list = PyList::empty(py);
    for placed in images {
        let image = placed.image;
        let dict = PyDict::new(py);
        dict.set_item("page", image.page_number)?;
        dict.set_item("name", image.xobject_name.clone())?;
        dict.set_item("bbox", placed.bbox.to_vec())?;
        dict.set_item("width", image.width)?;
        dict.set_item("height", image.height)?;
        dict.set_item("bits_per_component", image.bits_per_component)?;
        dict.set_item("color_space", image.color_space.clone())?;
        dict.set_item("filters", image.filter.clone())?;
        dict.set_item("inline", image.is_inline)?;
        dict.set_item("mask", image.is_mask)?;
        dict.set_item("soft_mask", image.is_smask)?;
        match run_oxide(|| engine.extract_image_bytes(&image, ImageOutputFormat::Png, None)) {
            Ok(bytes) => {
                dict.set_item("format", "png")?;
                dict.set_item("data", PyBytes::new(py, &bytes))?;
            }
            Err(err) => {
                dict.set_item("format", py.None())?;
                dict.set_item("data", py.None())?;
                dict.set_item("error", err.to_string())?;
            }
        }
        list.append(dict)?;
    }
    Ok(list.into_any().unbind())
}
