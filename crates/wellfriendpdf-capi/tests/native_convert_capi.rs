//! C ABI for the Rust-native text/Markdown/CSV/JSON/SVG to PDF converters.
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use wellfriendpdf_capi::*;

type Convert = unsafe extern "C" fn(
    *const u8,
    usize,
    *const c_char,
    *mut WellfriendBuffer,
    *mut *mut c_char,
) -> c_int;

fn call(convert: Convert, input: &[u8], options: Option<&str>) -> Result<Vec<u8>, String> {
    let options = options.map(|text| CString::new(text).unwrap());
    let mut buffer = WellfriendBuffer {
        data: ptr::null_mut(),
        len: 0,
    };
    let mut error: *mut c_char = ptr::null_mut();
    let status = unsafe {
        convert(
            input.as_ptr(),
            input.len(),
            options.as_ref().map_or(ptr::null(), |text| text.as_ptr()),
            &mut buffer,
            &mut error,
        )
    };
    if status == WELLFRIENDPDF_STATUS_OK {
        let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) }.to_vec();
        unsafe { wellfriendpdf_buffer_free(buffer) };
        Ok(bytes)
    } else {
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { wellfriendpdf_string_free(error) };
        Err(message)
    }
}

fn assert_pdf(bytes: &[u8], text: &str) {
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    assert_eq!(engine.page_count().unwrap(), 1);
    if !text.is_empty() {
        assert!(engine.get_page_text(1).unwrap().contains(text));
    }
}

#[test]
fn every_native_converter_is_exposed() {
    let pdf = call(wellfriendpdf_text_to_pdf, b"c api text", None).unwrap();
    assert_pdf(&pdf, "c api text");
    let pdf = call(
        wellfriendpdf_markdown_to_pdf,
        b"# Heading\n\nbody",
        Some(r#"{"pageSize":"Letter"}"#),
    )
    .unwrap();
    assert_pdf(&pdf, "Heading");
    let pdf = call(wellfriendpdf_csv_to_pdf, b"x,y\nleft,right\n", None).unwrap();
    assert_pdf(&pdf, "right");
    let pdf = call(wellfriendpdf_json_to_pdf, br#"{"key":"value"}"#, None).unwrap();
    assert_pdf(&pdf, "value");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;
    let pdf = call(wellfriendpdf_svg_to_pdf, svg, Some(r#"{"page":"fit"}"#)).unwrap();
    assert_pdf(&pdf, "");
}

#[test]
fn errors_carry_converter_codes() {
    let error = call(
        wellfriendpdf_csv_to_pdf,
        b"a,b,c\n",
        Some(r#"{"max_columns":2}"#),
    )
    .unwrap_err();
    assert!(error.contains("csv_column_limit_exceeded"), "{error}");
    let error = call(
        wellfriendpdf_markdown_to_pdf,
        b"x",
        Some(r#"{"base_dir":"."}"#),
    )
    .unwrap_err();
    assert!(error.contains("invalid_options"), "{error}");
}
