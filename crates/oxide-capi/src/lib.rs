//! C ABI for oxide-engine.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;
use std::sync::Arc;

use oxide_engine::{
    sdk, ContentEngine, DocType, ExtractOptions, OcrPolicy, ParseOptions, Result as OxideResult,
    TextExtractor,
};

pub mod ocr_backend;
pub use ocr_backend::{CAbiOcrEngine, OxideOcrBackend, OxideOcrEmitWordFn, OxideOcrRecognizeFn};

pub const OXIDE_STATUS_OK: c_int = 0;
pub const OXIDE_STATUS_NULL: c_int = 1;
pub const OXIDE_STATUS_ERROR: c_int = 2;
pub const OXIDE_STATUS_PANIC: c_int = 3;

#[repr(C)]
pub struct OxideDocument {
    engine: ContentEngine,
    /// An optional OCR backend registered via `oxide_document_set_ocr_backend`.
    /// When present, the `*_with_ocr` parse functions route scanned pages
    /// through it; the plain parse functions ignore it (digital-born only).
    ocr: Option<Arc<dyn oxide_engine::OcrEngine>>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OxideBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl OxideBuffer {
    fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }
}

/// Opens a PDF document from an in-memory byte slice.
///
/// # Safety
///
/// `data` must point to `len` readable bytes. If `error_out` is non-null, it
/// must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_open_from_bytes(
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> *mut OxideDocument {
    unsafe { open_document_from_parts(data, len, ptr::null(), 0, error_out) }
}

/// Opens a PDF document from bytes with an optional UTF-8 password.
///
/// `password == NULL && password_len == 0` means no password was supplied.
/// `password != NULL && password_len == 0` means an explicit empty password was
/// supplied. The password is used only during this open call and is not logged
/// or retained by the C ABI wrapper.
///
/// # Safety
///
/// `data` must point to `len` readable bytes. If `password` is non-null, it
/// must point to `password_len` readable bytes. Passing `password == NULL` with
/// `password_len > 0` returns an error. If `error_out` is non-null, it must be
/// writable and any returned string must be freed with `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_open_from_bytes_with_password(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    error_out: *mut *mut c_char,
) -> *mut OxideDocument {
    unsafe { open_document_from_parts(data, len, password, password_len, error_out) }
}

unsafe fn open_document_from_parts(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    error_out: *mut *mut c_char,
) -> *mut OxideDocument {
    clear_error(error_out);
    if data.is_null() {
        set_error(error_out, "data pointer is null");
        return ptr::null_mut();
    }
    if password.is_null() && password_len > 0 {
        set_error(
            error_out,
            "password pointer is null but password_len is non-zero",
        );
        return ptr::null_mut();
    }

    match catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { slice::from_raw_parts(data, len) }.to_vec();
        if password.is_null() {
            ContentEngine::open_bytes(bytes)
        } else {
            let password = unsafe { slice::from_raw_parts(password, password_len) };
            ContentEngine::open_bytes_with_password(bytes, password)
        }
    })) {
        Ok(Ok(engine)) => Box::into_raw(Box::new(OxideDocument { engine, ocr: None })),
        Ok(Err(err)) => {
            set_error(error_out, &err.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_error(error_out, "panic while opening document");
            ptr::null_mut()
        }
    }
}

/// Frees a document returned by `oxide_document_open_from_bytes`.
///
/// # Safety
///
/// `document` must be null or a pointer returned by
/// `oxide_document_open_from_bytes` that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_free(document: *mut OxideDocument) {
    if !document.is_null() {
        let _ = unsafe { Box::from_raw(document) };
    }
}

/// Frees a UTF-8 string returned by this C API.
///
/// # Safety
///
/// `value` must be null or a pointer returned by an oxide C-API string
/// function that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn oxide_string_free(value: *mut c_char) {
    if !value.is_null() {
        let _ = unsafe { CString::from_raw(value) };
    }
}

/// Frees an error string returned through an `error_out` parameter.
///
/// # Safety
///
/// `value` must be null or a pointer returned through an oxide C-API
/// `error_out` parameter that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn oxide_error_free(value: *mut c_char) {
    unsafe { oxide_string_free(value) };
}

/// Frees a byte buffer returned by this C API.
///
/// # Safety
///
/// `buffer` must be empty or a buffer returned by an oxide C-API function that
/// has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn oxide_buffer_free(buffer: OxideBuffer) {
    if !buffer.data.is_null() && buffer.len > 0 {
        let slice = ptr::slice_from_raw_parts_mut(buffer.data, buffer.len);
        let _ = unsafe { Box::from_raw(slice) };
    }
}

/// Returns the number of pages in a document.
///
/// # Safety
///
/// `document` must be a valid open document. `out_count` must be writable. If
/// `error_out` is non-null, it must be writable and any returned string must be
/// freed with `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_page_count(
    document: *const OxideDocument,
    out_count: *mut usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_count.is_null() {
            return Err("out_count pointer is null".into());
        }
        unsafe {
            *out_count = oxide(doc.engine.page_count())?;
        }
        Ok(())
    })
}

/// Extracts text from a document.
///
/// # Safety
///
/// `document` must be a valid open document. `out_text` must be writable and
/// any returned string must be freed with `oxide_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_extract_text(
    document: *const OxideDocument,
    page: usize,
    out_text: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_text.is_null() {
            return Err("out_text pointer is null".into());
        }
        let text = if page == 0 {
            oxide(TextExtractor::extract_default(&doc.engine))?
        } else {
            oxide(doc.engine.get_page_text(page))?
        };
        unsafe {
            *out_text = into_c_string(text);
        }
        Ok(())
    })
}

/// Extracts the tags-first semantic document as JSON.
///
/// # Safety
///
/// `document` must be a valid open document. `out_json` must be writable and
/// any returned string must be freed with `oxide_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_extract_semantic_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let semantic = oxide(doc.engine.extract_semantic_document(&[]))?;
        let json = serde_json::to_string(&semantic).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Parses the document into the canonical model and renders it as Markdown.
///
/// This is the AI/RAG-facing parser output: headings, paragraphs, lists,
/// tables, figures, and captions in recovered reading order. Uses default
/// parse options over all pages. Digital-born only — scanned pages degrade to a
/// placeholder (OCR is not wired through the C ABI).
///
/// # Safety
///
/// `document` must be a valid open document. `out_markdown` must be writable and
/// any returned string must be freed with `oxide_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_parse_markdown(
    document: *const OxideDocument,
    out_markdown: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_markdown.is_null() {
            return Err("out_markdown pointer is null".into());
        }
        let parsed = oxide(doc.engine.parse_document(&ParseOptions::default()))?;
        let markdown = parsed.to_markdown_default();
        unsafe {
            *out_markdown = into_c_string(markdown);
        }
        Ok(())
    })
}

/// Parses the document into the canonical [`Document`] model and returns it as
/// JSON. This is the SAME schema (1.1) the CLI `parse --format json`, the
/// server `/parse` endpoint, and the WASM `parseJson` binding emit — the single
/// canonical structured output. (Distinct from
/// `oxide_document_extract_semantic_json`, which serializes the older semantic
/// model and is retained only for back-compat; prefer this for new code.)
///
/// # Safety
///
/// `document` must be a valid open document. `out_json` must be writable and
/// any returned string must be freed with `oxide_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_parse_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let parsed = oxide(doc.engine.parse_document(&ParseOptions::default()))?;
        unsafe {
            *out_json = into_c_string(parsed.to_json());
        }
        Ok(())
    })
}

/// Registers a C-function-pointer OCR backend on the document, so the
/// `oxide_document_parse_markdown_ocr` / `oxide_document_parse_json_ocr`
/// functions route scanned pages through it. See [`OxideOcrBackend`] for the
/// callback contract. Pass a backend with a null `recognize` to clear a
/// previously-registered backend.
///
/// Returns `OXIDE_STATUS_OK` on success (including clearing), or
/// `OXIDE_STATUS_ERROR` if the document is null.
///
/// # Safety
///
/// `document` must be a valid open document. `backend.recognize` /
/// `backend.userdata` must remain valid until the document is freed or the
/// backend is cleared/replaced. `backend.name`, if non-null, must be a valid
/// NUL-terminated string for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_set_ocr_backend(
    document: *mut OxideDocument,
    backend: OxideOcrBackend,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if document.is_null() {
            return Err("document pointer is null".into());
        }
        // SAFETY: non-null, and the caller owns the document exclusively while
        // registering a backend (no concurrent parse in flight is a caller
        // contract, matching every other mutating C-API call).
        let doc = unsafe { &mut *document };
        // SAFETY: the descriptor's pointers satisfy the documented contract.
        doc.ocr = unsafe { CAbiOcrEngine::from_descriptor(backend) }
            .map(|e| Arc::new(e) as Arc<dyn oxide_engine::OcrEngine>);
        Ok(())
    })
}

/// Parses the document to canonical Markdown **with OCR** for scanned pages,
/// using the backend registered via `oxide_document_set_ocr_backend`. If no
/// backend is registered this behaves exactly like `oxide_document_parse_markdown`
/// (scanned pages degrade to the placeholder).
///
/// # Safety
///
/// Same as `oxide_document_parse_markdown`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_parse_markdown_ocr(
    document: *const OxideDocument,
    out_markdown: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_markdown.is_null() {
            return Err("out_markdown pointer is null".into());
        }
        let parsed = oxide(doc.engine.parse_document(&parse_options_with_ocr(doc)))?;
        unsafe {
            *out_markdown = into_c_string(parsed.to_markdown_default());
        }
        Ok(())
    })
}

/// Parses the document to canonical JSON **with OCR** for scanned pages, using
/// the backend registered via `oxide_document_set_ocr_backend`. If no backend is
/// registered this behaves exactly like `oxide_document_parse_json`.
///
/// # Safety
///
/// Same as `oxide_document_parse_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_parse_json_ocr(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let parsed = oxide(doc.engine.parse_document(&parse_options_with_ocr(doc)))?;
        unsafe {
            *out_json = into_c_string(parsed.to_json());
        }
        Ok(())
    })
}

/// Extracts structured key-value fields (invoice number/date/total, receipt
/// merchant/amount, form label→value pairs, line items) as JSON.
///
/// `doc_type` selects the document-type profile: pass a null pointer or one of
/// `"auto"`, `"invoice"`, `"receipt"`, `"form"`, `"generic"`. Null/empty/unknown
/// behaves as `"auto"` (auto-detect). Digital-born only — OCR is not wired
/// through the C ABI.
///
/// # Safety
///
/// `document` must be a valid open document. `doc_type` must be null or a valid
/// NUL-terminated C string. `out_json` must be writable and any returned string
/// must be freed with `oxide_string_free`. If `error_out` is non-null, it must
/// be writable and any returned string must be freed with `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_extract_fields_json(
    document: *const OxideDocument,
    doc_type: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        // A null or empty doc_type means auto-detect; an unrecognized value also
        // falls back to auto rather than erroring, matching the CLI/WASM surface.
        let doc_type = if doc_type.is_null() {
            None
        } else {
            let s = unsafe { std::ffi::CStr::from_ptr(doc_type) }
                .to_str()
                .map_err(|_| "doc_type is not valid UTF-8".to_string())?;
            DocType::parse(s)
        };
        let opts = ExtractOptions {
            doc_type,
            ..Default::default()
        };
        let fields = oxide(doc.engine.extract_fields(&opts))?;
        unsafe {
            *out_json = into_c_string(fields.to_json());
        }
        Ok(())
    })
}

/// Returns document metadata as JSON.
///
/// # Safety
///
/// `document` must be a valid open document. `out_json` must be writable and
/// any returned string must be freed with `oxide_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_info_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let info = oxide(doc.engine.document_info())?;
        let json = serde_json::to_string(&info).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Renders a page to PNG bytes.
///
/// # Safety
///
/// `document` must be a valid open document. `out_buffer` must be writable and
/// any returned buffer must be freed with `oxide_buffer_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_render_page_png(
    document: *const OxideDocument,
    page: usize,
    dpi: u32,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let png = oxide(doc.engine.render_page_png_fast(page, dpi))?;
        unsafe {
            *out_buffer = into_buffer(png);
        }
        Ok(())
    })
}

/// Renders a page to JPEG bytes.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_render_page_jpeg(
    document: *const OxideDocument,
    page: usize,
    dpi: u32,
    quality: u8,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let (jpeg, _, _) = oxide(oxide_engine::render_page_image(
            &doc.engine,
            page,
            dpi,
            oxide_engine::RasterImageFormat::Jpeg,
            quality,
        ))?;
        unsafe {
            *out_buffer = into_buffer(jpeg);
        }
        Ok(())
    })
}

/// Extracts/reorders pages into a new PDF. `pages` are 1-based; duplicates are
/// kept. A null/empty pages array means all pages.
///
/// # Safety
///
/// `document` must be valid. `pages` must point to `pages_len` readable
/// entries unless `pages_len` is zero. `out_buffer` must be writable and freed
/// with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_extract_pages_pdf(
    document: *const OxideDocument,
    pages: *const usize,
    pages_len: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let mut pages = unsafe { read_pages(pages, pages_len) }?;
        if pages.is_empty() {
            let total = oxide(doc.engine.page_count())?;
            pages = (1..=total).collect();
        }
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = oxide(doc.engine.extract_pages(&pages))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Alias for ordered page extraction that documents the organize workflow.
///
/// # Safety
///
/// Same contract as `oxide_document_extract_pages_pdf`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_organize_pdf(
    document: *const OxideDocument,
    pages: *const usize,
    pages_len: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe { oxide_document_extract_pages_pdf(document, pages, pages_len, out_buffer, error_out) }
}

/// Rotates selected pages and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `pages` must point to `pages_len` readable
/// entries unless `pages_len` is zero. `out_buffer` must be writable and freed
/// with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_rotate_pdf(
    document: *const OxideDocument,
    pages: *const usize,
    pages_len: usize,
    angle: c_int,
    relative: c_int,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let pages = unsafe { read_pages(pages, pages_len) }?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let rotation = if relative != 0 {
            oxide_engine::Rotation::Relative(angle)
        } else {
            oxide_engine::Rotation::Absolute(angle)
        };
        let out = oxide(oxide_engine::rotate_pages(&doc.engine, &pages, rotation))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Optimizes a document and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_optimize_pdf(
    document: *const OxideDocument,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let (out, _) = oxide(oxide_engine::optimize(&doc.engine))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Linearizes a document and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_linearize_pdf(
    document: *const OxideDocument,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = oxide(oxide_engine::linearize(&doc.engine))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Writes a normalized unencrypted copy of an opened document.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_decrypt_pdf(
    document: *const OxideDocument,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = oxide(oxide_engine::decrypt_pdf(&doc.engine))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Encrypts a document with AES-256 and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. Password pointers may be null or valid
/// NUL-terminated UTF-8 strings. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_encrypt_aes256_pdf(
    document: *const OxideDocument,
    user_password: *const c_char,
    owner_password: *const c_char,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        use oxide_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let user = unsafe { optional_c_string(user_password) }?.unwrap_or_default();
        let owner = unsafe { optional_c_string(owner_password) }?.unwrap_or_else(|| user.clone());
        let params = EncryptParams {
            user_password: secret_bytes(user.into_bytes()),
            owner_password: secret_bytes(owner.into_bytes()),
            permissions: -1,
            algorithm: EncryptAlgorithm::Aes256,
            encrypt_metadata: true,
        };
        let out = oxide(oxide_engine::encrypt(&doc.engine, &params))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts a document to HTML and returns an owned string.
///
/// # Safety
///
/// `document` must be valid. `out_html` must be writable and freed with
/// `oxide_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_to_html(
    document: *const OxideDocument,
    out_html: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_html.is_null() {
            return Err("out_html pointer is null".into());
        }
        let html = oxide(oxide_engine::html_string(&doc.engine, &[]))?;
        unsafe {
            *out_html = into_c_string(html);
        }
        Ok(())
    })
}

/// Converts a document to XLSX and returns an owned buffer.
///
/// # Safety
///
/// `document` must be valid. `layout` may be null or a valid NUL-terminated
/// UTF-8 string (`pages` or `tables`). `out_buffer` must be writable and freed
/// with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_to_xlsx(
    document: *const OxideDocument,
    layout: *const c_char,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let layout = unsafe { optional_c_string(layout) }?.unwrap_or_else(|| "pages".to_string());
        let layout = oxide_engine::XlsxLayout::parse(&layout)
            .ok_or_else(|| "layout must be pages or tables".to_string())?;
        let out = oxide(oxide_engine::pdf_to_xlsx(
            &doc.engine,
            &oxide_engine::XlsxOptions { layout },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts a document to PPTX and returns an owned buffer.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_to_pptx(
    document: *const OxideDocument,
    include_images: c_int,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = oxide(oxide_engine::pdf_to_pptx(
            &doc.engine,
            &oxide_engine::PptxOptions {
                include_images: include_images != 0,
            },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts a document to DOCX and returns an owned buffer.
///
/// # Safety
///
/// `document` must be valid. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_to_docx(
    document: *const OxideDocument,
    include_images: c_int,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = oxide(oxide_engine::pdf_to_docx(
            &doc.engine,
            &oxide_engine::DocxOptions {
                include_images: include_images != 0,
                layout: oxide_engine::DocxLayout::Flowing,
            },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts DOCX bytes to PDF and returns an owned buffer.
///
/// # Safety
///
/// `data` must point to `len` readable bytes. `out_buffer` must be writable and
/// freed with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_docx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = oxide(oxide_engine::docx_to_pdf(
            bytes,
            &oxide_engine::OfficeToPdfOptions::default(),
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts XLSX bytes to PDF and returns an owned buffer.
///
/// # Safety
///
/// `data` must point to `len` readable bytes. `out_buffer` must be writable and
/// freed with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_xlsx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = oxide(oxide_engine::xlsx_to_pdf(
            bytes,
            &oxide_engine::OfficeToPdfOptions::default(),
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts PPTX bytes to PDF and returns an owned buffer.
///
/// # Safety
///
/// `data` must point to `len` readable bytes. `out_buffer` must be writable and
/// freed with `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_pptx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = oxide(oxide_engine::pptx_to_pdf(
            bytes,
            &oxide_engine::OfficeToPdfOptions::default(),
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Returns the document font report as JSON.
///
/// # Safety
///
/// `document` must be valid. `out_json` must be writable and freed with
/// `oxide_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_fonts_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let fonts = oxide(doc.engine.list_fonts())?;
        let json = serde_json::to_string(&fonts).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Returns signature verification reports as JSON.
///
/// # Safety
///
/// `document` must be valid. `out_json` must be writable and freed with
/// `oxide_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_signatures_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let reports = oxide(doc.engine.verify_signatures())?;
        let json = serde_json::to_string(&reports).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Adds a text watermark and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `text` must be a valid NUL-terminated UTF-8
/// string. `out_buffer` must be writable and freed with `oxide_buffer_free`.
/// `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_watermark_text_pdf(
    document: *const OxideDocument,
    text: *const c_char,
    opacity: f64,
    rotation_degrees: f64,
    font_size: f64,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let text = unsafe { required_c_string(text, "text") }?;
        let input = oxide(oxide_engine::decrypt_pdf(&doc.engine))?;
        let out = oxide(oxide_engine::watermark_text_pdf(
            input,
            &text,
            oxide_engine::TextWatermarkOptions {
                opacity,
                rotation_degrees,
                font_size,
                ..Default::default()
            },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Adds page numbers and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `format` may be null or a valid NUL-terminated
/// UTF-8 string. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_add_page_numbers_pdf(
    document: *const OxideDocument,
    format: *const c_char,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let format = unsafe { optional_c_string(format) }?
            .unwrap_or_else(|| "Page {n} of {total}".to_string());
        let input = oxide(oxide_engine::decrypt_pdf(&doc.engine))?;
        let out = oxide(oxide_engine::add_page_numbers_pdf(
            input,
            oxide_engine::PageNumberOptions {
                format,
                ..Default::default()
            },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Builds a PDF from JPG/PNG image byte buffers.
///
/// # Safety
///
/// `images` and `lengths` must point to `count` readable entries when `count`
/// is nonzero. Each image pointer must point to its corresponding readable
/// byte slice. `out_buffer` must be writable and freed with
/// `oxide_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_images_to_pdf(
    images: *const *const u8,
    lengths: *const usize,
    count: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if count > 0 && (images.is_null() || lengths.is_null()) {
            return Err("images/lengths pointers are null".into());
        }
        let mut borrowed = Vec::with_capacity(count);
        for idx in 0..count {
            let ptr = unsafe { *images.add(idx) };
            let len = unsafe { *lengths.add(idx) };
            if ptr.is_null() {
                return Err(format!("image pointer {idx} is null"));
            }
            let bytes = unsafe { slice::from_raw_parts(ptr, len) };
            borrowed.push((bytes, None));
        }
        let out = oxide(oxide_engine::images_to_pdf_from_bytes(
            &borrowed,
            oxide_engine::ImageToPdfOptions::default(),
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Merges PDF byte buffers in order.
///
/// # Safety
///
/// `inputs` and `lengths` must point to `count` readable entries. Each input
/// pointer must point to its corresponding readable byte slice. `out_buffer`
/// must be writable and freed with `oxide_buffer_free`. `error_out`, if
/// non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_merge_pdfs_from_bytes(
    inputs: *const *const u8,
    lengths: *const usize,
    count: usize,
    out_buffer: *mut OxideBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if count == 0 {
            return Err("at least one input PDF is required".into());
        }
        if inputs.is_null() || lengths.is_null() {
            return Err("inputs/lengths pointers are null".into());
        }
        let mut engines = Vec::with_capacity(count);
        for idx in 0..count {
            let ptr = unsafe { *inputs.add(idx) };
            let len = unsafe { *lengths.add(idx) };
            if ptr.is_null() {
                return Err(format!("input pointer {idx} is null"));
            }
            let bytes = unsafe { slice::from_raw_parts(ptr, len) }.to_vec();
            engines.push(oxide(ContentEngine::open_bytes(bytes))?);
        }
        let mut specs = Vec::with_capacity(engines.len());
        for engine in &engines {
            let total = oxide(engine.page_count())?;
            specs.push((engine.document(), (1..=total).collect::<Vec<_>>()));
        }
        let out = oxide(oxide_engine::build_merged(&specs))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

// ── Report surfaces (shared oxide_engine::sdk facade) ─────────────────────────
//
// Each returns a versioned-JSON envelope string
// `{"schema_version", "kind", "report"}` — the SAME bytes Python's report
// methods return, since both call the identical facade. The returned string is
// caller-owned; free it with `oxide_string_free`. Output-producing operations
// (sanitize/canonicalize/redact) return the produced PDF via an `OxideBuffer`
// (free with `oxide_buffer_free`) AND the report string.

/// The original file bytes backing an open document (copied out of the reader).
fn doc_bytes(doc: &OxideDocument) -> Vec<u8> {
    doc.engine.document().reader().file_bytes().to_vec()
}

/// Run a facade report closure over the document bytes and write the resulting
/// JSON string to `out_json`. Shared implementation for every read-only report.
unsafe fn report_json_impl(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
    f: impl FnOnce(&[u8]) -> OxideResult<String>,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = oxide(f(&doc_bytes(doc)))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Run a facade output-producing closure and write both the produced bytes and
/// the JSON report. Shared implementation for sanitize/canonicalize/redact.
unsafe fn report_output_impl(
    document: *const OxideDocument,
    out_buffer: *mut OxideBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
    f: impl FnOnce(&[u8]) -> OxideResult<(Vec<u8>, String)>,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let (bytes, json) = oxide(f(&doc_bytes(doc)))?;
        unsafe {
            *out_buffer = into_buffer(bytes);
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Security report JSON. See `report_json_impl` for ownership.
///
/// # Safety
/// `document` must be a valid open document. `out_json`/`error_out` must be
/// writable; free the returned string with `oxide_string_free` / the error with
/// `oxide_error_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_security_report_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::security_report_json(b, None)
        })
    }
}

/// Parser diagnostics report JSON. `mode` is `strict`|`repair`|`audit` (NULL →
/// `repair`).
///
/// # Safety
/// See `oxide_document_security_report_json`; `mode` may be NULL or a
/// NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_parser_report_json(
    document: *const OxideDocument,
    mode: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let mode = unsafe { optional_c_string(mode) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let mode = mode.map_err(oxide_engine::OxideError::invalid_input)?;
            sdk::parser_report_json(b, mode.as_deref(), None)
        })
    }
}

/// Color / prepress report JSON. `profile` is `generic`|`pdfa`|`pdfx` (NULL →
/// `generic`).
///
/// # Safety
/// See `oxide_document_security_report_json`; `profile` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_color_report_json(
    document: *const OxideDocument,
    profile: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let profile = unsafe { optional_c_string(profile) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let profile = profile.map_err(oxide_engine::OxideError::invalid_input)?;
            sdk::color_report_json(b, profile.as_deref())
        })
    }
}

/// Standards-profile validation report JSON. `profile` is
/// `pdfa`|`pdfua`|`pdfx`|`security`|`all` (NULL → `all`).
///
/// # Safety
/// See `oxide_document_security_report_json`; `profile` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_validate_json(
    document: *const OxideDocument,
    profile: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let profile = unsafe { optional_c_string(profile) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let profile = profile.map_err(oxide_engine::OxideError::invalid_input)?;
            sdk::standards_profile_json(b, profile.as_deref(), None)
        })
    }
}

/// AcroForm field-inventory report JSON.
///
/// # Safety
/// See `oxide_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_forms_report_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::forms_report_json(b, None)
        })
    }
}

/// Annotation-inventory report JSON.
///
/// # Safety
/// See `oxide_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_annotations_report_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::annotation_report_json(b, None)
        })
    }
}

/// Page-operations report JSON (boxes, labels, destinations, preservation risk).
///
/// # Safety
/// See `oxide_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_pages_report_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::page_operations_report_json(b, None)
        })
    }
}

/// Combined interactive report JSON (forms + annotations + page operations).
///
/// # Safety
/// See `oxide_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_interactive_report_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::interactive_report_json(b, None)
        })
    }
}

/// RAG-ready semantic chunk-set JSON.
///
/// # Safety
/// See `oxide_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_chunks_json(
    document: *const OxideDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::chunk_report_json(b, None)
        })
    }
}

/// Sanitize the document. `policy` is `strict`|`balanced`|`preserve-visual`
/// (NULL → `balanced`). Writes the sanitized PDF to `out_buffer` and the JSON
/// report to `out_json`.
///
/// # Safety
/// `document` valid; `out_buffer`/`out_json`/`error_out` writable. Free the
/// buffer with `oxide_buffer_free`, the string with `oxide_string_free`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_sanitize_json(
    document: *const OxideDocument,
    policy: *const c_char,
    out_buffer: *mut OxideBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let policy = unsafe { optional_c_string(policy) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |b| {
            let policy = policy.map_err(oxide_engine::OxideError::invalid_input)?;
            sdk::sanitize_json(b, policy.as_deref(), None)
        })
    }
}

/// Canonicalize the document deterministically. `date_epoch` fixes the source
/// date epoch (pass a negative value to leave it unset). Writes the canonical
/// PDF to `out_buffer` and the audit JSON to `out_json`.
///
/// # Safety
/// See `oxide_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_canonicalize_json(
    document: *const OxideDocument,
    date_epoch: i64,
    has_date_epoch: c_int,
    out_buffer: *mut OxideBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let epoch = if has_date_epoch != 0 {
        Some(date_epoch)
    } else {
        None
    };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |b| {
            sdk::canonicalize_json(b, epoch, None)
        })
    }
}

/// Redact every occurrence of the given NUL-terminated `terms` (case
/// insensitive), full-rewrite, and verify absence. `strict != 0` fails the call
/// if a term survives. Writes the redacted PDF to `out_buffer` and a JSON report
/// (with verification) to `out_json`.
///
/// # Safety
/// `terms` must point to `terms_len` non-null NUL-terminated UTF-8 strings.
/// See `oxide_document_sanitize_json` for the outputs.
#[no_mangle]
pub unsafe extern "C" fn oxide_document_redact_terms_json(
    document: *const OxideDocument,
    terms: *const *const c_char,
    terms_len: usize,
    strict: c_int,
    out_buffer: *mut OxideBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let collected = unsafe { read_c_string_array(terms, terms_len) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |b| {
            let terms = collected
                .clone()
                .map_err(oxide_engine::OxideError::invalid_input)?;
            sdk::redact_terms_json(b, &terms, strict != 0, None)
        })
    }
}

/// SDK / ABI version and capability report as JSON (no document needed): engine
/// version, envelope version, compiled capabilities. Free with
/// `oxide_string_free`.
///
/// # Safety
/// `out_json`/`error_out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_feature_report_json(
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = oxide(sdk::feature_report_json())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Codec isolation diagnostic report over caller-supplied encoded stream bytes.
///
/// `filter` must be a NUL-terminated UTF-8 filter name such as `FlateDecode`.
/// `policy` may be NULL (defaults to `in_process`) or one of
/// `in_process`, `isolated_preferred`, `isolated_required`, `report_only`, or
/// `disabled`.
///
/// # Safety
/// `data` must point to `len` readable bytes unless `len == 0`.
/// `out_json`/`error_out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn oxide_codec_isolation_report_json(
    filter: *const c_char,
    data: *const u8,
    len: usize,
    policy: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let filter = unsafe { required_c_string(filter, "filter") };
    let policy = unsafe { optional_c_string(policy) };
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let filter = filter?;
        let policy = policy?;
        let input = unsafe { read_input_bytes(data, len, "data") }?;
        let json = oxide(sdk::codec_isolation_report_json(
            &filter,
            input,
            policy.as_deref(),
        ))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// The oxide-engine semantic version as a NUL-terminated string. The returned
/// pointer is owned by the caller and must be freed with `oxide_string_free`.
/// Safe to call (takes no pointers).
#[no_mangle]
pub extern "C" fn oxide_version() -> *mut c_char {
    into_c_string(oxide_engine::ENGINE_VERSION.to_string())
}

/// The C-ABI report envelope version (bump signals an envelope-shape change).
/// Safe to call (takes no pointers).
#[no_mangle]
pub extern "C" fn oxide_abi_version() -> u32 {
    oxide_engine::REPORT_ENVELOPE_VERSION
}

/// Read `len` NUL-terminated UTF-8 strings from a C string array.
unsafe fn read_c_string_array(
    ptr: *const *const c_char,
    len: usize,
) -> Result<Vec<String>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if ptr.is_null() {
        return Err("terms pointer is null".to_string());
    }
    let mut out = Vec::with_capacity(len);
    for idx in 0..len {
        let item = unsafe { *ptr.add(idx) };
        out.push(unsafe { required_c_string(item, "term") }?);
    }
    Ok(out)
}

fn checked_doc<'a>(document: *const OxideDocument) -> Result<&'a OxideDocument, String> {
    if document.is_null() {
        Err("document pointer is null".to_string())
    } else {
        Ok(unsafe { &*document })
    }
}

unsafe fn read_pages(pages: *const usize, pages_len: usize) -> Result<Vec<usize>, String> {
    if pages_len == 0 {
        return Ok(Vec::new());
    }
    if pages.is_null() {
        return Err("pages pointer is null".to_string());
    }
    Ok(unsafe { slice::from_raw_parts(pages, pages_len) }.to_vec())
}

unsafe fn optional_c_string(ptr: *const c_char) -> Result<Option<String>, String> {
    if ptr.is_null() {
        return Ok(None);
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| "string is not valid UTF-8".to_string())?;
    Ok(Some(s.to_string()))
}

unsafe fn required_c_string(ptr: *const c_char, name: &str) -> Result<String, String> {
    unsafe { optional_c_string(ptr) }?.ok_or_else(|| format!("{name} pointer is null"))
}

unsafe fn read_input_bytes<'a>(
    data: *const u8,
    len: usize,
    name: &str,
) -> Result<&'a [u8], String> {
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null() {
        return Err(format!("{name} pointer is null"));
    }
    Ok(unsafe { slice::from_raw_parts(data, len) })
}

fn ffi_status(error_out: *mut *mut c_char, f: impl FnOnce() -> Result<(), String>) -> c_int {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => OXIDE_STATUS_OK,
        Ok(Err(err)) => {
            set_error(error_out, &err);
            OXIDE_STATUS_ERROR
        }
        Err(_) => {
            set_error(error_out, "panic inside oxide C API");
            OXIDE_STATUS_PANIC
        }
    }
}

fn oxide<T>(result: OxideResult<T>) -> Result<T, String> {
    result.map_err(|err| err.to_string())
}

/// Build [`ParseOptions`] carrying the document's registered OCR backend (if
/// any). With a backend, uses `OcrPolicy::Auto` (scanned pages recognized) and a
/// generous per-page timeout as an engine-side backstop; without one, returns
/// default options (OCR off).
fn parse_options_with_ocr(doc: &OxideDocument) -> ParseOptions {
    match &doc.ocr {
        Some(engine) => ParseOptions {
            ocr: Some(Arc::clone(engine)),
            ocr_policy: OcrPolicy::Auto,
            ocr_timeout: Some(std::time::Duration::from_secs(120)),
            ..Default::default()
        },
        None => ParseOptions::default(),
    }
}

fn into_c_string(value: String) -> *mut c_char {
    let clean = value.replace('\0', "\u{FFFD}");
    CString::new(clean).expect("nul bytes replaced").into_raw()
}

fn into_buffer(bytes: Vec<u8>) -> OxideBuffer {
    if bytes.is_empty() {
        return OxideBuffer::empty();
    }
    let mut bytes = bytes.into_boxed_slice();
    let out = OxideBuffer {
        data: bytes.as_mut_ptr(),
        len: bytes.len(),
    };
    std::mem::forget(bytes);
    out
}

fn set_error(error_out: *mut *mut c_char, message: &str) {
    if !error_out.is_null() {
        unsafe {
            *error_out = into_c_string(message.to_string());
        }
    }
}

fn clear_error(error_out: *mut *mut c_char) {
    if !error_out.is_null() {
        unsafe {
            *error_out = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_engine::{crypto::secret_bytes, encrypt, EncryptAlgorithm, EncryptParams};
    use std::ffi::CStr;

    struct PdfBuilder {
        objects: Vec<Vec<u8>>,
    }

    impl PdfBuilder {
        fn new() -> Self {
            Self {
                objects: Vec::new(),
            }
        }

        fn add(&mut self, body: &str) {
            self.objects.push(body.as_bytes().to_vec());
        }

        fn add_stream(&mut self, stream: &[u8]) {
            let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
            body.extend_from_slice(stream);
            body.extend_from_slice(b"\nendstream");
            self.objects.push(body);
        }

        fn add_stream_with_dict(&mut self, dict_extra: &str, stream: &[u8]) {
            let mut body =
                format!("<< /Length {} {} >>\nstream\n", stream.len(), dict_extra).into_bytes();
            body.extend_from_slice(stream);
            body.extend_from_slice(b"\nendstream");
            self.objects.push(body);
        }

        fn build(&self) -> Vec<u8> {
            let mut pdf = Vec::new();
            pdf.extend_from_slice(b"%PDF-1.7\n");
            let mut offsets = Vec::new();
            for (i, body) in self.objects.iter().enumerate() {
                offsets.push(pdf.len());
                pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
                pdf.extend_from_slice(body);
                pdf.extend_from_slice(b"\nendobj\n");
            }
            let xref_start = pdf.len();
            pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
            pdf.extend_from_slice(b"0000000000 65535 f \n");
            for off in offsets {
                pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
            }
            pdf.extend_from_slice(
                format!(
                    "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                    self.objects.len() + 1,
                    xref_start
                )
                .as_bytes(),
            );
            pdf
        }
    }

    fn sample_pdf() -> Vec<u8> {
        let mut b = PdfBuilder::new();
        b.add("<< /Type /Catalog /Pages 2 0 R >>");
        b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        b.add(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        );
        b.add_stream(b"BT /F1 12 Tf 40 120 Td (Hello C API) Tj ET");
        b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        b.build()
    }

    fn encrypted_sample_pdf(password: &[u8]) -> Vec<u8> {
        let engine = ContentEngine::open_bytes(sample_pdf()).expect("sample opens");
        encrypt(
            &engine,
            &EncryptParams {
                algorithm: EncryptAlgorithm::Aes256,
                user_password: secret_bytes(password.to_vec()),
                owner_password: secret_bytes(b"owner-password".to_vec()),
                ..Default::default()
            },
        )
        .expect("encrypt sample")
    }

    #[test]
    fn capi_open_count_extract_and_free() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        assert!(error.is_null());

        let mut count = 0usize;
        let status = unsafe { oxide_document_page_count(doc, &mut count, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        assert_eq!(count, 1);

        let mut text = std::ptr::null_mut();
        let status = unsafe { oxide_document_extract_text(doc, 1, &mut text, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let extracted = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        assert!(extracted.contains("Hello C API"));
        unsafe {
            oxide_string_free(text);
            oxide_document_free(doc);
        }
    }

    #[test]
    fn capi_open_with_password_accepts_null_empty_and_ignored_passwords() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();

        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                std::ptr::null(),
                0,
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { oxide_document_free(doc) };

        let explicit_empty = [0u8; 1];
        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                explicit_empty.as_ptr(),
                0,
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { oxide_document_free(doc) };

        let ignored = b"ignored-for-unencrypted";
        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                ignored.as_ptr(),
                ignored.len(),
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { oxide_document_free(doc) };
    }

    #[test]
    fn capi_open_with_password_handles_encrypted_fixture_and_redacts_secret() {
        let password = b"open-sesame";
        let pdf = encrypted_sample_pdf(password);
        let mut error = std::ptr::null_mut();

        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                password.as_ptr(),
                password.len(),
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        let mut count = 0usize;
        let status = unsafe { oxide_document_page_count(doc, &mut count, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        assert_eq!(count, 1);
        unsafe { oxide_document_free(doc) };

        let wrong = b"do-not-echo-this-password";
        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                wrong.as_ptr(),
                wrong.len(),
                &mut error,
            )
        };
        assert!(doc.is_null());
        assert!(!error.is_null());
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        assert!(message.contains("password-protected") || message.contains("password"));
        assert!(
            !message.contains("do-not-echo-this-password"),
            "password leaked in error: {message}"
        );
        unsafe { oxide_error_free(error) };
    }

    #[test]
    fn capi_open_with_password_rejects_invalid_password_pointer_shape() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe {
            oxide_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                std::ptr::null(),
                4,
                &mut error,
            )
        };
        assert!(doc.is_null());
        assert!(!error.is_null());
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        assert!(message.contains("password pointer is null"));
        unsafe { oxide_error_free(error) };
    }

    #[test]
    fn capi_parse_markdown_json_and_fields() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        // parse → markdown: the canonical parser output, containing the text.
        let mut md = std::ptr::null_mut();
        let status = unsafe { oxide_document_parse_markdown(doc, &mut md, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let markdown = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(markdown.contains("Hello C API"), "markdown was: {markdown}");
        unsafe { oxide_string_free(md) };

        // parse → canonical JSON: must carry the schema version and the text.
        let mut json = std::ptr::null_mut();
        let status = unsafe { oxide_document_parse_json(doc, &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let parsed = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert!(parsed.contains("schema_version"), "json was: {parsed}");
        assert!(parsed.contains("Hello C API"));
        unsafe { oxide_string_free(json) };

        // extract-fields → JSON: null doc_type means auto-detect; must succeed
        // and produce a well-formed payload (this doc has no fields, which is
        // fine — the call must not error and must include the schema version).
        let mut fields = std::ptr::null_mut();
        let status = unsafe {
            oxide_document_extract_fields_json(doc, std::ptr::null(), &mut fields, &mut error)
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let fields_json = unsafe { CStr::from_ptr(fields) }
            .to_string_lossy()
            .into_owned();
        assert!(
            fields_json.contains("schema_version"),
            "fields: {fields_json}"
        );
        unsafe { oxide_string_free(fields) };

        unsafe { oxide_document_free(doc) };
    }

    #[test]
    fn capi_phase3_pdf_utilities_return_owned_buffers() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        let mut jpeg = OxideBuffer::empty();
        let status =
            unsafe { oxide_document_render_page_jpeg(doc, 1, 72, 80, &mut jpeg, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let jpeg_bytes = unsafe { std::slice::from_raw_parts(jpeg.data, jpeg.len) };
        assert!(jpeg_bytes.starts_with(&[0xFF, 0xD8]));
        unsafe { oxide_buffer_free(jpeg) };

        let pages = [1usize, 1usize];
        let mut organized = OxideBuffer::empty();
        let status = unsafe {
            oxide_document_organize_pdf(
                doc,
                pages.as_ptr(),
                pages.len(),
                &mut organized,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let org_bytes = unsafe { std::slice::from_raw_parts(organized.data, organized.len) };
        let re = ContentEngine::open_bytes(org_bytes.to_vec()).unwrap();
        assert_eq!(re.page_count().unwrap(), 2);
        unsafe { oxide_buffer_free(organized) };

        let text = CString::new("DRAFT").unwrap();
        let mut watermarked = OxideBuffer::empty();
        let status = unsafe {
            oxide_document_watermark_text_pdf(
                doc,
                text.as_ptr(),
                0.25,
                45.0,
                48.0,
                &mut watermarked,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let bytes = unsafe { std::slice::from_raw_parts(watermarked.data, watermarked.len) };
        let re = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
        assert!(re.get_page_text(1).unwrap().contains("DRAFT"));
        unsafe {
            oxide_buffer_free(watermarked);
            oxide_document_free(doc);
        }
    }

    #[test]
    fn capi_phase4_office_conversions_return_owned_buffers() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        let layout = CString::new("pages").unwrap();
        let mut xlsx = OxideBuffer::empty();
        let status = unsafe { oxide_document_to_xlsx(doc, layout.as_ptr(), &mut xlsx, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let xlsx_bytes = unsafe { std::slice::from_raw_parts(xlsx.data, xlsx.len) };
        assert!(xlsx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(xlsx_bytes, "xl/workbook.xml"));
        let mut xlsx_pdf = OxideBuffer::empty();
        let status = unsafe {
            oxide_xlsx_to_pdf(
                xlsx_bytes.as_ptr(),
                xlsx_bytes.len(),
                &mut xlsx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let xlsx_pdf_bytes = unsafe { std::slice::from_raw_parts(xlsx_pdf.data, xlsx_pdf.len) };
        assert!(xlsx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe { oxide_buffer_free(xlsx) };
        unsafe { oxide_buffer_free(xlsx_pdf) };

        let mut pptx = OxideBuffer::empty();
        let status = unsafe { oxide_document_to_pptx(doc, 1, &mut pptx, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let pptx_bytes = unsafe { std::slice::from_raw_parts(pptx.data, pptx.len) };
        assert!(pptx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(pptx_bytes, "ppt/presentation.xml"));
        let mut pptx_pdf = OxideBuffer::empty();
        let status = unsafe {
            oxide_pptx_to_pdf(
                pptx_bytes.as_ptr(),
                pptx_bytes.len(),
                &mut pptx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let pptx_pdf_bytes = unsafe { std::slice::from_raw_parts(pptx_pdf.data, pptx_pdf.len) };
        assert!(pptx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe { oxide_buffer_free(pptx_pdf) };

        let mut docx = OxideBuffer::empty();
        let status = unsafe { oxide_document_to_docx(doc, 1, &mut docx, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let docx_bytes = unsafe { std::slice::from_raw_parts(docx.data, docx.len) };
        assert!(docx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(docx_bytes, "word/document.xml"));
        let mut docx_pdf = OxideBuffer::empty();
        let status = unsafe {
            oxide_docx_to_pdf(
                docx_bytes.as_ptr(),
                docx_bytes.len(),
                &mut docx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let docx_pdf_bytes = unsafe { std::slice::from_raw_parts(docx_pdf.data, docx_pdf.len) };
        assert!(docx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe {
            oxide_buffer_free(pptx);
            oxide_buffer_free(docx);
            oxide_buffer_free(docx_pdf);
            oxide_document_free(doc);
        }
    }

    fn contains_ascii(bytes: &[u8], needle: &str) -> bool {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    #[test]
    fn capi_reports_null_document_error() {
        let mut count = 0usize;
        let mut error = std::ptr::null_mut();
        let status = unsafe { oxide_document_page_count(std::ptr::null(), &mut count, &mut error) };
        assert_eq!(status, OXIDE_STATUS_ERROR);
        assert!(!error.is_null());
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        assert!(message.contains("document pointer is null"));
        unsafe {
            oxide_error_free(error);
        }
    }

    // ── C-ABI OCR backend ────────────────────────────────────────────────────

    /// A 612×792 page whose only content is a full-page image → classified
    /// `Scanned`, routing it to the OCR path.
    fn scanned_page_pdf() -> Vec<u8> {
        let mut b = PdfBuilder::new();
        b.add("<< /Type /Catalog /Pages 2 0 R >>");
        b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        b.add(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>",
        );
        b.add_stream(b"q 612 0 0 792 0 0 cm /Im0 Do Q\n");
        b.add_stream_with_dict(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 \
             /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[0x80],
        );
        b.build()
    }

    /// A C `recognize` that emits one scripted word via the sink, ignoring the
    /// image. Proves the function-pointer backend reaches the document model.
    extern "C" fn mock_recognize(
        _userdata: *mut std::os::raw::c_void,
        _gray: *const u8,
        _width: u32,
        _height: u32,
        _dpi: u32,
        sink: *mut std::os::raw::c_void,
        emit: OxideOcrEmitWordFn,
    ) -> c_int {
        let text = CString::new("CabiWord").unwrap();
        emit(sink, text.as_ptr(), 72.0, 60.0, 200.0, 88.0, 0.9, 0);
        0
    }

    #[test]
    fn capi_function_pointer_ocr_backend_reaches_document_model() {
        let pdf = scanned_page_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        // Without a backend, the scanned page degrades to the placeholder.
        let mut md = std::ptr::null_mut();
        let status = unsafe { oxide_document_parse_markdown_ocr(doc, &mut md, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let before = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(!before.contains("CabiWord"), "no OCR yet: {before}");
        unsafe { oxide_string_free(md) };

        // Register the function-pointer backend and re-parse.
        let name = CString::new("c-mock").unwrap();
        let backend = OxideOcrBackend {
            userdata: std::ptr::null_mut(),
            recognize: Some(mock_recognize),
            max_concurrency: 1,
            name: name.as_ptr(),
        };
        let status = unsafe { oxide_document_set_ocr_backend(doc, backend, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);

        let mut md = std::ptr::null_mut();
        let status = unsafe { oxide_document_parse_markdown_ocr(doc, &mut md, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let after = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(
            after.contains("CabiWord"),
            "OCR word must reach the document model: {after}"
        );
        unsafe {
            oxide_string_free(md);
            oxide_document_free(doc);
        }
    }

    // ── Prompt-01 report surfaces ─────────────────────────────────────────────

    /// Open the sample PDF, returning the opaque handle (caller frees).
    fn open_sample() -> (*mut OxideDocument, Vec<u8>) {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        (doc, pdf)
    }

    /// Call a report fn, parse its JSON, assert the envelope kind, then free.
    fn report_envelope(
        f: unsafe extern "C" fn(*const OxideDocument, *mut *mut c_char, *mut *mut c_char) -> c_int,
        kind: &str,
    ) -> serde_json::Value {
        let (doc, _pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe { f(doc, &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK, "report fn returned error");
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["kind"], kind);
        assert!(value.get("report").is_some());
        unsafe {
            oxide_string_free(json);
            oxide_document_free(doc);
        }
        value
    }

    #[test]
    fn capi_read_only_report_envelopes() {
        report_envelope(oxide_document_security_report_json, "security_report");
        report_envelope(oxide_document_forms_report_json, "forms_report");
        report_envelope(oxide_document_annotations_report_json, "annotation_report");
        report_envelope(oxide_document_pages_report_json, "page_operations_report");
        report_envelope(oxide_document_interactive_report_json, "interactive_report");
        report_envelope(oxide_document_chunks_json, "chunk_set");
    }

    #[test]
    fn capi_parametrized_reports() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();

        // parser report with explicit mode.
        let mode = CString::new("audit").unwrap();
        let mut json = std::ptr::null_mut();
        let status =
            unsafe { oxide_document_parser_report_json(doc, mode.as_ptr(), &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "parser_report");
        assert_eq!(value["report"]["opened"], true);
        unsafe { oxide_string_free(json) };

        // color report with NULL profile (defaults to generic).
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            oxide_document_color_report_json(doc, std::ptr::null(), &mut json, &mut error)
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["kind"] == "color_report"
        );
        unsafe { oxide_string_free(json) };

        // validate with a profile.
        let profile = CString::new("all").unwrap();
        let mut json = std::ptr::null_mut();
        let status =
            unsafe { oxide_document_validate_json(doc, profile.as_ptr(), &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        unsafe { oxide_string_free(json) };

        unsafe { oxide_document_free(doc) };
    }

    #[test]
    fn capi_sanitize_and_canonicalize_output_and_report() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();

        let mut buf = OxideBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            oxide_document_sanitize_json(doc, std::ptr::null(), &mut buf, &mut json, &mut error)
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let bytes = unsafe { slice::from_raw_parts(buf.data, buf.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["kind"],
            "sanitize_report"
        );
        unsafe {
            oxide_buffer_free(buf);
            oxide_string_free(json);
        }

        let mut buf = OxideBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status =
            unsafe { oxide_document_canonicalize_json(doc, 0, 1, &mut buf, &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let bytes = unsafe { slice::from_raw_parts(buf.data, buf.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "canonicalize_report");
        assert_eq!(value["report"]["deterministic"], true);
        unsafe {
            oxide_buffer_free(buf);
            oxide_string_free(json);
            oxide_document_free(doc);
        }
    }

    #[test]
    fn capi_redact_terms_output_and_report() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();
        let term = CString::new("Hello").unwrap();
        let terms = [term.as_ptr()];
        let mut buf = OxideBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            oxide_document_redact_terms_json(
                doc,
                terms.as_ptr(),
                terms.len(),
                0,
                &mut buf,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let bytes = unsafe { slice::from_raw_parts(buf.data, buf.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        // The redacted output no longer surfaces the term.
        let reopened = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
        assert!(!reopened.get_page_text(1).unwrap().contains("Hello"));
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["kind"],
            "redaction_report"
        );
        unsafe {
            oxide_buffer_free(buf);
            oxide_string_free(json);
            oxide_document_free(doc);
        }
    }

    #[test]
    fn capi_feature_and_version() {
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe { oxide_feature_report_json(&mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "feature_report");
        assert!(value["report"]["engine_version"].is_string());
        assert_eq!(
            value["report"]["prompt04"]["scanner"]["default_implementation"],
            "safe_first_byte_chunked"
        );
        assert_eq!(
            value["report"]["prompt04"]["renderer_decode_scheduler"]["status"],
            "adopted_for_immediate_renderer_decode_paths"
        );
        assert_eq!(
            value["report"]["prompt05"]["decode_scheduler"]["status"],
            "adopted_for_prompt05_non_render_decode_paths"
        );
        assert_eq!(
            value["report"]["prompt06"]["native_replay"]["status"],
            "native_text_image_form_display_list_foundation"
        );
        assert_eq!(
            value["report"]["prompt06"]["prompt06b_multi_reference_audit"]["status"],
            "multi_reference_audit_complete"
        );
        assert_eq!(
            value["report"]["prompt07_transparency_compositing"]["status"],
            "native_foundation_with_prompt07b_closure"
        );
        assert_eq!(
            value["report"]["prompt07_transparency_compositing"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            value["report"]["prompt07_transparency_compositing"]["blend_modes"]["implemented"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode == "Luminosity")
        );
        assert_eq!(
            value["report"]["prompt07b_transparency_closure"]["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["prompt07b_transparency_closure"]["reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert!(value["report"]["prompt07b_transparency_closure"]
            ["luminosity_soft_mask_color_spaces"]["supported"]
            .as_array()
            .unwrap()
            .iter()
            .any(|space| space == "DeviceCMYK"));
        assert_eq!(
            value["report"]["prompt08_text_clipping_shading_patterns"]["status"],
            "native_common_paths_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["prompt08_text_clipping_shading_patterns"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            value["report"]["prompt08_text_clipping_shading_patterns"]["text_clipping"]
                ["rendering_modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode.as_i64() == Some(7))
        );
        assert_eq!(
            value["report"]["prompt08b_type3_cid_tensor_closure"]["status"],
            "complete_native_common_paths_with_reference_cluster_limits"
        );
        assert_eq!(
            value["report"]["prompt08b_type3_cid_tensor_closure"]["reference_audit"]
                ["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["prompt08b_type3_cid_tensor_closure"]["type7_tensor_patch"]["status"],
            "native_tensor_product_interior"
        );
        assert_eq!(
            value["report"]["prompt09_annotation_ocg_progressive_cache"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["prompt09b_annotation_progressive_cache_validation"]["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            value["report"]["prompt09b_annotation_progressive_cache_validation"]
                ["multi_reference_audit"]["oxide_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["prompt10_cjk_rtl_color_glyph_reference_harness"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["prompt10_cjk_rtl_color_glyph_reference_harness"]["closure_gates"]
                ["memory_cap_mb"],
            4096
        );
        assert_eq!(
            value["report"]["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]
                ["multi_reference_audit"]["oxide_outlier_failures"],
            0
        );
        unsafe { oxide_string_free(json) };

        let version = oxide_version();
        assert!(!version.is_null());
        let v = unsafe { CStr::from_ptr(version) }
            .to_string_lossy()
            .into_owned();
        assert!(!v.is_empty());
        unsafe { oxide_string_free(version) };

        assert_eq!(oxide_abi_version(), 1);
    }

    #[test]
    fn capi_codec_isolation_report_envelope() {
        let filter = CString::new("FlateDecode").unwrap();
        let policy = CString::new("report_only").unwrap();
        let input = oxide_engine::flate_encode(b"capi isolation", 6);
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            oxide_codec_isolation_report_json(
                filter.as_ptr(),
                input.as_ptr(),
                input.len(),
                policy.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, OXIDE_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "codec_isolation_report");
        assert_eq!(value["report"]["status"], "report_only");
        unsafe { oxide_string_free(json) };
    }

    #[test]
    fn capi_repeated_open_report_free_stress() {
        let pdf = sample_pdf();
        for _ in 0..50 {
            let mut error = std::ptr::null_mut();
            let doc =
                unsafe { oxide_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
            assert!(!doc.is_null());
            assert!(error.is_null());

            let mut json = std::ptr::null_mut();
            let status = unsafe { oxide_document_security_report_json(doc, &mut json, &mut error) };
            assert_eq!(status, OXIDE_STATUS_OK);
            assert!(!json.is_null());
            unsafe {
                oxide_string_free(json);
                oxide_document_free(doc);
            }
        }
    }

    #[test]
    fn capi_report_null_document_is_error_not_panic() {
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status =
            unsafe { oxide_document_security_report_json(std::ptr::null(), &mut json, &mut error) };
        assert_eq!(status, OXIDE_STATUS_ERROR);
        assert!(json.is_null());
        assert!(!error.is_null());
        unsafe { oxide_error_free(error) };
    }
}
