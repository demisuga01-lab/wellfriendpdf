use std::path::PathBuf;
use std::sync::Arc;

use oxide_engine::{
    ContentEngine, DocType, DocumentInfo, ExtractOptions, ImageLocateOptions, ImageOutputFormat,
    ParseOptions, SerializeOptions,
};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyList, PyModule, PyType};
use serde::Serialize;
use serde_json::json;

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

    #[pyo3(signature = (page=None))]
    fn extract_text(&self, page: Option<usize>) -> PyResult<String> {
        match page {
            Some(page) => run_oxide(|| self.engine.get_page_text(page)),
            None => all_text(&self.engine),
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

    fn document_model<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let document = run_oxide(|| self.engine.parse_document(&ParseOptions::default()))?;
        let value: serde_json::Value = serde_json::from_str(&document.to_json())
            .map_err(|err| OxideError::new_err(format!("document JSON error: {err}")))?;
        json_to_py(py, &value)
    }

    #[pyo3(signature = (detect_headings=true))]
    fn to_markdown(&self, detect_headings: bool) -> PyResult<String> {
        let document = run_oxide(|| self.engine.parse_document(&ParseOptions::default()))?;
        if detect_headings {
            Ok(document.to_markdown(&SerializeOptions::default()))
        } else {
            all_text(&self.engine)
        }
    }

    #[pyo3(signature = (detect_headings=true))]
    fn markdown(&self, detect_headings: bool) -> PyResult<String> {
        self.to_markdown(detect_headings)
    }

    fn to_html(&self) -> PyResult<String> {
        let document = run_oxide(|| self.engine.parse_document(&ParseOptions::default()))?;
        Ok(document.to_html(&SerializeOptions::default()))
    }

    #[pyo3(signature = (page, dpi=150))]
    fn render(&self, page: usize, dpi: u32) -> PyResult<Vec<u8>> {
        run_oxide(|| self.engine.render_page_png_fast(page, dpi))
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

    #[pyo3(signature = (detect_headings=true))]
    fn markdown(&self, detect_headings: bool) -> PyResult<String> {
        if !detect_headings {
            return self.text();
        }
        let options = ParseOptions {
            pages: vec![self.number],
            ..Default::default()
        };
        let document = run_oxide(|| self.engine.parse_document(&options))?;
        Ok(document.to_markdown(&SerializeOptions::default()))
    }

    #[pyo3(signature = (dpi=150))]
    fn render(&self, dpi: u32) -> PyResult<Vec<u8>> {
        run_oxide(|| self.engine.render_page_png_fast(self.number, dpi))
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

#[pymodule]
fn oxide(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("OxideError", py.get_type::<OxideError>())?;
    module.add_class::<PyDocument>()?;
    module.add_class::<PyPage>()?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
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

fn all_text(engine: &ContentEngine) -> PyResult<String> {
    let pages = run_oxide(|| engine.get_all_text())?;
    Ok(pages
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn json_to_py<'py, T: Serialize>(py: Python<'py>, value: &T) -> PyResult<Py<PyAny>> {
    let raw = serde_json::to_string(value)
        .map_err(|err| OxideError::new_err(format!("JSON serialization error: {err}")))?;
    let json = py.import("json")?;
    Ok(json.call_method1("loads", (raw,))?.unbind())
}

fn page_words<'py>(py: Python<'py>, engine: &ContentEngine, page: usize) -> PyResult<Py<PyAny>> {
    let layout = run_oxide(|| engine.analyze_page_layout(page))?;
    let mut words = Vec::new();
    for block in layout.blocks {
        for line in block.lines {
            let parts: Vec<&str> = line.text.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }
            let total_chars = parts
                .iter()
                .map(|w| w.chars().count())
                .sum::<usize>()
                .max(1);
            let mut offset = 0usize;
            let width = (line.bbox.x1 - line.bbox.x0).max(0.0);
            for word in parts {
                let len = word.chars().count();
                let x0 = line.bbox.x0 + width * (offset as f64 / total_chars as f64);
                offset += len;
                let x1 = line.bbox.x0 + width * (offset as f64 / total_chars as f64);
                words.push(json!({
                    "text": word,
                    "page": page,
                    "x0": x0,
                    "y0": line.bbox.y0,
                    "x1": x1,
                    "y1": line.bbox.y1,
                }));
            }
        }
    }
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
