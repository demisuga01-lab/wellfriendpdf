use super::*;
use serde_json::Value;

const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form1" layout="tb">
  <pageSet><pageArea name="P"><contentArea w="200pt" h="200pt"/><medium short="200pt" long="200pt"/></pageArea></pageSet>
  <field name="qty" w="160pt" h="16pt"><ui><numericEdit/></ui><value><decimal/></value>
    <event activity="exit"><script>xfa.host.messageBox("exit")</script></event></field>
  <field name="total" w="160pt" h="16pt"><ui><numericEdit/></ui><value><decimal/></value>
    <calculate><script>qty * 2</script></calculate></field>
  <field name="agree" w="20pt" h="20pt"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items></field>
</subform></template>"#;

/// A one-page PDF with an AcroForm /XFA packet array.
fn fixture() -> Vec<u8> {
    let config = r#"<config xmlns="http://www.xfa.org/schema/xci/3.0/"><acrobat><acrobat7><dynamicRender>required</dynamicRender></acrobat7></acrobat></config>"#;
    let data = r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data/></xfa:datasets>"#;
    let stream = |text: &str| format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len());
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Fields [] /XFA [(config) 5 0 R (template) 6 0 R (datasets) 7 0 R] >>".to_string(),
        stream(config),
        stream(TEMPLATE),
        stream(data),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

unsafe fn take_buffer(out: WellfriendBuffer) -> Vec<u8> {
    let bytes = unsafe { slice::from_raw_parts(out.data, out.len) }.to_vec();
    unsafe { wellfriendpdf_buffer_free(out) };
    bytes
}

unsafe fn command(session: *mut WellfriendXfaSession, json: &str) -> Result<Value, String> {
    let mut out = WellfriendBuffer::empty();
    let mut error = ptr::null_mut();
    let status = unsafe {
        wellfriendpdf_xfa_session_command_json(
            session,
            json.as_ptr(),
            json.len(),
            &mut out,
            &mut error,
        )
    };
    if status != WELLFRIENDPDF_STATUS_OK {
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { wellfriendpdf_error_free(error) };
        return Err(message);
    }
    Ok(serde_json::from_slice(&unsafe { take_buffer(out) }).unwrap())
}

#[test]
fn xfa_session_abi_drives_events_renders_and_saves() {
    unsafe {
        let pdf = fixture();
        let mut error = ptr::null_mut();
        let document = wellfriendpdf_document_open_from_bytes(pdf.as_ptr(), pdf.len(), &mut error);
        assert!(!document.is_null());
        let session = wellfriendpdf_xfa_session_open(document, ptr::null(), &mut error);
        assert!(
            !session.is_null(),
            "{:?}",
            (!error.is_null()).then(|| CStr::from_ptr(error))
        );
        // The session owns its copy of the document.
        wellfriendpdf_document_free(document);

        let outcome = command(
            session,
            r#"{"op":"set_value","som":"form1.qty","value":"3"}"#,
        )
        .unwrap();
        assert_eq!(outcome["committed"], true);
        assert_eq!(outcome["messages"][0], "exit");
        let total = command(session, r#"{"op":"value","som":"form1.total"}"#).unwrap();
        assert_eq!(total["value"], "6");
        command(session, r#"{"op":"click","som":"form1.agree"}"#).unwrap();
        let fields = command(session, r#"{"op":"fields"}"#).unwrap();
        assert_eq!(fields["fields"][2]["checked"], true);
        assert!(command(session, r#"{"op":"click","som":"form1.nonesuch"}"#).is_err());

        let mut out = WellfriendBuffer::empty();
        assert_eq!(
            wellfriendpdf_xfa_session_render_page_png(session, 1, 36, &mut out, &mut error),
            WELLFRIENDPDF_STATUS_OK
        );
        assert!(take_buffer(out).starts_with(b"\x89PNG"));
        let mut out = WellfriendBuffer::empty();
        assert_eq!(
            wellfriendpdf_xfa_session_save_pdf(session, &mut out, &mut error),
            WELLFRIENDPDF_STATUS_OK
        );
        let saved = take_buffer(out);
        assert!(saved.starts_with(&pdf));
        let reopened = ContentEngine::open_bytes(saved).unwrap();
        let mut again = reopened.xfa_session().unwrap();
        assert_eq!(again.field_value("form1.qty").unwrap(), Some("3".into()));

        // Null pointers fail cleanly.
        let mut out = WellfriendBuffer::empty();
        assert_eq!(
            wellfriendpdf_xfa_session_layout_pdf(ptr::null_mut(), &mut out, &mut error),
            WELLFRIENDPDF_STATUS_NULL
        );
        wellfriendpdf_error_free(error);
        wellfriendpdf_xfa_session_free(session);
    }
}
