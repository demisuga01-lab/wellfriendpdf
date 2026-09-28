//! Unexecuted ABI regression source. Do not substitute these for binding runs.
use super::*;
use serde_json::{json, Value};

fn fixture() -> Vec<u8> {
    use wellfriendpdf_engine::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
    let mut b = PdfBuilder::new();
    b.add_page(PageSize::custom(200.0, 200.0))
        .draw_text(
            "OLD",
            10.0,
            50.0,
            &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
        )
        .unwrap();
    b.to_bytes().unwrap()
}
unsafe fn call(session: *mut WellfriendStorySession, value: Value) -> Value {
    let operation = value["op"].as_str().unwrap_or("unknown").to_owned();
    let data = serde_json::to_vec(&value).unwrap();
    let mut out = WellfriendBuffer::empty();
    let mut error = ptr::null_mut();
    let status = unsafe {
        wellfriendpdf_story_session_command_json(
            session,
            data.as_ptr(),
            data.len(),
            ptr::null(),
            &mut out,
            &mut error,
        )
    };
    assert_eq!(
        status,
        0,
        "operation={operation}: {:?}",
        if error.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(error) })
        }
    );
    let value =
        serde_json::from_slice(unsafe { slice::from_raw_parts(out.data, out.len) }).unwrap();
    unsafe {
        wellfriendpdf_buffer_free(out);
    }
    value
}

#[test]
fn story_abi_owns_input_and_exports_checkpoint_with_exact_undo() {
    unsafe {
        let mut source = fixture();
        let expected = source.clone();
        let mut error = ptr::null_mut();
        let session = wellfriendpdf_story_session_open(
            source.as_ptr(),
            source.len(),
            ptr::null(),
            &mut error,
        );
        assert!(!session.is_null());
        source.fill(0);
        let status = call(session, json!({"op":"status"}));
        let request = json!({
            "story_id":"abi","input_sha256":status["revision_sha256"],
            "frames":[{"id":"body","page":1,"logical_range":[0,3],"expected_text":"OLD","rect":[10,10,190,180]}],
            "paragraphs":[{"id":"p","text":"Native replacement","preferred_font":"Approved","font_size":12,"line_height":14}],
            "fonts":[{"lookup_name":"Approved","bytes":wellfriendpdf_engine::render::get_fallback_font("Helvetica").unwrap()}]
        });
        let preview = call(session, json!({"op":"preview","request":request}));
        call(
            session,
            json!({"op":"checkpoint","request":request,"receipt":preview["receipt"]}),
        );
        let mut out = WellfriendBuffer::empty();
        assert_eq!(
            wellfriendpdf_story_session_bytes(session, &mut out, &mut error),
            0
        );
        let result = slice::from_raw_parts(out.data, out.len).to_vec();
        wellfriendpdf_buffer_free(out);
        assert!(ContentEngine::open_bytes(result)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("Native replacement"));
        assert_eq!(call(session, json!({"op":"undo"})), true);
        assert_eq!(
            wellfriendpdf_story_session_bytes(session, &mut out, &mut error),
            0
        );
        assert_eq!(slice::from_raw_parts(out.data, out.len), expected);
        wellfriendpdf_buffer_free(out);
        assert_eq!(call(session, json!({"op":"redo"})), true);
        wellfriendpdf_story_session_free(session);
    }
}

#[test]
fn story_abi_password_open_reports_a_decrypted_noncredentialed_working_copy() {
    unsafe {
        let source = fixture();
        let engine = ContentEngine::open_bytes(source).unwrap();
        let encrypted = wellfriendpdf_engine::utilities::encrypt_pdf(
            &engine,
            &wellfriendpdf_engine::EncryptParams {
                user_password: wellfriendpdf_engine::crypto::secret_bytes(b"reader".to_vec()),
                owner_password: wellfriendpdf_engine::crypto::secret_bytes(b"owner".to_vec()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut error = ptr::null_mut();
        let wrong = wellfriendpdf_story_session_open_with_password(
            encrypted.as_ptr(),
            encrypted.len(),
            b"wrong".as_ptr(),
            b"wrong".len(),
            ptr::null(),
            &mut error,
        );
        assert!(wrong.is_null());
        assert!(!error.is_null());
        wellfriendpdf_error_free(error);

        let password = b"owner";
        let session = wellfriendpdf_story_session_open_with_password(
            encrypted.as_ptr(),
            encrypted.len(),
            password.as_ptr(),
            password.len(),
            ptr::null(),
            &mut error,
        );
        assert!(!session.is_null());
        let status = call(session, json!({"op":"status"}));
        assert_eq!(status["source_security"]["source_was_encrypted"], true);
        assert_eq!(status["source_security"]["working_copy_decrypted"], true);
        assert_eq!(status["source_security"]["password_retained"], false);
        assert_eq!(status["source_security"]["authenticated_as_owner"], true);
        assert_ne!(
            status["source_security"]["source_input_sha256"],
            status["revision_sha256"]
        );
        wellfriendpdf_story_session_free(session);
    }
}

#[test]
fn story_abi_rejects_bad_outputs_lengths_cancellation_and_poisoned_state() {
    unsafe {
        let source = fixture();
        let mut error = ptr::null_mut();
        assert!(wellfriendpdf_story_session_open(
            ptr::dangling(),
            protocol::MAX_INPUT_BYTES + 1,
            ptr::null(),
            &mut error
        )
        .is_null());
        wellfriendpdf_error_free(error);
        let session = wellfriendpdf_story_session_open(
            source.as_ptr(),
            source.len(),
            ptr::null(),
            &mut error,
        );
        assert!(!session.is_null());
        let json = b"{\"op\":\"undo\"}";
        assert_eq!(
            wellfriendpdf_story_session_command_json(
                session,
                json.as_ptr(),
                json.len(),
                ptr::null(),
                ptr::null_mut(),
                &mut error
            ),
            WELLFRIENDPDF_STATUS_NULL
        );
        wellfriendpdf_error_free(error);
        let mut out = WellfriendBuffer::empty();
        assert_eq!(
            wellfriendpdf_story_session_command_json(
                session,
                ptr::dangling(),
                protocol::MAX_COMMAND_BYTES + 1,
                ptr::null(),
                &mut out,
                &mut error
            ),
            WELLFRIENDPDF_STATUS_ERROR
        );
        assert!(out.data.is_null());
        wellfriendpdf_error_free(error);
        let cancellation = wellfriendpdf_render_cancellation_new(&mut error);
        assert_eq!(
            wellfriendpdf_render_cancellation_cancel(cancellation, &mut error),
            0
        );
        assert_eq!(
            wellfriendpdf_story_session_command_json(
                session,
                json.as_ptr(),
                json.len(),
                cancellation,
                &mut out,
                &mut error
            ),
            WELLFRIENDPDF_STATUS_ERROR
        );
        assert_eq!(out.len, 0);
        wellfriendpdf_error_free(error);
        wellfriendpdf_render_cancellation_free(cancellation);
        assert_eq!(
            call(session, serde_json::json!({"op":"status"}))["history"]["undo_steps"],
            0
        );
        assert_eq!(
            output(session, &mut out, &mut error, |_| panic!(
                "synthetic ABI panic"
            )),
            WELLFRIENDPDF_STATUS_PANIC
        );
        wellfriendpdf_error_free(error);
        assert_eq!(
            wellfriendpdf_story_session_bytes(session, &mut out, &mut error),
            WELLFRIENDPDF_STATUS_ERROR
        );
        assert!(out.data.is_null());
        wellfriendpdf_error_free(error);
        wellfriendpdf_story_session_free(session);
        wellfriendpdf_story_session_free(ptr::null_mut());
    }
}
