//! ISO/TS 32004 PDF-MAC discovery and conservative verification posture.
//!
//! This module intentionally does not claim document-integrity validity from
//! structure alone. It parses the trailer `/AuthCode` shape, associated
//! encryption-dictionary inputs, and the CMS AuthenticatedData envelope. The
//! supported standalone profile verifies the patched ByteRange digest plus the
//! CMS PasswordRecipientInfo/HKDF/AES-KW/HMAC chain end to end when the file
//! key is recoverable.

use aes_kw::{KeyInit as AesKwKeyInit, KwAes256};
use cms::authenticated_data::AuthenticatedData;
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{EncryptedKey, PasswordRecipientInfo, RecipientInfo, RecipientInfos};
use cms::signed_data::EncapsulatedContentInfo;
use const_oid::ObjectIdentifier;
use der::asn1::{Any, AnyRef, OctetString, SetOfVec};
use der::{Decode, Encode, Sequence};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use spki::AlgorithmIdentifierOwned;
use subtle::ConstantTimeEq;
use x509_cert::attr::{Attribute, Attributes};
use zeroize::Zeroize;

use crate::crypto::{secret_bytes, SecretBytes};
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::writer::PdfWriter;

const OID_AUTHENTICATED_DATA: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.2");
const OID_PDF_MAC_INTEGRITY_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.0.32004.1.0");
const OID_PDF_MAC_WRAP_KDF: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.0.32004.1.1");
const OID_AES256_WRAP: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.45");
const PDF_MAC_KDF_SALT_LEN: usize = 32;
const PDF_MAC_INFO: &[u8] = b"PDFMAC";
const PDF_MAC_KEY_LEN: usize = 32;
const PDF_MAC_SHA256_LEN: usize = 32;
const PDF_MAC_BYTERANGE_ITERATIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfMacState {
    Absent,
    PresentUnverified,
    Valid,
    Invalid,
    Malformed,
    UnsupportedAlgorithm,
    KeyUnavailable,
    DuplicateStructure,
    ByteRangeInvalid,
    AuthenticationFailed,
    RevisionAfterMac,
    IntegrityScopeUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct PdfMacIntegrityInfo {
    version: u8,
    data_digest: OctetString,
    #[asn1(context_specific = "0", tag_mode = "IMPLICIT", optional = "true")]
    signature_digest: Option<OctetString>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfMacReport {
    pub schema_version: &'static str,
    pub source: &'static str,
    pub state: PdfMacState,
    pub detected: bool,
    pub required_by_permissions: Option<bool>,
    pub permission_source: String,
    pub auth_code_direct: bool,
    pub mac_location: Option<String>,
    pub has_mac: bool,
    pub mac_length: Option<usize>,
    pub has_byte_range: bool,
    pub byte_range: Option<Vec<i64>>,
    pub has_sig_obj_ref: bool,
    pub sig_obj_ref: Option<String>,
    pub encrypt_v: Option<i64>,
    pub kdf_salt_status: String,
    pub cms_content_type: Option<String>,
    pub cms_authenticated_data: bool,
    pub cms_pdf_mac_content_type: bool,
    pub cms_recipient_profile: String,
    pub generation_supported: bool,
    pub verification_supported: bool,
    pub verification_performed: bool,
    pub trusted_document_integrity: bool,
    pub secret_material_reported: bool,
    pub exact_limit: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfMacWriteReport {
    pub schema_version: &'static str,
    pub source: &'static str,
    pub status: &'static str,
    pub mac_location: &'static str,
    pub byte_range: Vec<i64>,
    pub placeholder_token_bytes: usize,
    pub final_token_bytes: usize,
    pub covered_byte_count: usize,
    pub covered_sha256: String,
    pub output_bytes: usize,
    pub verification_state: PdfMacState,
    pub placeholder_iterations: usize,
    pub secret_material_reported: bool,
    pub diagnostics: Vec<String>,
}

impl PdfMacReport {
    fn base() -> Self {
        Self {
            schema_version: "crypto_writer_closeout.pdf_mac.v1",
            source: "ISO/TS 32004:2024",
            state: PdfMacState::Absent,
            detected: false,
            required_by_permissions: None,
            permission_source: "not_available".to_string(),
            auth_code_direct: false,
            mac_location: None,
            has_mac: false,
            mac_length: None,
            has_byte_range: false,
            byte_range: None,
            has_sig_obj_ref: false,
            sig_obj_ref: None,
            encrypt_v: None,
            kdf_salt_status: "encrypt_dictionary_absent".to_string(),
            cms_content_type: None,
            cms_authenticated_data: false,
            cms_pdf_mac_content_type: false,
            cms_recipient_profile: "not_parsed".to_string(),
            generation_supported: true,
            verification_supported: true,
            verification_performed: false,
            trusted_document_integrity: false,
            secret_material_reported: false,
            exact_limit: "Standalone PDF-MAC generation and verification are implemented for the mapped AESV4 PasswordRecipientInfo/pdfMacWrapKdf/AES-256-KW/HMAC-SHA256/SHA-256 profile when the document file key is available; AttachedToSig extraction and non-SHA256 profiles remain unsupported exact. PFX providers are supported by the PubSec identity loader on non-WASM builds, not by PDF-MAC token structure.".to_string(),
            diagnostics: Vec::new(),
        }
    }

    fn malformed(&mut self, diagnostic: impl Into<String>) {
        self.state = PdfMacState::Malformed;
        self.diagnostics.push(diagnostic.into());
    }

    fn unsupported(&mut self, diagnostic: impl Into<String>) {
        if self.state != PdfMacState::Malformed {
            self.state = PdfMacState::UnsupportedAlgorithm;
        }
        self.diagnostics.push(diagnostic.into());
    }

    fn invalid(&mut self, diagnostic: impl Into<String>) {
        if !matches!(
            self.state,
            PdfMacState::Malformed | PdfMacState::UnsupportedAlgorithm
        ) {
            self.state = PdfMacState::Invalid;
        }
        self.diagnostics.push(diagnostic.into());
    }

    fn authentication_failed(&mut self, diagnostic: impl Into<String>) {
        if !matches!(
            self.state,
            PdfMacState::Malformed | PdfMacState::UnsupportedAlgorithm
        ) {
            self.state = PdfMacState::AuthenticationFailed;
        }
        self.diagnostics.push(diagnostic.into());
    }
}

pub fn pdf_mac_report(reader: &PdfReader) -> PdfMacReport {
    let mut report = PdfMacReport::base();
    let encrypt_dict = reader.encrypt_dictionary();
    inspect_encrypt_dictionary(encrypt_dict.as_ref(), &mut report);

    let Some(auth_code_obj) = reader.trailer().get("AuthCode") else {
        if report.required_by_permissions == Some(true) {
            report.malformed(
                "PDF-MAC is required by permission bit 13 but trailer /AuthCode is absent",
            );
        } else {
            report.state = PdfMacState::Absent;
        }
        return report;
    };

    report.detected = true;
    let Some(auth_code) = auth_code_obj.as_dict() else {
        report.malformed("trailer /AuthCode must be a direct dictionary");
        return report;
    };
    report.auth_code_direct = true;
    validate_pdf_mac_encryption_inputs(encrypt_dict.as_ref(), &mut report);
    inspect_auth_code_dictionary(auth_code, &mut report);
    if report.state == PdfMacState::PresentUnverified {
        report.diagnostics.push(
            "PDF-MAC token structure is present, but full MAC verification was not performed"
                .to_string(),
        );
    }
    report
}

fn validate_pdf_mac_encryption_inputs(
    encrypt_dict: Option<&PdfDictionary>,
    report: &mut PdfMacReport,
) {
    let Some(encrypt_dict) = encrypt_dict else {
        report.malformed(
            "PDF-MAC /AuthCode requires an encrypted document with an /Encrypt dictionary",
        );
        return;
    };
    match encrypt_dict.get_integer("V") {
        Some(v) if v >= 5 => {}
        Some(v) => report.malformed(format!(
            "PDF-MAC requires a PDF 2.0 encryption dictionary family, got /V {v}"
        )),
        None => report.malformed("PDF-MAC encryption dictionary is missing /V"),
    }
    if report.kdf_salt_status != "present_32_bytes" {
        report.malformed("PDF-MAC encryption dictionary requires a 32-byte /KDFSalt");
    }
}

pub fn pdf_mac_report_bytes(bytes: &[u8], password: Option<&[u8]>) -> Result<PdfMacReport> {
    let reader = match password {
        Some(password) => PdfReader::from_bytes_with_password(bytes.to_vec(), password)?,
        None => PdfReader::from_bytes(bytes.to_vec())?,
    };
    Ok(pdf_mac_report(&reader))
}

pub fn pdf_mac_verify_report_bytes(bytes: &[u8], password: Option<&[u8]>) -> Result<PdfMacReport> {
    let reader = match password {
        Some(password) => PdfReader::from_bytes_with_password(bytes.to_vec(), password)?,
        None => PdfReader::from_bytes(bytes.to_vec())?,
    };
    let mut report = pdf_mac_report(&reader);
    if matches!(report.state, PdfMacState::PresentUnverified) {
        verify_pdf_mac_standalone(&reader, &mut report);
    }
    Ok(report)
}

/// Write a full PDF with a standalone ISO/TS 32004 PDF-MAC token in the trailer.
///
/// The supplied writer must already be configured for the encrypted output to
/// be protected, and the caller must provide the corresponding unlocked file
/// key plus the 32-byte `/KDFSalt` that is serialized in the encryption
/// dictionary. The function never reports or serializes the file key, wrap key,
/// or MAC key.
pub fn write_standalone_pdf_mac(
    mut writer: PdfWriter,
    file_key: &[u8],
    kdf_salt: &[u8],
) -> Result<(Vec<u8>, PdfMacWriteReport)> {
    if file_key.len() != PDF_MAC_KEY_LEN {
        return Err(WellfriendError::MalformedPdf(format!(
            "PDF-MAC writer requires a 32-byte file key, got {}",
            file_key.len()
        )));
    }
    if kdf_salt.len() != PDF_MAC_KDF_SALT_LEN {
        return Err(WellfriendError::MalformedPdf(format!(
            "PDF-MAC writer requires a 32-byte /KDFSalt, got {}",
            kdf_salt.len()
        )));
    }
    let mut mac_key = secret_bytes(crate::crypto::random_bytes(PDF_MAC_KEY_LEN));
    let placeholder_token = map_pdf_mac_write_error(build_pdf_mac_token_for_supported_profile(
        file_key, kdf_salt, &mac_key, b"",
    ))?;
    let placeholder_hex = hex_upper(&placeholder_token);
    let mut byte_range = [0i64, 0, 0, 0];
    let mut stable: Option<(Vec<u8>, usize, usize, usize)> = None;
    let mut iterations = 0usize;
    for attempt in 1..=PDF_MAC_BYTERANGE_ITERATIONS {
        iterations = attempt;
        writer.set_trailer_extra(standalone_auth_code_dict(&placeholder_token, byte_range));
        let bytes = writer.write()?;
        let (hex_start, hex_end) = locate_unique_placeholder_hex(&bytes, &placeholder_hex)?;
        let mac_literal_start = hex_start.checked_sub(1).ok_or_else(|| {
            WellfriendError::MalformedPdf("PDF-MAC placeholder starts at byte 0".to_string())
        })?;
        let after_mac_literal = hex_end.checked_add(1).ok_or_else(|| {
            WellfriendError::MalformedPdf("PDF-MAC placeholder offset overflow".to_string())
        })?;
        let next_range = [
            0,
            mac_literal_start as i64,
            after_mac_literal as i64,
            bytes.len().saturating_sub(after_mac_literal) as i64,
        ];
        if next_range == byte_range {
            stable = Some((bytes, hex_start, hex_end, attempt));
            break;
        }
        byte_range = next_range;
    }
    let Some((mut bytes, hex_start, hex_end, stable_iterations)) = stable else {
        mac_key.zeroize();
        return Err(WellfriendError::MalformedPdf(
            "PDF-MAC ByteRange placeholder did not stabilize".to_string(),
        ));
    };
    let covered = pdf_mac_covered_bytes(&bytes, &byte_range)
        .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
    let covered_hash = Sha256::digest(&covered);
    let final_token = map_pdf_mac_write_error(build_pdf_mac_token_for_supported_profile(
        file_key, kdf_salt, &mac_key, &covered,
    ))?;
    mac_key.zeroize();
    if final_token.len() != placeholder_token.len() {
        return Err(WellfriendError::MalformedPdf(format!(
            "PDF-MAC final token length {} exceeded reserved placeholder {}",
            final_token.len(),
            placeholder_token.len()
        )));
    }
    let final_hex = hex_upper(&final_token);
    if final_hex.len() != hex_end - hex_start {
        return Err(WellfriendError::MalformedPdf(
            "PDF-MAC final token hex length changed".to_string(),
        ));
    }
    bytes[hex_start..hex_end].copy_from_slice(final_hex.as_bytes());
    let covered_after_patch = pdf_mac_covered_bytes(&bytes, &byte_range)
        .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
    if covered_after_patch != covered {
        return Err(WellfriendError::MalformedPdf(
            "PDF-MAC patch changed covered bytes".to_string(),
        ));
    }
    map_pdf_mac_write_error(verify_pdf_mac_token(
        &final_token,
        file_key,
        kdf_salt,
        &covered,
    ))?;
    let output_len = bytes.len();
    Ok((
        bytes,
        PdfMacWriteReport {
            schema_version: "crypto_writer_closeout.pdf_mac.write.v1",
            source: "ISO/TS 32004:2024",
            status: "implemented_with_limits",
            mac_location: "Standalone",
            byte_range: byte_range.to_vec(),
            placeholder_token_bytes: placeholder_token.len(),
            final_token_bytes: final_token.len(),
            covered_byte_count: covered.len(),
            covered_sha256: hex_upper(covered_hash.as_slice()),
            output_bytes: output_len,
            verification_state: PdfMacState::Valid,
            placeholder_iterations: stable_iterations.max(iterations),
            secret_material_reported: false,
            diagnostics: vec![
                "Standalone PDF-MAC AuthCode placeholder patched with stable ByteRange"
                    .to_string(),
                "CMS AuthenticatedData PasswordRecipientInfo/pdfMacWrapKdf/AES-256-KW/HMAC-SHA256 self-verification passed".to_string(),
            ],
        },
    ))
}

fn inspect_encrypt_dictionary(encrypt_dict: Option<&PdfDictionary>, report: &mut PdfMacReport) {
    let Some(encrypt_dict) = encrypt_dict else {
        return;
    };
    report.encrypt_v = encrypt_dict.get_integer("V");
    if let Some(p) = encrypt_dict.get_integer("P") {
        let required = ((p as i32 as u32) & (1u32 << 12)) == 0;
        report.required_by_permissions = Some(required);
        report.permission_source = "encrypt_dictionary_p_bit_13".to_string();
    } else {
        report.permission_source = "encrypt_dictionary_without_p".to_string();
    }
    match encrypt_dict.get("KDFSalt") {
        Some(PdfObject::String(bytes)) if bytes.len() == PDF_MAC_KDF_SALT_LEN => {
            report.kdf_salt_status = "present_32_bytes".to_string();
        }
        Some(PdfObject::String(bytes)) => {
            report.kdf_salt_status = format!("malformed_length_{}", bytes.len());
            report.malformed(format!(
                "PDF-MAC /KDFSalt must be 32 bytes, got {}",
                bytes.len()
            ));
        }
        Some(other) => {
            report.kdf_salt_status = format!("wrong_type_{}", other.variant_name());
            report.malformed("PDF-MAC /KDFSalt must be a byte string");
        }
        None => {
            report.kdf_salt_status = "absent".to_string();
        }
    }
}

fn inspect_auth_code_dictionary(auth_code: &PdfDictionary, report: &mut PdfMacReport) {
    let Some(location) = auth_code.get_name("MACLocation") else {
        report.malformed("AuthCode /MACLocation is required");
        return;
    };
    report.mac_location = Some(location.to_string());
    match location {
        "Standalone" => inspect_standalone_auth_code(auth_code, report),
        "AttachedToSig" => inspect_attached_auth_code(auth_code, report),
        other => report.unsupported(format!(
            "AuthCode /MACLocation /{other} is not a supported PDF-MAC location"
        )),
    }
}

fn inspect_standalone_auth_code(auth_code: &PdfDictionary, report: &mut PdfMacReport) {
    if auth_code.get("SigObjRef").is_some() {
        report.malformed("Standalone PDF-MAC AuthCode must not contain /SigObjRef");
    }
    match auth_code.get("MAC") {
        Some(PdfObject::String(bytes)) => {
            report.has_mac = true;
            report.mac_length = Some(bytes.len());
            inspect_mac_token(bytes, report);
        }
        Some(other) => report.malformed(format!(
            "Standalone PDF-MAC /MAC must be a byte string, got {}",
            other.variant_name()
        )),
        None => report.malformed("Standalone PDF-MAC AuthCode requires /MAC"),
    }
    match parse_byte_range(auth_code) {
        Ok(Some(range)) => {
            report.has_byte_range = true;
            report.byte_range = Some(range);
        }
        Ok(None) => report.malformed("Standalone PDF-MAC AuthCode requires /ByteRange"),
        Err(diagnostic) => report.malformed(diagnostic),
    }
    if !matches!(
        report.state,
        PdfMacState::Malformed | PdfMacState::UnsupportedAlgorithm
    ) {
        report.state = PdfMacState::PresentUnverified;
    }
}

fn inspect_attached_auth_code(auth_code: &PdfDictionary, report: &mut PdfMacReport) {
    if auth_code.get("MAC").is_some() {
        report.malformed("AttachedToSig PDF-MAC AuthCode must not contain /MAC");
    }
    if auth_code.get("ByteRange").is_some() {
        report.malformed("AttachedToSig PDF-MAC AuthCode must not contain /ByteRange");
    }
    match auth_code.get_reference("SigObjRef") {
        Some((number, generation)) => {
            report.has_sig_obj_ref = true;
            report.sig_obj_ref = Some(format!("{number} {generation} R"));
        }
        None => report.malformed("AttachedToSig PDF-MAC AuthCode requires /SigObjRef"),
    }
    if !matches!(
        report.state,
        PdfMacState::Malformed | PdfMacState::UnsupportedAlgorithm
    ) {
        report.state = PdfMacState::PresentUnverified;
        report
            .diagnostics
            .push("AttachedToSig PDF-MAC token extraction is not implemented".to_string());
    }
}

fn parse_byte_range(auth_code: &PdfDictionary) -> std::result::Result<Option<Vec<i64>>, String> {
    let Some(value) = auth_code.get("ByteRange") else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err("PDF-MAC /ByteRange must be an array".to_string());
    };
    if items.len() != 4 {
        return Err(format!(
            "PDF-MAC /ByteRange must contain 4 integers, got {}",
            items.len()
        ));
    }
    let mut out = Vec::with_capacity(4);
    for item in items {
        let Some(value) = item.as_integer() else {
            return Err("PDF-MAC /ByteRange entries must be integers".to_string());
        };
        if value < 0 {
            return Err("PDF-MAC /ByteRange entries must be non-negative".to_string());
        }
        out.push(value);
    }
    if out[0] != 0 {
        return Err("PDF-MAC /ByteRange must start at byte 0".to_string());
    }
    Ok(Some(out))
}

fn inspect_mac_token(bytes: &[u8], report: &mut PdfMacReport) {
    let ci = match ContentInfo::from_der(bytes) {
        Ok(ci) => ci,
        Err(err) => {
            report.malformed(format!("PDF-MAC /MAC is not DER CMS ContentInfo: {err}"));
            return;
        }
    };
    report.cms_content_type = Some(ci.content_type.to_string());
    if ci.content_type != OID_AUTHENTICATED_DATA {
        report.unsupported(format!(
            "PDF-MAC CMS ContentInfo type {} is unsupported; expected AuthenticatedData",
            ci.content_type
        ));
        return;
    }
    report.cms_authenticated_data = true;
    let authenticated = match AuthenticatedData::from_der(
        &ci.content
            .to_der()
            .map_err(|err| err.to_string())
            .unwrap_or_default(),
    ) {
        Ok(value) => value,
        Err(err) => {
            report.malformed(format!("PDF-MAC AuthenticatedData parse failed: {err}"));
            return;
        }
    };
    if authenticated.encap_content_info.econtent_type == OID_PDF_MAC_INTEGRITY_INFO {
        report.cms_pdf_mac_content_type = true;
    } else {
        report.unsupported(format!(
            "PDF-MAC encapsulated content type {} is unsupported",
            authenticated.encap_content_info.econtent_type
        ));
    }
    let recipient_count = authenticated.recip_infos.0.len();
    if recipient_count != 1 {
        report.malformed(format!(
            "PDF-MAC AuthenticatedData must contain exactly one recipientInfo, got {recipient_count}"
        ));
        return;
    }
    match authenticated.recip_infos.0.iter().next() {
        Some(cms::enveloped_data::RecipientInfo::Pwri(pwri)) => {
            report.cms_recipient_profile = "PasswordRecipientInfo".to_string();
            match &pwri.key_derivation_alg {
                Some(alg) if alg.oid == OID_PDF_MAC_WRAP_KDF => {}
                Some(alg) => report.unsupported(format!(
                    "PDF-MAC PasswordRecipientInfo KDF OID {} is unsupported",
                    alg.oid
                )),
                None => report.malformed("PDF-MAC PasswordRecipientInfo KDF is required"),
            }
        }
        Some(other) => report.unsupported(format!(
            "PDF-MAC recipient profile {} is unsupported",
            recipient_type_name(other)
        )),
        None => report.malformed("PDF-MAC AuthenticatedData contains no recipientInfo"),
    }
}

fn recipient_type_name(recipient: &cms::enveloped_data::RecipientInfo) -> &'static str {
    match recipient {
        cms::enveloped_data::RecipientInfo::Ktri(_) => "KeyTransRecipientInfo",
        cms::enveloped_data::RecipientInfo::Kari(_) => "KeyAgreeRecipientInfo",
        cms::enveloped_data::RecipientInfo::Kekri(_) => "KEKRecipientInfo",
        cms::enveloped_data::RecipientInfo::Pwri(_) => "PasswordRecipientInfo",
        cms::enveloped_data::RecipientInfo::Ori(_) => "OtherRecipientInfo",
    }
}

fn verify_pdf_mac_standalone(reader: &PdfReader, report: &mut PdfMacReport) {
    report.verification_performed = true;
    report.trusted_document_integrity = false;
    if report.mac_location.as_deref() != Some("Standalone") {
        report.state = PdfMacState::KeyUnavailable;
        report.diagnostics.push(
            "PDF-MAC verification currently supports Standalone MACLocation only".to_string(),
        );
        return;
    }
    let Some(ctx) = reader.encryption() else {
        report.state = PdfMacState::KeyUnavailable;
        report.diagnostics.push(
            "PDF-MAC verification requires an unlocked encrypted document file key".to_string(),
        );
        return;
    };
    let Some(encrypt_dict) = reader.encrypt_dictionary() else {
        report.malformed("PDF-MAC verification requires an /Encrypt dictionary");
        return;
    };
    let Some(kdf_salt) = pdf_mac_kdf_salt(&encrypt_dict, report) else {
        return;
    };
    let Some(auth_code) = reader
        .trailer()
        .get("AuthCode")
        .and_then(PdfObject::as_dict)
    else {
        report.malformed("PDF-MAC verification requires a direct trailer /AuthCode dictionary");
        return;
    };
    let Some(mac_token) = auth_code.get("MAC").and_then(|obj| match obj {
        PdfObject::String(bytes) => Some(bytes.as_slice()),
        _ => None,
    }) else {
        report.malformed("Standalone PDF-MAC verification requires byte-string /MAC");
        return;
    };
    let byte_range = match parse_byte_range(auth_code) {
        Ok(Some(range)) => range,
        Ok(None) => {
            report.state = PdfMacState::ByteRangeInvalid;
            report
                .diagnostics
                .push("Standalone PDF-MAC verification requires /ByteRange".to_string());
            return;
        }
        Err(diagnostic) => {
            report.state = PdfMacState::ByteRangeInvalid;
            report.diagnostics.push(diagnostic);
            return;
        }
    };
    let covered = match pdf_mac_covered_bytes(reader.file_bytes(), &byte_range) {
        Ok(bytes) => bytes,
        Err(diagnostic) => {
            report.state = PdfMacState::ByteRangeInvalid;
            report.diagnostics.push(diagnostic);
            return;
        }
    };
    match verify_pdf_mac_token(mac_token, &ctx.file_key, &kdf_salt, &covered) {
        Ok(()) => {
            report.state = PdfMacState::Valid;
            report.trusted_document_integrity = true;
            report.diagnostics.retain(|item| {
                item != "PDF-MAC token structure is present, but full MAC verification was not performed"
            });
            report.diagnostics.push(
                "PDF-MAC Standalone token verified with PasswordRecipientInfo/pdfMacWrapKdf/AES-256-KW/HMAC-SHA256"
                    .to_string(),
            );
        }
        Err(PdfMacVerifyFailure::Unsupported(diagnostic)) => report.unsupported(diagnostic),
        Err(PdfMacVerifyFailure::Malformed(diagnostic)) => report.malformed(diagnostic),
        Err(PdfMacVerifyFailure::Invalid(diagnostic)) => report.invalid(diagnostic),
        Err(PdfMacVerifyFailure::Authentication(diagnostic)) => {
            report.authentication_failed(diagnostic)
        }
    }
}

#[derive(Debug)]
enum PdfMacVerifyFailure {
    Malformed(String),
    Unsupported(String),
    Invalid(String),
    Authentication(String),
}

impl From<der::Error> for PdfMacVerifyFailure {
    fn from(err: der::Error) -> Self {
        Self::Malformed(format!("PDF-MAC DER parse/encode error: {err}"))
    }
}

fn pdf_mac_kdf_salt(encrypt_dict: &PdfDictionary, report: &mut PdfMacReport) -> Option<Vec<u8>> {
    match encrypt_dict.get("KDFSalt") {
        Some(PdfObject::String(bytes)) if bytes.len() == PDF_MAC_KDF_SALT_LEN => {
            Some(bytes.clone())
        }
        Some(PdfObject::String(bytes)) => {
            report.malformed(format!(
                "PDF-MAC /KDFSalt must be 32 bytes, got {}",
                bytes.len()
            ));
            None
        }
        Some(_) => {
            report.malformed("PDF-MAC /KDFSalt must be a byte string");
            None
        }
        None => {
            report.malformed("PDF-MAC verification requires /KDFSalt");
            None
        }
    }
}

fn pdf_mac_covered_bytes(file: &[u8], range: &[i64]) -> std::result::Result<Vec<u8>, String> {
    if range.len() != 4 {
        return Err("PDF-MAC /ByteRange must contain four entries".to_string());
    }
    if range.iter().any(|value| *value < 0) {
        return Err("PDF-MAC /ByteRange entries must be non-negative".to_string());
    }
    let a = range[0] as usize;
    let b = range[1] as usize;
    let c = range[2] as usize;
    let d = range[3] as usize;
    if a != 0 {
        return Err("PDF-MAC /ByteRange must start at byte 0".to_string());
    }
    if b > c {
        return Err("PDF-MAC /ByteRange ranges overlap or are unsorted".to_string());
    }
    let second_end = c
        .checked_add(d)
        .ok_or_else(|| "PDF-MAC /ByteRange length overflow".to_string())?;
    if b > file.len() || c > file.len() || second_end > file.len() {
        return Err("PDF-MAC /ByteRange is out of bounds".to_string());
    }
    let mut out = Vec::with_capacity(b + d);
    out.extend_from_slice(&file[a..a + b]);
    out.extend_from_slice(&file[c..second_end]);
    Ok(out)
}

fn standalone_auth_code_dict(token: &[u8], byte_range: [i64; 4]) -> PdfDictionary {
    let mut auth_code = PdfDictionary::empty();
    auth_code.insert(
        "ByteRange",
        PdfObject::Array(
            byte_range
                .iter()
                .map(|value| PdfObject::Integer(*value))
                .collect(),
        ),
    );
    auth_code.insert("MAC", PdfObject::String(token.to_vec()));
    auth_code.insert("MACLocation", PdfObject::Name("Standalone".to_string()));
    let mut trailer_extra = PdfDictionary::empty();
    trailer_extra.insert("AuthCode", PdfObject::Dictionary(auth_code));
    trailer_extra
}

fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

fn locate_unique_placeholder_hex(bytes: &[u8], placeholder_hex: &str) -> Result<(usize, usize)> {
    let needle = placeholder_hex.as_bytes();
    if needle.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "PDF-MAC placeholder token is empty".to_string(),
        ));
    }
    let mut found = None;
    for (idx, window) in bytes.windows(needle.len()).enumerate() {
        if window == needle {
            if found.is_some() {
                return Err(WellfriendError::MalformedPdf(
                    "PDF-MAC placeholder token is not unique".to_string(),
                ));
            }
            found = Some(idx);
        }
    }
    let start = found.ok_or_else(|| {
        WellfriendError::MalformedPdf("PDF-MAC placeholder token was not serialized".to_string())
    })?;
    if start == 0
        || bytes.get(start - 1) != Some(&b'<')
        || bytes.get(start + needle.len()) != Some(&b'>')
    {
        return Err(WellfriendError::MalformedPdf(
            "PDF-MAC placeholder is not a direct hex string".to_string(),
        ));
    }
    Ok((start, start + needle.len()))
}

fn map_pdf_mac_write_error<T>(result: std::result::Result<T, PdfMacVerifyFailure>) -> Result<T> {
    result.map_err(|err| match err {
        PdfMacVerifyFailure::Malformed(msg) | PdfMacVerifyFailure::Invalid(msg) => {
            WellfriendError::MalformedPdf(msg)
        }
        PdfMacVerifyFailure::Unsupported(msg) => WellfriendError::UnsupportedFeature(msg),
        PdfMacVerifyFailure::Authentication(msg) => WellfriendError::AuthenticationFailure(msg),
    })
}

fn verify_pdf_mac_token(
    token: &[u8],
    file_key: &[u8],
    kdf_salt: &[u8],
    covered_bytes: &[u8],
) -> std::result::Result<(), PdfMacVerifyFailure> {
    let ci = ContentInfo::from_der(token)?;
    if ci.content_type != OID_AUTHENTICATED_DATA {
        return Err(PdfMacVerifyFailure::Unsupported(format!(
            "PDF-MAC ContentInfo type {} is not AuthenticatedData",
            ci.content_type
        )));
    }
    let authenticated = AuthenticatedData::from_der(&ci.content.to_der()?)?;
    verify_pdf_mac_authenticated_data(&authenticated, file_key, kdf_salt, covered_bytes)
}

fn verify_pdf_mac_authenticated_data(
    authenticated: &AuthenticatedData,
    file_key: &[u8],
    kdf_salt: &[u8],
    covered_bytes: &[u8],
) -> std::result::Result<(), PdfMacVerifyFailure> {
    if authenticated.encap_content_info.econtent_type != OID_PDF_MAC_INTEGRITY_INFO {
        return Err(PdfMacVerifyFailure::Unsupported(format!(
            "PDF-MAC encapsulated content type {} is unsupported",
            authenticated.encap_content_info.econtent_type
        )));
    }
    if authenticated.digest_alg.as_ref().map(|alg| alg.oid)
        != Some(const_oid::db::rfc5912::ID_SHA_256)
    {
        return Err(PdfMacVerifyFailure::Unsupported(
            "PDF-MAC verifier supports SHA-256 digestAlgorithm only".to_string(),
        ));
    }
    if authenticated.mac_alg.oid != const_oid::db::rfc6268::ID_HMAC_WITH_SHA_256 {
        return Err(PdfMacVerifyFailure::Unsupported(format!(
            "PDF-MAC verifier supports HMAC-SHA256 only, got {}",
            authenticated.mac_alg.oid
        )));
    }
    if authenticated.unauth_attrs.is_some() {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC AuthenticatedData must not contain unauthenticated attributes".to_string(),
        ));
    }
    let auth_attrs = authenticated.auth_attrs.as_ref().ok_or_else(|| {
        PdfMacVerifyFailure::Malformed(
            "PDF-MAC AuthenticatedData requires authenticated attributes".to_string(),
        )
    })?;
    let integrity_der = authenticated
        .encap_content_info
        .econtent
        .as_ref()
        .ok_or_else(|| {
            PdfMacVerifyFailure::Malformed(
                "PDF-MAC AuthenticatedData requires encapsulated content".to_string(),
            )
        })?
        .decode_as::<OctetString>()
        .map_err(PdfMacVerifyFailure::from)?
        .as_bytes()
        .to_vec();
    let integrity = PdfMacIntegrityInfo::from_der(&integrity_der)?;
    if integrity.version != 0 {
        return Err(PdfMacVerifyFailure::Unsupported(format!(
            "PDF-MAC integrity info version {} is unsupported",
            integrity.version
        )));
    }
    let content_type = single_oid_attr(auth_attrs, const_oid::db::rfc5911::ID_CONTENT_TYPE)?;
    if content_type != OID_PDF_MAC_INTEGRITY_INFO {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC contentType authenticated attribute mismatch".to_string(),
        ));
    }
    let message_digest = single_octet_attr(auth_attrs, const_oid::db::rfc5911::ID_MESSAGE_DIGEST)?;
    let computed_message_digest = Sha256::digest(&integrity_der);
    if message_digest
        .as_slice()
        .ct_eq(computed_message_digest.as_slice())
        .unwrap_u8()
        != 1
    {
        return Err(PdfMacVerifyFailure::Authentication(
            "PDF-MAC messageDigest authenticated attribute mismatch".to_string(),
        ));
    }
    let wrapped_mac_key = password_recipient_wrapped_key(authenticated)?;
    let mut wrap_key = pdf_mac_wrap_key(file_key, kdf_salt)?;
    let mut mac_key = pdf_mac_unwrap_key(&wrap_key, wrapped_mac_key.as_bytes())?;
    wrap_key.zeroize();
    let auth_attrs_der = auth_attrs.to_der()?;
    let expected_mac = pdf_mac_hmac_sha256(&mac_key, &auth_attrs_der)?;
    mac_key.zeroize();
    if authenticated
        .mac
        .as_bytes()
        .ct_eq(&expected_mac)
        .unwrap_u8()
        != 1
    {
        return Err(PdfMacVerifyFailure::Authentication(
            "PDF-MAC HMAC verification failed".to_string(),
        ));
    }
    let covered_digest = Sha256::digest(covered_bytes);
    if integrity
        .data_digest
        .as_bytes()
        .ct_eq(covered_digest.as_slice())
        .unwrap_u8()
        != 1
    {
        return Err(PdfMacVerifyFailure::Invalid(
            "PDF-MAC covered ByteRange digest mismatch".to_string(),
        ));
    }
    Ok(())
}

fn password_recipient_wrapped_key(
    authenticated: &AuthenticatedData,
) -> std::result::Result<&EncryptedKey, PdfMacVerifyFailure> {
    let recipient_count = authenticated.recip_infos.0.len();
    if recipient_count != 1 {
        return Err(PdfMacVerifyFailure::Malformed(format!(
            "PDF-MAC AuthenticatedData must contain exactly one recipientInfo, got {recipient_count}"
        )));
    }
    match authenticated.recip_infos.0.iter().next() {
        Some(RecipientInfo::Pwri(pwri)) => {
            let kdf = pwri.key_derivation_alg.as_ref().ok_or_else(|| {
                PdfMacVerifyFailure::Malformed(
                    "PDF-MAC PasswordRecipientInfo requires keyDerivationAlgorithm".to_string(),
                )
            })?;
            if kdf.oid != OID_PDF_MAC_WRAP_KDF {
                return Err(PdfMacVerifyFailure::Unsupported(format!(
                    "PDF-MAC PasswordRecipientInfo KDF OID {} is unsupported",
                    kdf.oid
                )));
            }
            if kdf.parameters.is_some() {
                return Err(PdfMacVerifyFailure::Malformed(
                    "PDF-MAC pdfMacWrapKdf parameters must be absent".to_string(),
                ));
            }
            if pwri.key_enc_alg.oid != OID_AES256_WRAP {
                return Err(PdfMacVerifyFailure::Unsupported(format!(
                    "PDF-MAC PasswordRecipientInfo key-encryption OID {} is unsupported",
                    pwri.key_enc_alg.oid
                )));
            }
            if pwri.key_enc_alg.parameters.is_some() {
                return Err(PdfMacVerifyFailure::Malformed(
                    "PDF-MAC AES-256-KW parameters must be absent".to_string(),
                ));
            }
            Ok(&pwri.enc_key)
        }
        Some(other) => Err(PdfMacVerifyFailure::Unsupported(format!(
            "PDF-MAC recipient profile {} is unsupported",
            recipient_type_name(other)
        ))),
        None => Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC AuthenticatedData contains no recipientInfo".to_string(),
        )),
    }
}

fn pdf_mac_wrap_key(
    file_key: &[u8],
    kdf_salt: &[u8],
) -> std::result::Result<SecretBytes, PdfMacVerifyFailure> {
    if kdf_salt.len() != PDF_MAC_KDF_SALT_LEN {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC KDFSalt must be 32 bytes".to_string(),
        ));
    }
    let hk = hkdf::Hkdf::<Sha256>::new(Some(kdf_salt), file_key);
    let mut out = secret_bytes(vec![0u8; PDF_MAC_KEY_LEN]);
    hk.expand(PDF_MAC_INFO, &mut out)
        .map_err(|_| PdfMacVerifyFailure::Malformed("PDF-MAC HKDF expand failed".to_string()))?;
    Ok(out)
}

fn pdf_mac_unwrap_key(
    kek: &[u8],
    wrapped: &[u8],
) -> std::result::Result<SecretBytes, PdfMacVerifyFailure> {
    if kek.len() != PDF_MAC_KEY_LEN {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC AES-KW KEK must be 32 bytes".to_string(),
        ));
    }
    if wrapped.len() < 16 || !wrapped.len().is_multiple_of(8) {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC AES-KW wrapped key has invalid length".to_string(),
        ));
    }
    let cipher = KwAes256::new_from_slice(kek).map_err(|_| {
        PdfMacVerifyFailure::Malformed("PDF-MAC AES-KW key initialization failed".to_string())
    })?;
    let mut buf = secret_bytes(vec![0u8; wrapped.len() - aes_kw::IV_LEN]);
    if cipher.unwrap_key(wrapped, &mut buf).is_err() {
        buf.zeroize();
        return Err(PdfMacVerifyFailure::Authentication(
            "PDF-MAC AES-KW unwrap failed".to_string(),
        ));
    }
    Ok(buf)
}

fn pdf_mac_wrap_key_material(
    kek: &[u8],
    key: &[u8],
) -> std::result::Result<Vec<u8>, PdfMacVerifyFailure> {
    if key.is_empty() || !key.len().is_multiple_of(8) {
        return Err(PdfMacVerifyFailure::Malformed(
            "PDF-MAC AES-KW plaintext key length must be a non-empty multiple of 8".to_string(),
        ));
    }
    let cipher = KwAes256::new_from_slice(kek).map_err(|_| {
        PdfMacVerifyFailure::Malformed("PDF-MAC AES-KW key initialization failed".to_string())
    })?;
    let mut out = vec![0u8; key.len() + aes_kw::IV_LEN];
    let wrapped = cipher
        .wrap_key(key, &mut out)
        .map_err(|_| PdfMacVerifyFailure::Malformed("PDF-MAC AES-KW wrap failed".to_string()))?;
    Ok(wrapped.to_vec())
}

fn pdf_mac_hmac_sha256(
    key: &[u8],
    data: &[u8],
) -> std::result::Result<[u8; PDF_MAC_SHA256_LEN], PdfMacVerifyFailure> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| {
        PdfMacVerifyFailure::Malformed("PDF-MAC HMAC-SHA256 key initialization failed".to_string())
    })?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().into())
}

fn single_oid_attr(
    attrs: &Attributes,
    oid: ObjectIdentifier,
) -> std::result::Result<ObjectIdentifier, PdfMacVerifyFailure> {
    let value = single_attr_value(attrs, oid)?;
    value
        .decode_as::<ObjectIdentifier>()
        .map_err(PdfMacVerifyFailure::from)
}

fn single_octet_attr(
    attrs: &Attributes,
    oid: ObjectIdentifier,
) -> std::result::Result<Vec<u8>, PdfMacVerifyFailure> {
    let value = single_attr_value(attrs, oid)?;
    Ok(value
        .decode_as::<OctetString>()
        .map_err(PdfMacVerifyFailure::from)?
        .as_bytes()
        .to_vec())
}

fn single_attr_value(
    attrs: &Attributes,
    oid: ObjectIdentifier,
) -> std::result::Result<&Any, PdfMacVerifyFailure> {
    let mut found: Option<&Attribute> = None;
    for attr in attrs.iter().filter(|attr| attr.oid == oid) {
        if found.is_some() {
            return Err(PdfMacVerifyFailure::Malformed(format!(
                "PDF-MAC authenticated attribute {oid} is duplicated"
            )));
        }
        found = Some(attr);
    }
    let attr = found.ok_or_else(|| {
        PdfMacVerifyFailure::Malformed(format!("PDF-MAC authenticated attribute {oid} is missing"))
    })?;
    if attr.values.len() != 1 {
        return Err(PdfMacVerifyFailure::Malformed(format!(
            "PDF-MAC authenticated attribute {oid} must contain exactly one value"
        )));
    }
    attr.values.iter().next().ok_or_else(|| {
        PdfMacVerifyFailure::Malformed(format!("PDF-MAC authenticated attribute {oid} is empty"))
    })
}

fn any_from_der_value<T: Encode>(value: &T) -> der::Result<Any> {
    let der = value.to_der()?;
    Ok(Any::from(AnyRef::try_from(der.as_slice())?))
}

fn pdf_mac_attr(oid: ObjectIdentifier, value: Any) -> der::Result<Attribute> {
    Ok(Attribute {
        oid,
        values: SetOfVec::try_from(vec![value])?,
    })
}

fn build_pdf_mac_token_for_supported_profile(
    file_key: &[u8],
    kdf_salt: &[u8],
    mac_key: &[u8],
    covered_bytes: &[u8],
) -> std::result::Result<Vec<u8>, PdfMacVerifyFailure> {
    let mut wrap_key = pdf_mac_wrap_key(file_key, kdf_salt)?;
    let wrapped_mac_key = pdf_mac_wrap_key_material(&wrap_key, mac_key)?;
    wrap_key.zeroize();
    let data_digest = Sha256::digest(covered_bytes);
    let integrity = PdfMacIntegrityInfo {
        version: 0,
        data_digest: OctetString::new(data_digest.to_vec())
            .map_err(|err| PdfMacVerifyFailure::Malformed(err.to_string()))?,
        signature_digest: None,
    };
    let integrity_der = integrity.to_der()?;
    let content_octets = OctetString::new(integrity_der.clone())
        .map_err(|err| PdfMacVerifyFailure::Malformed(err.to_string()))?;
    let encap_content_info = EncapsulatedContentInfo {
        econtent_type: OID_PDF_MAC_INTEGRITY_INFO,
        econtent: Some(any_from_der_value(&content_octets)?),
    };
    let message_digest = Sha256::digest(&integrity_der);
    let attrs = Attributes::try_from(vec![
        pdf_mac_attr(
            const_oid::db::rfc5911::ID_CONTENT_TYPE,
            any_from_der_value(&OID_PDF_MAC_INTEGRITY_INFO)?,
        )?,
        pdf_mac_attr(
            const_oid::db::rfc5911::ID_MESSAGE_DIGEST,
            any_from_der_value(
                &OctetString::new(message_digest.to_vec())
                    .map_err(|err| PdfMacVerifyFailure::Malformed(err.to_string()))?,
            )?,
        )?,
    ])?;
    let auth_attrs_der = attrs.to_der()?;
    let mac = pdf_mac_hmac_sha256(mac_key, &auth_attrs_der)?;
    let pwri = PasswordRecipientInfo {
        version: CmsVersion::V0,
        key_derivation_alg: Some(AlgorithmIdentifierOwned {
            oid: OID_PDF_MAC_WRAP_KDF,
            parameters: None,
        }),
        key_enc_alg: AlgorithmIdentifierOwned {
            oid: OID_AES256_WRAP,
            parameters: None,
        },
        enc_key: EncryptedKey::new(wrapped_mac_key)
            .map_err(|err| PdfMacVerifyFailure::Malformed(err.to_string()))?,
    };
    let authenticated = AuthenticatedData {
        version: CmsVersion::V0,
        originator_info: None,
        recip_infos: RecipientInfos::try_from(vec![RecipientInfo::Pwri(pwri)])?,
        mac_alg: AlgorithmIdentifierOwned {
            oid: const_oid::db::rfc6268::ID_HMAC_WITH_SHA_256,
            parameters: None,
        },
        digest_alg: Some(AlgorithmIdentifierOwned {
            oid: const_oid::db::rfc5912::ID_SHA_256,
            parameters: None,
        }),
        encap_content_info,
        auth_attrs: Some(attrs),
        mac: OctetString::new(mac.to_vec())
            .map_err(|err| PdfMacVerifyFailure::Malformed(err.to_string()))?,
        unauth_attrs: None,
    };
    let authenticated_der = authenticated.to_der()?;
    let ci = ContentInfo {
        content_type: OID_AUTHENTICATED_DATA,
        content: Any::from(AnyRef::try_from(authenticated_der.as_slice())?),
    };
    Ok(ci.to_der()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{build_encryption, secret_bytes, EncryptAlgorithm, EncryptParams};
    use crate::writer::{OutputObject, PdfWriter};

    fn pdf_with_trailer_extra(extra: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");
        let obj1 = out.len();
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
        out.extend_from_slice(format!("{obj1:010} 00000 n \n").as_bytes());
        out.extend_from_slice(b"trailer\n<< /Size 2 /Root 1 0 R ");
        out.extend_from_slice(extra.as_bytes());
        out.extend_from_slice(b" >>\nstartxref\n");
        out.extend_from_slice(xref.to_string().as_bytes());
        out.extend_from_slice(b"\n%%EOF\n");
        out
    }

    #[test]
    fn pdf_mac_report_absent_without_auth_code() {
        let pdf = pdf_with_trailer_extra("");
        let reader = PdfReader::from_bytes(pdf).unwrap();
        let report = pdf_mac_report(&reader);
        assert_eq!(report.state, PdfMacState::Absent);
        assert!(!report.trusted_document_integrity);
    }

    #[test]
    fn pdf_mac_report_rejects_non_dictionary_auth_code() {
        let pdf = pdf_with_trailer_extra("/AuthCode 42");
        let reader = PdfReader::from_bytes(pdf).unwrap();
        let report = pdf_mac_report(&reader);
        assert_eq!(report.state, PdfMacState::Malformed);
    }

    #[test]
    fn pdf_mac_report_rejects_truncated_mac_token() {
        let pdf = pdf_with_trailer_extra(
            "/AuthCode << /MACLocation /Standalone /ByteRange [0 10 20 30] /MAC <010203> >>",
        );
        let reader = PdfReader::from_bytes(pdf).unwrap();
        let report = pdf_mac_report(&reader);
        assert_eq!(report.state, PdfMacState::Malformed);
        assert!(!report.trusted_document_integrity);
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.contains("ContentInfo")));
    }

    #[test]
    fn pdf_mac_verify_rejects_auth_code_without_encrypt_dictionary() {
        let pdf =
            pdf_with_trailer_extra("/AuthCode << /MACLocation /AttachedToSig /SigObjRef 1 0 R >>");
        let report = pdf_mac_verify_report_bytes(&pdf, None).unwrap();
        assert_eq!(report.state, PdfMacState::Malformed);
        assert!(!report.verification_performed);
        assert!(!report.trusted_document_integrity);
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.contains("/Encrypt")));
    }

    #[test]
    fn pdf_mac_hmac_sha256_matches_rfc4231_vector() {
        let key = vec![0x0b; 20];
        let mac = pdf_mac_hmac_sha256(&key, b"Hi There").unwrap();
        assert_eq!(
            hex(&mac),
            "B0344C61D8DB38535CA8AFCEAF0BF12B881DC200C9833DA726E9376C2E32CFF7"
        );
    }

    #[test]
    fn pdf_mac_aes256_kw_matches_rfc3394_vector() {
        let kek = hex_to_vec("000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F");
        let key = hex_to_vec("00112233445566778899AABBCCDDEEFF");
        let wrapped = pdf_mac_wrap_key_material(&kek, &key).unwrap();
        assert_eq!(
            hex(&wrapped),
            "64E8C3F9CE0F5BA263E9777905818A2A93C8191E7D6E8AE7"
        );
        let unwrapped = pdf_mac_unwrap_key(&kek, &wrapped).unwrap();
        assert_eq!(&*unwrapped, key.as_slice());
        let mut tampered = wrapped.clone();
        *tampered.last_mut().unwrap() ^= 0x80;
        assert!(matches!(
            pdf_mac_unwrap_key(&kek, &tampered).unwrap_err(),
            PdfMacVerifyFailure::Authentication(_)
        ));
    }

    #[test]
    fn pdf_mac_hkdf_matches_rfc5869_reference_for_same_primitive() {
        let ikm = vec![0x0b; 22];
        let salt = hex_to_vec("000102030405060708090A0B0C");
        let info = hex_to_vec("F0F1F2F3F4F5F6F7F8F9");
        let hk = hkdf::Hkdf::<Sha256>::new(Some(&salt), &ikm);
        let mut okm = vec![0u8; 42];
        hk.expand(&info, &mut okm).unwrap();
        assert_eq!(
            hex(&okm),
            "3CB25F25FAACD57A90434F64D0362F2A2D2D0A90CF1A5A4C5DB02D56ECC4C5BF34007208D5B887185865"
        );
    }

    #[test]
    fn pdf_mac_supported_authenticated_data_verifies_and_tamper_fails() {
        let file_key = [0x11u8; 32];
        let kdf_salt = [0x22u8; 32];
        let mac_key = [0x33u8; 32];
        let covered = b"covered bytes in byte range";
        let token =
            build_pdf_mac_token_for_supported_profile(&file_key, &kdf_salt, &mac_key, covered)
                .unwrap();
        verify_pdf_mac_token(&token, &file_key, &kdf_salt, covered).unwrap();

        let mut tampered = token.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(verify_pdf_mac_token(&tampered, &file_key, &kdf_salt, covered).is_err());
        assert!(matches!(
            verify_pdf_mac_token(&token, &file_key, &kdf_salt, b"changed covered bytes")
                .unwrap_err(),
            PdfMacVerifyFailure::Invalid(_)
        ));
    }

    #[test]
    fn pdf_mac_writer_creates_valid_standalone_aesv4_pdf_and_tamper_fails() {
        let mut catalog = PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        let mut stream_dict = PdfDictionary::empty();
        stream_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        let objects = vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Stream {
                    dict: stream_dict,
                    raw: b"covered plaintext stream".to_vec(),
                },
            },
        ];
        let params = EncryptParams {
            user_password: secret_bytes(Vec::new()),
            owner_password: secret_bytes(Vec::new()),
            permissions: -1,
            algorithm: EncryptAlgorithm::Aes256Gcm,
            encrypt_metadata: true,
        };
        let file_id = vec![0x44u8; 16];
        let state = build_encryption(&params, &file_id).unwrap();
        let file_key = state.file_key.clone();
        let kdf_salt = state.info.kdf_salt.clone().unwrap();
        let writer = PdfWriter::new(objects, 1)
            .with_id(Some(file_id))
            .with_encryption(state);
        let (bytes, write_report) = write_standalone_pdf_mac(writer, &file_key, &kdf_salt).unwrap();
        assert_eq!(write_report.verification_state, PdfMacState::Valid);
        assert!(!write_report.secret_material_reported);

        let report = pdf_mac_verify_report_bytes(&bytes, None).unwrap();
        assert_eq!(report.state, PdfMacState::Valid);
        assert!(report.trusted_document_integrity);

        let mut tampered = bytes;
        let stream_pos = tampered
            .windows(b"stream\n".len())
            .position(|window| window == b"stream\n")
            .expect("stream marker")
            + b"stream\n".len();
        tampered[stream_pos] ^= 0x01;
        let tamper_report = pdf_mac_verify_report_bytes(&tampered, None).unwrap();
        assert_ne!(tamper_report.state, PdfMacState::Valid);
        assert!(!tamper_report.trusted_document_integrity);
    }

    fn hex(bytes: &[u8]) -> String {
        const LUT: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(LUT[(b >> 4) as usize] as char);
            out.push(LUT[(b & 0x0F) as usize] as char);
        }
        out
    }

    fn hex_to_vec(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let bytes = s.as_bytes();
        assert_eq!(bytes.len() % 2, 0);
        for pair in bytes.chunks(2) {
            let hi = (pair[0] as char).to_digit(16).unwrap();
            let lo = (pair[1] as char).to_digit(16).unwrap();
            out.push(((hi << 4) | lo) as u8);
        }
        out
    }
}
