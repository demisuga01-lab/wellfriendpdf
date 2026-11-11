//! Interactive XFA form sessions (`XfaFormSession`): JSON commands in,
//! JSON / byte buffers out. C callers serialize all access to a session
//! handle (it is not thread-safe) and free it exactly once.
use super::*;
use wellfriendpdf_engine::{XfaFormSession, XfaSessionOptions};

#[cfg(test)]
#[path = "xfa_session_tests.rs"]
mod tests;

/// Largest JSON command accepted (bytes).
const MAX_XFA_COMMAND_BYTES: usize = 32 * 1024 * 1024;

#[repr(C)]
pub struct WellfriendXfaSession {
    // A caught panic poisons this handle; only free is allowed afterward.
    inner: Option<XfaFormSession>,
}

/// Opens an interactive XFA session over a document. `options_json` may be
/// NULL or `{"run_scripts":bool}`. The session keeps its own copy of the
/// document; the document handle may be freed afterwards.
///
/// # Safety
/// `document` must be a live handle; `options_json` NULL or NUL-terminated
/// UTF-8; `error_out` NULL or writable. Free the result with
/// `wellfriendpdf_xfa_session_free`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_open(
    document: *const WellfriendDocument,
    options_json: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut WellfriendXfaSession {
    clear_error(error_out);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let doc = checked_doc(document)?;
        let options = match unsafe { optional_c_string(options_json) }? {
            Some(json) => serde_json::from_str::<XfaSessionOptions>(&json)
                .map_err(|err| format!("XFA session options JSON: {err}"))?,
            None => XfaSessionOptions::default(),
        };
        wellfriendpdf(doc.engine.xfa_session_with_options(&options))
    }));
    match result {
        Ok(Ok(inner)) => Box::into_raw(Box::new(WellfriendXfaSession { inner: Some(inner) })),
        Ok(Err(error)) => {
            set_error(error_out, &error);
            ptr::null_mut()
        }
        Err(_) => {
            set_error(error_out, "panic opening XFA session");
            ptr::null_mut()
        }
    }
}

unsafe fn xfa_output(
    session: *mut WellfriendXfaSession,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
    work: impl FnOnce(&mut XfaFormSession) -> Result<Vec<u8>, String>,
) -> c_int {
    clear_error(error_out);
    if !out.is_null() {
        unsafe {
            *out = WellfriendBuffer::empty();
        }
    }
    if session.is_null() || out.is_null() {
        set_error(error_out, "XFA session and output pointers are required");
        return WELLFRIENDPDF_STATUS_NULL;
    }
    let handle = unsafe { &mut *session };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let inner = handle
            .inner
            .as_mut()
            .ok_or("XFA session is poisoned; free it and reopen")?;
        work(inner)
    }));
    match result {
        Ok(Ok(bytes)) => {
            unsafe {
                *out = into_buffer(bytes);
            }
            WELLFRIENDPDF_STATUS_OK
        }
        Ok(Err(error)) => {
            set_error(error_out, &error);
            WELLFRIENDPDF_STATUS_ERROR
        }
        Err(_) => {
            handle.inner = None;
            set_error(error_out, "panic in XFA session; handle is poisoned");
            WELLFRIENDPDF_STATUS_PANIC
        }
    }
}

/// Runs one JSON command (`fields`, `value`, `set_value`, `click`,
/// `dispatch`, `validations`, `script_log`, `export_data_xml`,
/// `page_count`, `form_type`; see `XfaFormSession::command_json`). The
/// UTF-8 JSON result is length-delimited (not NUL-terminated); free it
/// with `wellfriendpdf_buffer_free`.
///
/// # Safety
/// `session` must be live and exclusively accessed; `json` must name `len`
/// readable bytes; `out`/`error_out` must be writable and not overlap.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_command_json(
    session: *mut WellfriendXfaSession,
    json: *const u8,
    len: usize,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        xfa_output(session, out, error_out, |inner| {
            if json.is_null() || len == 0 || len > MAX_XFA_COMMAND_BYTES {
                return Err("XFA session command must name 1..=32 MiB of UTF-8 JSON".into());
            }
            let command = std::str::from_utf8(slice::from_raw_parts(json, len))
                .map_err(|err| format!("XFA session command is not UTF-8: {err}"))?;
            wellfriendpdf(inner.command_json(command)).map(String::into_bytes)
        })
    }
}

/// Renders XFA page `page` (1-based) of the session's current state as PNG.
///
/// # Safety
/// Same pointer and exclusivity rules as `wellfriendpdf_xfa_session_command_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_render_page_png(
    session: *mut WellfriendXfaSession,
    page: usize,
    dpi: u32,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        xfa_output(session, out, error_out, |inner| {
            if dpi == 0 || dpi > 1200 {
                return Err("dpi must be 1..=1200".into());
            }
            wellfriendpdf(inner.render_page_png(page, dpi))
        })
    }
}

/// Saves the session's data into the PDF (`datasets` packet, incremental
/// update after `preSave`, then `postSave`).
///
/// # Safety
/// Same pointer and exclusivity rules as `wellfriendpdf_xfa_session_command_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_save_pdf(
    session: *mut WellfriendXfaSession,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        xfa_output(session, out, error_out, |inner| {
            wellfriendpdf(inner.save_to_pdf())
        })
    }
}

/// The current XFA layout as a standalone PDF.
///
/// # Safety
/// Same pointer and exclusivity rules as `wellfriendpdf_xfa_session_command_json`.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_layout_pdf(
    session: *mut WellfriendXfaSession,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        xfa_output(session, out, error_out, |inner| {
            wellfriendpdf(inner.layout_pdf())
        })
    }
}

/// # Safety
/// Pass NULL or a live session exactly once, with no concurrent calls.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_xfa_session_free(session: *mut WellfriendXfaSession) {
    if !session.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(session));
        }));
    }
}
