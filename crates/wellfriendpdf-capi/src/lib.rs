//! C ABI for wellfriendpdf-engine.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;
use std::sync::Arc;

use wellfriendpdf_engine::{
    sdk, ContentEngine, DocType, ExtractOptions, OcrPolicy, ParseOptions,
    Result as WellfriendResult, TextExtractor,
};

pub mod ocr_backend;
pub use ocr_backend::{
    CAbiOcrEngine, WellfriendOcrBackend, WellfriendOcrEmitWordFn, WellfriendOcrRecognizeFn,
};

pub const WELLFRIENDPDF_STATUS_OK: c_int = 0;
pub const WELLFRIENDPDF_STATUS_NULL: c_int = 1;
pub const WELLFRIENDPDF_STATUS_ERROR: c_int = 2;
pub const WELLFRIENDPDF_STATUS_PANIC: c_int = 3;

#[repr(C)]
pub struct WellfriendDocument {
    engine: ContentEngine,
    /// An optional OCR backend registered via `wellfriendpdf_document_set_ocr_backend`.
    /// When present, the `*_with_ocr` parse functions route scanned pages
    /// through it; the plain parse functions ignore it (digital-born only).
    ocr: Option<Arc<dyn wellfriendpdf_engine::OcrEngine>>,
}

/// Opaque, owned Signature Validation signature-validation configuration.
///
/// The handle contains only public certificates, revocation evidence, and
/// policy metadata. It never stores private keys. Callers must synchronize
/// concurrent mutation themselves and must free the handle with
/// `wellfriendpdf_signature_validation_options_free`.
#[repr(C)]
pub struct WellfriendSignatureValidationOptions {
    options: wellfriendpdf_engine::VerifyOptions,
}

/// Opaque explicit trust-anchor collection for Signature Validation validation. This is
/// deliberately separate from untrusted intermediates and evidence: adding a
/// certificate here is the only C ABI operation that grants anchor trust.
#[repr(C)]
pub struct WellfriendSignatureTrustStore {
    store: wellfriendpdf_engine::TrustStore,
    distrusted_certificate_sha256: Vec<String>,
}

/// Opaque untrusted intermediate-certificate collection. Certificates in this
/// store can help path construction but are never promoted to trust anchors.
#[repr(C)]
pub struct WellfriendSignatureIntermediateStore {
    store: wellfriendpdf_engine::IntermediateStore,
}

/// Opaque caller-supplied/replayed revocation-evidence collection. Every item
/// remains untrusted until the shared engine validates its signature, scope,
/// freshness, and binding to a selected certificate path.
#[repr(C)]
pub struct WellfriendSignatureEvidenceStore {
    ocsp_responses_der: Vec<Vec<u8>>,
    crls_der: Vec<Vec<u8>>,
    bundle: Option<wellfriendpdf_engine::EvidenceBundle>,
}

/// Opaque bounded retrieval policy. Constructed policies start offline; an
/// explicit JSON policy with `enabled: true` is required before network I/O.
#[repr(C)]
pub struct WellfriendSignatureRetrievalPolicy {
    policy: wellfriendpdf_engine::RetrievalPolicy,
}

/// Opaque cooperative cancellation source. It can be freed immediately after
/// attaching it to options because options retain an owned clone of the token.
#[repr(C)]
pub struct WellfriendSignatureValidationCancellation {
    token: wellfriendpdf_engine::CancelToken,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WellfriendBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl WellfriendBuffer {
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
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_open_from_bytes(
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> *mut WellfriendDocument {
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
/// writable and any returned string must be freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_open_from_bytes_with_password(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    error_out: *mut *mut c_char,
) -> *mut WellfriendDocument {
    unsafe { open_document_from_parts(data, len, password, password_len, error_out) }
}

/// Opens a public-key encrypted PDF from bytes with explicit certificate and
/// private-key buffers. The certificate and key may be PEM or DER. The key is
/// used only during open and is not serialized by the C ABI wrapper.
///
/// # Safety
///
/// Every pointer must address the corresponding readable byte count. If
/// `error_out` is non-null, it must be writable and freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_open_pubsec_from_bytes(
    data: *const u8,
    len: usize,
    certificate: *const u8,
    certificate_len: usize,
    private_key: *const u8,
    private_key_len: usize,
    error_out: *mut *mut c_char,
) -> *mut WellfriendDocument {
    clear_error(error_out);
    if data.is_null() || certificate.is_null() || private_key.is_null() {
        set_error(
            error_out,
            "data, certificate, and private_key pointers are required",
        );
        return ptr::null_mut();
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { slice::from_raw_parts(data, len) }.to_vec();
        let cert = unsafe { slice::from_raw_parts(certificate, certificate_len) };
        let key = unsafe { slice::from_raw_parts(private_key, private_key_len) };
        let identity = wellfriendpdf_engine::PubSecIdentity::from_bytes(cert, key)?;
        let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
        ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)
    })) {
        Ok(Ok(engine)) => Box::into_raw(Box::new(WellfriendDocument { engine, ocr: None })),
        Ok(Err(err)) => {
            set_error(error_out, &err.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_error(error_out, "panic while opening PubSec document");
            ptr::null_mut()
        }
    }
}

/// Opens a public-key encrypted PDF from bytes with a PKCS #12/PFX provider
/// bundle and explicit password bytes.
///
/// # Safety
///
/// Every pointer must address the corresponding readable byte count. A null
/// password pointer is accepted only with password_len == 0. If `error_out` is
/// non-null, it must be writable and freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_open_pubsec_pfx_from_bytes(
    data: *const u8,
    len: usize,
    pfx: *const u8,
    pfx_len: usize,
    password: *const u8,
    password_len: usize,
    error_out: *mut *mut c_char,
) -> *mut WellfriendDocument {
    clear_error(error_out);
    if data.is_null() || pfx.is_null() || (password.is_null() && password_len != 0) {
        set_error(
            error_out,
            "data, pfx, and password pointer/length arguments are invalid",
        );
        return ptr::null_mut();
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { slice::from_raw_parts(data, len) }.to_vec();
        let pfx_bytes = unsafe { slice::from_raw_parts(pfx, pfx_len) };
        let password_bytes = if password_len == 0 {
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(password, password_len) }
        };
        let identity =
            wellfriendpdf_engine::PubSecIdentity::from_pkcs12_der(pfx_bytes, password_bytes)?;
        let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
        ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)
    })) {
        Ok(Ok(engine)) => Box::into_raw(Box::new(WellfriendDocument { engine, ocr: None })),
        Ok(Err(err)) => {
            set_error(error_out, &err.to_string());
            ptr::null_mut()
        }
        Err(_) => {
            set_error(error_out, "panic while opening PubSec PFX document");
            ptr::null_mut()
        }
    }
}

unsafe fn open_document_from_parts(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    error_out: *mut *mut c_char,
) -> *mut WellfriendDocument {
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
        Ok(Ok(engine)) => Box::into_raw(Box::new(WellfriendDocument { engine, ocr: None })),
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

/// Frees a document returned by `wellfriendpdf_document_open_from_bytes`.
///
/// # Safety
///
/// `document` must be null or a pointer returned by
/// `wellfriendpdf_document_open_from_bytes` that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_free(document: *mut WellfriendDocument) {
    if !document.is_null() {
        let _ = unsafe { Box::from_raw(document) };
    }
}

/// Frees a UTF-8 string returned by this C API.
///
/// # Safety
///
/// `value` must be null or a pointer returned by an wellfriendpdf C-API string
/// function that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_string_free(value: *mut c_char) {
    if !value.is_null() {
        let _ = unsafe { CString::from_raw(value) };
    }
}

/// Frees an error string returned through an `error_out` parameter.
///
/// # Safety
///
/// `value` must be null or a pointer returned through an wellfriendpdf C-API
/// `error_out` parameter that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_error_free(value: *mut c_char) {
    unsafe { wellfriendpdf_string_free(value) };
}

/// Frees a byte buffer returned by this C API.
///
/// # Safety
///
/// `buffer` must be empty or a buffer returned by an wellfriendpdf C-API function that
/// has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_buffer_free(buffer: WellfriendBuffer) {
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
/// freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_page_count(
    document: *const WellfriendDocument,
    out_count: *mut usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_count.is_null() {
            return Err("out_count pointer is null".into());
        }
        unsafe {
            *out_count = wellfriendpdf(doc.engine.page_count())?;
        }
        Ok(())
    })
}

/// Extracts text from a document.
///
/// # Safety
///
/// `document` must be a valid open document. `out_text` must be writable and
/// any returned string must be freed with `wellfriendpdf_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_extract_text(
    document: *const WellfriendDocument,
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
            wellfriendpdf(TextExtractor::extract_default(&doc.engine))?
        } else {
            wellfriendpdf(doc.engine.get_page_text(page))?
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
/// any returned string must be freed with `wellfriendpdf_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_extract_semantic_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let semantic = wellfriendpdf(doc.engine.extract_semantic_document(&[]))?;
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
/// any returned string must be freed with `wellfriendpdf_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_parse_markdown(
    document: *const WellfriendDocument,
    out_markdown: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_markdown.is_null() {
            return Err("out_markdown pointer is null".into());
        }
        let parsed = wellfriendpdf(doc.engine.parse_document(&ParseOptions::default()))?;
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
/// `wellfriendpdf_document_extract_semantic_json`, which serializes the older semantic
/// model and is retained only for back-compat; prefer this for new code.)
///
/// # Safety
///
/// `document` must be a valid open document. `out_json` must be writable and
/// any returned string must be freed with `wellfriendpdf_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_parse_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let parsed = wellfriendpdf(doc.engine.parse_document(&ParseOptions::default()))?;
        unsafe {
            *out_json = into_c_string(parsed.to_json());
        }
        Ok(())
    })
}

/// Registers a C-function-pointer OCR backend on the document, so the
/// `wellfriendpdf_document_parse_markdown_ocr` / `wellfriendpdf_document_parse_json_ocr`
/// functions route scanned pages through it. See [`WellfriendOcrBackend`] for the
/// callback contract. Pass a backend with a null `recognize` to clear a
/// previously-registered backend.
///
/// Returns `WELLFRIENDPDF_STATUS_OK` on success (including clearing), or
/// `WELLFRIENDPDF_STATUS_ERROR` if the document is null.
///
/// # Safety
///
/// `document` must be a valid open document. `backend.recognize` /
/// `backend.userdata` must remain valid until the document is freed or the
/// backend is cleared/replaced. `backend.name`, if non-null, must be a valid
/// NUL-terminated string for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_set_ocr_backend(
    document: *mut WellfriendDocument,
    backend: WellfriendOcrBackend,
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
            .map(|e| Arc::new(e) as Arc<dyn wellfriendpdf_engine::OcrEngine>);
        Ok(())
    })
}

/// Parses the document to canonical Markdown **with OCR** for scanned pages,
/// using the backend registered via `wellfriendpdf_document_set_ocr_backend`. If no
/// backend is registered this behaves exactly like `wellfriendpdf_document_parse_markdown`
/// (scanned pages degrade to the placeholder).
///
/// # Safety
///
/// Same as `wellfriendpdf_document_parse_markdown`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_parse_markdown_ocr(
    document: *const WellfriendDocument,
    out_markdown: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_markdown.is_null() {
            return Err("out_markdown pointer is null".into());
        }
        let parsed = wellfriendpdf(doc.engine.parse_document(&parse_options_with_ocr(doc)))?;
        unsafe {
            *out_markdown = into_c_string(parsed.to_markdown_default());
        }
        Ok(())
    })
}

/// Parses the document to canonical JSON **with OCR** for scanned pages, using
/// the backend registered via `wellfriendpdf_document_set_ocr_backend`. If no backend is
/// registered this behaves exactly like `wellfriendpdf_document_parse_json`.
///
/// # Safety
///
/// Same as `wellfriendpdf_document_parse_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_parse_json_ocr(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let parsed = wellfriendpdf(doc.engine.parse_document(&parse_options_with_ocr(doc)))?;
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
/// must be freed with `wellfriendpdf_string_free`. If `error_out` is non-null, it must
/// be writable and any returned string must be freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_extract_fields_json(
    document: *const WellfriendDocument,
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
        let fields = wellfriendpdf(doc.engine.extract_fields(&opts))?;
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
/// any returned string must be freed with `wellfriendpdf_string_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_info_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let info = wellfriendpdf(doc.engine.document_info())?;
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
/// any returned buffer must be freed with `wellfriendpdf_buffer_free`. If `error_out`
/// is non-null, it must be writable and any returned string must be freed with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_render_page_png(
    document: *const WellfriendDocument,
    page: usize,
    dpi: u32,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let png = wellfriendpdf(doc.engine.render_page_png_fast(page, dpi))?;
        unsafe {
            *out_buffer = into_buffer(png);
        }
        Ok(())
    })
}

/// Returns the canonical default render contract as JSON for one page.
///
/// `render_mode` may be null (the deterministic compatibility default),
/// `compat`, or `high`. The returned JSON is owned and must be freed with
/// `wellfriendpdf_string_free`.
///
/// # Safety
/// `document` must be a valid open document. `render_mode`, when non-null,
/// must point to a valid NUL-terminated UTF-8 string. `out_json` must be
/// writable; its returned value is owned by the caller. `error_out`, when
/// non-null, must be writable and its value freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_default_render_contract_json(
    document: *const WellfriendDocument,
    page: usize,
    dpi: u32,
    render_mode: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let mode_name =
            unsafe { optional_c_string(render_mode) }?.unwrap_or_else(|| "compat".to_string());
        let mode = wellfriendpdf_engine::RenderMode::from_name(&mode_name)
            .ok_or_else(|| format!("unsupported render mode '{mode_name}'"))?;
        let contract = wellfriendpdf(doc.engine.default_render_contract(page, dpi, mode))?;
        let json = serde_json::to_string(&contract).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Renders a page to PNG using a versioned canonical render-contract JSON.
///
/// The contract's document revision and every supported raster policy are
/// validated by the Rust core; unsupported policy combinations return a stable
/// C ABI error rather than being silently ignored.
///
/// # Safety
/// `document` must be a valid open document. `contract_json` must point to a
/// valid NUL-terminated UTF-8 JSON string. `out_buffer` must be writable and
/// its returned value freed with `wellfriendpdf_buffer_free`. `error_out`, when
/// non-null, must be writable and its value freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_render_page_png_with_contract_json(
    document: *const WellfriendDocument,
    contract_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let contract_json = unsafe { required_c_string(contract_json, "contract_json") }?;
        let contract: wellfriendpdf_engine::RenderContract =
            serde_json::from_str(&contract_json)
                .map_err(|err| format!("render contract JSON: {err}"))?;
        let png =
            wellfriendpdf(doc.engine.render_page_png_with_contract(
                &contract,
                &wellfriendpdf_engine::CancelToken::none(),
            ))?;
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_render_page_jpeg(
    document: *const WellfriendDocument,
    page: usize,
    dpi: u32,
    quality: u8,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let (jpeg, _, _) = wellfriendpdf(wellfriendpdf_engine::render_page_image(
            &doc.engine,
            page,
            dpi,
            wellfriendpdf_engine::RasterImageFormat::Jpeg,
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
/// with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_extract_pages_pdf(
    document: *const WellfriendDocument,
    pages: *const usize,
    pages_len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let mut pages = unsafe { read_pages(pages, pages_len) }?;
        if pages.is_empty() {
            let total = wellfriendpdf(doc.engine.page_count())?;
            pages = (1..=total).collect();
        }
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = wellfriendpdf(doc.engine.extract_pages(&pages))?;
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
/// Same contract as `wellfriendpdf_document_extract_pages_pdf`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_organize_pdf(
    document: *const WellfriendDocument,
    pages: *const usize,
    pages_len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        wellfriendpdf_document_extract_pages_pdf(document, pages, pages_len, out_buffer, error_out)
    }
}

/// Rotates selected pages and returns a new PDF.
///
/// # Safety
///
/// `document` must be valid. `pages` must point to `pages_len` readable
/// entries unless `pages_len` is zero. `out_buffer` must be writable and freed
/// with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_rotate_pdf(
    document: *const WellfriendDocument,
    pages: *const usize,
    pages_len: usize,
    angle: c_int,
    relative: c_int,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let pages = unsafe { read_pages(pages, pages_len) }?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let rotation = if relative != 0 {
            wellfriendpdf_engine::Rotation::Relative(angle)
        } else {
            wellfriendpdf_engine::Rotation::Absolute(angle)
        };
        let out = wellfriendpdf(wellfriendpdf_engine::rotate_pages(
            &doc.engine,
            &pages,
            rotation,
        ))?;
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_optimize_pdf(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let (out, _) = wellfriendpdf(wellfriendpdf_engine::optimize(&doc.engine))?;
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_linearize_pdf(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = wellfriendpdf(wellfriendpdf_engine::linearize(&doc.engine))?;
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_decrypt_pdf(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = wellfriendpdf(wellfriendpdf_engine::decrypt_pdf(&doc.engine))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Encrypts an opened document with `/Adobe.PubSec` for one recipient
/// certificate and returns the encrypted PDF.
///
/// # Safety
///
/// `document` must be valid. `recipient_certificate` must point to
/// `recipient_certificate_len` readable bytes. `out_buffer` must be writable
/// and freed with `wellfriendpdf_buffer_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pubsec_encrypt_pdf(
    document: *const WellfriendDocument,
    recipient_certificate: *const u8,
    recipient_certificate_len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if recipient_certificate.is_null() {
            return Err("recipient_certificate pointer is null".into());
        }
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let cert =
            unsafe { slice::from_raw_parts(recipient_certificate, recipient_certificate_len) };
        let recipient =
            wellfriendpdf(wellfriendpdf_engine::PubSecRecipientCertificate::from_bytes(cert))?;
        let options = wellfriendpdf_engine::PubSecEncryptOptions {
            recipients: vec![recipient],
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: wellfriendpdf_engine::CryptMethod::AesV2,
            recipient_id_mode: wellfriendpdf_engine::PubSecRecipientIdMode::IssuerAndSerial,
        };
        let (out, _) = wellfriendpdf(wellfriendpdf_engine::encrypt_pdf_pubsec(
            doc.engine.document().reader(),
            &options,
        ))?;
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_encrypt_aes256_pdf(
    document: *const WellfriendDocument,
    user_password: *const c_char,
    owner_password: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        use wellfriendpdf_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};
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
        let out = wellfriendpdf(wellfriendpdf_engine::encrypt(&doc.engine, &params))?;
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
/// `wellfriendpdf_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_to_html(
    document: *const WellfriendDocument,
    out_html: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_html.is_null() {
            return Err("out_html pointer is null".into());
        }
        let html = wellfriendpdf(wellfriendpdf_engine::html_string(&doc.engine, &[]))?;
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
/// with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_to_xlsx(
    document: *const WellfriendDocument,
    layout: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let layout = unsafe { optional_c_string(layout) }?.unwrap_or_else(|| "pages".to_string());
        let layout = wellfriendpdf_engine::XlsxLayout::parse(&layout)
            .ok_or_else(|| "layout must be pages or tables".to_string())?;
        let out = wellfriendpdf(wellfriendpdf_engine::pdf_to_xlsx(
            &doc.engine,
            &wellfriendpdf_engine::XlsxOptions { layout },
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_to_pptx(
    document: *const WellfriendDocument,
    include_images: c_int,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = wellfriendpdf(wellfriendpdf_engine::pdf_to_pptx(
            &doc.engine,
            &wellfriendpdf_engine::PptxOptions {
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_to_docx(
    document: *const WellfriendDocument,
    include_images: c_int,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let out = wellfriendpdf(wellfriendpdf_engine::pdf_to_docx(
            &doc.engine,
            &wellfriendpdf_engine::DocxOptions {
                include_images: include_images != 0,
                layout: wellfriendpdf_engine::DocxLayout::Flowing,
            },
        ))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

/// Converts a document to DOCX with an explicit form action policy layout mode.
///
/// # Safety
/// `layout` must be a NUL-terminated UTF-8 string. Other pointers follow
/// `wellfriendpdf_document_to_docx` ownership rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_to_docx_with_layout(
    document: *const WellfriendDocument,
    include_images: c_int,
    layout: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    let layout = unsafe { required_c_string(layout, "layout") };
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let layout = layout
            .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)
            .and_then(|value| {
                wellfriendpdf_engine::DocxLayout::parse(&value).ok_or_else(|| {
                    wellfriendpdf_engine::WellfriendError::invalid_input(
                        "unknown DOCX layout; use flowing, page-faithful, or hybrid",
                    )
                })
            })
            .map_err(|error| error.to_string())?;
        let out = wellfriendpdf(wellfriendpdf_engine::pdf_to_docx(
            &doc.engine,
            &wellfriendpdf_engine::DocxOptions {
                include_images: include_images != 0,
                layout,
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
/// freed with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_docx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = wellfriendpdf(wellfriendpdf_engine::docx_to_pdf(
            bytes,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
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
/// freed with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xlsx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = wellfriendpdf(wellfriendpdf_engine::xlsx_to_pdf(
            bytes,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
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
/// freed with `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_pptx_to_pdf(
    data: *const u8,
    len: usize,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let out = wellfriendpdf(wellfriendpdf_engine::pptx_to_pdf(
            bytes,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
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
/// `wellfriendpdf_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_fonts_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let fonts = wellfriendpdf(doc.engine.list_fonts())?;
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
/// `wellfriendpdf_string_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_signatures_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let reports = wellfriendpdf(doc.engine.verify_signatures())?;
        let json = serde_json::to_string(&reports).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Returns Signature Validation signature verification reports with explicit options JSON.
///
/// `options_json` may be NULL for defaults. When non-NULL it must be a
/// NUL-terminated UTF-8 JSON object accepted by
/// `wellfriendpdf_engine::verify_options_from_json`.
///
/// # Safety
///
/// `document` must be valid. `out_json` must be writable and freed with
/// `wellfriendpdf_string_free`. `options_json`, if non-null, must be a valid
/// NUL-terminated UTF-8 string. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_signatures_with_options_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let options_json = if options_json.is_null() {
            "{}".to_string()
        } else {
            unsafe { required_c_string(options_json, "options_json") }?
        };
        let options = wellfriendpdf(wellfriendpdf_engine::verify_options_from_json(
            &options_json,
        ))?;
        let reports = wellfriendpdf(doc.engine.verify_signatures_with_options(&options))?;
        let json = serde_json::to_string(&reports).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Returns Signature Validation signature validation reports together with the explicit
/// content-addressed evidence bundle accepted by the shared engine pipeline.
///
/// This opt-in function can include bounded DER evidence in the returned JSON;
/// callers must treat it as sensitive validation material and free it with
/// `wellfriendpdf_string_free`.
///
/// # Safety
///
/// `document` must be valid. `out_json` must be writable and freed with
/// `wellfriendpdf_string_free`. `options_json`, if non-null, must be a valid
/// NUL-terminated UTF-8 string. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_signature_validation_with_evidence_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let options_json = if options_json.is_null() {
            "{}".to_string()
        } else {
            unsafe { required_c_string(options_json, "options_json") }?
        };
        let options = wellfriendpdf(wellfriendpdf_engine::verify_options_from_json(
            &options_json,
        ))?;
        let outcome = wellfriendpdf(
            doc.engine
                .verify_signatures_with_options_and_evidence(&options),
        )?;
        let json = serde_json::to_string(&outcome).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Validate a caller-supplied RFC 3161 signature timestamp token.
///
/// `signature_value` must be the exact CMS SignerInfo.signature octets that
/// the token's TSTInfo.messageImprint claims to bind.
///
/// # Safety
///
/// `token` and `signature_value` must point to readable buffers. `out_json`
/// must be writable and freed with `wellfriendpdf_string_free`. `options_json`, if
/// non-null, must be a valid NUL-terminated UTF-8 VerifyOptions object.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_timestamp_token_validation_json(
    token: *const u8,
    token_len: usize,
    signature_value: *const u8,
    signature_value_len: usize,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if token.is_null() {
            return Err("token pointer is null".into());
        }
        if signature_value.is_null() {
            return Err("signature_value pointer is null".into());
        }
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let options_json = if options_json.is_null() {
            "{}".to_string()
        } else {
            unsafe { required_c_string(options_json, "options_json") }?
        };
        let token = unsafe { slice::from_raw_parts(token, token_len) };
        let signature_value =
            unsafe { slice::from_raw_parts(signature_value, signature_value_len) };
        let json = wellfriendpdf(sdk::timestamp_token_validation_json(
            token,
            signature_value,
            &options_json,
        ))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Allocates an opaque Signature Validation signature-validation options handle.
///
/// The handle starts offline with no implicit trust anchors. Use the explicit
/// add/set functions below to load certificates, evidence, validation time,
/// revocation mode, retrieval policy, and replay bundle.
///
/// # Safety
///
/// `error_out`, if non-null, must be writable and any returned string must be
/// freed with `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureValidationOptions {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureValidationOptions {
            options: wellfriendpdf_engine::VerifyOptions::default(),
        }))
    })) {
        Ok(options) => options,
        Err(_) => {
            set_error(
                error_out,
                "panic while creating signature validation options",
            );
            ptr::null_mut()
        }
    }
}

/// Frees a handle returned by `wellfriendpdf_signature_validation_options_new`.
///
/// # Safety
///
/// `options` must be null or a live handle returned by the constructor and
/// not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_free(
    options: *mut WellfriendSignatureValidationOptions,
) {
    if !options.is_null() {
        let _ = unsafe { Box::from_raw(options) };
    }
}

/// Allocates an explicit Signature Validation trust-anchor store. It begins empty and
/// never reads the platform trust store.
///
/// # Safety
///
/// `error_out` may be null or must point to writable storage for an error
/// string allocated by this library.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_trust_store_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureTrustStore {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureTrustStore {
            store: wellfriendpdf_engine::TrustStore::new(),
            distrusted_certificate_sha256: Vec::new(),
        }))
    })) {
        Ok(store) => store,
        Err(_) => {
            set_error(error_out, "panic while creating signature trust store");
            ptr::null_mut()
        }
    }
}

/// Frees a trust store returned by `wellfriendpdf_signature_trust_store_new`.
///
/// # Safety
///
/// `store` must be null or a live handle returned by
/// `wellfriendpdf_signature_trust_store_new` and not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_trust_store_free(
    store: *mut WellfriendSignatureTrustStore,
) {
    if !store.is_null() {
        let _ = unsafe { Box::from_raw(store) };
    }
}

/// Adds an explicit DER trust anchor. The input is parsed and canonicalized at
/// insertion time, so malformed bytes never enter the selected-anchor pool.
///
/// # Safety
///
/// `store` must be a live trust-store handle. `data` must point to `len`
/// readable bytes when `len` is nonzero. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_trust_store_add_anchor_der(
    store: *mut WellfriendSignatureTrustStore,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_trust_store_mut(store)?;
        let der = unsafe { read_input_bytes(data, len, "trust anchor") }?;
        if der.is_empty() {
            return Err("trust anchor DER must not be empty".to_string());
        }
        wellfriendpdf(store.store.add_der(der, "c_abi", None))?;
        Ok(())
    })
}

/// Adds a SHA-256 certificate fingerprint to the store's deny overlay. The
/// normalized value is applied whenever the store is attached to options.
///
/// # Safety
///
/// `store` must be a live trust-store handle. `fingerprint` must be a valid
/// NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_trust_store_add_distrusted_certificate_sha256(
    store: *mut WellfriendSignatureTrustStore,
    fingerprint: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_trust_store_mut(store)?;
        let fingerprint = unsafe { required_c_string(fingerprint, "fingerprint") }?;
        let normalized = wellfriendpdf(
            wellfriendpdf_engine::VerifyOptions::default()
                .with_distrusted_certificate_sha256(&fingerprint),
        )?
        .distrusted_certificate_sha256
        .into_iter()
        .next()
        .ok_or_else(|| "normalized distrust fingerprint was missing".to_string())?;
        if !store
            .distrusted_certificate_sha256
            .iter()
            .any(|existing| existing == &normalized)
        {
            store.distrusted_certificate_sha256.push(normalized);
            store.distrusted_certificate_sha256.sort();
        }
        Ok(())
    })
}

/// Allocates an opaque untrusted intermediate store.
///
/// # Safety
///
/// `error_out` may be null or must point to writable storage for an error
/// string allocated by this library.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_intermediate_store_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureIntermediateStore {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureIntermediateStore {
            store: wellfriendpdf_engine::IntermediateStore::new(),
        }))
    })) {
        Ok(store) => store,
        Err(_) => {
            set_error(
                error_out,
                "panic while creating signature intermediate store",
            );
            ptr::null_mut()
        }
    }
}

/// Frees an intermediate store returned by
/// `wellfriendpdf_signature_intermediate_store_new`.
///
/// # Safety
///
/// `store` must be null or a live handle returned by
/// `wellfriendpdf_signature_intermediate_store_new` and not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_intermediate_store_free(
    store: *mut WellfriendSignatureIntermediateStore,
) {
    if !store.is_null() {
        let _ = unsafe { Box::from_raw(store) };
    }
}

/// Adds a DER intermediate certificate. This parses and canonicalizes the
/// certificate but does not confer trust.
///
/// # Safety
///
/// `store` must be a live intermediate-store handle. `data` must point to
/// `len` readable bytes when `len` is nonzero. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_intermediate_store_add_der(
    store: *mut WellfriendSignatureIntermediateStore,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_intermediate_store_mut(store)?;
        let der = unsafe { read_input_bytes(data, len, "intermediate") }?;
        if der.is_empty() {
            return Err("intermediate DER must not be empty".to_string());
        }
        wellfriendpdf(store.store.add_der(der))?;
        Ok(())
    })
}

/// Allocates an opaque supplied/replayed evidence store.
///
/// # Safety
///
/// `error_out` may be null or must point to writable storage for an error
/// string allocated by this library.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_evidence_store_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureEvidenceStore {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureEvidenceStore {
            ocsp_responses_der: Vec::new(),
            crls_der: Vec::new(),
            bundle: None,
        }))
    })) {
        Ok(store) => store,
        Err(_) => {
            set_error(error_out, "panic while creating signature evidence store");
            ptr::null_mut()
        }
    }
}

/// Frees an evidence store returned by `wellfriendpdf_signature_evidence_store_new`.
///
/// # Safety
///
/// `store` must be null or a live handle returned by
/// `wellfriendpdf_signature_evidence_store_new` and not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_evidence_store_free(
    store: *mut WellfriendSignatureEvidenceStore,
) {
    if !store.is_null() {
        let _ = unsafe { Box::from_raw(store) };
    }
}

/// Adds DER OCSP bytes as caller-supplied evidence. Parsing and authorization
/// happen later against the selected path; this call only copies bounded input.
///
/// # Safety
///
/// `store` must be a live evidence-store handle. `data` must point to `len`
/// readable bytes when `len` is nonzero. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_evidence_store_add_ocsp_der(
    store: *mut WellfriendSignatureEvidenceStore,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_evidence_store_mut(store)?;
        let der = unsafe { read_input_bytes(data, len, "OCSP response") }?.to_vec();
        if der.is_empty() {
            return Err("OCSP response DER must not be empty".to_string());
        }
        store.ocsp_responses_der.push(der);
        Ok(())
    })
}

/// Adds DER CRL bytes as caller-supplied evidence. A CRL remains untrusted
/// until issuer, signature, scope, freshness, and policy are validated.
///
/// # Safety
///
/// `store` must be a live evidence-store handle. `data` must point to `len`
/// readable bytes when `len` is nonzero. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_evidence_store_add_crl_der(
    store: *mut WellfriendSignatureEvidenceStore,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_evidence_store_mut(store)?;
        let der = unsafe { read_input_bytes(data, len, "CRL") }?.to_vec();
        if der.is_empty() {
            return Err("CRL DER must not be empty".to_string());
        }
        store.crls_der.push(der);
        Ok(())
    })
}

/// Imports a portable evidence bundle into the opaque evidence store. Bundle
/// schema, hashes, duplicates, entry counts, and byte limits are checked here;
/// its contents are revalidated on every signature validation use.
///
/// # Safety
///
/// `store` must be a live evidence-store handle. `bundle_json` must be a valid
/// NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_evidence_store_set_bundle_json(
    store: *mut WellfriendSignatureEvidenceStore,
    bundle_json: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_evidence_store_mut(store)?;
        let bundle_json = unsafe { required_c_string(bundle_json, "bundle_json") }?;
        let bundle: wellfriendpdf_engine::EvidenceBundle = serde_json::from_str(&bundle_json)
            .map_err(|error| format!("evidence bundle JSON: {error}"))?;
        let budget = wellfriendpdf_engine::NetworkBudget::default();
        wellfriendpdf_engine::EvidenceStore::import_bundle(
            &bundle,
            budget.max_cache_entries,
            budget.max_cache_bytes,
        )
        .map_err(|error| format!("evidence bundle: {error}"))?;
        store.bundle = Some(bundle);
        Ok(())
    })
}

/// Allocates a bounded retrieval-policy handle. It starts offline.
///
/// # Safety
///
/// `error_out` may be null or must point to writable storage for an error
/// string allocated by this library.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_retrieval_policy_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureRetrievalPolicy {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureRetrievalPolicy {
            policy: wellfriendpdf_engine::RetrievalPolicy::offline(),
        }))
    })) {
        Ok(policy) => policy,
        Err(_) => {
            set_error(error_out, "panic while creating signature retrieval policy");
            ptr::null_mut()
        }
    }
}

/// Frees a retrieval-policy handle.
///
/// # Safety
///
/// `policy` must be null or a live handle returned by
/// `wellfriendpdf_signature_retrieval_policy_new` and not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_retrieval_policy_free(
    policy: *mut WellfriendSignatureRetrievalPolicy,
) {
    if !policy.is_null() {
        let _ = unsafe { Box::from_raw(policy) };
    }
}

/// Replaces a retrieval policy from a complete JSON object. Online I/O remains
/// disabled unless the object explicitly sets `enabled` to true.
///
/// # Safety
///
/// `policy` must be a live retrieval-policy handle. `policy_json` must be a
/// valid NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_retrieval_policy_set_json(
    policy: *mut WellfriendSignatureRetrievalPolicy,
    policy_json: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let policy = checked_signature_retrieval_policy_mut(policy)?;
        let policy_json = unsafe { required_c_string(policy_json, "policy_json") }?;
        let parsed: wellfriendpdf_engine::RetrievalPolicy = serde_json::from_str(&policy_json)
            .map_err(|error| format!("retrieval policy JSON: {error}"))?;
        parsed
            .validate()
            .map_err(|error| format!("retrieval policy: {error}"))?;
        policy.policy = parsed;
        Ok(())
    })
}

/// Allocates a cooperative cancellation source for signature validation.
///
/// # Safety
///
/// `error_out` may be null or must point to writable storage for an error
/// string allocated by this library.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_cancellation_new(
    error_out: *mut *mut c_char,
) -> *mut WellfriendSignatureValidationCancellation {
    clear_error(error_out);
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(WellfriendSignatureValidationCancellation {
            token: wellfriendpdf_engine::CancelToken::new(),
        }))
    })) {
        Ok(cancellation) => cancellation,
        Err(_) => {
            set_error(
                error_out,
                "panic while creating signature validation cancellation",
            );
            ptr::null_mut()
        }
    }
}

/// Signal cancellation. It is safe to call from another native thread while a
/// validation call is running with an attached clone.
///
/// # Safety
///
/// `cancellation` must be a live cancellation handle. `error_out` follows the
/// library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_cancellation_cancel(
    cancellation: *const WellfriendSignatureValidationCancellation,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let cancellation = checked_signature_validation_cancellation(cancellation)?;
        cancellation.token.cancel();
        Ok(())
    })
}

/// Frees a cancellation source. Attached option handles keep their own token
/// clone, so freeing this pointer does not invalidate an in-flight validation.
///
/// # Safety
///
/// `cancellation` must be null or a live handle returned by
/// `wellfriendpdf_signature_validation_cancellation_new` and not already freed.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_cancellation_free(
    cancellation: *mut WellfriendSignatureValidationCancellation,
) {
    if !cancellation.is_null() {
        let _ = unsafe { Box::from_raw(cancellation) };
    }
}

/// Copies an explicit trust store and its deny overlay into validation options.
///
/// # Safety
///
/// `options` and `store` must be live handles. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_apply_trust_store(
    options: *mut WellfriendSignatureValidationOptions,
    store: *const WellfriendSignatureTrustStore,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_trust_store(store)?;
        let options = checked_signature_validation_options_mut(options)?;
        let mut updated = options.options.clone().with_trust_store(&store.store);
        for fingerprint in &store.distrusted_certificate_sha256 {
            updated = wellfriendpdf(updated.with_distrusted_certificate_sha256(fingerprint))?;
        }
        options.options = updated;
        Ok(())
    })
}

/// Copies untrusted intermediate candidates into validation options.
///
/// # Safety
///
/// `options` and `store` must be live handles. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_apply_intermediate_store(
    options: *mut WellfriendSignatureValidationOptions,
    store: *const WellfriendSignatureIntermediateStore,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_intermediate_store(store)?;
        let options = checked_signature_validation_options_mut(options)?;
        options.options = options
            .options
            .clone()
            .with_intermediate_store(&store.store);
        Ok(())
    })
}

/// Copies supplied/replayed evidence into validation options. This does not
/// change trust-anchor configuration or enable online retrieval.
///
/// # Safety
///
/// `options` and `store` must be live handles. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_apply_evidence_store(
    options: *mut WellfriendSignatureValidationOptions,
    store: *const WellfriendSignatureEvidenceStore,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let store = checked_signature_evidence_store(store)?;
        let options = checked_signature_validation_options_mut(options)?;
        let mut updated = options.options.clone();
        updated
            .ocsp_responses_der
            .extend(store.ocsp_responses_der.iter().cloned());
        updated.crls_der.extend(store.crls_der.iter().cloned());
        if let Some(bundle) = &store.bundle {
            updated = wellfriendpdf(updated.with_evidence_bundle(bundle.clone()))?;
        }
        options.options = updated;
        Ok(())
    })
}

/// Copies a bounded retrieval policy into validation options.
///
/// # Safety
///
/// `options` and `policy` must be live handles. `error_out` follows the
/// library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_apply_retrieval_policy(
    options: *mut WellfriendSignatureValidationOptions,
    policy: *const WellfriendSignatureRetrievalPolicy,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let policy = checked_signature_retrieval_policy(policy)?;
        let options = checked_signature_validation_options_mut(options)?;
        options.options = wellfriendpdf(
            options
                .options
                .clone()
                .with_retrieval_policy(policy.policy.clone()),
        )?;
        Ok(())
    })
}

/// Attaches a clone of a cooperative cancellation source to validation options.
///
/// # Safety
///
/// `options` and `cancellation` must be live handles. `error_out` follows the
/// library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_cancellation(
    options: *mut WellfriendSignatureValidationOptions,
    cancellation: *const WellfriendSignatureValidationCancellation,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let cancellation = checked_signature_validation_cancellation(cancellation)?;
        let options = checked_signature_validation_options_mut(options)?;
        options.options = options
            .options
            .clone()
            .with_cancellation_token(cancellation.token.clone());
        Ok(())
    })
}

/// Adds DER trust-anchor certificate bytes to an opaque validation handle.
///
/// # Safety
///
/// `options` must be valid. `data` must address `len` readable bytes when
/// `len` is nonzero.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_add_trust_anchor_der(
    options: *mut WellfriendSignatureValidationOptions,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let der = unsafe { read_input_bytes(data, len, "trust anchor") }?.to_vec();
        if der.is_empty() {
            return Err("trust anchor DER must not be empty".to_string());
        }
        options.options.trust_anchors_der.push(der);
        Ok(())
    })
}

/// Adds an untrusted DER intermediate certificate to an opaque validation handle.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `data` must point to
/// `len` readable bytes when `len` is nonzero. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_add_intermediate_der(
    options: *mut WellfriendSignatureValidationOptions,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let der = unsafe { read_input_bytes(data, len, "intermediate") }?.to_vec();
        if der.is_empty() {
            return Err("intermediate DER must not be empty".to_string());
        }
        options.options.intermediates_der.push(der);
        Ok(())
    })
}

/// Adds a SHA-256 certificate fingerprint to the path deny list. The entry is
/// normalized by the Rust engine and is enforced during candidate and anchor
/// selection; it is not merely report metadata.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `fingerprint` must be
/// a valid NUL-terminated UTF-8 C string. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_add_distrusted_certificate_sha256(
    options: *mut WellfriendSignatureValidationOptions,
    fingerprint: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let fingerprint = unsafe { required_c_string(fingerprint, "fingerprint") }?;
        options.options = wellfriendpdf(
            options
                .options
                .clone()
                .with_distrusted_certificate_sha256(&fingerprint),
        )?;
        Ok(())
    })
}

/// Adds a caller-supplied DER OCSP response to an opaque validation handle.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `data` must point to
/// `len` readable bytes when `len` is nonzero. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_add_ocsp_der(
    options: *mut WellfriendSignatureValidationOptions,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let der = unsafe { read_input_bytes(data, len, "OCSP response") }?.to_vec();
        if der.is_empty() {
            return Err("OCSP response DER must not be empty".to_string());
        }
        options.options.ocsp_responses_der.push(der);
        Ok(())
    })
}

/// Adds a caller-supplied DER CRL to an opaque validation handle.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `data` must point to
/// `len` readable bytes when `len` is nonzero. `error_out` follows the library
/// error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_add_crl_der(
    options: *mut WellfriendSignatureValidationOptions,
    data: *const u8,
    len: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let der = unsafe { read_input_bytes(data, len, "CRL") }?.to_vec();
        if der.is_empty() {
            return Err("CRL DER must not be empty".to_string());
        }
        options.options.crls_der.push(der);
        Ok(())
    })
}

/// Sets an explicit Unix validation time. Use
/// `wellfriendpdf_signature_validation_options_clear_validation_time` to return to
/// the system clock.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `error_out` follows
/// the library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_validation_time_unix(
    options: *mut WellfriendSignatureValidationOptions,
    validation_time_unix: u64,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        options.options.validation_time_unix = Some(validation_time_unix);
        Ok(())
    })
}

/// Clears a caller-selected validation time so the engine uses its injected
/// system clock source.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `error_out` follows
/// the library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_clear_validation_time(
    options: *mut WellfriendSignatureValidationOptions,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        options.options.validation_time_unix = None;
        Ok(())
    })
}

/// Selects a revocation mode: 0 = not checked, 1 = offline strict,
/// 2 = offline best effort, 3 = online strict, 4 = online best effort.
/// Online modes still require an explicit bounded retrieval policy; selecting
/// a numeric mode alone never enables network access.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `error_out` follows
/// the library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_revocation_mode(
    options: *mut WellfriendSignatureValidationOptions,
    mode: c_int,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        options.options.revocation_mode = match mode {
            0 => wellfriendpdf_engine::SignatureRevocationMode::NotChecked,
            1 => wellfriendpdf_engine::SignatureRevocationMode::OfflineStrict,
            2 => wellfriendpdf_engine::SignatureRevocationMode::OfflineBestEffort,
            3 => wellfriendpdf_engine::SignatureRevocationMode::OnlineStrict,
            4 => wellfriendpdf_engine::SignatureRevocationMode::OnlineBestEffort,
            _ => return Err("unknown signature revocation mode".to_string()),
        };
        Ok(())
    })
}

/// Applies a complete bounded retrieval policy JSON object to an opaque
/// validation handle. Network retrieval remains disabled unless the policy's
/// explicit `enabled` field is true.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `policy_json` must be a
/// valid NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_retrieval_policy_json(
    options: *mut WellfriendSignatureValidationOptions,
    policy_json: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let policy_json = unsafe { required_c_string(policy_json, "policy_json") }?;
        let policy: wellfriendpdf_engine::RetrievalPolicy = serde_json::from_str(&policy_json)
            .map_err(|error| format!("retrieval policy JSON: {error}"))?;
        options.options = wellfriendpdf(options.options.clone().with_retrieval_policy(policy))?;
        Ok(())
    })
}

/// Applies an explicit CMS/PKIX algorithm-policy JSON object to an opaque
/// validation handle. A recognized legacy algorithm is still rejected when
/// this policy forbids it.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `policy_json` must be a
/// valid NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_algorithm_policy_json(
    options: *mut WellfriendSignatureValidationOptions,
    policy_json: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let policy_json = unsafe { required_c_string(policy_json, "policy_json") }?;
        let policy: wellfriendpdf_engine::SignatureAlgorithmPolicy =
            serde_json::from_str(&policy_json)
                .map_err(|error| format!("algorithm policy JSON: {error}"))?;
        options.options = wellfriendpdf(options.options.clone().with_algorithm_policy(policy))?;
        Ok(())
    })
}

/// Applies a replayable evidence-bundle JSON object to an opaque validation
/// handle. Imported certificates remain intermediates and all OCSP/CRL bytes
/// are revalidated by the engine.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `bundle_json` must be a
/// valid NUL-terminated UTF-8 C string. `error_out` follows the library error
/// ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_evidence_bundle_json(
    options: *mut WellfriendSignatureValidationOptions,
    bundle_json: *const c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let options = checked_signature_validation_options_mut(options)?;
        let bundle_json = unsafe { required_c_string(bundle_json, "bundle_json") }?;
        let bundle: wellfriendpdf_engine::EvidenceBundle = serde_json::from_str(&bundle_json)
            .map_err(|error| format!("evidence bundle JSON: {error}"))?;
        options.options = wellfriendpdf(options.options.clone().with_evidence_bundle(bundle))?;
        Ok(())
    })
}

/// Sets bounded certificate-path search limits on an opaque validation handle.
///
/// # Safety
///
/// `options` must be a live validation-options handle. `error_out` follows
/// the library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_signature_validation_options_set_path_limits(
    options: *mut WellfriendSignatureValidationOptions,
    max_chain_depth: usize,
    max_path_candidates: usize,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if max_chain_depth == 0 || max_path_candidates == 0 {
            return Err("path limits must be greater than zero".to_string());
        }
        let options = checked_signature_validation_options_mut(options)?;
        options.options.max_chain_depth = max_chain_depth;
        options.options.max_path_candidates = max_path_candidates;
        Ok(())
    })
}

/// Validates signatures using an opaque options handle and returns the normal
/// structured report JSON. The configuration handle remains caller-owned.
///
/// # Safety
///
/// `document` and `options` must be live handles. `out_json` must point to
/// writable storage for a string allocated by this library, which the caller
/// must free with the exported string-free function. `error_out` follows the
/// library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_signatures_with_options_handle(
    document: *const WellfriendDocument,
    options: *const WellfriendSignatureValidationOptions,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let options = checked_signature_validation_options(options)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".to_string());
        }
        let reports = wellfriendpdf(doc.engine.verify_signatures_with_options(&options.options))?;
        let json = serde_json::to_string(&reports).map_err(|error| error.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Validates signatures using an opaque options handle and returns the report
/// plus exportable accepted evidence. The configuration handle is not consumed.
///
/// # Safety
///
/// `document` and `options` must be live handles. `out_json` must point to
/// writable storage for a string allocated by this library, which the caller
/// must free with the exported string-free function. `error_out` follows the
/// library error ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_signature_validation_with_evidence_handle(
    document: *const WellfriendDocument,
    options: *const WellfriendSignatureValidationOptions,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        let options = checked_signature_validation_options(options)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".to_string());
        }
        let outcome = wellfriendpdf(
            doc.engine
                .verify_signatures_with_options_and_evidence(&options.options),
        )?;
        let json = serde_json::to_string(&outcome).map_err(|error| error.to_string())?;
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
/// string. `out_buffer` must be writable and freed with `wellfriendpdf_buffer_free`.
/// `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_watermark_text_pdf(
    document: *const WellfriendDocument,
    text: *const c_char,
    opacity: f64,
    rotation_degrees: f64,
    font_size: f64,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let text = unsafe { required_c_string(text, "text") }?;
        let input = wellfriendpdf(wellfriendpdf_engine::decrypt_pdf(&doc.engine))?;
        let out = wellfriendpdf(wellfriendpdf_engine::watermark_text_pdf(
            input,
            &text,
            wellfriendpdf_engine::TextWatermarkOptions {
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_add_page_numbers_pdf(
    document: *const WellfriendDocument,
    format: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        let format = unsafe { optional_c_string(format) }?
            .unwrap_or_else(|| "Page {n} of {total}".to_string());
        let input = wellfriendpdf(wellfriendpdf_engine::decrypt_pdf(&doc.engine))?;
        let out = wellfriendpdf(wellfriendpdf_engine::add_page_numbers_pdf(
            input,
            wellfriendpdf_engine::PageNumberOptions {
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
/// `wellfriendpdf_buffer_free`. `error_out`, if non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_images_to_pdf(
    images: *const *const u8,
    lengths: *const usize,
    count: usize,
    out_buffer: *mut WellfriendBuffer,
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
        let out = wellfriendpdf(wellfriendpdf_engine::images_to_pdf_from_bytes(
            &borrowed,
            wellfriendpdf_engine::ImageToPdfOptions::default(),
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
/// must be writable and freed with `wellfriendpdf_buffer_free`. `error_out`, if
/// non-null, must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_merge_pdfs_from_bytes(
    inputs: *const *const u8,
    lengths: *const usize,
    count: usize,
    out_buffer: *mut WellfriendBuffer,
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
            engines.push(wellfriendpdf(ContentEngine::open_bytes(bytes))?);
        }
        let mut specs = Vec::with_capacity(engines.len());
        for engine in &engines {
            let total = wellfriendpdf(engine.page_count())?;
            specs.push((engine.document(), (1..=total).collect::<Vec<_>>()));
        }
        let out = wellfriendpdf(wellfriendpdf_engine::build_merged(&specs))?;
        unsafe {
            *out_buffer = into_buffer(out);
        }
        Ok(())
    })
}

// ── Report surfaces (shared wellfriendpdf_engine::sdk facade) ─────────────────────────
//
// Each returns a versioned-JSON envelope string
// `{"schema_version", "kind", "report"}` — the SAME bytes Python's report
// methods return, since both call the identical facade. The returned string is
// caller-owned; free it with `wellfriendpdf_string_free`. Output-producing operations
// (sanitize/canonicalize/redact) return the produced PDF via an `WellfriendBuffer`
// (free with `wellfriendpdf_buffer_free`) AND the report string.

/// The original file bytes backing an open document (copied out of the reader).
fn doc_bytes(doc: &WellfriendDocument) -> Vec<u8> {
    doc.engine.document().reader().file_bytes().to_vec()
}

/// Run a facade report closure over the document bytes and write the resulting
/// JSON string to `out_json`. Shared implementation for every read-only report.
unsafe fn report_json_impl(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
    f: impl FnOnce(&[u8]) -> WellfriendResult<String>,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = wellfriendpdf(f(&doc_bytes(doc)))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Run a facade output-producing closure and write both the produced bytes and
/// the JSON report. Shared implementation for sanitize/canonicalize/redact.
unsafe fn report_output_impl(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
    f: impl FnOnce(&[u8]) -> WellfriendResult<(Vec<u8>, String)>,
) -> c_int {
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let (bytes, json) = wellfriendpdf(f(&doc_bytes(doc)))?;
        unsafe {
            *out_buffer = into_buffer(bytes);
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Effective runtime configuration JSON. `config_json` may be NULL or a
/// NUL-terminated JSON/TOML-like runtime configuration string. The returned
/// string is owned by the caller and must be freed with
/// `wellfriendpdf_string_free`.
///
/// # Safety
/// `out_json`/`error_out` must be writable when non-null.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_runtime_effective_config_json(
    config_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let config = unsafe { optional_c_string(config_json) };
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let config = config.map_err(|err| err.to_string())?;
        let json = wellfriendpdf(sdk::runtime_effective_config_json(config.as_deref()))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Runtime capability report JSON for the two public modes.
///
/// # Safety
/// `out_json`/`error_out` must be writable when non-null.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_runtime_capabilities_json(
    config_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let config = unsafe { optional_c_string(config_json) };
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let config = config.map_err(|err| err.to_string())?;
        let json = wellfriendpdf(sdk::runtime_capabilities_json(config.as_deref()))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// OCR provider-family matrix JSON.
///
/// # Safety
/// `out_json`/`error_out` must be writable when non-null.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_ocr_provider_matrix_json(
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = wellfriendpdf(sdk::ocr_provider_matrix_json())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Security report JSON. See `report_json_impl` for ownership.
///
/// # Safety
/// `document` must be a valid open document. `out_json`/`error_out` must be
/// writable; free the returned string with `wellfriendpdf_string_free` / the error with
/// `wellfriendpdf_error_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_security_report_json(
    document: *const WellfriendDocument,
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
/// See `wellfriendpdf_document_security_report_json`; `mode` may be NULL or a
/// NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_parser_report_json(
    document: *const WellfriendDocument,
    mode: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let mode = unsafe { optional_c_string(mode) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let mode = mode.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::parser_report_json(b, mode.as_deref(), None)
        })
    }
}

/// Color / prepress report JSON. `profile` is `generic`|`pdfa`|`pdfx` (NULL →
/// `generic`).
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `profile` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_color_report_json(
    document: *const WellfriendDocument,
    profile: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let profile = unsafe { optional_c_string(profile) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let profile = profile.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::color_report_json(b, profile.as_deref())
        })
    }
}

/// Standards-profile validation report JSON. `profile` is
/// `pdfa`|`pdfua`|`pdfx`|`security`|`all` (NULL → `all`).
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `profile` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_validate_json(
    document: *const WellfriendDocument,
    profile: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let profile = unsafe { optional_c_string(profile) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let profile = profile.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::standards_profile_json(b, profile.as_deref(), None)
        })
    }
}

/// AcroForm field-inventory report JSON.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_forms_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::forms_report_json(b, None)
        })
    }
}

/// Incremental Signing Standards clause-mapped PDF/A validation report JSON. `target` may be NULL
/// or a profile label such as `PDF/A-2B`.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `target` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pdfa_standards_json(
    document: *const WellfriendDocument,
    target: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let target = unsafe { optional_c_string(target) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let target = target.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::pdfa_standards_json(b, target.as_deref(), None)
        })
    }
}

/// Incremental Signing Standards clause-mapped PDF/UA validation report JSON. `target` may be NULL
/// or `PDF/UA-1`.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `target` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pdfua_standards_json(
    document: *const WellfriendDocument,
    target: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let target = unsafe { optional_c_string(target) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let target = target.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::pdfua_standards_json(b, target.as_deref(), None)
        })
    }
}

/// Incremental Signing Standards clause-mapped PDF/X validation report JSON. `target` may be NULL
/// or a profile label such as `PDF/X-4`.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `target` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pdfx_standards_json(
    document: *const WellfriendDocument,
    target: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let target = unsafe { optional_c_string(target) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let target = target.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::pdfx_standards_json(b, target.as_deref(), None)
        })
    }
}

/// Incremental Signing Standards combined PDF/A + PDF/UA + PDF/X validation report JSON with
/// cross-profile conflicts. A single profile passing never hides another
/// failing. `target` may be NULL.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`; `target` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_standards_all_json(
    document: *const WellfriendDocument,
    target: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let target = unsafe { optional_c_string(target) };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let target = target.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::standards_all_json(b, target.as_deref(), None)
        })
    }
}

/// Incremental Signing Standards append-only incremental signing plan. `key_pem`/`cert_pem` are the
/// signer material (never logged). `certify` in 1..=3 plans a certification
/// (DocMDP) signature; any other value plans an approval signature. Returns the
/// placeholder capacity plan JSON (required vs. reserved bytes, fit, ByteRange).
///
/// # Safety
/// `document` must be a live handle. `key_pem`/`cert_pem` must be
/// NUL-terminated UTF-8. `out_json`/`error_out` must be writable; free the
/// returned string with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_sign_plan_json(
    document: *const WellfriendDocument,
    key_pem: *const c_char,
    cert_pem: *const c_char,
    placeholder_size: usize,
    certify: c_int,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let key = unsafe { required_c_string(key_pem, "key_pem") };
    let cert = unsafe { required_c_string(cert_pem, "cert_pem") };
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let key = key.clone()?;
        let cert = cert.clone()?;
        let signer = wellfriendpdf(wellfriendpdf_engine::PdfSigner::from_pem(&key, &cert, &[]))?;
        let intent = if (1..=3).contains(&certify) {
            wellfriendpdf_engine::SigningIntent::Certification {
                docmdp_permissions: certify as u8,
            }
        } else {
            wellfriendpdf_engine::SigningIntent::Approval
        };
        let options = wellfriendpdf_engine::IncrementalSigningOptions {
            signature: wellfriendpdf_engine::SignatureOptions {
                contents_reserved_bytes: placeholder_size.max(1),
                ..Default::default()
            },
            intent,
            retry_larger_placeholder: true,
            max_placeholder_bytes: 256 * 1024,
        };
        let plan = wellfriendpdf(wellfriendpdf_engine::plan_signature_placeholder(
            doc.engine.document(),
            &signer,
            &options,
        ))?;
        let json = serde_json::to_string(&plan).map_err(|err| err.to_string())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Incremental Signing Standards append-only incremental signing. Produces a signed PDF whose
/// original bytes are preserved as a prefix, reopened and validated before it
/// is returned. `key_pem`/`cert_pem` are the signer material (never logged).
/// `certify` in 1..=3 creates a certification (DocMDP) signature; otherwise an
/// approval signature. `field_name`/`reason` may be NULL. Returns the signed
/// PDF via `out_buffer` and an `IncrementalSignResult` JSON via `out_json`.
///
/// # Safety
/// `document` must be a live handle. `key_pem`/`cert_pem` must be
/// NUL-terminated UTF-8; `field_name`/`reason` may be NULL. `out_buffer` must
/// be freed with `wellfriendpdf_buffer_free` and `out_json` with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_sign_pdf(
    document: *const WellfriendDocument,
    key_pem: *const c_char,
    cert_pem: *const c_char,
    placeholder_size: usize,
    certify: c_int,
    field_name: *const c_char,
    reason: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let key = unsafe { required_c_string(key_pem, "key_pem") };
    let cert = unsafe { required_c_string(cert_pem, "cert_pem") };
    let field = unsafe { optional_c_string(field_name) };
    let reason = unsafe { optional_c_string(reason) };
    ffi_status(error_out, || {
        let doc = checked_doc(document)?;
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let key = key.clone()?;
        let cert = cert.clone()?;
        let field = field.clone()?;
        let reason = reason.clone()?;
        let signer = wellfriendpdf(wellfriendpdf_engine::PdfSigner::from_pem(&key, &cert, &[]))?;
        let mut signature = wellfriendpdf_engine::SignatureOptions {
            contents_reserved_bytes: placeholder_size.max(1),
            ..Default::default()
        };
        if let Some(field) = field {
            signature.field_name = field;
        }
        signature.reason = reason;
        let intent = if (1..=3).contains(&certify) {
            wellfriendpdf_engine::SigningIntent::Certification {
                docmdp_permissions: certify as u8,
            }
        } else {
            wellfriendpdf_engine::SigningIntent::Approval
        };
        let options = wellfriendpdf_engine::IncrementalSigningOptions {
            signature,
            intent,
            retry_larger_placeholder: true,
            max_placeholder_bytes: 256 * 1024,
        };
        let result = wellfriendpdf(wellfriendpdf_engine::sign_incremental(
            doc.engine.document(),
            wellfriendpdf_engine::IncrementalSigner::Local(&signer),
            &options,
        ))?;
        if !result.post_sign.signature_valid {
            return Err("post-sign validation failed; signed output not written".into());
        }
        let json = serde_json::to_string(&result).map_err(|err| err.to_string())?;
        unsafe {
            *out_buffer = into_buffer(result.signed_pdf);
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

macro_rules! xfa_document_report {
    ($name:ident, $sdk_fn:ident) => {
        /// Returns an owned XFA Runtime XFA JSON report.
        ///
        /// # Safety
        /// `document` must be a live Wellfriend handle. `out_json` and `error_out`
        /// must follow `wellfriendpdf_document_security_report_json` ownership rules.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            document: *const WellfriendDocument,
            out_json: *mut *mut c_char,
            error_out: *mut *mut c_char,
        ) -> c_int {
            unsafe {
                report_json_impl(document, out_json, error_out, |bytes| {
                    sdk::$sdk_fn(bytes, None)
                })
            }
        }
    };
}

xfa_document_report!(wellfriendpdf_document_xfa_report_json, xfa_report_json);
xfa_document_report!(wellfriendpdf_document_xfa_extract_json, xfa_extract_json);
xfa_document_report!(
    wellfriendpdf_document_xfa_script_report_json,
    xfa_script_report_json
);
xfa_document_report!(
    wellfriendpdf_document_xfa_security_report_json,
    xfa_security_report_json
);

/// Bounded XFA Runtime XFA runtime report.
///
/// # Safety
/// `script_policy` may be NULL or a NUL-terminated UTF-8 policy string. Other
/// pointers follow `wellfriendpdf_document_security_report_json` ownership rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_xfa_runtime_report_json(
    document: *const WellfriendDocument,
    script_policy: *const c_char,
    execute_events: c_int,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let policy = unsafe { optional_c_string(script_policy) };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let policy = policy.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::xfa_runtime_report_json(bytes, policy.as_deref(), execute_events != 0, None)
        })
    }
}

/// Annotation-inventory report JSON.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_annotations_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::annotation_report_json(b, None)
        })
    }
}

xfa_document_report!(
    wellfriendpdf_document_rich_media_report_json,
    rich_media_report_json
);
xfa_document_report!(
    wellfriendpdf_document_annotation_media_redaction_report_json,
    annotation_media_redaction_report_json
);
xfa_document_report!(
    wellfriendpdf_document_secure_mutation_report_json,
    secure_mutation_report_json
);
xfa_document_report!(
    wellfriendpdf_document_secure_mutation_closeout_report_json,
    secure_mutation_closeout_report_json
);
xfa_document_report!(
    wellfriendpdf_document_form_js_report_json,
    form_js_report_json
);
xfa_document_report!(
    wellfriendpdf_document_form_action_graph_json,
    form_action_graph_json
);
xfa_document_report!(
    wellfriendpdf_document_interactive_data_report_json,
    interactive_data_closeout_report_json
);
xfa_document_report!(
    wellfriendpdf_document_form_action_policy_report_json,
    form_action_policy_report_json
);
xfa_document_report!(
    wellfriendpdf_document_advanced_editing_report_json,
    advanced_editing_report_json
);
xfa_document_report!(
    wellfriendpdf_document_advanced_editing_closeout_report_json,
    advanced_editing_closeout_report_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_history_report_json,
    writer_history_report_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_history_font_reconstruction_report_json,
    writer_history_font_reconstruction_report_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_history_object_stream_report_json,
    writer_history_object_stream_report_json
);
xfa_document_report!(
    wellfriendpdf_document_compression_office_report_json,
    compression_office_report_json
);
xfa_document_report!(
    wellfriendpdf_document_crypto_writer_report_json,
    crypto_writer_report_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_determinism_audit_json,
    writer_determinism_audit_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_external_diff_json,
    writer_external_diff_json
);
xfa_document_report!(
    wellfriendpdf_document_writer_closeout_report_json,
    writer_closeout_report_json
);
xfa_document_report!(
    wellfriendpdf_document_pubsec_report_json,
    pubsec_report_json
);
xfa_document_report!(
    wellfriendpdf_document_aes_gcm_report_json,
    aes_gcm_report_json
);
xfa_document_report!(
    wellfriendpdf_document_pdf_mac_report_json,
    pdf_mac_report_json
);
xfa_document_report!(
    wellfriendpdf_document_pdf_mac_verify_json,
    pdf_mac_verify_json
);

/// Writes an AESV4 encrypted full-rewrite PDF with a standalone PDF-MAC token.
///
/// # Safety
/// Returns an owned buffer and owned JSON report. The output buffer must be
/// freed with `wellfriendpdf_buffer_free`; the report string must be freed with
/// `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pdf_mac_create_pdf(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::pdf_mac_create_json(bytes, None)
        })
    }
}

/// crypto writer crypto tamper policy report JSON.
///
/// # Safety
/// `out_json` and `error_out` must follow report ownership rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_crypto_tamper_test_json(
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = wellfriendpdf(sdk::crypto_tamper_test_json())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Analyze writer history raster-to-vector candidates for one page.
///
/// # Safety
/// `options_json` may be NULL or a valid NUL-terminated UTF-8
/// RasterVectorizationOptions JSON object.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_writer_history_raster_vector_report_json(
    document: *const WellfriendDocument,
    page: usize,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::writer_history_raster_vector_report_json(bytes, page, options.as_deref(), None)
        })
    }
}

/// Report writer history persistent history store behavior.
///
/// # Safety
/// `out_json` and `error_out` must follow report ownership rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_writer_history_history_report_json(
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = wellfriendpdf(sdk::writer_history_history_report_json())?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Save a full-rewrite PDF with xref-stream and object-stream packing.
///
/// # Safety
/// Returns an owned buffer and owned report string.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_writer_history_pack_object_streams_pdf(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::writer_history_pack_object_streams_json(bytes, None)
        })
    }
}

/// Save a compression and Office optimized full-rewrite PDF.
///
/// # Safety
/// `options_json` may be NULL or a valid NUL-terminated UTF-8
/// CompressionOfficeOptimizeOptions JSON object.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_compression_office_optimize_pdf(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options
                .clone()
                .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::compression_office_optimize_pdf_json(bytes, options.as_deref(), None)
        })
    }
}

/// Inspect DOCX/PPTX/XLSX bytes under compression and Office package security policy.
///
/// # Safety
/// `data` must point to `len` readable bytes and `format` must be a valid
/// NUL-terminated UTF-8 string (`docx`, `pptx`, or `xlsx`).
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_compression_office_office_inspect_json(
    data: *const u8,
    len: usize,
    format: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let format = unsafe { required_c_string(format, "format") };
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let format = format.clone()?;
        let json = wellfriendpdf(sdk::compression_office_office_inspect_json(bytes, &format))?;
        unsafe {
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Convert DOCX/PPTX/XLSX bytes to PDF through the compression and Office report path.
///
/// # Safety
/// `data` must point to `len` readable bytes and `format` must be a valid
/// NUL-terminated UTF-8 string (`docx`, `pptx`, or `xlsx`).
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_compression_office_office_to_pdf(
    data: *const u8,
    len: usize,
    format: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let format = unsafe { required_c_string(format, "format") };
    ffi_status(error_out, || {
        if out_buffer.is_null() {
            return Err("out_buffer pointer is null".into());
        }
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let bytes = unsafe { read_input_bytes(data, len, "data") }?;
        let format = format.clone()?;
        let (out, json) =
            wellfriendpdf(sdk::compression_office_office_to_pdf_json(bytes, &format))?;
        unsafe {
            *out_buffer = into_buffer(out);
            *out_json = into_c_string(json);
        }
        Ok(())
    })
}

/// Inspect a advanced editing closeout page-local multi-run range model.
///
/// # Safety
/// `document` must be live and output pointers writable; the returned string
/// is owned by the caller and released with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_closeout_text_range_analyze_json(
    document: *const WellfriendDocument,
    page: usize,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::advanced_editing_closeout_text_range_analyze_json(bytes, page, None)
        })
    }
}

/// Apply a advanced editing closeout multi-run request represented by versioned JSON.
///
/// # Safety
/// `request_json` must be a NUL-terminated UTF-8 string; output pointers use
/// the standard owned-buffer and owned-string free functions.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_closeout_text_range_edit_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::advanced_editing_closeout_text_range_edit_json(
                bytes,
                &request.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// List advanced editing vector objects as an owned JSON string.
///
/// # Safety
/// `document` must be a live handle; output pointers must be writable and the
/// returned string must be released with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_vector_list_json(
    document: *const WellfriendDocument,
    page: usize,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::advanced_editing_vector_list_json(bytes, page, None)
        })
    }
}

/// Apply a advanced editing text edit and return owned PDF bytes plus owned JSON.
///
/// # Safety
/// Input strings must be valid NUL-terminated UTF-8 (`options_json` may be
/// NULL). Output pointers must be writable and released with
/// `wellfriendpdf_buffer_free` and `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_text_edit_json(
    document: *const WellfriendDocument,
    page: usize,
    old_text: *const c_char,
    new_text: *const c_char,
    mode: *const c_char,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let old_text = unsafe { required_c_string(old_text, "old_text") };
    let new_text = unsafe { required_c_string(new_text, "new_text") };
    let mode = unsafe { required_c_string(mode, "mode") };
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::advanced_editing_text_edit_json(
                bytes,
                page,
                &old_text.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &new_text.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &mode.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                options
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                None,
            )
        })
    }
}

/// Apply a advanced editing vector edit and return owned PDF bytes plus owned JSON.
///
/// # Safety
/// `stable_id` and `operation_json` must be valid NUL-terminated UTF-8;
/// `options_json` may be NULL. Standard output ownership rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_vector_edit_json(
    document: *const WellfriendDocument,
    page: usize,
    stable_id: *const c_char,
    operation_json: *const c_char,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let stable_id = unsafe { required_c_string(stable_id, "stable_id") };
    let operation = unsafe { required_c_string(operation_json, "operation_json") };
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::advanced_editing_vector_edit_json(
                bytes,
                page,
                &stable_id.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &operation.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                options
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                None,
            )
        })
    }
}

/// Fit an Ink annotation and return owned PDF bytes plus owned JSON.
///
/// # Safety
/// `options_json` may be NULL; document and output pointers must follow the
/// standard live-handle and explicit-free ownership rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_editing_ink_fit_json(
    document: *const WellfriendDocument,
    page: usize,
    annotation_index: usize,
    options_json: *const c_char,
    signature_policy_override: c_int,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::advanced_editing_ink_fit_json(
                bytes,
                page,
                annotation_index,
                options
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                signature_policy_override != 0,
                None,
            )
        })
    }
}

/// Return source editing's canonical provenance/operator-editing architecture report.
///
/// # Safety
/// `document` must be live and output pointers writable; release the returned
/// JSON with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::source_editing_report_json(bytes, None)
        })
    }
}

/// Resolve source editing parser-backed source provenance for a text selection.
///
/// # Safety
/// `source_text` and `replacement_text` are NUL-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_provenance_json(
    document: *const WellfriendDocument,
    page: usize,
    source_text: *const c_char,
    replacement_text: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let source = unsafe { required_c_string(source_text, "source_text") };
    let replacement = unsafe { required_c_string(replacement_text, "replacement_text") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::source_editing_provenance_json(
                bytes,
                page,
                &source
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &replacement
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Plan a source editing source-level text mutation.  Refusals are returned as JSON
/// without producing output bytes.
///
/// # Safety
/// `request_json` is a NUL-terminated UTF-8 request.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_edit_eligibility_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::source_editing_edit_eligibility_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Apply a source editing operator-preserving text edit and return owned PDF bytes
/// plus a stable operation report.
///
/// # Safety
/// Standard document and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_operator_text_edit_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::source_editing_operator_text_edit_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// List source editing canonical vector/path source provenance.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_path_provenance_json(
    document: *const WellfriendDocument,
    page: usize,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::source_editing_path_provenance_json(bytes, page, None)
        })
    }
}

/// Apply a source editing operator-preserving vector/path/graphics mutation.
///
/// # Safety
/// Input strings are NUL-terminated UTF-8 and returned outputs are owned.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_source_editing_path_edit_json(
    document: *const WellfriendDocument,
    page: usize,
    stable_id: *const c_char,
    operation_json: *const c_char,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let stable_id = unsafe { required_c_string(stable_id, "stable_id") };
    let operation = unsafe { required_c_string(operation_json, "operation_json") };
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::source_editing_path_edit_json(
                bytes,
                page,
                &stable_id
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &operation
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                options
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                None,
            )
        })
    }
}

/// Return editing transactions's scene/transaction/font architecture report.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::editing_transactions_report_json(bytes, None)
        })
    }
}

/// Build a editing transactions editable scene graph. `pages_json` may be NULL or a JSON
/// array of one-based page numbers.
///
/// # Safety
/// Strings are NUL-terminated UTF-8; returned JSON is owned.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_scene_report_json(
    document: *const WellfriendDocument,
    pages_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let pages = unsafe { optional_c_string(pages_json) };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::editing_transactions_scene_report_json(
                bytes,
                pages
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                None,
            )
        })
    }
}

/// Resolve a editing transactions scene selection/hit-test request.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_scene_select_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::editing_transactions_scene_select_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Plan an atomic editing transactions scene text transaction.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_transaction_plan_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::editing_transactions_transaction_plan_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Apply an atomic editing transactions scene text transaction and return owned PDF bytes
/// plus an owned JSON report.
///
/// # Safety
/// Standard document and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_transaction_apply_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::editing_transactions_transaction_apply_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Report text identity/grapheme/bidi/shaping mappings.
///
/// # Safety
/// Strings are NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_text_map_json(
    document: *const WellfriendDocument,
    text: *const c_char,
    direction: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let text = unsafe { required_c_string(text, "text") };
    let direction = unsafe { optional_c_string(direction) };
    unsafe {
        report_json_impl(document, out_json, error_out, |_bytes| {
            sdk::editing_transactions_text_map_json(
                &text
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                direction
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
            )
        })
    }
}

/// Preview editing transactions shaping.
///
/// # Safety
/// Strings are NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_shape_text_json(
    document: *const WellfriendDocument,
    text: *const c_char,
    direction: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let text = unsafe { required_c_string(text, "text") };
    let direction = unsafe { optional_c_string(direction) };
    unsafe {
        report_json_impl(document, out_json, error_out, |_bytes| {
            sdk::editing_transactions_shape_text_json(
                &text
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                direction
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
            )
        })
    }
}

/// Plan editing transactions deterministic subset reconstruction.
///
/// # Safety
/// Strings are NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_font_subset_plan_json(
    document: *const WellfriendDocument,
    text: *const c_char,
    direction: *const c_char,
    policy: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let text = unsafe { required_c_string(text, "text") };
    let direction = unsafe { optional_c_string(direction) };
    let policy = unsafe { optional_c_string(policy) };
    unsafe {
        report_json_impl(document, out_json, error_out, |_bytes| {
            sdk::editing_transactions_font_subset_plan_json(
                &text
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                direction
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
                policy
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
            )
        })
    }
}

/// Report editing transactions font substitution policy/scoring.
///
/// # Safety
/// Strings are NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_editing_transactions_font_substitution_report_json(
    document: *const WellfriendDocument,
    requested_family: *const c_char,
    text: *const c_char,
    policy: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let family = unsafe { required_c_string(requested_family, "requested_family") };
    let text = unsafe { required_c_string(text, "text") };
    let policy = unsafe { optional_c_string(policy) };
    unsafe {
        report_json_impl(document, out_json, error_out, |_bytes| {
            sdk::editing_transactions_font_substitution_report_json(
                &family
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &text
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                policy
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?
                    .as_deref(),
            )
        })
    }
}

/// Return text reflow's geometric/semantic reflow architecture report.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_report_json(bytes, None)
        })
    }
}

/// Analyze a text reflow geometric reflow region.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_layout_analyze_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_layout_analyze_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Report text reflow semantic layout reconstruction.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_semantic_layout_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_semantic_layout_json(bytes, None)
        })
    }
}

/// Report text reflow reading order.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reading_order_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_reading_order_report_json(bytes, None)
        })
    }
}

/// Report text reflow flow graph.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_flow_graph_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_flow_graph_report_json(bytes, None)
        })
    }
}

/// Preview a text reflow reflow request without mutating bytes.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reflow_preview_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_reflow_preview_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Query text reflow's ordered overflow escalation without mutating bytes.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_overflow_report_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_overflow_report_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Query text reflow hard/soft constraint evidence without mutating bytes.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_constraints_report_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_constraints_report_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Query text reflow confidence and review enforcement without mutating bytes.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_confidence_report_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_confidence_report_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Validate explicitly supplied text reflow output against this immutable source
/// document. The input byte slice is borrowed only for this call; reports use
/// the standard owned-string free function.
///
/// # Safety
/// `output_pdf` must point to `output_pdf_len` readable bytes unless the
/// length is zero, and `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_validate_reflow_output_json(
    document: *const WellfriendDocument,
    output_pdf: *const u8,
    output_pdf_len: usize,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let output = unsafe { read_input_bytes(output_pdf, output_pdf_len, "output_pdf") };
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let output = output.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let request = request.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::text_reflow_validate_reflow_output_json(bytes, output, &request, None)
        })
    }
}

/// Apply text reflow GeometricBlock reflow and return owned PDF bytes plus report.
///
/// # Safety
/// Standard document and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reflow_region_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::text_reflow_reflow_region_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Apply text reflow SemanticDocument reflow and return owned PDF bytes plus report.
///
/// # Safety
/// Standard document and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reflow_document_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::text_reflow_reflow_document_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Execute text reflow's canonical inverse operation and return restored owned
/// PDF bytes plus a typed replay/undo report.
///
/// # Safety
/// `output_pdf` must point to `output_pdf_len` readable bytes unless the
/// length is zero, and `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_undo_reflow_json(
    document: *const WellfriendDocument,
    output_pdf: *const u8,
    output_pdf_len: usize,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let output = unsafe { read_input_bytes(output_pdf, output_pdf_len, "output_pdf") };
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let output = output.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let request = request.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::text_reflow_undo_reflow_json(bytes, output, &request, None)
        })
    }
}

/// Return the document subsystems table, math, OCR, annotation, form, and XFA feature
/// report for an immutable document handle.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_subsystems_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_subsystems_report_json(bytes, None)
        })
    }
}

/// Analyze source-linked document subsystems subsystems.
///
/// # Safety
/// Standard immutable document and owned string output rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_subsystems_analyze_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_subsystems_analyze_json(bytes, None)
        })
    }
}

/// Plan a typed document subsystems operation without changing the document handle.
///
/// # Safety
/// `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_subsystems_plan_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_subsystems_plan_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Apply a typed document subsystems operation, returning owned output bytes and the
/// canonical transaction/appearance report.
///
/// # Safety
/// Standard document, request, and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_subsystems_apply_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::document_subsystems_apply_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Replay and undo a typed document subsystems operation against an immutable source
/// handle, returning restored owned bytes and a typed inverse report.
///
/// # Safety
/// `output_pdf` must point to `output_pdf_len` readable bytes unless empty and
/// `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_subsystems_undo_json(
    document: *const WellfriendDocument,
    output_pdf: *const u8,
    output_pdf_len: usize,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let output = unsafe { read_input_bytes(output_pdf, output_pdf_len, "output_pdf") };
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let output = output.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let request = request.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::document_subsystems_undo_json(bytes, output, &request, None)
        })
    }
}

/// Return the document security accessibility/redaction/sanitization feature report.
///
/// # Safety
/// `document` must be live and output pointers writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_report_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_security_report_json(bytes, None)
        })
    }
}

/// Analyze document security structure, repair, redaction, and sanitizer state.
///
/// # Safety
/// Standard immutable document and owned string output rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_analyze_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_security_analyze_json(bytes, None)
        })
    }
}

/// Plan a typed document security operation without changing the document handle.
///
/// # Safety
/// `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_plan_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_security_plan_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Apply a typed document security operation, returning owned output bytes and report.
///
/// # Safety
/// Standard document, request, and owned-output pointer rules apply.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_apply_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::document_security_apply_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Replay and undo a typed document security operation from an immutable source handle.
///
/// # Safety
/// `output_pdf` must point to `output_pdf_len` readable bytes unless empty and
/// `request_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_undo_json(
    document: *const WellfriendDocument,
    output_pdf: *const u8,
    output_pdf_len: usize,
    request_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let output = unsafe { read_input_bytes(output_pdf, output_pdf_len, "output_pdf") };
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let output = output.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let request = request.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::document_security_undo_json(bytes, output, &request, None)
        })
    }
}

/// Run document security residual-data verification with a JSON term array.
///
/// # Safety
/// `terms_json` must be NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_document_security_verify_residual_json(
    document: *const WellfriendDocument,
    terms_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let terms = unsafe { required_c_string(terms_json, "terms_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::document_security_verify_residual_json(
                bytes,
                &terms
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Store or preview a text reflow reviewed structure correction.
///
/// # Safety
/// `correction_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reflow_approve_structure_json(
    document: *const WellfriendDocument,
    correction_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let correction = unsafe { required_c_string(correction_json, "correction_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_reflow_approve_structure_json(
                bytes,
                &correction
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

/// Report text reflow reflow operation/undo/redo evidence.
///
/// # Safety
/// `request_json` is NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_text_reflow_reflow_operation_report_json(
    document: *const WellfriendDocument,
    request_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let request = unsafe { required_c_string(request_json, "request_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::text_reflow_reflow_operation_report_json(
                bytes,
                &request
                    .clone()
                    .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}
xfa_document_report!(
    wellfriendpdf_document_associated_files_report_json,
    associated_files_report_json
);
xfa_document_report!(
    wellfriendpdf_document_mask_redaction_report_json,
    mask_redaction_report_json
);

/// Analyze secure mutation signature-aware edit policy.
///
/// # Safety
/// `document` must be a live handle, `operation` a NUL-terminated UTF-8
/// string, and output pointers writable under the standard C ABI ownership
/// rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_edit_policy_report_json(
    document: *const WellfriendDocument,
    operation: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let operation = unsafe { required_c_string(operation, "operation") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let operation =
                operation.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::edit_policy_report_json(bytes, &operation, None)
        })
    }
}

/// annotation/media redaction annotation appearance report using optional JSON options.
///
/// # Safety
/// `options_json` may be NULL or a NUL-terminated UTF-8 JSON string. Output
/// ownership follows `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_annotation_appearance_report_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::annotation_appearance_report_json(bytes, options.as_deref(), None)
        })
    }
}

/// annotation/media redaction non-axis redaction plan using a required JSON options document.
///
/// # Safety
/// `options_json` must be a NUL-terminated UTF-8 string. Output ownership
/// follows `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_nonaxis_redaction_plan_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { required_c_string(options_json, "options_json") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::nonaxis_redaction_plan_json(bytes, &options, None)
        })
    }
}

/// Page-operations report JSON (boxes, labels, destinations, preservation risk).
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_pages_report_json(
    document: *const WellfriendDocument,
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
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_interactive_report_json(
    document: *const WellfriendDocument,
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
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_chunks_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::chunk_report_json(b, None)
        })
    }
}

/// Semantic Closeout provenance-aware RAG chunk-set JSON.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_advanced_chunks_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::advanced_chunk_report_json(b, &[], None)
        })
    }
}

/// Full Semantic Closeout semantic binding bundle as versioned owned JSON.
///
/// # Safety
/// See `wellfriendpdf_document_security_report_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_semantic_bundle_json(
    document: *const WellfriendDocument,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            sdk::semantic_binding_report_json(b, &[], None)
        })
    }
}

/// Semantic Closeout semantic and dictionary-token search as versioned owned JSON.
///
/// # Safety
/// `query` must be a non-null NUL-terminated UTF-8 string. See
/// `wellfriendpdf_document_security_report_json` for output ownership.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_semantic_search_json(
    document: *const WellfriendDocument,
    query: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let query = unsafe { required_c_string(query, "query") };
    unsafe {
        report_json_impl(document, out_json, error_out, |b| {
            let query = query.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::semantic_search_report_json(b, &[], &query, None)
        })
    }
}

/// Produce XFA Runtime XFA preview PDF bytes and a versioned report.
///
/// # Safety
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_xfa_render_json(
    document: *const WellfriendDocument,
    script_policy: *const c_char,
    execute_events: c_int,
    dpi: u32,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let policy = unsafe { optional_c_string(script_policy) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let policy = policy.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::xfa_render_preview_json(bytes, policy.as_deref(), execute_events != 0, dpi, None)
        })
    }
}

/// Flatten supported static XFA under an explicit mode.
///
/// # Safety
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_xfa_flatten_json(
    document: *const WellfriendDocument,
    mode: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let mode = unsafe { optional_c_string(mode) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let mode = mode.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::xfa_flatten_json(bytes, mode.as_deref(), None)
        })
    }
}

/// Sanitize XFA packets/scripts/events/connections under an explicit mode.
///
/// # Safety
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_xfa_sanitize_json(
    document: *const WellfriendDocument,
    mode: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let mode = unsafe { optional_c_string(mode) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let mode = mode.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::xfa_sanitize_json(bytes, mode.as_deref(), None)
        })
    }
}

/// Export annotation/media redaction annotation XFDF bytes and a versioned JSON report.
///
/// # Safety
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_annotation_xfdf_export_json(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::annotation_xfdf_export_json(bytes, None)
        })
    }
}

/// Import annotation/media redaction annotation XFDF bytes.
///
/// # Safety
/// `xfdf` must point to `xfdf_len` readable bytes. `options_json` may be NULL.
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_annotation_xfdf_import_json(
    document: *const WellfriendDocument,
    xfdf: *const u8,
    xfdf_len: usize,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    let xfdf = if xfdf_len == 0 {
        Ok(Vec::new())
    } else if xfdf.is_null() {
        Err("xfdf pointer is null".to_string())
    } else {
        Ok(unsafe { slice::from_raw_parts(xfdf, xfdf_len) }.to_vec())
    };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let xfdf = xfdf.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::annotation_xfdf_import_json(bytes, &xfdf, options.as_deref(), None)
        })
    }
}

/// Generate annotation/media redaction annotation appearance streams.
///
/// # Safety
/// `options_json` may be NULL. Output ownership matches
/// `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_annotation_appearance_generate_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::annotation_appearance_generate_json(bytes, options.as_deref(), None)
        })
    }
}

/// Apply a annotation/media redaction rich-media policy.
///
/// # Safety
/// `mode` and `custom_json` may be NULL. Output ownership matches
/// `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_rich_media_sanitize_json(
    document: *const WellfriendDocument,
    mode: *const c_char,
    custom_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let mode = unsafe { optional_c_string(mode) };
    let custom = unsafe { optional_c_string(custom_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let mode = mode.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let custom = custom.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::rich_media_sanitize_json(bytes, mode.as_deref(), custom.as_deref(), None)
        })
    }
}

/// Flatten static annotation/media redaction media posters without media decode or execution.
///
/// # Safety
/// Output ownership matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_rich_media_flatten_poster_json(
    document: *const WellfriendDocument,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::rich_media_flatten_poster_json(bytes, None)
        })
    }
}

/// Apply annotation/media redaction non-axis polygon image redaction.
///
/// # Safety
/// `options_json` must be a NUL-terminated UTF-8 string. Output ownership
/// matches `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_nonaxis_redaction_apply_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { required_c_string(options_json, "options_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::nonaxis_redaction_apply_json(bytes, &options, None)
        })
    }
}

macro_rules! secure_mutation_redaction_output {
    ($name:ident, $sdk_fn:ident) => {
        /// Apply a secure mutation image redaction operation.
        ///
        /// # Safety
        /// The document handle must be live, options must be NUL-terminated
        /// UTF-8 JSON, and output pointers must be writable. The caller frees
        /// the returned buffer/string with the standard Wellfriend free functions.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            document: *const WellfriendDocument,
            options_json: *const c_char,
            out_buffer: *mut WellfriendBuffer,
            out_json: *mut *mut c_char,
            error_out: *mut *mut c_char,
        ) -> c_int {
            let options = unsafe { required_c_string(options_json, "options_json") };
            unsafe {
                report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
                    let options =
                        options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
                    sdk::$sdk_fn(bytes, &options, None)
                })
            }
        }
    };
}

secure_mutation_redaction_output!(
    wellfriendpdf_document_redact_image_mask_json,
    redact_image_mask_json
);

macro_rules! form_action_policy_policy_output {
    ($name:ident, $sdk_fn:ident) => {
        /// Apply a form action policy form-action policy and return owned PDF/report buffers.
        ///
        /// # Safety
        /// The document handle must be live. `options_json` may be NULL or a
        /// NUL-terminated UTF-8 JSON object. Output pointers must be writable
        /// and are freed with the standard Wellfriend buffer/string free functions.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            document: *const WellfriendDocument,
            options_json: *const c_char,
            out_buffer: *mut WellfriendBuffer,
            out_json: *mut *mut c_char,
            error_out: *mut *mut c_char,
        ) -> c_int {
            let options = unsafe { optional_c_string(options_json) };
            unsafe {
                report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
                    let options =
                        options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
                    sdk::$sdk_fn(bytes, options.as_deref(), None)
                })
            }
        }
    };
}

form_action_policy_policy_output!(
    wellfriendpdf_document_form_js_sanitize_json,
    form_js_sanitize_json
);
form_action_policy_policy_output!(
    wellfriendpdf_document_form_js_flatten_values_json,
    form_js_flatten_values_json
);

/// Audit one DOCX layout mode and return a versioned report.
///
/// # Safety
/// `layout` must be a NUL-terminated UTF-8 string and other pointers follow
/// the standard report ownership contract.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_word_pagination_audit_json(
    document: *const WellfriendDocument,
    layout: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let layout = unsafe { required_c_string(layout, "layout") };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            let layout = layout.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::word_pagination_audit_json(bytes, &layout, None)
        })
    }
}
secure_mutation_redaction_output!(
    wellfriendpdf_document_redact_inline_image_json,
    redact_inline_image_json
);

/// Add an associated-file payload and return owned PDF/report buffers.
///
/// # Safety
/// `payload` must address `payload_len` readable bytes (or be NULL for zero
/// length), `options_json` must be NUL-terminated UTF-8, the document must be
/// live, and all output pointers must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_add_json(
    document: *const WellfriendDocument,
    payload: *const u8,
    payload_len: usize,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { required_c_string(options_json, "options_json") };
    let payload = if payload_len == 0 {
        Ok(Vec::new())
    } else if payload.is_null() {
        Err("payload pointer is null".to_string())
    } else {
        Ok(unsafe { slice::from_raw_parts(payload, payload_len) }.to_vec())
    };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let payload = payload.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::associated_files_add_json(bytes, &payload, &options, None)
        })
    }
}

#[no_mangle]
/// Update one owner-specific associated-file association.
///
/// # Safety
/// The document and output pointers must be valid; `payload` must address
/// `payload_len` bytes and `options_json` must be NUL-terminated UTF-8.
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_update_owner_json(
    document: *const WellfriendDocument,
    payload: *const u8,
    payload_len: usize,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { required_c_string(options_json, "options_json") };
    let payload = if payload_len == 0 {
        Ok(Vec::new())
    } else if payload.is_null() {
        Err("payload pointer is null".to_string())
    } else {
        Ok(unsafe { slice::from_raw_parts(payload, payload_len) }.to_vec())
    };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::associated_files_update_owner_json(
                bytes,
                &payload.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

#[no_mangle]
/// Remove one owner-specific associated-file association.
///
/// # Safety
/// The document and output pointers must be valid and `options_json` must be a
/// NUL-terminated UTF-8 request.
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_remove_owner_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { required_c_string(options_json, "options_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::associated_files_remove_owner_json(
                bytes,
                &options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

#[no_mangle]
/// Incrementally update a form value under structural signature policy.
///
/// # Safety
/// The document and output pointers must be valid; both string inputs must be
/// NUL-terminated UTF-8.
pub unsafe extern "C" fn wellfriendpdf_document_incremental_form_edit_json(
    document: *const WellfriendDocument,
    field_name: *const c_char,
    value: *const c_char,
    signature_policy_override: bool,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let field_name = unsafe { required_c_string(field_name, "field_name") };
    let value = unsafe { required_c_string(value, "value") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::incremental_form_edit_json(
                bytes,
                &field_name.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &value.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                signature_policy_override,
                None,
            )
        })
    }
}

#[no_mangle]
/// Plan a Pades LTV signature-preserving form value update.
///
/// # Safety
/// The document pointer must be valid; string inputs must be NUL-terminated
/// UTF-8. The returned string must be freed with `wellfriendpdf_string_free`.
pub unsafe extern "C" fn wellfriendpdf_document_signature_preserving_form_plan_json(
    document: *const WellfriendDocument,
    field_name: *const c_char,
    value: *const c_char,
    options_json: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let field_name = unsafe { required_c_string(field_name, "field_name") };
    let value = unsafe { required_c_string(value, "value") };
    let options_json = if options_json.is_null() {
        Ok("{}".to_string())
    } else {
        unsafe { required_c_string(options_json, "options_json") }
    };
    unsafe {
        report_json_impl(document, out_json, error_out, |bytes| {
            sdk::signature_preserving_form_plan_json(
                bytes,
                &field_name.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &value.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &options_json.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                None,
            )
        })
    }
}

#[no_mangle]
/// Apply a Pades LTV signature-preserving form value update.
///
/// # Safety
/// The document and output pointers must be valid; string inputs must be
/// NUL-terminated UTF-8. The caller owns both output buffer and report string.
pub unsafe extern "C" fn wellfriendpdf_document_signature_preserving_form_edit_json(
    document: *const WellfriendDocument,
    field_name: *const c_char,
    value: *const c_char,
    options_json: *const c_char,
    explicit_invalidation_override: bool,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let field_name = unsafe { required_c_string(field_name, "field_name") };
    let value = unsafe { required_c_string(value, "value") };
    let options_json = if options_json.is_null() {
        Ok("{}".to_string())
    } else {
        unsafe { required_c_string(options_json, "options_json") }
    };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            sdk::signature_preserving_form_edit_json(
                bytes,
                &field_name.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &value.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                &options_json.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                explicit_invalidation_override,
                None,
            )
        })
    }
}

macro_rules! secure_mutation_closeout_policy_output {
    ($name:ident, $sdk_fn:ident) => {
        /// Apply a secure mutation closeout incremental JSON mutation under signature policy.
        ///
        /// # Safety
        /// The document and output pointers must be valid and `options_json`
        /// must be a NUL-terminated UTF-8 request.
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            document: *const WellfriendDocument,
            options_json: *const c_char,
            signature_policy_override: bool,
            out_buffer: *mut WellfriendBuffer,
            out_json: *mut *mut c_char,
            error_out: *mut *mut c_char,
        ) -> c_int {
            let options = unsafe { required_c_string(options_json, "options_json") };
            unsafe {
                report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
                    sdk::$sdk_fn(
                        bytes,
                        &options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?,
                        signature_policy_override,
                        None,
                    )
                })
            }
        }
    };
}

secure_mutation_closeout_policy_output!(
    wellfriendpdf_document_incremental_annotation_edit_json,
    incremental_annotation_edit_json
);
secure_mutation_closeout_policy_output!(
    wellfriendpdf_document_incremental_page_property_edit_json,
    incremental_page_property_edit_json
);

/// Extract an associated file into an owned output buffer.
///
/// # Safety
/// `stable_id` must be NUL-terminated UTF-8, the document must be live, and
/// output pointers must be writable and later freed by the caller.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_extract_json(
    document: *const WellfriendDocument,
    stable_id: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let stable_id = unsafe { required_c_string(stable_id, "stable_id") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let stable_id =
                stable_id.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::associated_files_extract_json(bytes, &stable_id, None)
        })
    }
}

/// Remove associated files selected by a JSON string array of stable ids.
///
/// # Safety
/// `stable_ids_json` must be NUL-terminated UTF-8 JSON, the document must be
/// live, and output pointers must follow the standard owned-buffer rules.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_remove_json(
    document: *const WellfriendDocument,
    stable_ids_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let stable_ids = unsafe { required_c_string(stable_ids_json, "stable_ids_json") };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let stable_ids =
                stable_ids.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            let stable_ids: Vec<String> = serde_json::from_str(&stable_ids).map_err(|error| {
                wellfriendpdf_engine::WellfriendError::invalid_input(error.to_string())
            })?;
            sdk::associated_files_remove_json(bytes, &stable_ids, None)
        })
    }
}

/// Apply the secure mutation associated-file sanitizer.
///
/// # Safety
/// The document handle must be live; optional JSON must be NULL or
/// NUL-terminated UTF-8; output pointers must be writable and released with
/// the standard Wellfriend free functions.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_associated_files_sanitize_json(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let options = unsafe { optional_c_string(options_json) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |bytes| {
            let options = options.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::associated_files_sanitize_json(bytes, options.as_deref(), None)
        })
    }
}

/// Sanitize the document. `policy` is `strict`|`balanced`|`preserve-visual`
/// (NULL → `balanced`). Writes the sanitized PDF to `out_buffer` and the JSON
/// report to `out_json`.
///
/// # Safety
/// `document` valid; `out_buffer`/`out_json`/`error_out` writable. Free the
/// buffer with `wellfriendpdf_buffer_free`, the string with `wellfriendpdf_string_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_sanitize_json(
    document: *const WellfriendDocument,
    policy: *const c_char,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let policy = unsafe { optional_c_string(policy) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |b| {
            let policy = policy.map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::sanitize_json(b, policy.as_deref(), None)
        })
    }
}

/// Canonicalize the document deterministically. `date_epoch` fixes the source
/// date epoch (pass a negative value to leave it unset). Writes the canonical
/// PDF to `out_buffer` and the audit JSON to `out_json`.
///
/// # Safety
/// See `wellfriendpdf_document_sanitize_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_canonicalize_json(
    document: *const WellfriendDocument,
    date_epoch: i64,
    has_date_epoch: c_int,
    out_buffer: *mut WellfriendBuffer,
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
/// See `wellfriendpdf_document_sanitize_json` for the outputs.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_document_redact_terms_json(
    document: *const WellfriendDocument,
    terms: *const *const c_char,
    terms_len: usize,
    strict: c_int,
    out_buffer: *mut WellfriendBuffer,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    let collected = unsafe { read_c_string_array(terms, terms_len) };
    unsafe {
        report_output_impl(document, out_buffer, out_json, error_out, |b| {
            let terms = collected
                .clone()
                .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)?;
            sdk::redact_terms_json(b, &terms, strict != 0, None)
        })
    }
}

/// SDK / ABI version and capability report as JSON (no document needed): engine
/// version, envelope version, compiled capabilities. Free with
/// `wellfriendpdf_string_free`.
///
/// # Safety
/// `out_json`/`error_out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_feature_report_json(
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> c_int {
    ffi_status(error_out, || {
        if out_json.is_null() {
            return Err("out_json pointer is null".into());
        }
        let json = wellfriendpdf(sdk::feature_report_json())?;
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
pub unsafe extern "C" fn wellfriendpdf_codec_isolation_report_json(
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
        let json = wellfriendpdf(sdk::codec_isolation_report_json(
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

/// The wellfriendpdf-engine semantic version as a NUL-terminated string. The returned
/// pointer is owned by the caller and must be freed with `wellfriendpdf_string_free`.
/// Safe to call (takes no pointers).
#[no_mangle]
pub extern "C" fn wellfriendpdf_version() -> *mut c_char {
    into_c_string(wellfriendpdf_engine::ENGINE_VERSION.to_string())
}

/// The C-ABI report envelope version (bump signals an envelope-shape change).
/// Safe to call (takes no pointers).
#[no_mangle]
pub extern "C" fn wellfriendpdf_abi_version() -> u32 {
    wellfriendpdf_engine::REPORT_ENVELOPE_VERSION
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

fn checked_doc<'a>(document: *const WellfriendDocument) -> Result<&'a WellfriendDocument, String> {
    if document.is_null() {
        Err("document pointer is null".to_string())
    } else {
        Ok(unsafe { &*document })
    }
}

fn checked_signature_validation_options<'a>(
    options: *const WellfriendSignatureValidationOptions,
) -> Result<&'a WellfriendSignatureValidationOptions, String> {
    if options.is_null() {
        Err("signature validation options pointer is null".to_string())
    } else {
        Ok(unsafe { &*options })
    }
}

fn checked_signature_validation_options_mut<'a>(
    options: *mut WellfriendSignatureValidationOptions,
) -> Result<&'a mut WellfriendSignatureValidationOptions, String> {
    if options.is_null() {
        Err("signature validation options pointer is null".to_string())
    } else {
        Ok(unsafe { &mut *options })
    }
}

fn checked_signature_trust_store<'a>(
    store: *const WellfriendSignatureTrustStore,
) -> Result<&'a WellfriendSignatureTrustStore, String> {
    if store.is_null() {
        Err("signature trust store pointer is null".to_string())
    } else {
        Ok(unsafe { &*store })
    }
}

fn checked_signature_trust_store_mut<'a>(
    store: *mut WellfriendSignatureTrustStore,
) -> Result<&'a mut WellfriendSignatureTrustStore, String> {
    if store.is_null() {
        Err("signature trust store pointer is null".to_string())
    } else {
        Ok(unsafe { &mut *store })
    }
}

fn checked_signature_intermediate_store<'a>(
    store: *const WellfriendSignatureIntermediateStore,
) -> Result<&'a WellfriendSignatureIntermediateStore, String> {
    if store.is_null() {
        Err("signature intermediate store pointer is null".to_string())
    } else {
        Ok(unsafe { &*store })
    }
}

fn checked_signature_intermediate_store_mut<'a>(
    store: *mut WellfriendSignatureIntermediateStore,
) -> Result<&'a mut WellfriendSignatureIntermediateStore, String> {
    if store.is_null() {
        Err("signature intermediate store pointer is null".to_string())
    } else {
        Ok(unsafe { &mut *store })
    }
}

fn checked_signature_evidence_store<'a>(
    store: *const WellfriendSignatureEvidenceStore,
) -> Result<&'a WellfriendSignatureEvidenceStore, String> {
    if store.is_null() {
        Err("signature evidence store pointer is null".to_string())
    } else {
        Ok(unsafe { &*store })
    }
}

fn checked_signature_evidence_store_mut<'a>(
    store: *mut WellfriendSignatureEvidenceStore,
) -> Result<&'a mut WellfriendSignatureEvidenceStore, String> {
    if store.is_null() {
        Err("signature evidence store pointer is null".to_string())
    } else {
        Ok(unsafe { &mut *store })
    }
}

fn checked_signature_retrieval_policy<'a>(
    policy: *const WellfriendSignatureRetrievalPolicy,
) -> Result<&'a WellfriendSignatureRetrievalPolicy, String> {
    if policy.is_null() {
        Err("signature retrieval policy pointer is null".to_string())
    } else {
        Ok(unsafe { &*policy })
    }
}

fn checked_signature_retrieval_policy_mut<'a>(
    policy: *mut WellfriendSignatureRetrievalPolicy,
) -> Result<&'a mut WellfriendSignatureRetrievalPolicy, String> {
    if policy.is_null() {
        Err("signature retrieval policy pointer is null".to_string())
    } else {
        Ok(unsafe { &mut *policy })
    }
}

fn checked_signature_validation_cancellation<'a>(
    cancellation: *const WellfriendSignatureValidationCancellation,
) -> Result<&'a WellfriendSignatureValidationCancellation, String> {
    if cancellation.is_null() {
        Err("signature validation cancellation pointer is null".to_string())
    } else {
        Ok(unsafe { &*cancellation })
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
        Ok(Ok(())) => WELLFRIENDPDF_STATUS_OK,
        Ok(Err(err)) => {
            set_error(error_out, &err);
            WELLFRIENDPDF_STATUS_ERROR
        }
        Err(_) => {
            set_error(error_out, "panic inside wellfriendpdf C API");
            WELLFRIENDPDF_STATUS_PANIC
        }
    }
}

fn wellfriendpdf<T>(result: WellfriendResult<T>) -> Result<T, String> {
    result.map_err(|err| err.to_string())
}

/// Build [`ParseOptions`] carrying the document's registered OCR backend (if
/// any). With a backend, uses `OcrPolicy::Auto` (scanned pages recognized) and a
/// generous per-page timeout as an engine-side backstop; without one, returns
/// default options (OCR off).
fn parse_options_with_ocr(doc: &WellfriendDocument) -> ParseOptions {
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

fn into_buffer(bytes: Vec<u8>) -> WellfriendBuffer {
    if bytes.is_empty() {
        return WellfriendBuffer::empty();
    }
    let mut bytes = bytes.into_boxed_slice();
    let out = WellfriendBuffer {
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
    use std::ffi::CStr;
    use wellfriendpdf_engine::{crypto::secret_bytes, encrypt, EncryptAlgorithm, EncryptParams};

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
        b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>");
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
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        assert!(error.is_null());

        let mut count = 0usize;
        let status = unsafe { wellfriendpdf_document_page_count(doc, &mut count, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        assert_eq!(count, 1);

        let mut text = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_document_extract_text(doc, 1, &mut text, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let extracted = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        assert!(extracted.contains("Hello C API"));
        unsafe {
            wellfriendpdf_string_free(text);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_render_contract_round_trips_to_png() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        let mut contract = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_default_render_contract_json(
                doc,
                1,
                72,
                std::ptr::null(),
                &mut contract,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        assert!(!contract.is_null());
        let mut png = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_render_page_png_with_contract_json(
                doc, contract, &mut png, &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let bytes = unsafe { slice::from_raw_parts(png.data, png.len) };
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        unsafe {
            wellfriendpdf_buffer_free(png);
            wellfriendpdf_string_free(contract);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_open_with_password_accepts_null_empty_and_ignored_passwords() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();

        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                std::ptr::null(),
                0,
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { wellfriendpdf_document_free(doc) };

        let explicit_empty = [0u8; 1];
        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                explicit_empty.as_ptr(),
                0,
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { wellfriendpdf_document_free(doc) };

        let ignored = b"ignored-for-unencrypted";
        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
                pdf.as_ptr(),
                pdf.len(),
                ignored.as_ptr(),
                ignored.len(),
                &mut error,
            )
        };
        assert!(!doc.is_null());
        assert!(error.is_null());
        unsafe { wellfriendpdf_document_free(doc) };
    }

    #[test]
    fn capi_open_with_password_handles_encrypted_fixture_and_redacts_secret() {
        let password = b"open-sesame";
        let pdf = encrypted_sample_pdf(password);
        let mut error = std::ptr::null_mut();

        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
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
        let status = unsafe { wellfriendpdf_document_page_count(doc, &mut count, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        assert_eq!(count, 1);
        unsafe { wellfriendpdf_document_free(doc) };

        let wrong = b"do-not-echo-this-password";
        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
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
        unsafe { wellfriendpdf_error_free(error) };
    }

    #[test]
    fn capi_open_with_password_rejects_invalid_password_pointer_shape() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc = unsafe {
            wellfriendpdf_document_open_from_bytes_with_password(
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
        unsafe { wellfriendpdf_error_free(error) };
    }

    #[test]
    fn capi_parse_markdown_json_and_fields() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        // parse → markdown: the canonical parser output, containing the text.
        let mut md = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_document_parse_markdown(doc, &mut md, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let markdown = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(markdown.contains("Hello C API"), "markdown was: {markdown}");
        unsafe { wellfriendpdf_string_free(md) };

        // parse → canonical JSON: must carry the schema version and the text.
        let mut json = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_document_parse_json(doc, &mut json, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let parsed = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert!(parsed.contains("schema_version"), "json was: {parsed}");
        assert!(parsed.contains("Hello C API"));
        unsafe { wellfriendpdf_string_free(json) };

        // extract-fields → JSON: null doc_type means auto-detect; must succeed
        // and produce a well-formed payload (this doc has no fields, which is
        // fine — the call must not error and must include the schema version).
        let mut fields = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_extract_fields_json(
                doc,
                std::ptr::null(),
                &mut fields,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let fields_json = unsafe { CStr::from_ptr(fields) }
            .to_string_lossy()
            .into_owned();
        assert!(
            fields_json.contains("schema_version"),
            "fields: {fields_json}"
        );
        unsafe { wellfriendpdf_string_free(fields) };

        unsafe { wellfriendpdf_document_free(doc) };
    }

    #[test]
    fn capi_signature_options_json_returns_owned_report_and_rejects_bad_options() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        let options =
            CString::new(r#"{"policy_profile":"offline_strict","online":false}"#).unwrap();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_signatures_with_options_json(
                doc,
                options.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(text, "[]");
        unsafe { wellfriendpdf_string_free(json) };

        let mut outcome_json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_signature_validation_with_evidence_json(
                doc,
                options.as_ptr(),
                &mut outcome_json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let outcome = unsafe { CStr::from_ptr(outcome_json) }
            .to_string_lossy()
            .into_owned();
        assert!(outcome.contains("evidence_bundle"), "outcome: {outcome}");
        unsafe { wellfriendpdf_string_free(outcome_json) };

        let bad = CString::new(r#"{"trust_anchors_der_hex":["not hex"]}"#).unwrap();
        let mut bad_json = std::ptr::null_mut();
        let mut bad_error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_signatures_with_options_json(
                doc,
                bad.as_ptr(),
                &mut bad_json,
                &mut bad_error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(bad_json.is_null());
        assert!(!bad_error.is_null());
        let message = unsafe { CStr::from_ptr(bad_error) }.to_string_lossy();
        assert!(message.contains("not valid hex DER"), "message: {message}");
        unsafe {
            wellfriendpdf_error_free(bad_error);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_signature_validation_options_handle_has_owned_lifecycle_and_reports() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        assert!(error.is_null());

        for _ in 0..16 {
            let options = unsafe { wellfriendpdf_signature_validation_options_new(&mut error) };
            assert!(!options.is_null());
            assert!(error.is_null());

            let status = unsafe {
                wellfriendpdf_signature_validation_options_set_path_limits(
                    options, 0, 1, &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
            assert!(!error.is_null());
            unsafe { wellfriendpdf_error_free(error) };
            error = std::ptr::null_mut();

            let status = unsafe {
                wellfriendpdf_signature_validation_options_add_trust_anchor_der(
                    options,
                    std::ptr::null(),
                    1,
                    &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
            assert!(!error.is_null());
            unsafe { wellfriendpdf_error_free(error) };
            error = std::ptr::null_mut();

            let status = unsafe {
                wellfriendpdf_signature_validation_options_set_revocation_mode(
                    options, 1, &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());

            let status = unsafe {
                wellfriendpdf_signature_validation_options_set_revocation_mode(
                    options, 3, &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());

            let status = unsafe {
                wellfriendpdf_signature_validation_options_set_revocation_mode(
                    options, 4, &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());

            let algorithm_policy = CString::new(r#"{"allow_rsa_pkcs1v15":false}"#).unwrap();
            let status = unsafe {
                wellfriendpdf_signature_validation_options_set_algorithm_policy_json(
                    options,
                    algorithm_policy.as_ptr(),
                    &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());

            let denied = CString::new("00".repeat(32)).unwrap();
            let status = unsafe {
                wellfriendpdf_signature_validation_options_add_distrusted_certificate_sha256(
                    options,
                    denied.as_ptr(),
                    &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());

            let mut report = std::ptr::null_mut();
            let status = unsafe {
                wellfriendpdf_document_signatures_with_options_handle(
                    doc,
                    options,
                    &mut report,
                    &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert_eq!(unsafe { CStr::from_ptr(report) }.to_bytes(), b"[]");
            unsafe { wellfriendpdf_string_free(report) };

            let mut outcome = std::ptr::null_mut();
            let status = unsafe {
                wellfriendpdf_document_signature_validation_with_evidence_handle(
                    doc,
                    options,
                    &mut outcome,
                    &mut error,
                )
            };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(unsafe { CStr::from_ptr(outcome) }
                .to_string_lossy()
                .contains("evidence_bundle"));
            unsafe {
                wellfriendpdf_string_free(outcome);
                wellfriendpdf_signature_validation_options_free(options);
            }
        }
        unsafe { wellfriendpdf_document_free(doc) };
    }

    #[test]
    fn capi_signature_component_handles_are_owned_and_cancellation_is_observed() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        assert!(error.is_null());

        let trust = unsafe { wellfriendpdf_signature_trust_store_new(&mut error) };
        let intermediates = unsafe { wellfriendpdf_signature_intermediate_store_new(&mut error) };
        let evidence = unsafe { wellfriendpdf_signature_evidence_store_new(&mut error) };
        let retrieval = unsafe { wellfriendpdf_signature_retrieval_policy_new(&mut error) };
        let cancellation =
            unsafe { wellfriendpdf_signature_validation_cancellation_new(&mut error) };
        let options = unsafe { wellfriendpdf_signature_validation_options_new(&mut error) };
        assert!(!trust.is_null());
        assert!(!intermediates.is_null());
        assert!(!evidence.is_null());
        assert!(!retrieval.is_null());
        assert!(!cancellation.is_null());
        assert!(!options.is_null());
        assert!(error.is_null());

        let status = unsafe {
            wellfriendpdf_signature_trust_store_add_anchor_der(
                trust,
                b"not-a-certificate".as_ptr(),
                b"not-a-certificate".len(),
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(!error.is_null());
        unsafe { wellfriendpdf_error_free(error) };
        error = std::ptr::null_mut();

        let status = unsafe {
            wellfriendpdf_signature_evidence_store_add_ocsp_der(
                evidence,
                b"untrusted-ocsp".as_ptr(),
                b"untrusted-ocsp".len(),
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let status = unsafe {
            wellfriendpdf_signature_evidence_store_add_crl_der(
                evidence,
                b"untrusted-crl".as_ptr(),
                b"untrusted-crl".len(),
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let offline = CString::new(r#"{"enabled":false}"#).unwrap();
        let status = unsafe {
            wellfriendpdf_signature_retrieval_policy_set_json(
                retrieval,
                offline.as_ptr(),
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);

        for status in [
            unsafe {
                wellfriendpdf_signature_validation_options_apply_trust_store(
                    options, trust, &mut error,
                )
            },
            unsafe {
                wellfriendpdf_signature_validation_options_apply_intermediate_store(
                    options,
                    intermediates,
                    &mut error,
                )
            },
            unsafe {
                wellfriendpdf_signature_validation_options_apply_evidence_store(
                    options, evidence, &mut error,
                )
            },
            unsafe {
                wellfriendpdf_signature_validation_options_apply_retrieval_policy(
                    options, retrieval, &mut error,
                )
            },
            unsafe {
                wellfriendpdf_signature_validation_options_set_cancellation(
                    options,
                    cancellation,
                    &mut error,
                )
            },
        ] {
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(error.is_null());
        }

        let status = unsafe {
            wellfriendpdf_signature_validation_cancellation_cancel(cancellation, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let mut report = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_signatures_with_options_handle(
                doc,
                options,
                &mut report,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(report.is_null());
        let message = unsafe { CStr::from_ptr(error) }.to_string_lossy();
        assert!(
            message.contains("operation cancelled"),
            "message: {message}"
        );

        unsafe {
            wellfriendpdf_error_free(error);
            wellfriendpdf_signature_validation_options_free(options);
            wellfriendpdf_signature_validation_cancellation_free(cancellation);
            wellfriendpdf_signature_retrieval_policy_free(retrieval);
            wellfriendpdf_signature_evidence_store_free(evidence);
            wellfriendpdf_signature_intermediate_store_free(intermediates);
            wellfriendpdf_signature_trust_store_free(trust);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_phase3_pdf_utilities_return_owned_buffers() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        let mut jpeg = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_render_page_jpeg(doc, 1, 72, 80, &mut jpeg, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let jpeg_bytes = unsafe { std::slice::from_raw_parts(jpeg.data, jpeg.len) };
        assert!(jpeg_bytes.starts_with(&[0xFF, 0xD8]));
        unsafe { wellfriendpdf_buffer_free(jpeg) };

        let pages = [1usize, 1usize];
        let mut organized = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_organize_pdf(
                doc,
                pages.as_ptr(),
                pages.len(),
                &mut organized,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let org_bytes = unsafe { std::slice::from_raw_parts(organized.data, organized.len) };
        let re = ContentEngine::open_bytes(org_bytes.to_vec()).unwrap();
        assert_eq!(re.page_count().unwrap(), 2);
        unsafe { wellfriendpdf_buffer_free(organized) };

        let text = CString::new("DRAFT").unwrap();
        let mut watermarked = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_watermark_text_pdf(
                doc,
                text.as_ptr(),
                0.25,
                45.0,
                48.0,
                &mut watermarked,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let bytes = unsafe { std::slice::from_raw_parts(watermarked.data, watermarked.len) };
        let re = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
        assert!(re.get_page_text(1).unwrap().contains("DRAFT"));
        unsafe {
            wellfriendpdf_buffer_free(watermarked);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_phase4_office_conversions_return_owned_buffers() {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        let layout = CString::new("pages").unwrap();
        let mut xlsx = WellfriendBuffer::empty();
        let status =
            unsafe { wellfriendpdf_document_to_xlsx(doc, layout.as_ptr(), &mut xlsx, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let xlsx_bytes = unsafe { std::slice::from_raw_parts(xlsx.data, xlsx.len) };
        assert!(xlsx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(xlsx_bytes, "xl/workbook.xml"));
        let mut xlsx_pdf = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_xlsx_to_pdf(
                xlsx_bytes.as_ptr(),
                xlsx_bytes.len(),
                &mut xlsx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let xlsx_pdf_bytes = unsafe { std::slice::from_raw_parts(xlsx_pdf.data, xlsx_pdf.len) };
        assert!(xlsx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe { wellfriendpdf_buffer_free(xlsx) };
        unsafe { wellfriendpdf_buffer_free(xlsx_pdf) };

        let mut pptx = WellfriendBuffer::empty();
        let status = unsafe { wellfriendpdf_document_to_pptx(doc, 1, &mut pptx, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let pptx_bytes = unsafe { std::slice::from_raw_parts(pptx.data, pptx.len) };
        assert!(pptx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(pptx_bytes, "ppt/presentation.xml"));
        let mut pptx_pdf = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_pptx_to_pdf(
                pptx_bytes.as_ptr(),
                pptx_bytes.len(),
                &mut pptx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let pptx_pdf_bytes = unsafe { std::slice::from_raw_parts(pptx_pdf.data, pptx_pdf.len) };
        assert!(pptx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe { wellfriendpdf_buffer_free(pptx_pdf) };

        let mut docx = WellfriendBuffer::empty();
        let status = unsafe { wellfriendpdf_document_to_docx(doc, 1, &mut docx, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let docx_bytes = unsafe { std::slice::from_raw_parts(docx.data, docx.len) };
        assert!(docx_bytes.starts_with(b"PK"));
        assert!(contains_ascii(docx_bytes, "word/document.xml"));
        let mut docx_pdf = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_docx_to_pdf(
                docx_bytes.as_ptr(),
                docx_bytes.len(),
                &mut docx_pdf,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let docx_pdf_bytes = unsafe { std::slice::from_raw_parts(docx_pdf.data, docx_pdf.len) };
        assert!(docx_pdf_bytes.starts_with(b"%PDF-"));
        unsafe {
            wellfriendpdf_buffer_free(pptx);
            wellfriendpdf_buffer_free(docx);
            wellfriendpdf_buffer_free(docx_pdf);
            wellfriendpdf_document_free(doc);
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
        let status =
            unsafe { wellfriendpdf_document_page_count(std::ptr::null(), &mut count, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(!error.is_null());
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        assert!(message.contains("document pointer is null"));
        unsafe {
            wellfriendpdf_error_free(error);
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
        emit: WellfriendOcrEmitWordFn,
    ) -> c_int {
        let text = CString::new("CabiWord").unwrap();
        emit(sink, text.as_ptr(), 72.0, 60.0, 200.0, 88.0, 0.9, 0);
        0
    }

    #[test]
    fn capi_function_pointer_ocr_backend_reaches_document_model() {
        let pdf = scanned_page_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());

        // Without a backend, the scanned page degrades to the placeholder.
        let mut md = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_document_parse_markdown_ocr(doc, &mut md, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let before = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(!before.contains("CabiWord"), "no OCR yet: {before}");
        unsafe { wellfriendpdf_string_free(md) };

        // Register the function-pointer backend and re-parse.
        let name = CString::new("c-mock").unwrap();
        let backend = WellfriendOcrBackend {
            userdata: std::ptr::null_mut(),
            recognize: Some(mock_recognize),
            max_concurrency: 1,
            name: name.as_ptr(),
        };
        let status = unsafe { wellfriendpdf_document_set_ocr_backend(doc, backend, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);

        let mut md = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_document_parse_markdown_ocr(doc, &mut md, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let after = unsafe { CStr::from_ptr(md) }.to_string_lossy().into_owned();
        assert!(
            after.contains("CabiWord"),
            "OCR word must reach the document model: {after}"
        );
        unsafe {
            wellfriendpdf_string_free(md);
            wellfriendpdf_document_free(doc);
        }
    }

    // ── binding-surface report surfaces ─────────────────────────────────────────────

    /// Open the sample PDF, returning the opaque handle (caller frees).
    fn open_sample() -> (*mut WellfriendDocument, Vec<u8>) {
        let pdf = sample_pdf();
        let mut error = std::ptr::null_mut();
        let doc =
            unsafe { wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error) };
        assert!(!doc.is_null());
        (doc, pdf)
    }

    /// Call a report fn, parse its JSON, assert the envelope kind, then free.
    fn report_envelope(
        f: unsafe extern "C" fn(
            *const WellfriendDocument,
            *mut *mut c_char,
            *mut *mut c_char,
        ) -> c_int,
        kind: &str,
    ) -> serde_json::Value {
        let (doc, _pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe { f(doc, &mut json, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK, "report fn returned error");
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["kind"], kind);
        assert!(value.get("report").is_some());
        unsafe {
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
        value
    }

    #[test]
    fn capi_read_only_report_envelopes() {
        report_envelope(
            wellfriendpdf_document_security_report_json,
            "security_report",
        );
        report_envelope(wellfriendpdf_document_forms_report_json, "forms_report");
        let xfa = report_envelope(wellfriendpdf_document_xfa_report_json, "xfa_report");
        assert_eq!(xfa["report"]["schema_version"], "xfa_runtime.xfa.v1");
        report_envelope(
            wellfriendpdf_document_xfa_extract_json,
            "xfa_extract_report",
        );
        report_envelope(
            wellfriendpdf_document_xfa_script_report_json,
            "xfa_script_report",
        );
        report_envelope(
            wellfriendpdf_document_xfa_security_report_json,
            "xfa_security_report",
        );
        report_envelope(
            wellfriendpdf_document_annotations_report_json,
            "annotation_report",
        );
        let media = report_envelope(
            wellfriendpdf_document_rich_media_report_json,
            "rich_media_report",
        );
        assert_eq!(
            media["report"]["schema_version"],
            "annotation_media_redaction.annotation-xfdf-media-redaction.v1"
        );
        report_envelope(
            wellfriendpdf_document_annotation_media_redaction_report_json,
            "annotation_media_redaction_report",
        );
        let secure_mutation = report_envelope(
            wellfriendpdf_document_secure_mutation_report_json,
            "secure_mutation_report",
        );
        assert_eq!(
            secure_mutation["report"]["schema_version"],
            "secure_mutation.mask-inline-associated-signature-policy.v1"
        );
        let secure_mutation_closeout = report_envelope(
            wellfriendpdf_document_secure_mutation_closeout_report_json,
            "secure_mutation_closeout_report",
        );
        assert_eq!(
            secure_mutation_closeout["report"]["schema_version"],
            "secure_mutation_closeout.advanced-secure-mutation-closure.v1"
        );
        report_envelope(wellfriendpdf_document_form_js_report_json, "form_js_report");
        report_envelope(
            wellfriendpdf_document_form_action_graph_json,
            "form_action_graph",
        );
        report_envelope(
            wellfriendpdf_document_interactive_data_report_json,
            "interactive_data_report",
        );
        let form_action_policy = report_envelope(
            wellfriendpdf_document_form_action_policy_report_json,
            "form_action_policy_report",
        );
        assert_eq!(
            form_action_policy["report"]["schema_version"],
            "form_action_policy.form-js-interactive-docx-layout.v1"
        );
        let advanced_editing = report_envelope(
            wellfriendpdf_document_advanced_editing_report_json,
            "advanced_editing_report",
        );
        assert_eq!(
            advanced_editing["report"]["schema_version"],
            "advanced_editing.vertical-rtl-patch-vector-ink-editing.v1"
        );
        let advanced_editing_closeout = report_envelope(
            wellfriendpdf_document_advanced_editing_closeout_report_json,
            "advanced_editing_closeout_report",
        );
        assert_eq!(
            advanced_editing_closeout["report"]["schema_version"],
            "advanced_editing_closeout.multirun-form-appearance-closure.v1"
        );
        let source_editing = report_envelope(
            wellfriendpdf_document_source_editing_report_json,
            "source_editing_report",
        );
        assert_eq!(
            source_editing["report"]["schema_version"],
            "source_editing.provenance-operator-editing.v1"
        );
        let document_subsystems = report_envelope(
            wellfriendpdf_document_document_subsystems_report_json,
            "document_subsystems_report",
        );
        assert_eq!(
            document_subsystems["report"]["feature_matrix"]["schema_version"],
            "document_subsystems.tables-math-ocr-forms-annotations.v1"
        );
        report_envelope(
            wellfriendpdf_document_associated_files_report_json,
            "associated_files_report",
        );
        report_envelope(
            wellfriendpdf_document_pages_report_json,
            "page_operations_report",
        );
        report_envelope(
            wellfriendpdf_document_interactive_report_json,
            "interactive_report",
        );
        report_envelope(wellfriendpdf_document_chunks_json, "chunk_set");
        let advanced = report_envelope(
            wellfriendpdf_document_advanced_chunks_json,
            "advanced_rag_chunk_set",
        );
        assert_eq!(
            advanced["report"]["schema_version"],
            "semantic_closeout.rag_chunk.v1"
        );
        let semantic = report_envelope(
            wellfriendpdf_document_semantic_bundle_json,
            "semantic_binding_report",
        );
        assert_eq!(
            semantic["report"]["schema_version"],
            "semantic_closeout.semantic_binding.v1"
        );
        assert_eq!(semantic["report"]["privacy"]["cloud_upload_default"], false);
    }

    #[test]
    fn capi_advanced_editing_closeout_range_analyze_and_edit_return_owned_outputs() {
        let (doc, _pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_advanced_editing_closeout_text_range_analyze_json(
                doc, 1, &mut json, &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["kind"],
            "advanced_editing_closeout_multi_run_range_model"
        );
        assert_eq!(
            value["report"]["schema_version"],
            "advanced_editing_closeout.multirun-form-appearance-closure.v1"
        );
        assert!(value["report"]["logical_text"]
            .as_str()
            .unwrap()
            .starts_with("Hello"));
        unsafe { wellfriendpdf_string_free(json) };

        let request = CString::new(
            r#"{
                "page":1,
                "logical_start":0,
                "logical_end":11,
                "replacement_text":"CABI20B",
                "mode":"paragraph_reflow_horizontal",
                "style_policy":"inherit_leading",
                "options":{
                    "region":[20.0,80.0,180.0,140.0],
                    "font_size":12.0,
                    "line_spacing":1.2,
                    "max_lines_or_columns":4096,
                    "overflow_policy":"error",
                    "signature_policy_override":false,
                    "deterministic":true
                }
            }"#,
        )
        .unwrap();
        let mut output = WellfriendBuffer::empty();
        let mut edit_json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_advanced_editing_closeout_text_range_edit_json(
                doc,
                request.as_ptr(),
                &mut output,
                &mut edit_json,
                &mut error,
            )
        };
        let error_text = if error.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK, "{error_text}");
        let bytes = unsafe { slice::from_raw_parts(output.data, output.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        let report = unsafe { CStr::from_ptr(edit_json) }.to_string_lossy();
        assert!(report.contains("advanced_editing_closeout_multi_run_text_edit_report"));
        assert!(report.contains("\"replacement_extracts\":true"));
        unsafe {
            wellfriendpdf_buffer_free(output);
            wellfriendpdf_string_free(edit_json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_source_editing_operator_surfaces_return_owned_outputs() {
        let (doc, _pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let source = CString::new("Hello C API").unwrap();
        let replacement = CString::new("HELLO C ABI").unwrap();
        let status = unsafe {
            wellfriendpdf_document_source_editing_provenance_json(
                doc,
                1,
                source.as_ptr(),
                replacement.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(text.contains("source_editing_provenance_report"));
        unsafe { wellfriendpdf_string_free(json) };

        let request = CString::new(
            r#"{
                "requested_mode":"operator_preserving",
                "page":1,
                "source_text":"Hello C API",
                "replacement_text":"HELLO C ABI",
                "signature_policy_override":false
            }"#,
        )
        .unwrap();
        let mut output = WellfriendBuffer::empty();
        let mut edit_json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_source_editing_operator_text_edit_json(
                doc,
                request.as_ptr(),
                &mut output,
                &mut edit_json,
                &mut error,
            )
        };
        let error_text = if error.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK, "{error_text}");
        let bytes = unsafe { slice::from_raw_parts(output.data, output.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        let report = unsafe { CStr::from_ptr(edit_json) }.to_string_lossy();
        assert!(report.contains("source_editing_operator_text_edit"));
        unsafe {
            wellfriendpdf_buffer_free(output);
            wellfriendpdf_string_free(edit_json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_editing_transactions_scene_transaction_font_surfaces_return_owned_outputs() {
        let (doc, _pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();

        let status = unsafe {
            wellfriendpdf_document_editing_transactions_report_json(doc, &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(text.contains("editing_transactions.scene-transactions-fonts-shaping.v1"));
        unsafe { wellfriendpdf_string_free(json) };

        let pages = CString::new("[1]").unwrap();
        let status = unsafe {
            wellfriendpdf_document_editing_transactions_scene_report_json(
                doc,
                pages.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let scene = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(scene.contains("editing_transactions_scene_report"));
        assert!(scene.contains("\"nodes\""));
        unsafe { wellfriendpdf_string_free(json) };

        let request = CString::new(
            r#"{
                "requested_mode":"operator_preserving",
                "page":1,
                "source_text":"Hello C API",
                "replacement_text":"HELLO C ABI"
            }"#,
        )
        .unwrap();
        let status = unsafe {
            wellfriendpdf_document_editing_transactions_transaction_plan_json(
                doc,
                request.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let plan = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(plan.contains("editing_transactions_transaction_plan"));
        assert!(plan.contains("transaction_id"));
        unsafe { wellfriendpdf_string_free(json) };

        let sample_text = CString::new("A\u{0301}B").unwrap();
        let direction = CString::new("ltr").unwrap();
        let status = unsafe {
            wellfriendpdf_document_editing_transactions_text_map_json(
                doc,
                sample_text.as_ptr(),
                direction.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let map = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(map.contains("editing_transactions_text_map"));
        assert!(map.contains("grapheme_clusters"));
        unsafe { wellfriendpdf_string_free(json) };

        let policy = CString::new("reuse_embedded_subset").unwrap();
        let status = unsafe {
            wellfriendpdf_document_editing_transactions_font_subset_plan_json(
                doc,
                sample_text.as_ptr(),
                direction.as_ptr(),
                policy.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let subset = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(subset.contains("editing_transactions_font_subset_plan"));
        assert!(subset.contains("deterministic_subset_tag"));
        unsafe {
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_text_reflow_reflow_surfaces_return_owned_outputs() {
        let (doc, pdf) = open_sample();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();

        let status =
            unsafe { wellfriendpdf_document_text_reflow_report_json(doc, &mut json, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(text.contains("text_reflow.geometric-semantic-reflow.v1"));
        unsafe { wellfriendpdf_string_free(json) };

        let request = CString::new(
            r#"{"requested_mode":"geometric_block","page":1,"source_text":"Hello C API","replacement_text":"World C API","region":[10.0,10.0,260.0,90.0],"language":"en","hyphenation":true,"layout_constraints":[{"constraint_id":"capi_soft_height","variable":"region_height","relation":"ge","value":500.0,"priority":"weak"}]}"#,
        )
        .unwrap();
        let status = unsafe {
            wellfriendpdf_document_text_reflow_reflow_preview_json(
                doc,
                request.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let preview = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(preview.contains("text_reflow_reflow_preview"));
        assert!(preview.contains("no_overlay"));
        unsafe { wellfriendpdf_string_free(json) };
        json = std::ptr::null_mut();

        for (operation, expected) in [
            (
                wellfriendpdf_document_text_reflow_overflow_report_json
                    as unsafe extern "C" fn(
                        *const WellfriendDocument,
                        *const c_char,
                        *mut *mut c_char,
                        *mut *mut c_char,
                    ) -> c_int,
                "text_reflow_overflow_report",
            ),
            (
                wellfriendpdf_document_text_reflow_constraints_report_json,
                "text_reflow_constraints_report",
            ),
            (
                wellfriendpdf_document_text_reflow_confidence_report_json,
                "text_reflow_confidence_report",
            ),
        ] {
            let status = unsafe { operation(doc, request.as_ptr(), &mut json, &mut error) };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK, "{expected}");
            let report = unsafe { CStr::from_ptr(json) }.to_string_lossy();
            assert!(report.contains(expected), "{report}");
            if expected == "text_reflow_constraints_report" {
                assert!(report.contains("capi_soft_height"), "{report}");
                assert!(report.contains("unsatisfied_soft_constraints"), "{report}");
            }
            unsafe { wellfriendpdf_string_free(json) };
            json = std::ptr::null_mut();
        }

        let mut geometric_output = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_text_reflow_reflow_region_json(
                doc,
                request.as_ptr(),
                &mut geometric_output,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        unsafe { wellfriendpdf_string_free(json) };
        json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_text_reflow_validate_reflow_output_json(
                doc,
                geometric_output.data,
                geometric_output.len,
                request.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let validation = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(validation.contains("text_reflow_validate_reflow_output"));
        assert!(validation.contains("\"valid\":true"));
        unsafe { wellfriendpdf_string_free(json) };
        json = std::ptr::null_mut();
        let mut restored = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_text_reflow_undo_reflow_json(
                doc,
                geometric_output.data,
                geometric_output.len,
                request.as_ptr(),
                &mut restored,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let undo = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(undo.contains("text_reflow_undo_reflow"));
        assert!(undo.contains("\"byte_exact_restoration\":true"));
        let restored_bytes = unsafe { slice::from_raw_parts(restored.data, restored.len) };
        assert_eq!(restored_bytes, pdf.as_slice());
        unsafe {
            wellfriendpdf_string_free(json);
            wellfriendpdf_buffer_free(geometric_output);
            wellfriendpdf_buffer_free(restored);
        }
        json = std::ptr::null_mut();

        let semantic_without_approval = CString::new(
            r#"{"requested_mode":"semantic_document","page":1,"source_text":"Hello C API","replacement_text":"World C API","region":[10.0,10.0,260.0,90.0],"language":"en"}"#,
        )
        .unwrap();
        let mut output = WellfriendBuffer::empty();
        let status = unsafe {
            wellfriendpdf_document_text_reflow_reflow_document_json(
                doc,
                semantic_without_approval.as_ptr(),
                &mut output,
                &mut json,
                &mut error,
            )
        };
        assert_ne!(status, WELLFRIENDPDF_STATUS_OK);
        let refusal = unsafe { CStr::from_ptr(error) }.to_string_lossy();
        // SemanticDocument must refuse an unapproved low-confidence structure
        // before source mutation. The central policy may refuse before the
        // later paragraph-ambiguity gate; both the engine and ABI expose the
        // policy's exact `refuse` result.
        assert!(
            refusal.contains("\"refuse\"") && refusal.contains("confidence policy"),
            "unexpected typed semantic refusal: {refusal}"
        );
        unsafe { wellfriendpdf_string_free(error) };
        unsafe {
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_pades_ltv_timestamp_and_signature_preserving_plan_return_owned_reports() {
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let token = b"not-a-rfc3161-token";
        let signature_value = b"cms-signature-value";
        let options = CString::new("{}").unwrap();
        let status = unsafe {
            wellfriendpdf_timestamp_token_validation_json(
                token.as_ptr(),
                token.len(),
                signature_value.as_ptr(),
                signature_value.len(),
                options.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "timestamp_token_validation");
        assert_eq!(
            value["report"]["token_type"],
            serde_json::Value::String("signature_timestamp".to_string())
        );
        assert_eq!(
            value["report"]["status"],
            serde_json::Value::String("malformed".to_string())
        );
        unsafe { wellfriendpdf_string_free(json) };

        let (doc, _pdf) = open_sample();
        let field = CString::new("PadesLTVField").unwrap();
        let value_text = CString::new("PadesLTVValue").unwrap();
        let mut plan_json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_signature_preserving_form_plan_json(
                doc,
                field.as_ptr(),
                value_text.as_ptr(),
                options.as_ptr(),
                &mut plan_json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(plan_json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "signature_preserving_edit_plan");
        assert_eq!(
            value["report"]["schema_version"],
            serde_json::Value::String(
                wellfriendpdf_engine::PADES_LTV_SIGNATURE_LTV_EDIT_SCHEMA_VERSION.to_string()
            )
        );
        assert_eq!(value["report"]["prefix_preservation_required"], true);
        unsafe {
            wellfriendpdf_string_free(plan_json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_annotation_media_redaction_xfdf_export_returns_owned_artifact_and_report() {
        let (doc, _pdf) = open_sample();
        let mut output = WellfriendBuffer::empty();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_annotation_xfdf_export_json(
                doc,
                &mut output,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len) };
        assert!(bytes.starts_with(b"<?xml"));
        let report = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        assert!(report.contains("annotation_xfdf_export_report"));
        unsafe {
            wellfriendpdf_buffer_free(output);
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_semantic_closeout_semantic_search_and_null_handling() {
        let (doc, _pdf) = open_sample();
        let query = CString::new("Hello").unwrap();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_semantic_search_json(doc, query.as_ptr(), &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "semantic_search_report");
        assert_eq!(value["report"]["query"], "Hello");
        assert_eq!(value["report"]["provenance_preserved"], true);
        assert!(!value["report"]["semantic_matches"]
            .as_array()
            .unwrap()
            .is_empty());
        unsafe { wellfriendpdf_string_free(json) };

        let mut null_json = std::ptr::null_mut();
        let mut null_error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_semantic_search_json(
                doc,
                std::ptr::null(),
                &mut null_json,
                &mut null_error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(null_json.is_null());
        assert!(!null_error.is_null());
        unsafe {
            wellfriendpdf_error_free(null_error);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_parametrized_reports() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();

        // parser report with explicit mode.
        let mode = CString::new("audit").unwrap();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_parser_report_json(doc, mode.as_ptr(), &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "parser_report");
        assert_eq!(value["report"]["opened"], true);
        unsafe { wellfriendpdf_string_free(json) };

        // color report with NULL profile (defaults to generic).
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_color_report_json(doc, std::ptr::null(), &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        assert!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["kind"] == "color_report"
        );
        unsafe { wellfriendpdf_string_free(json) };

        // validate with a profile.
        let profile = CString::new("all").unwrap();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_validate_json(doc, profile.as_ptr(), &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        unsafe { wellfriendpdf_string_free(json) };

        unsafe { wellfriendpdf_document_free(doc) };
    }

    #[test]
    fn capi_sanitize_and_canonicalize_output_and_report() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();

        let mut buf = WellfriendBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_sanitize_json(
                doc,
                std::ptr::null(),
                &mut buf,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
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
            wellfriendpdf_buffer_free(buf);
            wellfriendpdf_string_free(json);
        }

        let mut buf = WellfriendBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_canonicalize_json(doc, 0, 1, &mut buf, &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let bytes = unsafe { slice::from_raw_parts(buf.data, buf.len) };
        assert!(bytes.starts_with(b"%PDF-"));
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "canonicalize_report");
        assert_eq!(value["report"]["deterministic"], true);
        unsafe {
            wellfriendpdf_buffer_free(buf);
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_redact_terms_output_and_report() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();
        let term = CString::new("Hello").unwrap();
        let terms = [term.as_ptr()];
        let mut buf = WellfriendBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_redact_terms_json(
                doc,
                terms.as_ptr(),
                terms.len(),
                0,
                &mut buf,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
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
            wellfriendpdf_buffer_free(buf);
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_feature_and_version() {
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe { wellfriendpdf_feature_report_json(&mut json, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "feature_report");
        assert!(value["report"]["engine_version"].is_string());
        assert_eq!(
            value["report"]["codec_boundary"]["scanner"]["default_implementation"],
            "safe_first_byte_chunked"
        );
        assert_eq!(
            value["report"]["codec_boundary"]["renderer_decode_scheduler"]["status"],
            "adopted_for_immediate_renderer_decode_paths"
        );
        assert_eq!(
            value["report"]["decode_scheduler"]["decode_scheduler"]["status"],
            "adopted_for_decode_scheduler_non_render_decode_paths"
        );
        assert_eq!(
            value["report"]["native_renderer"]["native_replay"]["status"],
            "native_text_image_form_display_list_foundation"
        );
        assert_eq!(
            value["report"]["native_renderer"]["reference_renderer_multi_reference_audit"]
                ["status"],
            "multi_reference_audit_complete"
        );
        assert_eq!(
            value["report"]["transparency_rendering_transparency_compositing"]["status"],
            "native_foundation_with_transparency_closeout_closure"
        );
        assert_eq!(
            value["report"]["transparency_rendering_transparency_compositing"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            value["report"]["transparency_rendering_transparency_compositing"]["blend_modes"]
                ["implemented"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode == "Luminosity")
        );
        assert_eq!(
            value["report"]["transparency_closeout_transparency_closure"]["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["transparency_closeout_transparency_closure"]["reference_audit"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert!(
            value["report"]["transparency_closeout_transparency_closure"]
                ["luminosity_soft_mask_color_spaces"]["supported"]
                .as_array()
                .unwrap()
                .iter()
                .any(|space| space == "DeviceCMYK")
        );
        assert_eq!(
            value["report"]["advanced_rendering_text_clipping_shading_patterns"]["status"],
            "native_common_paths_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["advanced_rendering_text_clipping_shading_patterns"]["reference_audit"]
                ["memory_cap_mb"],
            4096
        );
        assert!(
            value["report"]["advanced_rendering_text_clipping_shading_patterns"]["text_clipping"]
                ["rendering_modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mode| mode.as_i64() == Some(7))
        );
        assert_eq!(
            value["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["status"],
            "complete_native_common_paths_with_reference_cluster_limits"
        );
        assert_eq!(
            value["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["reference_audit"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["type7_tensor_patch"]
                ["status"],
            "native_tensor_product_interior"
        );
        assert_eq!(
            value["report"]["annotation_ocg_rendering_annotation_ocg_progressive_cache"]["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["renderer_validation_annotation_progressive_cache_validation"]
                ["status"],
            "implemented_and_proven"
        );
        assert_eq!(
            value["report"]["renderer_validation_annotation_progressive_cache_validation"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
                ["status"],
            "implemented_with_bounded_unsupported_reports"
        );
        assert_eq!(
            value["report"]["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
                ["closure_gates"]["memory_cap_mb"],
            4096
        );
        assert_eq!(
            value["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
                ["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_color_glyph_hinting"
        );
        assert_eq!(
            value["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]
                ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
                ["svg_in_opentype"]["status"],
            "safe_static_subset_rendered_active_constructs_blocked"
        );
        assert_eq!(
            value["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
                ["closure_gates"]["public_report_schema"],
            "additive_feature_report_colrv_svg_bitmap"
        );
        assert_eq!(
            value["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["colrv1_clip_stack"]["status"],
            "implemented"
        );
        assert_eq!(
            value["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
                ["closure_gates"]["public_report_schema"],
            "additive_feature_report_colrv_gradient_composite"
        );
        assert_eq!(
            value["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["status"],
            "complete"
        );
        assert_eq!(
            value["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["porter_duff_plus_composites"]["implemented_modes"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(
            value["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
                ["closure_gates"]["public_report_schema"],
            "additive_feature_report_porterduff_radial_color_glyph"
        );
        assert_eq!(
            value["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["status"],
            "complete_with_native_cmm_hard_blocked_precise"
        );
        assert_eq!(
            value["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_fuzz"]
                ["fuzz_target_count"],
            25
        );
        assert_eq!(
            value["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_closeout"]
                ["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            value["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["native_cmm_backend"]
                ["backend_used_in_current_build"],
            "safe-rust-plus-qcms"
        );
        assert_eq!(
            value["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["closure_gates"]
                ["public_report_schema"],
            "additive_feature_report_renderer_fuzz_cmm"
        );
        let native_cmm_backend =
            &value["report"]["native_cmm_backend_native_littlecms_cmm_backend_closure"];
        assert_eq!(native_cmm_backend["status"], "complete");
        assert_eq!(
            native_cmm_backend["feature_flag"]["name"],
            "native-cmm-lcms2"
        );
        assert_eq!(
            native_cmm_backend["closure_gates"]["public_report_schema"],
            "additive_feature_report_native_cmm_backend"
        );
        let prepress_cmm =
            &value["report"]["prepress_cmm_prepress_cmm_device_link_separation_plates"];
        assert_eq!(prepress_cmm["status"], "complete");
        assert_eq!(
            prepress_cmm["closure_gates"]["public_report_schema"],
            "additive_feature_report_prepress_cmm"
        );
        assert_eq!(
            prepress_cmm["separation_framebuffer"]["cache_key_includes_plate_state"],
            true
        );
        let nchannel_plate_prepress =
            &value["report"]["nchannel_plate_prepress_nchannel_plate_reference_closure"];
        assert_eq!(nchannel_plate_prepress["status"], "complete");
        assert_eq!(
            nchannel_plate_prepress["closure_gates"]["public_report_schema"],
            "additive_feature_report_nchannel_plate_prepress"
        );
        assert_eq!(
            nchannel_plate_prepress["reference_audit"]["pdfium"],
            "required_and_run_by_nchannel_plate_prepress_audit"
        );
        let prepress_proofing =
            &value["report"]["prepress_proofing_full_overprint_prepress_closeout"];
        assert_eq!(prepress_proofing["status"], "complete");
        assert_eq!(
            prepress_proofing["closure_gates"]["public_report_schema"],
            "additive_feature_report_prepress_proofing"
        );
        assert_eq!(
            prepress_proofing["reference_audit"]["wellfriendpdf_outlier_failures"],
            0
        );
        assert_eq!(
            prepress_proofing["reference_audit"]["unclassified_failures"],
            0
        );
        let semantic_intelligence = &value["report"]
            ["semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout"];
        assert_eq!(semantic_intelligence["status"], "complete");
        assert_eq!(
            semantic_intelligence["closure_gates"]["public_report_schema"],
            "additive_feature_report_semantic_intelligence"
        );
        assert_eq!(
            semantic_intelligence["privacy_defaults"]["cloud_upload_default"],
            false
        );
        let cjk_dictionary_layout =
            &value["report"]["cjk_dictionary_layout_cjk_dictionary_layout_backend_closure"];
        assert_eq!(cjk_dictionary_layout["status"], "complete");
        assert_eq!(
            cjk_dictionary_layout["closure_gates"]["public_report_schema"],
            "additive_feature_report_cjk_dictionary_layout"
        );
        assert_eq!(
            cjk_dictionary_layout["dictionary_provider"]["external_pack_support"],
            "implemented"
        );
        assert_eq!(
            cjk_dictionary_layout["layout_backend"]["local_backend_status"],
            "unsupported_reported_no_runtime"
        );
        let semantic_closeout =
            &value["report"]["semantic_closeout_semantic_binding_rag_benchmark_closeout"];
        assert_eq!(semantic_closeout["status"], "complete");
        assert_eq!(
            semantic_closeout["closure_gates"]["public_report_schema"],
            "additive_feature_report_semantic_closeout"
        );
        assert_eq!(semantic_closeout["closure_counts"]["blocked"], 0);
        assert_eq!(semantic_closeout["privacy"]["cloud_upload_default"], false);
        assert_eq!(
            semantic_closeout["tableformer_table_transformer_hook"]
                ["model_can_rewrite_deterministic_text"],
            false
        );
        let xfa_runtime = &value["report"]["xfa_runtime_xfa_runtime_sandbox_closure"];
        assert_eq!(xfa_runtime["status"], "complete_bounded_foundation");
        assert_eq!(xfa_runtime["closure_counts"]["blocked"], 0);
        assert_eq!(
            xfa_runtime["closure_gates"]["public_report_schema"],
            "additive_feature_report_xfa_runtime"
        );
        unsafe { wellfriendpdf_string_free(json) };

        let version = wellfriendpdf_version();
        assert!(!version.is_null());
        let v = unsafe { CStr::from_ptr(version) }
            .to_string_lossy()
            .into_owned();
        assert!(!v.is_empty());
        unsafe { wellfriendpdf_string_free(version) };

        assert_eq!(wellfriendpdf_abi_version(), 1);
    }

    #[test]
    fn capi_codec_isolation_report_envelope() {
        let filter = CString::new("FlateDecode").unwrap();
        let policy = CString::new("report_only").unwrap();
        let input = wellfriendpdf_engine::flate_encode(b"capi isolation", 6);
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_codec_isolation_report_json(
                filter.as_ptr(),
                input.as_ptr(),
                input.len(),
                policy.as_ptr(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let text = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kind"], "codec_isolation_report");
        assert_eq!(value["report"]["status"], "report_only");
        unsafe { wellfriendpdf_string_free(json) };
    }

    #[test]
    fn capi_repeated_open_report_free_stress() {
        let pdf = sample_pdf();
        for _ in 0..50 {
            let mut error = std::ptr::null_mut();
            let doc = unsafe {
                wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error)
            };
            assert!(!doc.is_null());
            assert!(error.is_null());

            let mut json = std::ptr::null_mut();
            let status =
                unsafe { wellfriendpdf_document_security_report_json(doc, &mut json, &mut error) };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
            assert!(!json.is_null());
            unsafe {
                wellfriendpdf_string_free(json);
                wellfriendpdf_document_free(doc);
            }
        }
    }

    #[test]
    fn capi_report_null_document_is_error_not_panic() {
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_security_report_json(std::ptr::null(), &mut json, &mut error)
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(json.is_null());
        assert!(!error.is_null());
        unsafe { wellfriendpdf_error_free(error) };
    }

    // ── Incremental Signing Standards clause-mapped standards + incremental signing ───────────────

    /// Generate an ephemeral RSA-2048 key + self-signed leaf certificate as PEM
    /// strings for the C-ABI signing tests.
    fn ephemeral_signer_pem() -> (String, String) {
        use der::asn1::GeneralizedTime;
        use der::{DateTime, EncodePem};
        use rand_core::OsRng;
        use rsa::pkcs1v15::SigningKey as RsaSigningKey;
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rsa::{RsaPrivateKey, RsaPublicKey};
        use sha2::Sha256;
        use spki::SubjectPublicKeyInfoOwned;
        use std::str::FromStr;
        use x509_cert::builder::{Builder, CertificateBuilder, Profile};
        use x509_cert::name::Name;
        use x509_cert::serial_number::SerialNumber;
        use x509_cert::time::{Time, Validity};

        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa key");
        let signing_key = RsaSigningKey::<Sha256>::new(private_key.clone());
        let public_key = RsaPublicKey::from(&private_key);
        let spki_der = public_key.to_public_key_der().expect("spki der");
        let spki = SubjectPublicKeyInfoOwned::try_from(spki_der.as_bytes()).expect("spki");
        let subject = Name::from_str(
            "CN=Wellfriend C-ABI IncrementalSigningStandards Test,O=Wellfriend Test,C=US",
        )
        .unwrap();
        let validity = Validity {
            not_before: Time::GeneralTime(GeneralizedTime::from_date_time(
                DateTime::new(2020, 1, 1, 0, 0, 0).unwrap(),
            )),
            not_after: Time::GeneralTime(GeneralizedTime::from_date_time(
                DateTime::new(2040, 1, 1, 0, 0, 0).unwrap(),
            )),
        };
        let builder = CertificateBuilder::new(
            Profile::Leaf {
                issuer: subject.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            SerialNumber::from(0x2026_0801u32),
            validity,
            subject,
            spki,
            &signing_key,
        )
        .expect("cert builder");
        let cert = builder
            .build::<rsa::pkcs1v15::Signature>()
            .expect("cert build");
        let key_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("key pem")
            .to_string();
        let cert_pem = cert.to_pem(LineEnding::LF).expect("cert pem");
        (key_pem, cert_pem)
    }

    #[test]
    fn capi_standards_report_envelopes() {
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();
        for (name, f) in [
            (
                "pdfa_standards_validation",
                wellfriendpdf_document_pdfa_standards_json
                    as unsafe extern "C" fn(
                        *const WellfriendDocument,
                        *const c_char,
                        *mut *mut c_char,
                        *mut *mut c_char,
                    ) -> c_int,
            ),
            (
                "pdfua_standards_validation",
                wellfriendpdf_document_pdfua_standards_json,
            ),
            (
                "pdfx_standards_validation",
                wellfriendpdf_document_pdfx_standards_json,
            ),
            (
                "standards_all_validation",
                wellfriendpdf_document_standards_all_json,
            ),
        ] {
            let mut json = std::ptr::null_mut();
            let status = unsafe { f(doc, std::ptr::null(), &mut json, &mut error) };
            assert_eq!(status, WELLFRIENDPDF_STATUS_OK, "{name} returned error");
            let text = unsafe { CStr::from_ptr(json) }
                .to_string_lossy()
                .into_owned();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["kind"], name);
            assert!(value.get("report").is_some());
            unsafe { wellfriendpdf_string_free(json) };
        }
        unsafe { wellfriendpdf_document_free(doc) };
    }

    #[test]
    fn capi_sign_plan_and_sign_pdf_real_runtime() {
        let (key_pem, cert_pem) = ephemeral_signer_pem();
        let key = CString::new(key_pem).unwrap();
        let cert = CString::new(cert_pem).unwrap();
        let (doc, pdf) = open_sample();
        let mut error = std::ptr::null_mut();

        // Plan: a tiny placeholder cannot fit an RSA-2048 CMS.
        let mut plan_json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_sign_plan_json(
                doc,
                key.as_ptr(),
                cert.as_ptr(),
                8,
                0,
                &mut plan_json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let plan = unsafe { CStr::from_ptr(plan_json) }
            .to_string_lossy()
            .into_owned();
        let plan_value: serde_json::Value = serde_json::from_str(&plan).unwrap();
        assert_eq!(plan_value["fits"], false);
        assert!(plan_value["required_bytes"].as_u64().unwrap() > 8);
        unsafe { wellfriendpdf_string_free(plan_json) };

        // Sign: real append-only signature; output reopens and validates.
        let mut buf = WellfriendBuffer::empty();
        let mut json = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_sign_pdf(
                doc,
                key.as_ptr(),
                cert.as_ptr(),
                16384,
                0,
                std::ptr::null(),
                std::ptr::null(),
                &mut buf,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let signed = unsafe { slice::from_raw_parts(buf.data, buf.len) };
        assert!(signed.starts_with(b"%PDF-"));
        assert!(signed.starts_with(&pdf), "original bytes must be a prefix");
        let report = unsafe { CStr::from_ptr(json) }
            .to_string_lossy()
            .into_owned();
        let report_value: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(report_value["post_sign"]["signature_valid"], true);
        assert_eq!(report_value["prefix_preserved"], true);

        // Reopen the produced bytes through the C ABI and validate the signature.
        let signed_vec = signed.to_vec();
        let reopened = unsafe {
            wellfriendpdf_document_open_from_bytes(
                signed_vec.as_ptr(),
                signed_vec.len(),
                &mut error,
            )
        };
        assert!(!reopened.is_null());
        let mut sig_json = std::ptr::null_mut();
        let status =
            unsafe { wellfriendpdf_document_signatures_json(reopened, &mut sig_json, &mut error) };
        assert_eq!(status, WELLFRIENDPDF_STATUS_OK);
        let sigs = unsafe { CStr::from_ptr(sig_json) }
            .to_string_lossy()
            .into_owned();
        let sigs_value: serde_json::Value = serde_json::from_str(&sigs).unwrap();
        assert!(!sigs_value.as_array().unwrap().is_empty());
        unsafe {
            wellfriendpdf_string_free(sig_json);
            wellfriendpdf_document_free(reopened);
            wellfriendpdf_buffer_free(buf);
            wellfriendpdf_string_free(json);
            wellfriendpdf_document_free(doc);
        }
    }

    #[test]
    fn capi_sign_null_and_malformed_are_errors_not_panics() {
        // Null document → error.
        let key =
            CString::new("-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----").unwrap();
        let cert =
            CString::new("-----BEGIN CERTIFICATE-----\nnope\n-----END CERTIFICATE-----").unwrap();
        let mut json = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_sign_plan_json(
                std::ptr::null(),
                key.as_ptr(),
                cert.as_ptr(),
                16384,
                0,
                &mut json,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(json.is_null());
        assert!(!error.is_null());
        unsafe { wellfriendpdf_error_free(error) };

        // Malformed key on a valid document → error, no panic.
        let (doc, _pdf) = open_sample();
        let mut error = std::ptr::null_mut();
        let mut buf = WellfriendBuffer::empty();
        let mut sjson = std::ptr::null_mut();
        let status = unsafe {
            wellfriendpdf_document_sign_pdf(
                doc,
                key.as_ptr(),
                cert.as_ptr(),
                16384,
                0,
                std::ptr::null(),
                std::ptr::null(),
                &mut buf,
                &mut sjson,
                &mut error,
            )
        };
        assert_eq!(status, WELLFRIENDPDF_STATUS_ERROR);
        assert!(sjson.is_null());
        assert!(!error.is_null());
        unsafe {
            wellfriendpdf_error_free(error);
            wellfriendpdf_document_free(doc);
        }
    }
}
