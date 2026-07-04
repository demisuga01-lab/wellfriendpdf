//! Bounded AcroForm field-data exchange.
//!
//! This module intentionally implements the high-value, safe subset used by
//! form automation: field names and scalar values. It does not execute actions,
//! JavaScript, or import annotation XFDF payloads.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::engine::ContentEngine;
use crate::error::{OxideError, Result};
use crate::info::decode_pdf_text_string;
use crate::{forms_report, EditMode, PdfEditor};

const MAX_FORM_DATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_FORM_FIELDS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormDataFormat {
    Json,
    Fdf,
    Xfdf,
}

impl FormDataFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "fdf" => Some(Self::Fdf),
            "xfdf" => Some(Self::Xfdf),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Fdf => "fdf",
            Self::Xfdf => "xfdf",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDataField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDataSet {
    pub fields: Vec<FormDataField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDataApplyReport {
    pub imported_fields: usize,
    pub applied_fields: usize,
    pub unknown_fields: Vec<String>,
    pub output_bytes: usize,
    pub diagnostics: Vec<String>,
}

pub fn export_form_data(engine: &ContentEngine, format: FormDataFormat) -> Result<Vec<u8>> {
    let report = forms_report(engine)?;
    let fields = report
        .fields
        .iter()
        .filter(|field| !field.is_signature)
        .map(|field| FormDataField {
            name: field.full_name.clone(),
            value: field.value.clone().unwrap_or_default(),
        })
        .collect();
    let data = FormDataSet { fields };
    encode_form_data(&data, format)
}

pub fn parse_form_data(bytes: &[u8], format: FormDataFormat) -> Result<FormDataSet> {
    if bytes.len() > MAX_FORM_DATA_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "form data input is {} bytes, exceeding cap {MAX_FORM_DATA_BYTES}",
            bytes.len()
        )));
    }
    let data = match format {
        FormDataFormat::Json => parse_json_form_data(bytes)?,
        FormDataFormat::Fdf => parse_fdf_form_data(bytes)?,
        FormDataFormat::Xfdf => parse_xfdf_form_data(bytes)?,
    };
    if data.fields.len() > MAX_FORM_FIELDS {
        return Err(OxideError::ResourceLimit(format!(
            "form data has {} fields, exceeding cap {MAX_FORM_FIELDS}",
            data.fields.len()
        )));
    }
    Ok(data)
}

pub fn apply_form_data_pdf(
    input: Vec<u8>,
    form_data: &[u8],
    format: FormDataFormat,
) -> Result<(Vec<u8>, FormDataApplyReport)> {
    let data = parse_form_data(form_data, format)?;
    let engine = ContentEngine::open_bytes(input.clone())?;
    let form_report = forms_report(&engine)?;
    let mut field_types = BTreeMap::new();
    for field in &form_report.fields {
        field_types.insert(field.full_name.clone(), field.field_type.clone());
    }

    let mut editor = PdfEditor::open_bytes(input)?;
    let mut unknown = Vec::new();
    let mut applied = 0usize;
    let mut seen = BTreeSet::new();
    for field in &data.fields {
        if !seen.insert(field.name.clone()) {
            continue;
        }
        let Some(kind) = field_types.get(&field.name) else {
            unknown.push(field.name.clone());
            continue;
        };
        match kind.as_str() {
            "checkbox" | "radio" => {
                editor.set_form_checkbox(&field.name, parse_bool_value(&field.value));
            }
            "choice" => {
                editor.set_form_choice(&field.name, &field.value);
            }
            "signature" => {
                unknown.push(field.name.clone());
                continue;
            }
            _ => {
                editor.set_form_text(&field.name, &field.value);
            }
        }
        applied += 1;
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    let report = FormDataApplyReport {
        imported_fields: data.fields.len(),
        applied_fields: applied,
        unknown_fields: unknown,
        output_bytes: output.len(),
        diagnostics: vec![
            "forms.exchange.fields_only".to_string(),
            "forms.exchange.actions_not_executed".to_string(),
        ],
    };
    Ok((output, report))
}

fn encode_form_data(data: &FormDataSet, format: FormDataFormat) -> Result<Vec<u8>> {
    match format {
        FormDataFormat::Json => serde_json::to_vec_pretty(data).map_err(json_error),
        FormDataFormat::Fdf => Ok(write_fdf(data)),
        FormDataFormat::Xfdf => Ok(write_xfdf(data).into_bytes()),
    }
}

fn write_fdf(data: &FormDataSet) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"%FDF-1.2\n1 0 obj\n<< /FDF << /Fields [\n");
    for field in &data.fields {
        out.extend_from_slice(b"<< /T ");
        write_pdf_hex_text(&field.name, &mut out);
        out.extend_from_slice(b" /V ");
        write_pdf_hex_text(&field.value, &mut out);
        out.extend_from_slice(b" >>\n");
    }
    out.extend_from_slice(b"] >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
    out
}

fn write_xfdf(data: &FormDataSet) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n\
         <fields>\n",
    );
    for field in &data.fields {
        out.push_str("  <field name=\"");
        out.push_str(&xml_escape(&field.name));
        out.push_str("\"><value>");
        out.push_str(&xml_escape(&field.value));
        out.push_str("</value></field>\n");
    }
    out.push_str("</fields>\n</xfdf>\n");
    out
}

fn parse_json_form_data(bytes: &[u8]) -> Result<FormDataSet> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(json_error)?;
    if value.get("fields").is_some() {
        return serde_json::from_value(value).map_err(json_error);
    }
    let Some(map) = value.as_object() else {
        return Err(OxideError::MalformedPdf(
            "JSON form data must be an object or {\"fields\":[...]}".to_string(),
        ));
    };
    let fields = map
        .iter()
        .map(|(name, value)| FormDataField {
            name: name.clone(),
            value: value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()),
        })
        .collect();
    Ok(FormDataSet { fields })
}

fn json_error(error: serde_json::Error) -> OxideError {
    OxideError::MalformedPdf(format!("invalid JSON form data: {error}"))
}

fn parse_fdf_form_data(bytes: &[u8]) -> Result<FormDataSet> {
    let mut fields = Vec::new();
    let mut pos = 0usize;
    while let Some(t_pos) = find_token(bytes, b"/T", pos) {
        let (name, after_name) = parse_pdf_scalar(bytes, t_pos + 2)?;
        let next_t = find_token(bytes, b"/T", after_name).unwrap_or(bytes.len());
        if let Some(v_pos) = find_token(&bytes[..next_t], b"/V", after_name) {
            let (value, after_value) = parse_pdf_scalar(bytes, v_pos + 2)?;
            fields.push(FormDataField { name, value });
            pos = after_value;
        } else {
            pos = after_name;
        }
    }
    Ok(FormDataSet { fields })
}

fn parse_xfdf_form_data(bytes: &[u8]) -> Result<FormDataSet> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| OxideError::MalformedPdf(format!("XFDF is not UTF-8: {err}")))?;
    let lower = text.to_ascii_lowercase();
    if lower.contains("<!doctype")
        || lower.contains("<!entity")
        || lower.contains("system")
        || lower.contains("public")
    {
        return Err(OxideError::UnsupportedFeature(
            "XFDF external entities and DTDs are not supported".to_string(),
        ));
    }
    let mut fields = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = text[pos..].find("<field") {
        let start = pos + rel;
        let tag_end = text[start..]
            .find('>')
            .map(|i| start + i)
            .ok_or_else(|| OxideError::MalformedPdf("XFDF field tag is not closed".to_string()))?;
        let tag = &text[start..=tag_end];
        let Some(name) = attr_value(tag, "name") else {
            pos = tag_end + 1;
            continue;
        };
        let close = text[tag_end + 1..]
            .find("</field>")
            .map(|i| tag_end + 1 + i)
            .ok_or_else(|| OxideError::MalformedPdf("XFDF field is not closed".to_string()))?;
        let body = &text[tag_end + 1..close];
        let value = if let Some(vs) = body.find("<value>") {
            let content_start = vs + "<value>".len();
            let ve = body[content_start..]
                .find("</value>")
                .map(|i| content_start + i)
                .unwrap_or(body.len());
            xml_unescape(&body[content_start..ve])
        } else {
            String::new()
        };
        fields.push(FormDataField {
            name: xml_unescape(&name),
            value,
        });
        pos = close + "</field>".len();
    }
    Ok(FormDataSet { fields })
}

fn write_pdf_hex_text(text: &str, out: &mut Vec<u8>) {
    out.push(b'<');
    out.extend_from_slice(b"FEFF");
    for code in text.encode_utf16() {
        out.extend_from_slice(format!("{code:04X}").as_bytes());
    }
    out.push(b'>');
}

fn find_token(haystack: &[u8], token: &[u8], start: usize) -> Option<usize> {
    haystack
        .get(start..)?
        .windows(token.len())
        .position(|window| window == token)
        .map(|pos| pos + start)
}

fn parse_pdf_scalar(bytes: &[u8], start: usize) -> Result<(String, usize)> {
    let mut pos = skip_ws(bytes, start);
    match bytes.get(pos).copied() {
        Some(b'(') => parse_literal_string(bytes, pos),
        Some(b'<') if bytes.get(pos + 1) != Some(&b'<') => parse_hex_string(bytes, pos),
        Some(b'/') => {
            pos += 1;
            let begin = pos;
            while pos < bytes.len() && !is_pdf_delim(bytes[pos]) {
                pos += 1;
            }
            Ok((String::from_utf8_lossy(&bytes[begin..pos]).to_string(), pos))
        }
        _ => Err(OxideError::MalformedPdf(
            "FDF scalar value must be a PDF string or name".to_string(),
        )),
    }
}

fn parse_literal_string(bytes: &[u8], start: usize) -> Result<(String, usize)> {
    let mut out = Vec::new();
    let mut depth = 1i32;
    let mut pos = start + 1;
    while pos < bytes.len() {
        let byte = bytes[pos];
        pos += 1;
        match byte {
            b'\\' => {
                if let Some(next) = bytes.get(pos).copied() {
                    pos += 1;
                    out.push(match next {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        other => other,
                    });
                }
            }
            b'(' => {
                depth += 1;
                out.push(byte);
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((decode_pdf_text_string(&out), pos));
                }
                out.push(byte);
            }
            other => out.push(other),
        }
    }
    Err(OxideError::MalformedPdf(
        "unterminated FDF literal string".to_string(),
    ))
}

fn parse_hex_string(bytes: &[u8], start: usize) -> Result<(String, usize)> {
    let mut hex = Vec::new();
    let mut pos = start + 1;
    while pos < bytes.len() {
        let byte = bytes[pos];
        pos += 1;
        if byte == b'>' {
            if hex.len() % 2 == 1 {
                hex.push(b'0');
            }
            let mut out = Vec::with_capacity(hex.len() / 2);
            for pair in hex.chunks_exact(2) {
                let hi = hex_value(pair[0]).ok_or_else(|| {
                    OxideError::MalformedPdf("invalid FDF hex string".to_string())
                })?;
                let lo = hex_value(pair[1]).ok_or_else(|| {
                    OxideError::MalformedPdf("invalid FDF hex string".to_string())
                })?;
                out.push((hi << 4) | lo);
            }
            return Ok((decode_pdf_text_string(&out), pos));
        }
        if !byte.is_ascii_whitespace() {
            hex.push(byte);
        }
    }
    Err(OxideError::MalformedPdf(
        "unterminated FDF hex string".to_string(),
    ))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn is_pdf_delim(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'/' | b'<' | b'>' | b'[' | b']' | b'(' | b')')
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some(start) = tag.find(&needle) {
            let value_start = start + needle.len();
            let end = tag[value_start..].find(quote)? + value_start;
            return Some(tag[value_start..end].to_string());
        }
    }
    None
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_bool_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "on" | "true" | "1" | "checked"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fdf_roundtrips_unicode_fields() {
        let data = FormDataSet {
            fields: vec![FormDataField {
                name: "parent.child".to_string(),
                value: "Alice ✓".to_string(),
            }],
        };
        let fdf = encode_form_data(&data, FormDataFormat::Fdf).unwrap();
        let parsed = parse_form_data(&fdf, FormDataFormat::Fdf).unwrap();
        assert_eq!(parsed, data);
    }

    #[test]
    fn xfdf_rejects_external_entities() {
        let err = parse_form_data(
            br#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY e SYSTEM "file:///x">]>"#,
            FormDataFormat::Xfdf,
        )
        .unwrap_err();
        assert!(err.to_string().contains("external entities"));
    }
}
