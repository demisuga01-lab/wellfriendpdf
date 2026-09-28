//! Caller-owned retained editor. C callers serialize all session access/free;
//! a separate live cancellation handle may be signalled from another thread.
use super::*;
use wellfriendpdf_engine::{
    linked_stories::LinkedStorySession, story_session_protocol as protocol,
};

#[cfg(test)]
#[path = "story_session_tests.rs"]
mod tests;

#[repr(C)]
pub struct WellfriendStorySession {
    // A caught panic poisons this handle; only free is allowed afterward.
    inner: Option<LinkedStorySession>,
}

fn token(cancellation: *const WellfriendRenderCancellation) -> Result<CancelToken, String> {
    if cancellation.is_null() {
        Ok(CancelToken::none())
    } else {
        Ok(checked_render_cancellation(cancellation)?.token.clone())
    }
}

/// Opens a retained session from a copied PDF buffer (1..=256 MiB).
/// # Safety
/// `data` must name `len` readable bytes. Optional cancellation/error pointers
/// must remain live for the call. Free the result exactly once with session_free.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_open(
    data: *const u8,
    len: usize,
    cancellation: *const WellfriendRenderCancellation,
    error_out: *mut *mut c_char,
) -> *mut WellfriendStorySession {
    unsafe { story_session_open_impl(data, len, ptr::null(), 0, cancellation, error_out) }
}

/// Opens a retained session from an optionally password-protected PDF. An
/// encrypted input requires its permissions/owner password.
/// Encrypted input is unlocked once and converted into the explicitly reported
/// unencrypted working revision used by all request and receipt hashes.
///
/// # Safety
/// `data`/`password` must name their readable lengths (a zero password length
/// permits a null password pointer). Other ownership rules match session_open.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_open_with_password(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    cancellation: *const WellfriendRenderCancellation,
    error_out: *mut *mut c_char,
) -> *mut WellfriendStorySession {
    unsafe { story_session_open_impl(data, len, password, password_len, cancellation, error_out) }
}

unsafe fn story_session_open_impl(
    data: *const u8,
    len: usize,
    password: *const u8,
    password_len: usize,
    cancellation: *const WellfriendRenderCancellation,
    error_out: *mut *mut c_char,
) -> *mut WellfriendStorySession {
    // Serialized .NET/C hosts may transfer ownership between threads. Keep
    // this property checked by the compiler when native builds are authorized.
    fn require_send<T: Send>() {}
    require_send::<LinkedStorySession>();
    clear_error(error_out);
    if data.is_null() || len == 0 || len > protocol::MAX_INPUT_BYTES {
        set_error(
            error_out,
            "story input must name 1..=256 MiB readable bytes",
        );
        return ptr::null_mut();
    }
    if password_len > 0 && password.is_null() {
        set_error(error_out, "story password pointer is null");
        return ptr::null_mut();
    }
    match catch_unwind(AssertUnwindSafe(|| {
        let cancel = token(cancellation)?;
        wellfriendpdf(protocol::open_with_password(
            unsafe { slice::from_raw_parts(data, len) },
            if password_len == 0 {
                &[]
            } else {
                unsafe { slice::from_raw_parts(password, password_len) }
            },
            &cancel,
        ))
    })) {
        Ok(Ok(inner)) => Box::into_raw(Box::new(WellfriendStorySession { inner: Some(inner) })),
        Ok(Err(error)) => {
            set_error(error_out, &error);
            ptr::null_mut()
        }
        Err(_) => {
            set_error(error_out, "panic opening story session");
            ptr::null_mut()
        }
    }
}

// Validate destinations before any mutation. Every failure leaves an empty
// buffer. No cancellation check is added after a successful mutation.
unsafe fn output(
    session: *mut WellfriendStorySession,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
    work: impl FnOnce(&mut LinkedStorySession) -> Result<Vec<u8>, String>,
) -> c_int {
    clear_error(error_out);
    if !out.is_null() {
        unsafe {
            *out = WellfriendBuffer::empty();
        }
    }
    if session.is_null() || out.is_null() {
        set_error(error_out, "story session and output pointers are required");
        return WELLFRIENDPDF_STATUS_NULL;
    }
    let handle = unsafe { &mut *session };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let inner = handle
            .inner
            .as_mut()
            .ok_or("story session is poisoned; close it and reopen")?;
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
            set_error(error_out, "panic in story session; handle is poisoned");
            WELLFRIENDPDF_STATUS_PANIC
        }
    }
}

/// Dispatches a v1 UTF-8 JSON command. JSON output is length-delimited, not NUL
/// terminated. Free it with wellfriendpdf_buffer_free; errors with error_free.
/// # Safety
/// Session access/free must be externally serialized. `json` must name len
/// readable bytes (<=32 MiB). All non-null pointers must remain live for the call.
/// Output/error slots must not overlap each other, input bytes or any handle.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_command_json(
    session: *mut WellfriendStorySession,
    json: *const u8,
    len: usize,
    cancellation: *const WellfriendRenderCancellation,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        output(session, out, error_out, |inner| {
            if json.is_null() || len == 0 || len > protocol::MAX_COMMAND_BYTES {
                return Err("story command must name 1..=32 MiB UTF-8 JSON bytes".into());
            }
            let cancel = token(cancellation)?;
            wellfriendpdf(protocol::execute_json(
                inner,
                slice::from_raw_parts(json, len),
                &cancel,
            ))
        })
    }
}

/// Copies the current PDF revision; the session retains its own bytes.
/// # Safety
/// Requires live, exclusively accessed session and writable output/error pointers.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_bytes(
    session: *mut WellfriendStorySession,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe { output(session, out, error_out, |inner| Ok(inner.bytes().to_vec())) }
}

/// Explicit canonical preview rendering, bounded to 300 DPI / 16 million pixels.
/// # Safety
/// Same live-pointer and exclusive-session requirements as command_json.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_render_page_png(
    session: *mut WellfriendStorySession,
    page: usize,
    dpi: u32,
    cancellation: *const WellfriendRenderCancellation,
    out: *mut WellfriendBuffer,
    error_out: *mut *mut c_char,
) -> c_int {
    unsafe {
        output(session, out, error_out, |inner| {
            wellfriendpdf(protocol::render_page_png(
                inner,
                page,
                dpi,
                &token(cancellation)?,
            ))
        })
    }
}

/// # Safety
/// Pass NULL or a live session exactly once, with no concurrent session calls.
#[no_mangle]
pub unsafe extern "C" fn wellfriendpdf_story_session_free(session: *mut WellfriendStorySession) {
    if !session.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            drop(Box::from_raw(session));
        }));
    }
}
