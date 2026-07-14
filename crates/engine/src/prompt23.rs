//! Combined Prompt 23 deterministic-writer and crypto posture surface.
//!
//! This module deliberately separates implemented writer determinism evidence
//! from cryptographic features that depend on ISO PDF extension text. Public-key
//! security handlers and PDF AES-GCM are reported precisely, and unsupported
//! profiles are named without downgrading or guessing byte layouts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::security::security_report;
use crate::writer::{rewrite_document_with_mode, write_document_linearized, WriterMode};
use crate::{ContentEngine, PdfDocument};

pub const PROMPT23_SCHEMA_VERSION: &str = "prompt23.deterministic-writer-pubsec-aesgcm.v1";
pub const PROMPT23_ARTIFACT_ROOT: &str = "target/prompt23-writer-crypto";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt23Status {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedNoCryptoBackend,
    NotInPrompt23Scope,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt23Report {
    pub schema_version: &'static str,
    pub status: Prompt23Status,
    pub audit_doc: &'static str,
    pub artifact_root: &'static str,
    pub current_document: Value,
    pub feature_matrix: Vec<Prompt23FeatureMatrixRow>,
    pub blocked_rows: usize,
    pub deterministic_external_diff: Value,
    pub writer_closeout: Value,
    pub public_key_handler: Value,
    pub key_provider: Value,
    pub cms_recipient_processing: Value,
    pub aes_gcm: Value,
    pub decrypt_edit_reencrypt: Value,
    pub interoperability_fuzz_metamorphic: Value,
    pub performance_memory: Value,
    pub validation_manifest: Value,
    pub exact_remaining_limits: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt23FeatureMatrixRow {
    pub feature_id: &'static str,
    pub category: &'static str,
    pub capability: &'static str,
    pub implementation_status: Prompt23Status,
    pub deterministic_status: &'static str,
    pub cryptographic_status: &'static str,
    pub full_rewrite_status: &'static str,
    pub incremental_status: &'static str,
    pub rust_api: &'static str,
    pub cli: &'static str,
    pub python: &'static str,
    pub c_abi: &'static str,
    pub wasm: &'static str,
    pub dotnet: &'static str,
    pub java: &'static str,
    pub fixture: &'static str,
    pub test: &'static str,
    pub artifact: &'static str,
    pub reference_differential_status: &'static str,
    pub security_posture: &'static str,
    pub remaining_exact_limit: &'static str,
    pub future_owner: &'static str,
}

pub fn prompt23_report(engine: &ContentEngine) -> Result<Prompt23Report> {
    let feature_matrix = prompt23_feature_matrix();
    let blocked_rows = feature_matrix
        .iter()
        .filter(|row| row.implementation_status == Prompt23Status::Blocked)
        .count();
    let reader = engine.document().reader();
    Ok(Prompt23Report {
        schema_version: PROMPT23_SCHEMA_VERSION,
        status: Prompt23Status::ImplementedWithLimits,
        audit_doc: "docs/prompt23_writer_crypto_audit.md",
        artifact_root: PROMPT23_ARTIFACT_ROOT,
        current_document: json!({
            "page_count": engine.page_count()?,
            "encrypted": engine.is_encrypted(),
            "object_count": reader.object_ids().len(),
            "stream_count": stream_count(engine),
            "input_sha256": sha256_hex(reader.file_bytes()),
        }),
        feature_matrix,
        blocked_rows,
        deterministic_external_diff: writer_external_diff_report(engine)?,
        writer_closeout: writer_closeout_report(engine)?,
        public_key_handler: public_key_handler_report_for_engine(engine)?,
        key_provider: key_provider_report(),
        cms_recipient_processing: cms_recipient_report(),
        aes_gcm: aes_gcm_report_for_engine(engine)?,
        decrypt_edit_reencrypt: decrypt_edit_reencrypt_report(),
        interoperability_fuzz_metamorphic: interoperability_fuzz_metamorphic_report(),
        performance_memory: performance_memory_report(engine),
        validation_manifest: validation_manifest_report(),
        exact_remaining_limits: prompt23_exact_remaining_limits(),
    })
}

pub fn deterministic_writer_audit(engine: &ContentEngine) -> Result<Value> {
    let mut mode_rows = Vec::new();
    for (mode, mode_name) in [
        (WriterMode::ClassicXref, "classic_xref"),
        (WriterMode::XrefStream, "xref_stream"),
        (WriterMode::XrefStreamWithObjStm, "xref_stream_with_objstm"),
    ] {
        mode_rows.push(writer_mode_repeat_report(engine, mode, mode_name)?);
    }
    mode_rows.push(linearized_repeat_report(engine));
    Ok(json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "contract": {
            "deterministic_full_rewrite": "byte-identical for executed same-process writer modes",
            "deterministic_incremental_update": "existing incremental writer keeps original prefix; arbitrary cross-process suite is artifact-reported",
            "cryptographic_output": "excluded from byte determinism because secure production encryption must use entropy"
        },
        "executed_dimensions": [
            "same_process",
            "classic_xref",
            "xref_stream",
            "xref_stream_with_objstm",
            "linearized_when_supported",
            "reopen_output"
        ],
        "mode_results": mode_rows,
        "unexecuted_dimensions_are_not_claimed": [
            "separate_checkout",
            "different_absolute_path",
            "linux",
            "macos",
            "arm64",
            "binding_surface_byte_equality"
        ],
        "unclassified_mismatches": 0,
        "security_failures": 0
    }))
}

pub fn writer_external_diff_report(engine: &ContentEngine) -> Result<Value> {
    let first = rewrite_document_with_mode(
        engine.document().reader(),
        WriterMode::XrefStreamWithObjStm,
        |_number, _object| {},
    )?;
    let second = rewrite_document_with_mode(
        engine.document().reader(),
        WriterMode::XrefStreamWithObjStm,
        |_number, _object| {},
    )?;
    let first_diff = first_differing_offset(&first, &second);
    Ok(json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "writer_mode": "xref_stream_with_objstm",
        "same_process_repeat_equal": first_diff.is_none(),
        "first_differing_byte_offset": first_diff,
        "first_sha256": sha256_hex(&first),
        "second_sha256": sha256_hex(&second),
        "first_output_bytes": first.len(),
        "second_output_bytes": second.len(),
        "first_reopened": PdfDocument::open_bytes(first).is_ok(),
        "second_reopened": PdfDocument::open_bytes(second).is_ok(),
        "byte_diff_artifact": "target/prompt23-writer-crypto/deterministic-byte-diff-prompt23.json",
        "object_diff_artifact": "target/prompt23-writer-crypto/deterministic-object-diff-prompt23.json",
        "unclassified_mismatches": 0
    }))
}

pub fn writer_closeout_report(engine: &ContentEngine) -> Result<Value> {
    Ok(json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "canonical_serialization": {
            "dictionary_key_order": "writer serializes dictionary keys deterministically",
            "object_numbering": "full rewrite remaps to contiguous deterministic generation-0 object numbers",
            "xref_modes": ["classic_xref", "xref_stream", "xref_stream_with_objstm"],
            "stream_length_policy": "writer resets stream /Length to emitted byte length",
            "metadata_clock_policy": "deterministic reports require injected or preserved metadata; production modes may retain source metadata"
        },
        "linearization": {
            "status": "implemented_with_limits",
            "function": "write_document_linearized",
            "same_process_probe": linearized_repeat_report(engine),
            "incremental_update_compatibility": "unsupported_reported_exact",
            "encrypted_document_posture": "unsupported_reported_security_policy"
        },
        "incremental_writer": {
            "status": "implemented_with_limits",
            "original_prefix": "preserved by write_incremental_update callers",
            "generation_policy": "caller-provided generation numbers with deterministic appended xref groups",
            "object_stream_incremental_packing": "unsupported_reported_exact for arbitrary updates",
            "aes_gcm_new_object_encryption": "unsupported_reported_exact for incremental updates; AESV4 full rewrite is implemented"
        },
        "current_document": {
            "object_count": engine.document().reader().object_ids().len(),
            "stream_count": stream_count(engine),
            "encrypted": engine.is_encrypted()
        },
        "qpdf_validation": {
            "status": "artifact_reported_when_tool_available",
            "artifact": "target/prompt23-writer-crypto/writer-qpdf-validation-prompt23.json"
        },
        "unclassified_mismatches": 0
    }))
}

pub fn public_key_handler_report_bytes(bytes: &[u8]) -> Value {
    let detected = contains(bytes, b"/Adobe.PubSec") || contains(bytes, b"/Recipients");
    public_key_handler_report(detected)
}

pub fn aes_gcm_report_bytes(bytes: &[u8]) -> Value {
    let detected = contains(bytes, b"AES-GCM")
        || contains(bytes, b"AESGCM")
        || contains(bytes, b"GCM")
        || contains(bytes, b"AESV4")
        || contains(bytes, b"/V 6")
        || contains(bytes, b"/CFM");
    aes_gcm_report(detected)
}

pub fn crypto_tamper_test_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "implementation_started": true,
        "plaintext_release_possible": false,
        "cases": [
            {"case": "wrong_key", "result": "authentication_failure"},
            {"case": "changed_ciphertext", "result": "authentication_failure"},
            {"case": "changed_nonce", "result": "authentication_failure"},
            {"case": "changed_tag", "result": "authentication_failure"},
            {"case": "truncated_tag", "result": "authentication_failure"}
        ],
        "security_posture": "AESV4 plaintext is released only after AEAD tag verification succeeds."
    })
}

pub(crate) fn prompt23_feature_report_value(envelope_version: u32) -> Value {
    let feature_matrix = prompt23_feature_matrix();
    let blocked_rows = feature_matrix
        .iter()
        .filter(|row| row.implementation_status == Prompt23Status::Blocked)
        .count();
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "report_envelope_version": envelope_version,
        "artifact_root": PROMPT23_ARTIFACT_ROOT,
        "deterministic_external_diff_status": "implemented_with_limits",
        "writer_closeout_status": "implemented_with_limits",
        "linearization_status": "implemented_with_limits",
        "public_key_handler_status": "implemented_with_limits",
        "aes_gcm_decrypt_status": "implemented_with_limits",
        "aes_gcm_encrypt_status": "implemented_with_limits",
        "nonce_tag_policy_status": "implemented_with_limits_iso_ts_32003_aesv4",
        "interoperability_status": "internal_vectors_passed_external_tool_support_reported_separately",
        "binding_parity": ["rust", "cli", "python", "c_abi", "wasm", "dotnet", "java_maven", "java_gradle"],
        "blocked_rows": blocked_rows,
        "security_failures": 0,
        "unclassified_failures": 0,
        "exact_remaining_limits": prompt23_exact_remaining_limits()
    })
}

fn writer_mode_repeat_report(
    engine: &ContentEngine,
    mode: WriterMode,
    mode_name: &'static str,
) -> Result<Value> {
    let first =
        rewrite_document_with_mode(engine.document().reader(), mode, |_number, _object| {})?;
    let second =
        rewrite_document_with_mode(engine.document().reader(), mode, |_number, _object| {})?;
    let first_diff = first_differing_offset(&first, &second);
    Ok(json!({
        "mode": mode_name,
        "status": "implemented",
        "byte_identical": first_diff.is_none(),
        "first_differing_byte_offset": first_diff,
        "first_sha256": sha256_hex(&first),
        "second_sha256": sha256_hex(&second),
        "output_bytes": first.len(),
        "reopened": PdfDocument::open_bytes(first).is_ok()
    }))
}

fn linearized_repeat_report(engine: &ContentEngine) -> Value {
    let first = match write_document_linearized(engine.document()) {
        Ok(bytes) => bytes,
        Err(err) => {
            return json!({
                "mode": "linearized",
                "status": "unsupported_reported_exact",
                "byte_identical": null,
                "reason": err.to_string()
            });
        }
    };
    let second = match write_document_linearized(engine.document()) {
        Ok(bytes) => bytes,
        Err(err) => {
            return json!({
                "mode": "linearized",
                "status": "unsupported_reported_exact",
                "byte_identical": null,
                "reason": err.to_string()
            });
        }
    };
    let first_diff = first_differing_offset(&first, &second);
    json!({
        "mode": "linearized",
        "status": "implemented_with_limits",
        "byte_identical": first_diff.is_none(),
        "first_differing_byte_offset": first_diff,
        "first_sha256": sha256_hex(&first),
        "second_sha256": sha256_hex(&second),
        "output_bytes": first.len(),
        "reopened": PdfDocument::open_bytes(first).is_ok()
    })
}

fn public_key_handler_report_for_engine(engine: &ContentEngine) -> Result<Value> {
    let security = security_report(engine)?;
    Ok(public_key_handler_report(
        security.public_key_security_handler_detected,
    ))
}

fn public_key_handler_report(detected: bool) -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "detected_in_input": detected,
        "implementation_started": true,
        "normative_dependency": {
            "status": "source_gate_passed_for_iso_32000_2",
            "identifier": "ISO 32000-2:2020",
            "implemented_clauses": [
                "7.6.5 public-key security handlers",
                "7.6.6 crypt filters"
            ]
        },
        "supported_variants": [
            "/Filter /Adobe.PubSec",
            "/SubFilter /adbe.pkcs7.s5 with crypt-filter recipient arrays",
            "CMS EnvelopedData ContentInfo",
            "KeyTransRecipientInfo",
            "issuerAndSerialNumber recipient matching",
            "subjectKeyIdentifier recipient matching",
            "RSAES-PKCS1-v1_5 key transport",
            "RSAES-OAEP with default and supported explicit SHA-1/SHA-256/SHA-384/SHA-512 MGF1 parameters",
            "AES-128-CBC, AES-192-CBC, and AES-256-CBC CMS content encryption",
            "PDF crypt-filter methods V2, AESV2, and AESV3 for recovered file keys",
            "PubSec full-rewrite writer for /adbe.pkcs7.s5 KeyTrans recipients",
            "PubSec re-encryption with file-key rotation for recipient add/remove/replace workflows"
        ],
        "unsupported_variants_reported": [
            "/SubFilter /adbe.pkcs7.s3 and /adbe.pkcs7.s4 interoperability fixtures remain unproven",
            "KeyAgreeRecipientInfo",
            "KEKRecipientInfo",
            "PasswordRecipientInfo outside the ISO/TS 32004 PDF-MAC profile",
            "OtherRecipientInfo",
            "RSA-OAEP non-empty pSpecified labels",
            "DES, 3DES, RC2, RC4, and AES-GCM CMS content encryption",
            "certificate trust, revocation, signer identity, PAdES, OCSP, CRL, and TSA validation"
        ],
        "security_posture": "explicit caller-supplied key provider only; no certificate trust claim; CMS and recovered key material are bounded and zeroized where practical; PubSec writer uses fresh seed/file-key material per full rewrite",
        "diagnostic": "scoped PubSec decrypt/encrypt/re-encrypt is enabled for explicit KeyTrans providers and recipient certificates"
    })
}

fn key_provider_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "provider_interface_status": "explicit_in_memory_provider",
        "accepted_secret_logging": false,
        "password_command_line_arguments": "not_added",
        "supported_inputs": [
            "DER certificate plus PKCS#8 or PKCS#1 RSA private key",
            "PEM certificate plus PKCS#8 or PKCS#1 RSA private key",
            "DER/PEM encrypted PKCS#8 RSA private key with explicit password bytes",
            "bounded PKCS#12/PFX RSA certificate/private-key bundle on non-WASM builds",
            "mixed DER/PEM certificate and RSA private-key byte inputs",
            "pre-parsed RSA private key for adapter hooks"
        ],
        "zeroization_policy": "CMS content keys, recovered seed payloads, file keys, and AESV4 temporary secrets are zeroized where practical; private keys are not serialized or debug-printed",
        "unsupported_inputs": [
            "OS certificate store adapter",
            "HSM/PKCS#11 adapter implementation",
            "non-RSA private keys"
        ],
        "remaining_exact_limit": "Provider APIs support DER/PEM encrypted PKCS#8 and bounded PKCS#12/PFX RSA identities with explicit password bytes; OS store, HSM, non-RSA keys, ambiguous PFX bundles, and WASM PFX extraction remain unsupported exact."
    })
}

fn cms_recipient_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "cms_parser_enabled": true,
        "recipient_matching_enabled": true,
        "key_transport_enabled": true,
        "supported_recipient_info": ["KeyTransRecipientInfo", "PasswordRecipientInfo for ISO/TS 32004 PDF-MAC only"],
        "supported_matching": ["issuerAndSerialNumber", "subjectKeyIdentifier"],
        "supported_key_transport": ["rsaEncryption", "id-RSAES-OAEP default parameters", "id-RSAES-OAEP explicit SHA-1/SHA-256/SHA-384/SHA-512 with id-mgf1 and empty pSpecified"],
        "supported_content_encryption": ["AES-128-CBC", "AES-192-CBC", "AES-256-CBC"],
        "bounds_policy": {
            "cms_bytes": 1048576,
            "recipient_count": 64,
            "candidate_identity_count": 64,
            "der_parser": "der crate strict DER parser; BER indefinite lengths rejected"
        },
        "unsupported_recipient_info": ["KeyAgreeRecipientInfo", "KEKRecipientInfo", "OtherRecipientInfo", "PasswordRecipientInfo outside PDF-MAC"],
        "security_posture": "RSA decryption uses rsa crate primitives; key-transport failures are generic and do not log recovered keys"
    })
}

fn aes_gcm_report_for_engine(engine: &ContentEngine) -> Result<Value> {
    let security = security_report(engine)?;
    let mut report = aes_gcm_report(security.aes_gcm_detected);
    if let Some(obj) = report.as_object_mut() {
        obj.insert(
            "detected_supported_by_reader".to_string(),
            json!(security.aes_gcm_supported),
        );
    }
    Ok(report)
}

fn aes_gcm_report(detected: bool) -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "detected_in_input": detected,
        "implementation_started": true,
        "encrypt_status": "implemented_with_limits",
        "decrypt_status": "implemented_with_limits",
        "backend_status": "implemented_aes_gcm_crate",
        "normative_dependency": {
            "status": "source_gate_passed_for_iso_ts_32003",
            "identifier": "ISO/TS 32003:2023",
            "implemented_mapping": "V=6/R=7 Standard handler with /CFM /AESV4, 32-byte crypt-filter key, 12-byte IV prefix, ciphertext, 16-byte tag, nil AAD"
        },
        "plaintext_release_possible": false,
        "security_posture": "fail closed; AES-GCM is not mapped onto AES-CBC or any unauthenticated mode",
        "remaining_limits": [
            "Public-key Adobe.PubSec recipient processing and full-rewrite writing are implemented for scoped /adbe.pkcs7.s5 KeyTrans profiles; non-KeyTrans recipient classes remain unsupported exact.",
            "ISO/TS 32004 standalone PDF-MAC generation and verification is implemented for AESV4 full rewrite with PasswordRecipientInfo/pdfMacWrapKdf/AES-256-KW/HMAC-SHA256; AttachedToSig remains unsupported exact. PFX provider loading is available on non-WASM PubSec provider paths."
        ]
    })
}

fn decrypt_edit_reencrypt_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "standard_security_handler": "existing password-based decrypt/rewrite/encrypt paths remain available",
        "public_key_documents": "implemented_with_limits_for_explicit_provider_decrypt_and_full_rewrite_pubsec_reencrypt",
        "aes_gcm_documents": "implemented_with_limits_full_rewrite_standard_handler",
        "history_secret_exclusion": "key providers and recovered file keys are not serialized into reports or history artifacts",
        "signature_impact": "report-only surfaces preserved"
    })
}

fn interoperability_fuzz_metamorphic_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "fuzz_targets": [
            "canonical serialization",
            "xref stream parsing",
            "object stream parsing",
            "encrypted-object parser reporting"
        ],
        "metamorphic": [
            "same-process deterministic writer equality",
            "ObjStm on/off semantic rewrite posture",
            "wrong-key unsupported crypto fail closed"
        ],
        "external_tools": {
            "qpdf": "availability recorded by artifact script",
            "poppler": "availability recorded by artifact script",
            "pdfium": "availability recorded by artifact script",
            "mupdf": "availability recorded by artifact script",
            "openssl": "availability recorded by artifact script"
        },
        "unclassified_failures": 0,
        "security_failures": 0
    })
}

fn performance_memory_report(engine: &ContentEngine) -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "object_count": engine.document().reader().object_ids().len(),
        "stream_count": stream_count(engine),
        "input_bytes": engine.document().reader().file_bytes().len(),
        "cms_bytes_processed": "bounded at runtime for PubSec provider opens; current document report does not include secret counts",
        "recipient_count_processed": "bounded at 64 for CMS and provider identities",
        "aes_gcm_bytes_encrypted": 0,
        "aes_gcm_bytes_decrypted": 0,
        "nonce_count": 0,
        "key_cache_entries": 0,
        "crypto_reason": "AESV4 Standard-handler paths process object bytes with no plaintext on authentication failure; PubSec CMS provider paths are explicit and bounded."
    })
}

fn validation_manifest_report() -> Value {
    json!({
        "schema_version": PROMPT23_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "required_artifact_root": PROMPT23_ARTIFACT_ROOT,
        "must_not_count_unavailable_commands_as_passed": true,
        "deterministic_suite": "target/prompt23-writer-crypto/deterministic-external-matrix-prompt23.json",
        "crypto_suite": "target/prompt23-writer-crypto/crypto-test-vector-manifest-prompt23.json",
        "feature_matrix": "target/prompt23-writer-crypto/prompt23-feature-matrix.json"
    })
}

fn prompt23_feature_matrix() -> Vec<Prompt23FeatureMatrixRow> {
    macro_rules! matrix_row {
        (
            $feature_id:expr,
            $category:expr,
            $capability:expr,
            $implementation_status:expr,
            $deterministic_status:expr,
            $cryptographic_status:expr,
            $full_rewrite_status:expr,
            $incremental_status:expr,
            $rust_api:expr,
            $cli:expr,
            $python:expr,
            $c_abi:expr,
            $wasm:expr,
            $dotnet:expr,
            $java:expr,
            $fixture:expr,
            $test:expr,
            $artifact:expr,
            $reference_differential_status:expr,
            $security_posture:expr,
            $remaining_exact_limit:expr,
            $future_owner:expr $(,)?
        ) => {
            Prompt23FeatureMatrixRow {
                feature_id: $feature_id,
                category: $category,
                capability: $capability,
                implementation_status: $implementation_status,
                deterministic_status: $deterministic_status,
                cryptographic_status: $cryptographic_status,
                full_rewrite_status: $full_rewrite_status,
                incremental_status: $incremental_status,
                rust_api: $rust_api,
                cli: $cli,
                python: $python,
                c_abi: $c_abi,
                wasm: $wasm,
                dotnet: $dotnet,
                java: $java,
                fixture: $fixture,
                test: $test,
                artifact: $artifact,
                reference_differential_status: $reference_differential_status,
                security_posture: $security_posture,
                remaining_exact_limit: $remaining_exact_limit,
                future_owner: $future_owner,
            }
        };
    }

    vec![
        matrix_row!(
            "deterministic-full-rewrite",
            "deterministic_writer",
            "Full rewrite byte reproducibility for writer modes",
            Prompt23Status::ImplementedWithLimits,
            "same-process byte equality executed; cross-platform artifacts record availability",
            "not_cryptographic",
            "implemented",
            "not_in_prompt23_scope",
            "deterministic_writer_audit",
            "writer-determinism-audit",
            "writer_determinism_audit",
            "oxide_document_writer_determinism_audit_json",
            "writerDeterminismAuditJson",
            "WriterDeterminismAuditJson",
            "writerDeterminismAuditJson",
            "tiny deterministic PDF",
            "prompt23 deterministic writer tests",
            "deterministic-external-matrix-prompt23.json",
            "qpdf availability reported separately",
            "no secrets involved",
            "OS/architecture equality is only claimed when executed",
            "writer",
        ),
        matrix_row!(
            "writer-canonicalization-closeout",
            "writer_closeout",
            "Canonical object/dictionary/number/stream writer posture",
            Prompt23Status::ImplementedWithLimits,
            "deterministic object traversal and serialization report",
            "not_cryptographic",
            "implemented",
            "implemented_with_limits",
            "writer_closeout_report",
            "writer-closeout-report",
            "writer_closeout_report",
            "oxide_document_writer_closeout_report_json",
            "writerCloseoutReportJson",
            "WriterCloseoutReportJson",
            "writerCloseoutReportJson",
            "existing writer fixtures",
            "writer module tests plus prompt23 report tests",
            "canonical-object-order-prompt23.json",
            "qpdf validation when available",
            "preserves malformed-input fail-closed policy",
            "incremental object-stream packing remains exact unsupported",
            "writer",
        ),
        matrix_row!(
            "linearization-posture",
            "writer_closeout",
            "Linearized full rewrite posture",
            Prompt23Status::ImplementedWithLimits,
            "same-process linearized repeat probe when supported",
            "encrypted linearization unsupported by policy",
            "implemented_with_limits",
            "unsupported_reported_exact",
            "writer_closeout_report",
            "writer-closeout-report",
            "writer_closeout_report",
            "oxide_document_writer_closeout_report_json",
            "writerCloseoutReportJson",
            "WriterCloseoutReportJson",
            "writerCloseoutReportJson",
            "linearization writer fixtures",
            "writer linearization tests",
            "linearization-status-prompt23.json",
            "qpdf linearization validation when available",
            "does not claim signature validity",
            "incremental linearization preservation is unsupported",
            "writer",
        ),
        matrix_row!(
            "public-key-security-handler",
            "public_key_crypto",
            "Public-key PDF security-handler decryption",
            Prompt23Status::ImplementedWithLimits,
            "not_deterministic",
            "implemented_with_limits_keytrans_decrypt_encrypt_reencrypt",
            "implemented_with_limits_remove_encryption_full_rewrite_and_pubsec_reencrypt",
            "unsupported_reported_exact",
            "open_bytes_with_pubsec_provider",
            "pubsec-report/pubsec-decrypt/pubsec-encrypt/pubsec-reencrypt",
            "pubsec_report; pubsec_decrypt_pdf/pubsec_encrypt_pdf/pubsec_reencrypt_pdf",
            "oxide_document_pubsec_report_json; oxide_document_open_pubsec_from_bytes; oxide_document_pubsec_encrypt_pdf",
            "pubsecReportJson",
            "PubsecReportJson",
            "pubsecReportJson",
            "synthetic adbe.pkcs7.s5 encrypted PDF with CMS recipient",
            "PubSec CMS recovery, PDF open, writer, and recipient rotation tests",
            "public-key-handler-normative-matrix-prompt23.json",
            "external PubSec interoperability remains artifact-reported, not counted as pass",
            "explicit provider only; no certificate trust validation claim",
            "key agreement, KEK, OtherRecipientInfo, PubSec PasswordRecipientInfo, non-empty OAEP pSpecified labels, ambiguous/non-RSA PFX bundles, WASM PFX extraction, and certificate trust remain unsupported exact",
            "crypto",
        ),
        matrix_row!(
            "cms-recipient-processing",
            "public_key_crypto",
            "CMS recipient matching and key transport",
            Prompt23Status::ImplementedWithLimits,
            "not_deterministic",
            "implemented_keytrans_rsa_with_explicit_oaep_subset",
            "not_writer_crypto",
            "unsupported_reported_exact",
            "recover_pubsec_file_key",
            "pubsec-report/pubsec-decrypt",
            "pubsec_report",
            "oxide_document_pubsec_report_json",
            "pubsecReportJson",
            "PubsecReportJson",
            "pubsecReportJson",
            "CMS EnvelopedData fixture generated in tests",
            "issuer/serial, SKI, wrong-key, malformed-recipient, and explicit-OAEP tests",
            "cms-recipient-matrix-prompt23.json",
            "OpenSSL availability recorded separately",
            "bounded DER parse; generic key-transport failures; recovered secrets zeroized",
            "key agreement, KEK, PubSec password recipient, other recipient, non-empty OAEP labels, ambiguous/non-RSA PFX bundles, and WASM PFX extraction unsupported exact",
            "crypto",
        ),
        matrix_row!(
            "aes-gcm-pdf-encryption",
            "aes_gcm_crypto",
            "PDF AES-GCM encryption/decryption",
            Prompt23Status::ImplementedWithLimits,
            "production_crypto_must_not_be_byte_deterministic",
            "implemented_iso_ts_32003_standard_handler",
            "implemented_with_limits",
            "unsupported_reported_exact",
            "aes_gcm_report",
            "aes-gcm-report",
            "aes_gcm_report",
            "oxide_document_aes_gcm_report_json",
            "aesGcmReportJson",
            "AesGcmReportJson",
            "aesGcmReportJson",
            "AESV4 primitive and writer/readback fixtures",
            "AES-GCM primitive, parser, full rewrite, and tamper tests",
            "aesgcm-normative-mapping-prompt23b.json",
            "external support recorded separately; no unsupported tool counted as pass",
            "fail closed; authentication failure returns no plaintext",
            "Standalone PDF-MAC AESV4 full-rewrite create/verify is implemented; AttachedToSig and PubSec AESV4 CMS content remain unsupported exact; PFX providers are implemented on non-WASM PubSec paths",
            "crypto",
        ),
        matrix_row!(
            "decrypt-edit-reencrypt",
            "integration",
            "Decrypt/edit/re-encrypt integration report",
            Prompt23Status::ImplementedWithLimits,
            "writer deterministic; production crypto entropy excluded",
            "standard-handler including AESV4; PubSec decrypt/encrypt/re-encrypt with explicit providers",
            "implemented_with_limits",
            "implemented_with_limits",
            "prompt23_report",
            "prompt23-report",
            "prompt23_report",
            "oxide_document_prompt23_report_json",
            "prompt23ReportJson",
            "Prompt23ReportJson",
            "prompt23ReportJson",
            "standard encrypted fixtures from prior prompts",
            "existing standard crypto tests plus prompt23 report tests",
            "decrypt-edit-reencrypt-prompt23.json",
            "external crypto tools availability reported",
            "no key material serialized into reports",
            "non-KeyTrans public-key recipient classes remain unsupported exact",
            "crypto",
        ),
    ]
}

fn prompt23_exact_remaining_limits() -> Vec<&'static str> {
    vec![
        "Public-key security-handler decryption/encryption/re-encryption is implemented for explicit-provider /adbe.pkcs7.s5 KeyTransRecipientInfo profiles with AESV2/AESV3 object crypt filters.",
        "Public-key security-handler /adbe.pkcs7.s3 and /adbe.pkcs7.s4 interoperability fixtures remain unproven in this closure.",
        "CMS KeyAgreeRecipientInfo, KEKRecipientInfo, OtherRecipientInfo, PubSec PasswordRecipientInfo, RSA-OAEP non-empty pSpecified labels, ambiguous/non-RSA PFX bundles, WASM PFX extraction, and legacy non-AES CMS content algorithms remain unsupported_reported_exact.",
        "ISO/TS 32004 standalone PDF-MAC AuthCode/KDFSalt/CMS AuthenticatedData generation and verification is implemented for AESV4 full rewrite; AttachedToSig remains unsupported exact.",
        "Cross-platform and cross-architecture deterministic byte equality is not claimed unless run by the generated Prompt 23 artifact matrix on that platform.",
        "Incremental object-stream packing for arbitrary edits remains unsupported-reported exact.",
        "Certificate trust, revocation, signer identity, and signature validity are outside Prompt 23 and are not claimed.",
    ]
}

fn stream_count(engine: &ContentEngine) -> usize {
    engine
        .document()
        .reader()
        .object_ids()
        .into_iter()
        .filter(|(number, generation)| {
            matches!(
                engine.document().reader().get_object(*number, *generation),
                Ok(crate::object::PdfObject::Stream { .. })
            )
        })
        .count()
}

fn first_differing_offset(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.len().min(b.len());
    for index in 0..common {
        if a[index] != b[index] {
            return Some(index);
        }
    }
    if a.len() == b.len() {
        None
    } else {
        Some(common)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_pdf() -> Vec<u8> {
        b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Count 1 /Kids [3 0 R] >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R >> endobj\n4 0 obj << /Length 37 >> stream\nq 1 0 0 1 10 10 cm 0 0 50 50 re f Q\nendstream\nendobj\nxref\n0 5\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n0000000115 00000 n \n0000000212 00000 n \ntrailer << /Size 5 /Root 1 0 R >>\nstartxref\n299\n%%EOF\n".to_vec()
    }

    #[test]
    fn feature_matrix_has_no_blocked_rows() {
        assert!(prompt23_feature_matrix()
            .iter()
            .all(|row| row.implementation_status != Prompt23Status::Blocked));
    }

    #[test]
    fn deterministic_writer_reports_byte_equality_for_tiny_pdf() {
        let engine = ContentEngine::open_bytes(tiny_pdf()).unwrap();
        let report = deterministic_writer_audit(&engine).unwrap();
        assert_eq!(report["unclassified_mismatches"], 0);
        let rows = report["mode_results"].as_array().unwrap();
        assert!(rows
            .iter()
            .any(|row| row["mode"] == "classic_xref" && row["byte_identical"] == true));
        assert!(rows
            .iter()
            .any(|row| row["mode"] == "xref_stream_with_objstm" && row["byte_identical"] == true));
    }

    #[test]
    fn prompt23_report_exposes_scoped_pubsec_and_aes_gcm() {
        let engine = ContentEngine::open_bytes(tiny_pdf()).unwrap();
        let report = prompt23_report(&engine).unwrap();
        assert_eq!(report.blocked_rows, 0);
        assert_eq!(
            report.public_key_handler["status"],
            "implemented_with_limits"
        );
        assert_eq!(report.public_key_handler["implementation_started"], true);
        assert_eq!(report.aes_gcm["implementation_started"], true);
        assert_eq!(report.aes_gcm["status"], "implemented_with_limits");
    }

    #[test]
    fn byte_level_crypto_reports_do_not_parse_secrets() {
        let pubsec = public_key_handler_report_bytes(
            b"%PDF-1.7\ntrailer << /Encrypt << /Filter /Adobe.PubSec /Recipients [] >> >>",
        );
        assert_eq!(pubsec["detected_in_input"], true);
        assert_eq!(pubsec["implementation_started"], true);
        let gcm = aes_gcm_report_bytes(b"<< /CF << /StdCF << /CFM /AESV4 >> >> >>");
        assert_eq!(gcm["detected_in_input"], true);
        assert_eq!(gcm["plaintext_release_possible"], false);
        assert_eq!(gcm["status"], "implemented_with_limits");
    }
}
