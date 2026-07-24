//! Digital signature creation and verification (`pdfsig`-equivalent).
//!
//! Signing appends a signature field and detached CMS `SignedData` in an
//! incremental-update revision, preserving the original file bytes as an exact
//! prefix. Verification of each signature field:
//!   1. reads the `/ByteRange` and hashes those exact original file bytes,
//!   2. parses `/Contents` as a PKCS#7 / CMS `SignedData` (RFC 5652),
//!   3. verifies the signer's RSA signature over the signed attributes (or the
//!      content digest directly), checking the `messageDigest` signed-attribute
//!      against the byte-range digest, and
//!   4. reports the signer certificate's details and whether the signature
//!      covers the whole file or the document was modified after signing.
//!
//! # Honest scope
//!
//! "Valid" here means **cryptographically valid**: the signature math checks
//! out against the signer certificate's public key and the signed digest
//! matches the `/ByteRange` bytes. Prompt 24 additionally provides explicit
//! anchor-based PKIX validation, caller-supplied and opt-in controlled
//! AIA/OCSP/CRL evidence, and replayable validated evidence bundles. RSA
//! PKCS#1 v1.5, RSA-PSS, and the supported ECDSA curves are verified with exact
//! algorithm parameters. Prompt 25 extends the same validation pipeline with
//! RFC 3161 signature timestamp validation, DSS/VRI evidence replay, B-T/B-LT
//! level reporting, and signature-preserving edit policy hooks without creating
//! a second signature engine.

use cms::builder::{create_signing_time_attribute, SignedDataBuilder, SignerInfoBuilder};
use cms::cert::{x509::Certificate, CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::ContentInfo;
use cms::signed_data::{EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo};
use const_oid::ObjectIdentifier;
use der::asn1::{GeneralizedTime, SetOfVec};
use der::{Decode, DecodePem, Encode, Sequence};
use pkix_path::{DefaultVerifier, TrustAnchor, ValidationPolicy};
use pkix_path_builder::{CertPool, PathBuilderConfig};
use pkix_revocation::{discover_crl_signer, CrlChecker, OcspChecker, RevocationChecker};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use spki::AlgorithmIdentifierOwned;
use x509_cert::attr::{Attribute, AttributeValue};
use x509_cert::crl::CertificateList;
use x509_cert::ext::pkix::crl::IssuingDistributionPoint;
use x509_cert::ext::pkix::name::{DistributionPointName, GeneralName};
use x509_cert::ext::pkix::{
    AuthorityInfoAccessSyntax, CrlDistributionPoints, ExtendedKeyUsage, KeyUsage,
};
use x509_ocsp::builder::OcspRequestBuilder;
use x509_ocsp::ext::Nonce as OcspNonce;
use x509_ocsp::{BasicOcspResponse, OcspResponse, OcspResponseStatus, Request as OcspRequest};

use crate::cancel::CancelToken;
use crate::document::PdfDocument;
use crate::error::{OxideError, Result};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::signature_evidence::{
    EvidenceBundle, EvidenceKind, EvidenceRecord, EvidenceStore, OcspNoncePolicy, RetrievalKind,
    RetrievalMethod, RetrievalPolicy, RetrievalSession, RetrievalTrace,
};
use crate::writer::{serialize_object, write_incremental_update_raw, RawIncrementalObject};

// OIDs we care about.
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_SIGNING_CERTIFICATE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.12");
const OID_SIGNING_CERTIFICATE_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const OID_CMS_ALGORITHM_PROTECTION: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.52");
const OID_RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
const OID_MGF1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.8");
const OID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const OID_ECDSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");
const OID_ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_ID_CT_TST_INFO: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const OID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const OID_SIGNATURE_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
const OID_AD_OCSP: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.48.1");
const OID_AD_CA_ISSUERS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.48.2");
const OID_KP_TIME_STAMPING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");
const OID_OCSP_BASIC_RESPONSE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.48.1.1");
const OID_DELTA_CRL_INDICATOR: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.27");
const OID_ISSUING_DISTRIBUTION_POINT: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.28");
const MAX_AIA_RETRIEVED_CERTIFICATES: usize = 32;

const BYTE_RANGE_PLACEHOLDER: &[u8] = b"[9999999999 9999999999 9999999999 9999999999]";
const MAX_BYTE_RANGE_FIELD: u64 = 9_999_999_999;
const DEFAULT_CONTENTS_RESERVED_BYTES: usize = 16 * 1024;
pub const PROMPT24_SIGNATURE_VALIDATION_SCHEMA_VERSION: &str =
    "prompt24.certificate-trust-pades-ocsp-crl-validation.v1";
pub const PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION: &str =
    "prompt25.tsa-dss-ltv-mdp-signature-edits.v1";

/// Additive public capability report for the Prompt 24 validation pipeline.
///
/// This is deliberately a capability report, not a release verdict. It lets
/// every SDK binding expose the same supported boundary without promoting
/// unavailable platform adapters or Prompt 25 functionality to success.
pub(crate) fn prompt24_feature_report_value(envelope_version: u32) -> Value {
    json!({
        "schema_version": PROMPT24_SIGNATURE_VALIDATION_SCHEMA_VERSION,
        "status": "implemented_with_limits_not_release_attested",
        "report_envelope_version": envelope_version,
        "pipeline": [
            "pdf_signature_discovery_and_revision_analysis",
            "strict_byterange_and_contents_source_binding",
            "bounded_detached_cms_signeddata_validation",
            "exact_signer_certificate_resolution",
            "signed_attribute_and_signature_algorithm_policy",
            "bounded_pkix_path_build_and_validation",
            "supplied_or_opt_in_retrieved_revocation_evidence",
            "pades_baseline_structural_and_policy_result"
        ],
        "pdf_signature_discovery": "implemented",
        "cms_signeddata": "implemented_with_supported_algorithm_limits",
        "signer_certificate_resolution": "implemented_exact_match_no_arbitrary_fallback",
        "signature_algorithms": {
            "rsa_pkcs1v15": "implemented_policy_controlled",
            "rsa_pss": "implemented_policy_controlled",
            "ecdsa_p256": "implemented_policy_controlled",
            "ecdsa_p384": "implemented_policy_controlled",
            "sha1": "parsed_but_policy_forbidden_by_default",
            "other_algorithms": "unsupported_reported_exact"
        },
        "pkix": {
            "explicit_trust_anchors": "implemented",
            "untrusted_intermediates": "implemented",
            "distrust_overlay": "implemented",
            "bounded_deterministic_path_building": "implemented",
            "rfc5280_style_validation": "implemented_with_dependency_supported_extension_limits",
            "platform_trust_store": "not_implicit_optional_provider_later"
        },
        "retrieval": {
            "default": "offline",
            "http_https": "implemented_opt_in_bounded_ssrf_checked",
            "aia_issuer_certificates": "implemented_untrusted_intermediates_only",
            "ocsp": "implemented_with_signature_authorization_freshness_and_nonce_policy",
            "crl": "implemented_with_signature_scope_delta_and_indirect_posture",
            "persistent_evidence_cache": "implemented_native_explicit_path",
            "evidence_export_import_replay": "implemented",
            "wasm_online": "unsupported_without_explicit_host_transport"
        },
        "pades": {
            "baseline": "implemented_with_explicit_path_and_revocation_policy_result",
            "signature_timestamps": "implemented_prompt25_rfc3161_signature_timestamp_validation",
            "dss_vri_ltv": "implemented_prompt25_dss_vri_replay_and_blt_evidence_status",
            "docmdp_fieldmdp_enforcement": "implemented_prompt25_structural_policy_for_supported_signature_preserving_edits",
            "archive_timestamps_blta": "not_claimed_without_archive_timestamp_validation"
        },
        "binding_runtime_surfaces": {
            "rust": "implemented",
            "cli": "implemented",
            "python": "implemented_owned_options",
            "c_abi": "implemented_owned_options",
            "dotnet": "implemented_safehandle_options",
            "java": "implemented_autocloseable_options",
            "wasm": "implemented_offline_supplied_evidence_only"
        },
        "release_attestation": {
            "full_workspace_and_historical_gates": "not_yet_attested",
            "independent_interoperability": "not_yet_attested",
            "fuzz_and_performance_campaign": "not_yet_attested",
            "final_closure_commit": "absent"
        },
        "exact_remaining_limits": [
            "Prompt 25 does not claim B-LTA/archive timestamp validation, general public signing, or viewer-specific certification UI behavior.",
            "Platform trust stores are never implicit and remain an optional provider integration.",
            "Only the supported signature algorithms are accepted; all others are explicit unsupported results.",
            "Online retrieval is unavailable to WASM without a host-controlled transport.",
            "This capability report does not attest final Prompt 25 release gates."
        ]
    })
}

/// RSA signing identity used by [`sign_document`].
///
/// The first certificate is the signer certificate; remaining certificates are
/// embedded as chain material for validators. The current writer applies RSA
/// PKCS#1 v1.5 with SHA-256.
#[derive(Clone)]
pub struct PdfSigner {
    private_key: RsaPrivateKey,
    certificates: Vec<Certificate>,
}

impl PdfSigner {
    /// Build a signer from DER-encoded RSA private key and X.509 certificates.
    ///
    /// `private_key_der` may be PKCS#8 or PKCS#1. `certificate_der` is the
    /// signer certificate; `chain_der` are optional issuer certificates.
    pub fn from_der(
        private_key_der: &[u8],
        certificate_der: &[u8],
        chain_der: &[&[u8]],
    ) -> Result<Self> {
        let private_key = RsaPrivateKey::from_pkcs8_der(private_key_der)
            .or_else(|_| RsaPrivateKey::from_pkcs1_der(private_key_der))
            .map_err(|e| OxideError::UnsupportedFeature(format!("signature RSA key: {e}")))?;
        let mut certificates = Vec::with_capacity(chain_der.len() + 1);
        certificates.push(
            Certificate::from_der(certificate_der)
                .map_err(|e| OxideError::MalformedPdf(format!("signature certificate: {e}")))?,
        );
        for cert in chain_der {
            certificates.push(Certificate::from_der(cert).map_err(|e| {
                OxideError::MalformedPdf(format!("signature chain certificate: {e}"))
            })?);
        }
        Ok(Self {
            private_key,
            certificates,
        })
    }

    /// Build a signer from PEM-encoded RSA private key and X.509 certificates.
    pub fn from_pem(
        private_key_pem: &str,
        certificate_pem: &str,
        chain_pem: &[&str],
    ) -> Result<Self> {
        let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
            .map_err(|e| OxideError::UnsupportedFeature(format!("signature RSA key: {e}")))?;
        let mut certificates = Vec::with_capacity(chain_pem.len() + 1);
        certificates.push(
            Certificate::from_pem(certificate_pem.as_bytes())
                .map_err(|e| OxideError::MalformedPdf(format!("signature certificate PEM: {e}")))?,
        );
        for cert in chain_pem {
            certificates.push(Certificate::from_pem(cert.as_bytes()).map_err(|e| {
                OxideError::MalformedPdf(format!("signature chain certificate PEM: {e}"))
            })?);
        }
        Ok(Self {
            private_key,
            certificates,
        })
    }

    pub fn signer_certificate(&self) -> &Certificate {
        &self.certificates[0]
    }

    /// DER encoding of the signer certificate, e.g. to pin it as a trust anchor
    /// in [`VerifyOptions`].
    pub fn signer_certificate_der(&self) -> Result<Vec<u8>> {
        self.certificates[0]
            .to_der()
            .map_err(|e| OxideError::MalformedPdf(format!("signer certificate encode: {e}")))
    }
}

/// Options for [`sign_document`].
#[derive(Debug, Clone)]
pub struct SignatureOptions {
    /// Signature field name (`/T`). Defaults to `Sig1`.
    pub field_name: String,
    /// 1-based page for the visible widget. Defaults to page 1.
    pub page: usize,
    /// Widget rectangle `[x0, y0, x1, y1]`. `None` creates an invisible field.
    pub rect: Option<[f64; 4]>,
    pub signer_name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact_info: Option<String>,
    /// Raw PDF date string for `/M`, e.g. `D:20260622000000Z`.
    pub signing_time: Option<String>,
    /// DER-encoded RFC 3161 `TimeStampToken` (`ContentInfo`) to embed as the
    /// CMS `signatureTimeStampToken` unsigned attribute.
    ///
    /// The core signer does not contact a TSA. Callers that need PAdES-B-T
    /// obtain a token from their TSA/policy layer and pass the DER token here.
    /// Verification validates the token imprint and TSA signer/path when the
    /// caller supplies the applicable Prompt 25 trust and revocation policy.
    pub timestamp_token_der: Option<Vec<u8>>,
    /// Reserved CMS size in bytes. The DER CMS must fit in this placeholder.
    pub contents_reserved_bytes: usize,
}

impl Default for SignatureOptions {
    fn default() -> Self {
        Self {
            field_name: "Sig1".to_string(),
            page: 1,
            rect: None,
            signer_name: None,
            reason: None,
            location: None,
            contact_info: None,
            signing_time: None,
            timestamp_token_der: None,
            contents_reserved_bytes: DEFAULT_CONTENTS_RESERVED_BYTES,
        }
    }
}

/// Overall cryptographic verdict for one signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureValidity {
    /// Signature math verifies and the signed digest matches the byte ranges.
    Valid,
    /// Parsed fine but the signature/digest does not verify (tampering or
    /// wrong key) — the document content within the signed ranges changed, or
    /// the signature is corrupt.
    Invalid,
    /// The signature algorithm is not supported (e.g. ECDSA, RSA-PSS).
    UnsupportedAlgorithm,
    /// The signature dictionary or CMS blob could not be parsed.
    Error,
}

/// How much of the file the signature covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// The `/ByteRange` (its two ranges plus the `/Contents` gap) spans the
    /// entire file — nothing was appended after signing.
    WholeFile,
    /// Bytes exist after the signed ranges — an incremental update was appended
    /// after this signature, i.e. the document was modified after signing.
    ModifiedAfterSigning,
}

/// Options controlling signature verification, most importantly the set of
/// trust anchors the verifier accepts.
///
/// With no anchors configured, **no signature is reported as trusted** — only
/// its cryptographic integrity and coverage are established. This is the safe
/// default: a cryptographically valid signature from a self-signed or unknown
/// certificate must never be conflated with an authentic, trusted signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredTrustAnchor {
    der: Vec<u8>,
    /// Stable SHA-256 fingerprint of the canonical DER certificate.
    pub fingerprint_sha256: String,
    /// Caller-supplied provenance such as a file path, KeyStore alias, or
    /// platform provider identifier. This is metadata only; it does not grant
    /// trust on its own.
    pub origin: String,
    /// Optional caller-defined trust purpose recorded alongside the anchor.
    pub purpose: Option<String>,
}

impl ConfiguredTrustAnchor {
    /// Canonical DER bytes owned by this explicit anchor.
    pub fn der(&self) -> &[u8] {
        &self.der
    }
}

/// Explicit, deterministic trust-anchor collection for signature validation.
///
/// The store never treats embedded CMS, AIA, or intermediate certificates as
/// anchors. Use [`VerifyOptions::with_trust_store`] to attach it to a
/// validation invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    anchors: Vec<ConfiguredTrustAnchor>,
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse and add one DER certificate as an explicit trust anchor.
    pub fn add_der(
        &mut self,
        der: &[u8],
        origin: impl Into<String>,
        purpose: Option<String>,
    ) -> Result<()> {
        let certificate = Certificate::from_der(der).map_err(|error| {
            OxideError::invalid_input(format!("trust-anchor certificate DER: {error}"))
        })?;
        let canonical_der = certificate.to_der().map_err(|error| {
            OxideError::invalid_input(format!("trust-anchor certificate DER encode: {error}"))
        })?;
        let fingerprint_sha256 = hex_upper(&Sha256::digest(&canonical_der));
        if self
            .anchors
            .iter()
            .any(|anchor| anchor.fingerprint_sha256 == fingerprint_sha256)
        {
            return Ok(());
        }
        self.anchors.push(ConfiguredTrustAnchor {
            der: canonical_der,
            fingerprint_sha256,
            origin: origin.into(),
            purpose,
        });
        self.anchors.sort_by(|left, right| {
            left.fingerprint_sha256
                .cmp(&right.fingerprint_sha256)
                .then_with(|| left.origin.cmp(&right.origin))
        });
        Ok(())
    }

    /// Parse and add one PEM certificate as an explicit trust anchor.
    pub fn add_pem(
        &mut self,
        pem: &[u8],
        origin: impl Into<String>,
        purpose: Option<String>,
    ) -> Result<()> {
        let certificate = Certificate::from_pem(pem).map_err(|error| {
            OxideError::invalid_input(format!("trust-anchor certificate PEM: {error}"))
        })?;
        let der = certificate.to_der().map_err(|error| {
            OxideError::invalid_input(format!("trust-anchor certificate PEM encode: {error}"))
        })?;
        self.add_der(&der, origin, purpose)
    }

    pub fn anchors(&self) -> &[ConfiguredTrustAnchor] {
        &self.anchors
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }
}

/// Explicit untrusted intermediate-certificate collection. Its entries are
/// path-building candidates only and are never promoted to trust anchors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntermediateStore {
    certificates_der: Vec<Vec<u8>>,
}

impl IntermediateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_der(&mut self, der: &[u8]) -> Result<()> {
        let certificate = Certificate::from_der(der).map_err(|error| {
            OxideError::invalid_input(format!("intermediate certificate DER: {error}"))
        })?;
        let canonical_der = certificate.to_der().map_err(|error| {
            OxideError::invalid_input(format!("intermediate certificate DER encode: {error}"))
        })?;
        if !self
            .certificates_der
            .iter()
            .any(|existing| existing == &canonical_der)
        {
            self.certificates_der.push(canonical_der);
            self.certificates_der.sort();
        }
        Ok(())
    }

    pub fn add_pem(&mut self, pem: &[u8]) -> Result<()> {
        let certificate = Certificate::from_pem(pem).map_err(|error| {
            OxideError::invalid_input(format!("intermediate certificate PEM: {error}"))
        })?;
        let der = certificate.to_der().map_err(|error| {
            OxideError::invalid_input(format!("intermediate certificate PEM encode: {error}"))
        })?;
        self.add_der(&der)
    }

    pub fn certificates_der(&self) -> &[Vec<u8>] {
        &self.certificates_der
    }
}

/// Explicit, reportable policy for CMS signer algorithms and PKIX RSA key
/// sizes. Parsing an algorithm remains separate from accepting it under this
/// policy: legacy SHA-1 can be represented for diagnostics, but the default
/// policy refuses it for signature validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignatureAlgorithmPolicy {
    pub allow_sha1: bool,
    pub allow_rsa_pkcs1v15: bool,
    pub allow_rsa_pss: bool,
    pub allow_ecdsa_p256: bool,
    pub allow_ecdsa_p384: bool,
    pub min_rsa_key_bits: u16,
}

impl Default for SignatureAlgorithmPolicy {
    fn default() -> Self {
        Self {
            allow_sha1: false,
            allow_rsa_pkcs1v15: true,
            allow_rsa_pss: true,
            allow_ecdsa_p256: true,
            allow_ecdsa_p384: true,
            min_rsa_key_bits: 2048,
        }
    }
}

impl SignatureAlgorithmPolicy {
    fn validate(&self) -> Result<()> {
        if self.min_rsa_key_bits < 1024 || self.min_rsa_key_bits > 16_384 {
            return Err(OxideError::invalid_input(
                "algorithm policy min_rsa_key_bits must be between 1024 and 16384",
            ));
        }
        Ok(())
    }

    fn allows_digest(&self, oid: &ObjectIdentifier) -> bool {
        match *oid {
            OID_SHA1 => self.allow_sha1,
            OID_SHA256 | OID_SHA384 | OID_SHA512 => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerifyOptions {
    /// DER-encoded certificates the verifier explicitly trusts as roots (or
    /// pinned signer certificates). A signer is `Trusted` only if it chains to
    /// one of these and is within its validity period.
    pub trust_anchors_der: Vec<Vec<u8>>,
    /// DER-encoded untrusted intermediates supplied by the caller. These are
    /// candidate path-building material only; they are never trusted as anchors.
    pub intermediates_der: Vec<Vec<u8>>,
    /// SHA-256 fingerprints of certificates that must never participate in a
    /// selected path, including roots, intermediates, and signer certificates.
    /// The deny list is evaluated during candidate selection, not merely
    /// reported after a trusted path has already been selected.
    pub distrusted_certificate_sha256: Vec<String>,
    /// DER-encoded OCSP responses supplied by the caller for offline
    /// revocation evaluation.
    pub ocsp_responses_der: Vec<Vec<u8>>,
    /// DER-encoded CRLs supplied by the caller for offline revocation
    /// evaluation.
    pub crls_der: Vec<Vec<u8>>,
    /// Explicit validation time as Unix seconds. `None` uses the current
    /// system clock and reports that clock source.
    pub validation_time_unix: Option<u64>,
    /// Named policy profile controlling trust/revocation strictness.
    pub policy_profile: SignatureValidationPolicyProfile,
    /// Explicit CMS and certificate algorithm policy. The default accepts
    /// SHA-2 RSA PKCS#1 v1.5/RSA-PSS and P-256/P-384 ECDSA, requires RSA 2048
    /// bits or stronger, and rejects SHA-1.
    pub algorithm_policy: SignatureAlgorithmPolicy,
    /// Revocation mode. The default preserves the historical offline behavior:
    /// revocation material is inventoried but not required for trust.
    pub revocation_mode: SignatureRevocationMode,
    /// Online retrieval is opt-in. When enabled together with an allowed
    /// retrieval policy, the validator uses the bounded shared AIA/OCSP/CRL
    /// transport; otherwise it reports the explicit offline posture.
    pub allow_online_retrieval: bool,
    /// Bounded, opt-in network policy shared by AIA, OCSP, and CRL retrieval.
    /// The default is offline and rejects private/local destinations.
    pub retrieval_policy: RetrievalPolicy,
    /// Optional replayable evidence bundle. Imported certificates remain
    /// untrusted intermediates and every OCSP/CRL item is revalidated.
    pub evidence_bundle: Option<EvidenceBundle>,
    /// Maximum chain depth passed to the bounded path builder/validator.
    pub max_chain_depth: usize,
    /// DFS node budget passed to the bounded path builder.
    pub max_path_candidates: usize,
    /// Cooperative cancellation shared by the caller and the complete
    /// PDF/CMS/PKIX/revocation pipeline. The default token is never cancelled;
    /// bindings that expose cancellation attach an explicit clone through
    /// [`Self::with_cancellation_token`].
    pub cancellation: CancelToken,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self {
            trust_anchors_der: Vec::new(),
            intermediates_der: Vec::new(),
            distrusted_certificate_sha256: Vec::new(),
            ocsp_responses_der: Vec::new(),
            crls_der: Vec::new(),
            validation_time_unix: None,
            // Preserve the historical deterministic, no-network default while
            // describing it accurately. Callers that want named strict
            // evidence semantics select a profile explicitly.
            policy_profile: SignatureValidationPolicyProfile::Custom,
            algorithm_policy: SignatureAlgorithmPolicy::default(),
            revocation_mode: SignatureRevocationMode::NotChecked,
            allow_online_retrieval: false,
            retrieval_policy: RetrievalPolicy::offline(),
            evidence_bundle: None,
            max_chain_depth: 10,
            max_path_candidates: 4096,
            cancellation: CancelToken::none(),
        }
    }
}

impl VerifyOptions {
    /// Add a DER-encoded trust anchor certificate.
    pub fn with_trust_anchor_der(mut self, der: Vec<u8>) -> Self {
        self.trust_anchors_der.push(der);
        self
    }

    /// Add a DER-encoded untrusted intermediate certificate.
    pub fn with_intermediate_der(mut self, der: Vec<u8>) -> Self {
        self.intermediates_der.push(der);
        self
    }

    /// Attach explicit anchor metadata and canonical DER from a typed trust
    /// store. No intermediate or retrieved certificate is implicitly trusted.
    pub fn with_trust_store(mut self, store: &TrustStore) -> Self {
        self.trust_anchors_der
            .extend(store.anchors().iter().map(|anchor| anchor.der.clone()));
        self
    }

    /// Attach untrusted intermediate candidates from a typed store.
    pub fn with_intermediate_store(mut self, store: &IntermediateStore) -> Self {
        self.intermediates_der
            .extend(store.certificates_der().iter().cloned());
        self
    }

    /// Add a certificate SHA-256 deny-list entry. Accepted formats are 64 hex
    /// digits, optionally separated by colon, dash, underscore, or whitespace.
    pub fn with_distrusted_certificate_sha256(mut self, fingerprint: &str) -> Result<Self> {
        let fingerprint = normalize_certificate_fingerprint(fingerprint)?;
        if !self
            .distrusted_certificate_sha256
            .iter()
            .any(|existing| existing == &fingerprint)
        {
            self.distrusted_certificate_sha256.push(fingerprint);
            self.distrusted_certificate_sha256.sort();
        }
        Ok(self)
    }

    /// Add a DER-encoded OCSP response for offline revocation evaluation.
    pub fn with_ocsp_response_der(mut self, der: Vec<u8>) -> Self {
        self.ocsp_responses_der.push(der);
        self
    }

    /// Add a DER-encoded CRL for offline revocation evaluation.
    pub fn with_crl_der(mut self, der: Vec<u8>) -> Self {
        self.crls_der.push(der);
        self
    }

    /// Set an explicit validation time as Unix seconds.
    pub fn with_validation_time_unix(mut self, unix: u64) -> Self {
        self.validation_time_unix = Some(unix);
        self
    }

    /// Replace the explicit CMS/PKIX algorithm policy after validating its
    /// resource-safe key-size range.
    pub fn with_algorithm_policy(mut self, policy: SignatureAlgorithmPolicy) -> Result<Self> {
        policy.validate()?;
        self.algorithm_policy = policy;
        Ok(self)
    }

    /// Require supplied offline revocation evidence to establish a non-revoked
    /// result before trust can become `Trusted`.
    pub fn with_offline_revocation_strict(mut self) -> Self {
        self.revocation_mode = SignatureRevocationMode::OfflineStrict;
        self.policy_profile = SignatureValidationPolicyProfile::OfflineWithSuppliedEvidence;
        self
    }

    /// Select a named Prompt 24 policy profile. Selecting an online profile
    /// is explicit network opt-in, but callers can still replace the bounded
    /// retrieval policy afterwards with [`Self::with_retrieval_policy`].
    pub fn with_policy_profile(mut self, profile: SignatureValidationPolicyProfile) -> Self {
        self.policy_profile = profile;
        match profile {
            SignatureValidationPolicyProfile::OfflineStrict
            | SignatureValidationPolicyProfile::OfflineWithSuppliedEvidence => {
                self.revocation_mode = SignatureRevocationMode::OfflineStrict;
                self.allow_online_retrieval = false;
                self.retrieval_policy = RetrievalPolicy::offline();
            }
            SignatureValidationPolicyProfile::OnlineStrict => {
                self.revocation_mode = SignatureRevocationMode::OnlineStrict;
                self.allow_online_retrieval = true;
                self.retrieval_policy = RetrievalPolicy::online();
            }
            SignatureValidationPolicyProfile::OnlineBestEvidence => {
                self.revocation_mode = SignatureRevocationMode::OnlineBestEffort;
                self.allow_online_retrieval = true;
                self.retrieval_policy = RetrievalPolicy::online();
            }
            SignatureValidationPolicyProfile::Custom => {}
        }
        self
    }

    /// Enable controlled online retrieval and require fresh usable revocation
    /// evidence before a path is treated as trusted.
    pub fn with_online_revocation_strict(self) -> Self {
        self.with_policy_profile(SignatureValidationPolicyProfile::OnlineStrict)
    }

    /// Enable controlled online retrieval while retaining an explicit
    /// indeterminate result when the best available evidence is incomplete.
    /// Missing or failed evidence never becomes a `good` decision.
    pub fn with_online_best_evidence(self) -> Self {
        self.with_policy_profile(SignatureValidationPolicyProfile::OnlineBestEvidence)
    }

    /// Supply a portable evidence bundle for deterministic offline replay.
    pub fn with_evidence_bundle(mut self, bundle: EvidenceBundle) -> Result<Self> {
        append_evidence_bundle(&mut self, &bundle)?;
        self.evidence_bundle = Some(bundle);
        Ok(self)
    }

    /// Enable bounded AIA/OCSP/CRL retrieval using the supplied policy.
    pub fn with_retrieval_policy(mut self, policy: RetrievalPolicy) -> Result<Self> {
        policy
            .validate()
            .map_err(|error| OxideError::invalid_input(format!("retrieval policy: {error}")))?;
        self.allow_online_retrieval = policy.enabled;
        self.retrieval_policy = policy;
        Ok(self)
    }

    /// Attach a cooperative cancellation token. The caller keeps a clone and
    /// can signal cancellation while validation is in progress; all later
    /// signature, path, and retrieval stages observe the same token.
    pub fn with_cancellation_token(mut self, cancellation: CancelToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

/// Parse Prompt 24 validation options from a stable JSON object.
///
/// Binary inputs are hex-encoded DER arrays. Supported keys:
///
/// - `trust_anchors_der_hex`
/// - `intermediates_der_hex`
/// - `distrusted_certificate_sha256`
/// - `ocsp_responses_der_hex`
/// - `crls_der_hex`
/// - `validation_time_unix`
/// - `revocation`
/// - `policy_profile`
/// - `algorithm_policy`
/// - `online`
/// - `max_chain_depth`
/// - `max_path_candidates`
/// - `retrieval_policy`
/// - `evidence_bundle`
pub fn verify_options_from_json(options_json: &str) -> Result<VerifyOptions> {
    let trimmed = options_json.trim();
    if trimmed.is_empty() {
        return Ok(VerifyOptions::default());
    }
    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|err| OxideError::invalid_input(format!("signature options JSON: {err}")))?;
    if value.is_null() {
        return Ok(VerifyOptions::default());
    }
    let obj = value.as_object().ok_or_else(|| {
        OxideError::invalid_input("signature options JSON must be an object or null")
    })?;
    let mut options = VerifyOptions::default();
    for der in parse_hex_array(obj, "trust_anchors_der_hex")? {
        options = options.with_trust_anchor_der(der);
    }
    for der in parse_hex_array(obj, "intermediates_der_hex")? {
        options = options.with_intermediate_der(der);
    }
    if let Some(values) = obj.get("distrusted_certificate_sha256") {
        let values = values.as_array().ok_or_else(|| {
            OxideError::invalid_input("distrusted_certificate_sha256 must be an array")
        })?;
        for (index, value) in values.iter().enumerate() {
            let fingerprint = value.as_str().ok_or_else(|| {
                OxideError::invalid_input(format!(
                    "distrusted_certificate_sha256[{index}] must be a string"
                ))
            })?;
            options = options.with_distrusted_certificate_sha256(fingerprint)?;
        }
    }
    for der in parse_hex_array(obj, "ocsp_responses_der_hex")? {
        options = options.with_ocsp_response_der(der);
    }
    for der in parse_hex_array(obj, "crls_der_hex")? {
        options = options.with_crl_der(der);
    }
    if let Some(unix) = obj
        .get("validation_time_unix")
        .and_then(serde_json::Value::as_u64)
    {
        options.validation_time_unix = Some(unix);
    }
    let requested_revocation = obj
        .get("revocation")
        .and_then(serde_json::Value::as_str)
        .map(parse_signature_revocation_mode)
        .transpose()?;
    if let Some(profile) = obj
        .get("policy_profile")
        .and_then(serde_json::Value::as_str)
    {
        options = options.with_policy_profile(parse_signature_policy_profile(profile)?);
    }
    if let Some(policy) = obj.get("algorithm_policy") {
        let policy: SignatureAlgorithmPolicy =
            serde_json::from_value(policy.clone()).map_err(|error| {
                OxideError::invalid_input(format!(
                    "algorithm_policy must be a valid object: {error}"
                ))
            })?;
        options = options.with_algorithm_policy(policy)?;
    }
    if let Some(online) = obj.get("online").and_then(serde_json::Value::as_bool) {
        options.allow_online_retrieval = online;
        options.retrieval_policy.enabled = online;
    }
    if let Some(policy) = obj.get("retrieval_policy") {
        let policy: RetrievalPolicy = serde_json::from_value(policy.clone()).map_err(|error| {
            OxideError::invalid_input(format!("retrieval_policy must be a valid object: {error}"))
        })?;
        options = options.with_retrieval_policy(policy)?;
    }
    if let Some(bundle_value) = obj.get("evidence_bundle") {
        let bundle: EvidenceBundle =
            serde_json::from_value(bundle_value.clone()).map_err(|error| {
                OxideError::invalid_input(format!("evidence_bundle must be valid: {error}"))
            })?;
        options = options.with_evidence_bundle(bundle)?;
    }
    if let Some(depth) = obj
        .get("max_chain_depth")
        .and_then(serde_json::Value::as_u64)
    {
        options.max_chain_depth = usize::try_from(depth)
            .map_err(|_| OxideError::invalid_input("max_chain_depth is too large"))?
            .max(1);
    }
    if let Some(candidates) = obj
        .get("max_path_candidates")
        .and_then(serde_json::Value::as_u64)
    {
        options.max_path_candidates = usize::try_from(candidates)
            .map_err(|_| OxideError::invalid_input("max_path_candidates is too large"))?
            .max(1);
    }
    if let Some(mode) = requested_revocation {
        // A caller may intentionally pair a custom revocation mode with a
        // named profile. Preserve that explicit override, while the profile
        // remains visible in the deterministic policy report.
        options.revocation_mode = mode;
    }
    Ok(options)
}

fn append_evidence_bundle(options: &mut VerifyOptions, bundle: &EvidenceBundle) -> Result<()> {
    bundle
        .validate(
            options.retrieval_policy.budget.max_cache_entries,
            options.retrieval_policy.budget.max_cache_bytes,
        )
        .map_err(|error| OxideError::invalid_input(format!("evidence bundle: {error}")))?;
    for record in &bundle.records {
        let bytes = record
            .bytes()
            .map_err(|error| OxideError::invalid_input(format!("evidence bundle: {error}")))?;
        match record.kind {
            EvidenceKind::Certificate => options.intermediates_der.push(bytes),
            EvidenceKind::Ocsp => options.ocsp_responses_der.push(bytes),
            EvidenceKind::Crl => options.crls_der.push(bytes),
        }
    }
    Ok(())
}

fn parse_hex_array(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<Vec<u8>>> {
    let Some(value) = obj.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| OxideError::invalid_input(format!("{key} must be an array")))?;
    values
        .iter()
        .enumerate()
        .map(|(idx, value)| {
            let text = value.as_str().ok_or_else(|| {
                OxideError::invalid_input(format!("{key}[{idx}] must be a hex string"))
            })?;
            decode_hex_der(text).map_err(|err| {
                OxideError::invalid_input(format!("{key}[{idx}] is not valid hex DER: {err}"))
            })
        })
        .collect()
}

fn decode_hex_der(text: &str) -> std::result::Result<Vec<u8>, &'static str> {
    let mut nibbles = Vec::new();
    for byte in text.bytes() {
        match byte {
            b'0'..=b'9' => nibbles.push(byte - b'0'),
            b'a'..=b'f' => nibbles.push(byte - b'a' + 10),
            b'A'..=b'F' => nibbles.push(byte - b'A' + 10),
            b' ' | b'\n' | b'\r' | b'\t' | b':' | b'-' | b'_' => {}
            _ => return Err("invalid character"),
        }
    }
    if nibbles.len() % 2 != 0 {
        return Err("odd number of hex digits");
    }
    Ok(nibbles
        .chunks_exact(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect())
}

fn parse_signature_revocation_mode(value: &str) -> Result<SignatureRevocationMode> {
    match value {
        "not_checked" | "not-checked" | "disabled" => Ok(SignatureRevocationMode::NotChecked),
        "offline_strict"
        | "offline-strict"
        | "offline_supplied_only"
        | "offline-supplied-only"
        | "require_any_fresh_evidence"
        | "require-any-fresh-evidence" => Ok(SignatureRevocationMode::OfflineStrict),
        "offline_best_effort" | "offline-best-effort" => {
            Ok(SignatureRevocationMode::OfflineBestEffort)
        }
        "online_strict" | "online-strict" | "online_hard_fail" | "online-hard-fail"
        | "require_fresh_good" | "require-fresh-good" => Ok(SignatureRevocationMode::OnlineStrict),
        "online_best_effort"
        | "online-best-effort"
        | "online_best_evidence"
        | "online-best-evidence"
        | "soft_fail_network"
        | "soft-fail-network" => Ok(SignatureRevocationMode::OnlineBestEffort),
        _ => Err(OxideError::invalid_input(format!(
            "unknown signature revocation mode '{value}'"
        ))),
    }
}

fn parse_signature_policy_profile(value: &str) -> Result<SignatureValidationPolicyProfile> {
    match value {
        "offline_strict" | "offline-strict" => Ok(SignatureValidationPolicyProfile::OfflineStrict),
        "offline_with_supplied_evidence" | "offline-with-supplied-evidence" => {
            Ok(SignatureValidationPolicyProfile::OfflineWithSuppliedEvidence)
        }
        "online_strict" | "online-strict" => Ok(SignatureValidationPolicyProfile::OnlineStrict),
        "online_best_evidence" | "online-best-evidence" => {
            Ok(SignatureValidationPolicyProfile::OnlineBestEvidence)
        }
        "custom" => Ok(SignatureValidationPolicyProfile::Custom),
        _ => Err(OxideError::invalid_input(format!(
            "unknown signature policy profile '{value}'"
        ))),
    }
}

/// Prompt 24 validation policy profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureValidationPolicyProfile {
    OfflineStrict,
    OfflineWithSuppliedEvidence,
    OnlineStrict,
    OnlineBestEvidence,
    Custom,
}

/// Prompt 24 revocation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureRevocationMode {
    /// Preserve historical behavior: inventory material but do not require
    /// revocation evidence for trust.
    NotChecked,
    /// Require caller-supplied OCSP/CRL evidence to establish non-revocation.
    OfflineStrict,
    /// Evaluate supplied evidence when present; missing evidence remains
    /// explicit but does not alone block the legacy trust field.
    OfflineBestEffort,
    /// Use caller-supplied, replayed, cached, or explicitly enabled online
    /// evidence and require a fresh usable decision for every required path
    /// certificate before trust is established.
    OnlineStrict,
    /// Collect the best available caller-supplied, replayed, cached, or
    /// online evidence. Incomplete evidence remains explicit and does not
    /// establish a trusted-valid result.
    OnlineBestEffort,
}

impl SignatureRevocationMode {
    fn requires_evidence(self) -> bool {
        !matches!(self, Self::NotChecked)
    }

    fn requires_online_retrieval(self) -> bool {
        matches!(self, Self::OnlineStrict | Self::OnlineBestEffort)
    }
}

/// Fine-grained Prompt 24 status taxonomy. This intentionally avoids reducing
/// signature validation to one boolean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureValidationState {
    Valid,
    Invalid,
    Indeterminate,
    Malformed,
    UnsupportedAlgorithm,
    UnsupportedProfile,
    EvidenceMissing,
    EvidenceStale,
    ConflictingEvidence,
    Untrusted,
    Revoked,
    RevocationUnknown,
    NotYetValid,
    Expired,
    PolicyRejected,
    ModifiedAfterSigning,
    PartialDocumentCoverage,
    ByteRangeInvalid,
    DigestMismatch,
    SignatureMathInvalid,
    SignerCertificateAmbiguous,
    SignerCertificateMissing,
    PathNotFound,
    PathInvalid,
    NonceMismatch,
    NetworkDisabled,
    NetworkFailure,
    DeferredToLaterPrompt,
    NotChecked,
}

/// Stable, ETSI-inspired top-level validation indication. Detailed states
/// remain in the component reports; this is a deterministic summary for
/// bindings and command-line exit classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureValidationIndication {
    Passed,
    Failed,
    Indeterminate,
    NotEvaluated,
}

/// Exact reason family behind [`SignatureValidationIndication`]. This avoids
/// treating an unavailable revocation response as a mathematical signature
/// failure, or a post-signature revision as a trusted current-file pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureValidationSubindication {
    None,
    ValidationIndeterminate,
    PdfStructureInvalid,
    CmsMalformed,
    DigestMismatch,
    SignatureMathInvalid,
    SignerCertificateMissing,
    SignerCertificateAmbiguous,
    PathNotFound,
    PathInvalid,
    CertificateUntrusted,
    CertificateRevoked,
    RevocationEvidenceMissing,
    RevocationEvidenceStale,
    RevocationUnknown,
    NetworkDisabled,
    NetworkFailure,
    UnsupportedAlgorithm,
    UnsupportedProfile,
    PolicyRejected,
    DocumentModifiedAfterSigning,
    DeferredToLaterPrompt,
    NotEvaluated,
}

/// Prompt 24 policy metadata included in every report.
#[derive(Debug, Clone, Serialize)]
pub struct SignatureValidationPolicyReport {
    pub profile: SignatureValidationPolicyProfile,
    pub revocation_mode: SignatureRevocationMode,
    pub algorithm_policy: SignatureAlgorithmPolicy,
    pub validation_time_unix: u64,
    pub validation_time_source: String,
    pub online_retrieval_enabled: bool,
    pub evidence_cache_configured: bool,
    pub ocsp_nonce_policy: OcspNoncePolicy,
    pub max_chain_depth: usize,
    pub max_path_candidates: usize,
    pub trust_anchor_count: usize,
    pub intermediate_count: usize,
    pub distrust_entry_count: usize,
    pub supplied_ocsp_count: usize,
    pub supplied_crl_count: usize,
}

impl Default for SignatureValidationPolicyReport {
    fn default() -> Self {
        Self {
            profile: SignatureValidationPolicyProfile::Custom,
            revocation_mode: SignatureRevocationMode::NotChecked,
            algorithm_policy: SignatureAlgorithmPolicy::default(),
            validation_time_unix: 0,
            validation_time_source: "not_evaluated".to_string(),
            online_retrieval_enabled: false,
            evidence_cache_configured: false,
            ocsp_nonce_policy: OcspNoncePolicy::Disabled,
            max_chain_depth: 0,
            max_path_candidates: 0,
            trust_anchor_count: 0,
            intermediate_count: 0,
            distrust_entry_count: 0,
            supplied_ocsp_count: 0,
            supplied_crl_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CertificatePathValidationReport {
    pub status: SignatureValidationState,
    pub signer_certificate_status: SignatureValidationState,
    pub anchor_parse_errors: Vec<String>,
    pub intermediate_parse_errors: Vec<String>,
    pub candidate_paths_tried: usize,
    pub selected_anchor_index: Option<usize>,
    pub selected_path_subjects: Vec<String>,
    pub selected_path_serials: Vec<String>,
    pub validation_error: Option<String>,
    pub implemented_checks: Vec<&'static str>,
}

impl Default for CertificatePathValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NotChecked,
            signer_certificate_status: SignatureValidationState::NotChecked,
            anchor_parse_errors: Vec::new(),
            intermediate_parse_errors: Vec::new(),
            candidate_paths_tried: 0,
            selected_anchor_index: None,
            selected_path_subjects: Vec::new(),
            selected_path_serials: Vec::new(),
            validation_error: None,
            implemented_checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CertificateRevocationDecision {
    pub path_index: usize,
    pub subject: String,
    pub serial_hex: String,
    pub status: SignatureValidationState,
    pub evidence_type: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevocationValidationReport {
    pub status: SignatureValidationState,
    pub ocsp_responses_supplied: usize,
    pub crls_supplied: usize,
    pub certificate_decisions: Vec<CertificateRevocationDecision>,
    pub errors: Vec<String>,
}

impl Default for RevocationValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NotChecked,
            ocsp_responses_supplied: 0,
            crls_supplied: 0,
            certificate_decisions: Vec::new(),
            errors: Vec::new(),
        }
    }
}

/// CMS checks retained independently from PDF coverage and certificate trust.
/// A mathematical signature is never used as evidence that its CMS container
/// or signed attributes met the PDF/CAdES profile.
#[derive(Debug, Clone, Serialize)]
pub struct CmsValidationReport {
    pub status: SignatureValidationState,
    pub content_info: SignatureValidationState,
    pub detached_content: SignatureValidationState,
    pub signer_info_count: usize,
    pub digest_algorithm_declared: SignatureValidationState,
    pub signed_attributes: SignatureValidationState,
    pub content_type_attribute: SignatureValidationState,
    pub message_digest_attribute: SignatureValidationState,
    pub signing_certificate_reference: SignatureValidationState,
    pub cms_algorithm_protection: SignatureValidationState,
}

impl Default for CmsValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NotChecked,
            content_info: SignatureValidationState::NotChecked,
            detached_content: SignatureValidationState::NotChecked,
            signer_info_count: 0,
            digest_algorithm_declared: SignatureValidationState::NotChecked,
            signed_attributes: SignatureValidationState::NotChecked,
            content_type_attribute: SignatureValidationState::NotChecked,
            message_digest_attribute: SignatureValidationState::NotChecked,
            signing_certificate_reference: SignatureValidationState::NotChecked,
            cms_algorithm_protection: SignatureValidationState::NotChecked,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PadesValidationReport {
    /// Final Prompt 24 PAdES validation result. This is not a structure-only
    /// classifier: it incorporates the separately reported certificate-path
    /// and revocation-policy outcomes when they are available.
    pub status: SignatureValidationState,
    /// PDF/CMS/ESS baseline conformance alone. A value of `valid` here does
    /// not imply trusted PAdES validation; consult `status`, path, and
    /// revocation fields for the final policy result.
    pub structural_status: SignatureValidationState,
    pub detected_profile: String,
    pub validated_level: String,
    /// Whether the exact historical revision selected by `/ByteRange` was
    /// structurally covered. This is independent from later incremental
    /// updates: an earlier PAdES signature can remain conformant for its own
    /// revision even when the current file has changed.
    pub signed_revision_coverage_status: SignatureValidationState,
    /// Whether the current file still equals the signed revision. Prompt 25
    /// policy/edit surfaces apply DocMDP and FieldMDP decisions separately so
    /// this field remains a revision-integrity result, not a permission result.
    pub current_document_status: SignatureValidationState,
    /// Format-level path state. It is intentionally separate from baseline
    /// conformance: a PAdES B-B signature can be structurally conformant while
    /// the caller's trust policy rejects its certificate path.
    pub certificate_path_status: SignatureValidationState,
    /// Policy-level revocation state, never collapsed into profile syntax.
    pub revocation_status: SignatureValidationState,
    pub higher_level_evidence_present: bool,
    pub higher_level_evidence_status: SignatureValidationState,
    pub missing_requirements: Vec<String>,
}

impl Default for PadesValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NotChecked,
            structural_status: SignatureValidationState::NotChecked,
            detected_profile: "unknown".to_string(),
            validated_level: "none".to_string(),
            signed_revision_coverage_status: SignatureValidationState::NotChecked,
            current_document_status: SignatureValidationState::NotChecked,
            certificate_path_status: SignatureValidationState::NotChecked,
            revocation_status: SignatureValidationState::NotChecked,
            higher_level_evidence_present: false,
            higher_level_evidence_status: SignatureValidationState::NotChecked,
            missing_requirements: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkValidationReport {
    pub status: SignatureValidationState,
    pub aia_fetching: SignatureValidationState,
    pub ocsp_fetching: SignatureValidationState,
    pub crl_fetching: SignatureValidationState,
    pub fetch_traces: Vec<RetrievalTrace>,
    pub retrieved_evidence: Vec<NetworkEvidenceReport>,
    pub note: String,
}

/// Metadata only for fetched evidence. Raw bytes are held in a caller-owned
/// evidence bundle/cache rather than emitted into ordinary signature reports.
#[derive(Debug, Clone, Serialize)]
pub struct NetworkEvidenceReport {
    pub kind: String,
    pub source_uri: String,
    pub sha256: String,
    pub byte_count: usize,
    pub cache_hit: bool,
}

impl Default for NetworkValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NetworkDisabled,
            aia_fetching: SignatureValidationState::NetworkDisabled,
            ocsp_fetching: SignatureValidationState::NetworkDisabled,
            crl_fetching: SignatureValidationState::NetworkDisabled,
            fetch_traces: Vec::new(),
            retrieved_evidence: Vec::new(),
            note: "network retrieval disabled; only caller-supplied offline evidence is used"
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt24SignatureValidationReport {
    pub schema_version: &'static str,
    pub policy: SignatureValidationPolicyReport,
    pub cms: CmsValidationReport,
    pub signer_resolution: SignatureValidationState,
    pub certificate_inventory_count: usize,
    pub path: CertificatePathValidationReport,
    pub revocation: RevocationValidationReport,
    pub pades: PadesValidationReport,
    pub network: NetworkValidationReport,
    pub deferred_evidence: Vec<String>,
    pub warnings: Vec<String>,
    pub overall: SignatureValidationState,
    pub overall_reason: String,
    pub indication: SignatureValidationIndication,
    pub subindication: SignatureValidationSubindication,
}

impl Default for Prompt24SignatureValidationReport {
    fn default() -> Self {
        Self {
            schema_version: PROMPT24_SIGNATURE_VALIDATION_SCHEMA_VERSION,
            policy: SignatureValidationPolicyReport::default(),
            cms: CmsValidationReport::default(),
            signer_resolution: SignatureValidationState::NotChecked,
            certificate_inventory_count: 0,
            path: CertificatePathValidationReport::default(),
            revocation: RevocationValidationReport::default(),
            pades: PadesValidationReport::default(),
            network: NetworkValidationReport::default(),
            deferred_evidence: Vec::new(),
            warnings: Vec::new(),
            overall: SignatureValidationState::NotChecked,
            overall_reason: "not evaluated".to_string(),
            indication: SignatureValidationIndication::NotEvaluated,
            subindication: SignatureValidationSubindication::NotEvaluated,
        }
    }
}

/// Whether the signer's certificate is *trusted*, evaluated against the
/// configured trust anchors. This is distinct from cryptographic integrity
/// ([`SignatureValidity`]): a signature can be cryptographically `Valid` while
/// its signer is `NotVerified` or `Untrusted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureTrust {
    /// No trust anchors were configured, so chain trust was not evaluated. The
    /// signature may be cryptographically valid, but the signer is unverified —
    /// this is **not** a statement that the signer is trustworthy.
    NotVerified,
    /// The signer certificate chains to a configured trust anchor and is within
    /// its validity period (and not revoked by embedded material).
    Trusted,
    /// Chain evaluation ran but the signer does not chain to any configured
    /// anchor — e.g. a self-signed certificate, or an unknown issuer.
    Untrusted,
    /// The signer chains to an anchor but is outside its validity period.
    Expired,
    /// The signer certificate was revoked per embedded revocation material.
    Revoked,
}

/// The overall, honest verdict combining integrity, trust, and coverage. A
/// signature is [`SignatureStatus::Trusted`] **only** if its integrity verifies,
/// its signer chains to a configured trust anchor, and it covers the whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureStatus {
    /// Integrity verified, signer trusted to a configured anchor, whole-file
    /// coverage. The strongest verdict.
    Trusted,
    /// Integrity verified, but trust to a configured anchor was not established
    /// (no anchors configured, untrusted root, expired, or revoked — see the
    /// `trust` field for which).
    ValidUntrusted,
    /// Integrity verified, but a validated OCSP or CRL decision revoked the
    /// signer or an issuing certificate. This is distinct from an untrusted
    /// chain so callers never need to infer revocation from a generic status.
    Revoked,
    /// Integrity verified for the bytes it covers, but the document was modified
    /// after signing (content was appended outside the signed range).
    ValidButModified,
    /// The integrity check failed: content within the signed range changed or
    /// the signature is corrupt.
    Invalid,
    /// The signature algorithm is not supported.
    UnsupportedAlgorithm,
    /// The signature could not be parsed/verified at all.
    Error,
}

/// PAdES baseline level inferred from the signature and embedded LTV material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PadesLevel {
    /// Core CMS signature only.
    BaselineB,
    /// CMS signature contains a parseable RFC 3161 timestamp token.
    BaselineT,
    /// Timestamp plus matching DSS validation material (`/Certs` and
    /// `/OCSPs` or `/CRLs`) is embedded for offline validation.
    BaselineLT,
    /// Document/archive timestamp over the DSS. Not emitted by the current
    /// writer, but reported for future-compatible readers if detected later.
    BaselineLTA,
}

/// Revocation status derived from embedded DSS material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationStatus {
    /// No embedded revocation material was available.
    NotChecked,
    /// OCSP/CRL bytes were embedded but this verifier did not derive a
    /// definitive signer status from them.
    EmbeddedMaterial,
    /// A parseable embedded CRL did not list the signer certificate serial.
    GoodFromEmbeddedCrl,
    /// A parseable embedded CRL listed the signer certificate serial.
    RevokedByEmbeddedCrl,
    /// Revocation material was present but malformed or not usable.
    Unknown,
}

/// Long-term-validation material supplied by a caller and embedded in `/DSS`.
///
/// `signature_index` is 1-based and matches [`SignatureReport::index`]. When
/// omitted, the material is associated with every signature in the document.
/// Certificate DER is supplemented with the signer certificates already present
/// in each CMS signature so the DSS always carries the signer chain known to
/// Oxide.
#[derive(Debug, Clone, Default)]
pub struct LtvMaterial {
    pub signature_index: Option<usize>,
    pub certificates_der: Vec<Vec<u8>>,
    pub ocsp_responses_der: Vec<Vec<u8>>,
    pub crls_der: Vec<Vec<u8>>,
}

impl LtvMaterial {
    fn is_empty(&self) -> bool {
        self.certificates_der.is_empty()
            && self.ocsp_responses_der.is_empty()
            && self.crls_der.is_empty()
    }
}

/// Per-signature LTV/PAdES validation report.
#[derive(Debug, Clone, Serialize)]
pub struct LtvReport {
    pub pades_level: PadesLevel,
    pub timestamp_token_count: usize,
    pub invalid_timestamp_token_count: usize,
    pub dss_present: bool,
    pub vri_key: Option<String>,
    pub vri_matched: bool,
    pub embedded_certs: usize,
    pub embedded_ocsp_responses: usize,
    pub embedded_crls: usize,
    pub revocation_status: RevocationStatus,
    pub note: String,
}

impl Default for LtvReport {
    fn default() -> Self {
        Self {
            pades_level: PadesLevel::BaselineB,
            timestamp_token_count: 0,
            invalid_timestamp_token_count: 0,
            dss_present: false,
            vri_key: None,
            vri_matched: false,
            embedded_certs: 0,
            embedded_ocsp_responses: 0,
            embedded_crls: 0,
            revocation_status: RevocationStatus::NotChecked,
            note: "no timestamp token or DSS validation material found".to_string(),
        }
    }
}

/// Where a timestamp token was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampTokenType {
    SignatureTimestamp,
    DocumentTimestamp,
    DssVriTimestamp,
    ArchiveTimestamp,
    Unsupported,
}

/// RFC 3161 token validation for one discovered timestamp token.
#[derive(Debug, Clone, Serialize)]
pub struct TimestampValidationReport {
    pub token_type: TimestampTokenType,
    pub location: String,
    pub status: SignatureValidationState,
    pub raw_token_sha256: String,
    pub token_bytes: usize,
    pub content_info_status: SignatureValidationState,
    pub signed_data_status: SignatureValidationState,
    pub tst_info_status: SignatureValidationState,
    pub message_imprint_status: SignatureValidationState,
    pub cms_signature_status: SignatureValidationState,
    pub tsa_certificate_status: SignatureValidationState,
    pub tsa_path_status: SignatureValidationState,
    pub tsa_eku_status: SignatureValidationState,
    pub policy_oid: Option<String>,
    pub serial_hex: Option<String>,
    pub gen_time_unix: Option<u64>,
    pub gen_time: Option<String>,
    pub hash_algorithm: Option<String>,
    pub message_imprint_digest_hex: Option<String>,
    pub expected_imprint_digest_hex: Option<String>,
    pub tsa_subject: Option<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl TimestampValidationReport {
    fn new(token_type: TimestampTokenType, location: String, der: &[u8]) -> Self {
        Self {
            token_type,
            location,
            status: SignatureValidationState::NotChecked,
            raw_token_sha256: hex_lower(&Sha256::digest(der)),
            token_bytes: der.len(),
            content_info_status: SignatureValidationState::NotChecked,
            signed_data_status: SignatureValidationState::NotChecked,
            tst_info_status: SignatureValidationState::NotChecked,
            message_imprint_status: SignatureValidationState::NotChecked,
            cms_signature_status: SignatureValidationState::NotChecked,
            tsa_certificate_status: SignatureValidationState::NotChecked,
            tsa_path_status: SignatureValidationState::NotChecked,
            tsa_eku_status: SignatureValidationState::NotChecked,
            policy_oid: None,
            serial_hex: None,
            gen_time_unix: None,
            gen_time: None,
            hash_algorithm: None,
            message_imprint_digest_hex: None,
            expected_imprint_digest_hex: None,
            tsa_subject: None,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn is_valid(&self) -> bool {
        self.status == SignatureValidationState::Valid
    }
}

/// DSS/VRI inventory and Prompt 25 validation posture for one signature.
#[derive(Debug, Clone, Serialize)]
pub struct DssValidationReport {
    pub status: SignatureValidationState,
    pub dss_present: bool,
    pub vri_key: Option<String>,
    pub vri_matched: bool,
    pub global_cert_count: usize,
    pub global_ocsp_count: usize,
    pub global_crl_count: usize,
    pub matched_cert_count: usize,
    pub matched_ocsp_count: usize,
    pub matched_crl_count: usize,
    pub evidence_replayable_offline: bool,
    pub validation_material_status: SignatureValidationState,
    pub warnings: Vec<String>,
}

impl Default for DssValidationReport {
    fn default() -> Self {
        Self {
            status: SignatureValidationState::NotChecked,
            dss_present: false,
            vri_key: None,
            vri_matched: false,
            global_cert_count: 0,
            global_ocsp_count: 0,
            global_crl_count: 0,
            matched_cert_count: 0,
            matched_ocsp_count: 0,
            matched_crl_count: 0,
            evidence_replayable_offline: false,
            validation_material_status: SignatureValidationState::NotChecked,
            warnings: Vec::new(),
        }
    }
}

/// Prompt 25 extension report attached to every canonical signature result.
#[derive(Debug, Clone, Serialize)]
pub struct Prompt25SignatureLtvEditReport {
    pub schema_version: &'static str,
    pub timestamp_tokens: Vec<TimestampValidationReport>,
    pub signature_timestamp_status: SignatureValidationState,
    pub dss: DssValidationReport,
    pub ltv_status: SignatureValidationState,
    pub achieved_pades_level: PadesLevel,
    pub validation_indication: SignatureValidationIndication,
    pub validation_subindication: SignatureValidationSubindication,
    pub docmdp_status: SignatureValidationState,
    pub fieldmdp_status: SignatureValidationState,
    pub post_signature_modification_status: SignatureValidationState,
    pub signature_preserving_edit_status: SignatureValidationState,
    pub remaining_deferrals: Vec<String>,
    pub warnings: Vec<String>,
}

impl Default for Prompt25SignatureLtvEditReport {
    fn default() -> Self {
        Self {
            schema_version: PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
            timestamp_tokens: Vec::new(),
            signature_timestamp_status: SignatureValidationState::NotChecked,
            dss: DssValidationReport::default(),
            ltv_status: SignatureValidationState::NotChecked,
            achieved_pades_level: PadesLevel::BaselineB,
            validation_indication: SignatureValidationIndication::NotEvaluated,
            validation_subindication: SignatureValidationSubindication::NotEvaluated,
            docmdp_status: SignatureValidationState::NotChecked,
            fieldmdp_status: SignatureValidationState::NotChecked,
            post_signature_modification_status: SignatureValidationState::NotChecked,
            signature_preserving_edit_status: SignatureValidationState::NotChecked,
            remaining_deferrals: vec![
                "PAdES B-LTA archive timestamp validation is classified but not promoted without a validated archive timestamp chain".to_string(),
                "DocMDP/FieldMDP enforcement is evaluated by the Prompt 25 modification classifier before signature-preserving edits".to_string(),
            ],
            warnings: Vec::new(),
        }
    }
}

/// Signer certificate details (reported, not trust-validated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    pub serial_hex: String,
    pub not_before: String,
    pub not_after: String,
}

/// Audit-grade check bits for one signature.
///
/// These fields deliberately separate PDF container checks, byte-range digest
/// binding, CMS verification, chain trust, timestamp presence, and LTV material.
/// A caller must not infer CMS or trust validation from ByteRange parsing alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SignatureCheckDetails {
    pub byte_range_present: bool,
    pub byte_range_well_formed: bool,
    pub byte_range_in_bounds: bool,
    pub byte_range_non_overlapping: bool,
    pub byte_range_contents_gap_matches: bool,
    pub byte_range_covers_whole_file: bool,
    pub contents_present: bool,
    pub digest_matches: bool,
    pub cms_verified: bool,
    pub chain_verified: bool,
    pub revocation_checked: bool,
    pub timestamp_present: bool,
    pub timestamp_verified: bool,
    pub ltv_material_present: bool,
    pub ltv_verified: bool,
    pub docmdp_evaluated: bool,
    pub fieldmdp_evaluated: bool,
    pub signed_bytes: usize,
    pub byte_range: Option<[usize; 4]>,
}

/// Stable identity of an indirect PDF object referenced by a signature report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PdfSignatureObjectIdentity {
    pub number: u32,
    pub generation: u16,
}

/// How the signature dictionary was discovered. A standalone `/Type /Sig`
/// object is reported separately from an AcroForm signature field and is never
/// silently treated as a form signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureDiscoveryKind {
    AcroformSignatureField,
    OrphanedSignatureDictionary,
}

/// A single signature field's verification report.
#[derive(Debug, Clone, Serialize)]
pub struct SignatureReport {
    /// 1-based signature index in discovery order.
    pub index: usize,
    /// 1-based position of this result within the CMS `SignerInfos` SET.
    /// Multiple results can therefore share the same PDF signature `index`.
    pub cms_signer_index: Option<usize>,
    /// Total number of CMS `SignerInfo` values carried by this PDF signature.
    pub cms_signer_count: usize,
    /// Whether this is an AcroForm signature field or a standalone signature
    /// dictionary that was not owned by an AcroForm field.
    pub discovery_kind: SignatureDiscoveryKind,
    /// Indirect field object when the signature was found through AcroForm.
    pub field_object: Option<PdfSignatureObjectIdentity>,
    /// Indirect signature dictionary object, when one exists.
    pub signature_object: Option<PdfSignatureObjectIdentity>,
    /// Exact raw `/Contents` token span in the source PDF when it was safely
    /// located. The ByteRange gap must equal this span before CMS validation.
    pub contents_span: Option<[usize; 2]>,
    /// End offset of the covered historical revision derived from ByteRange.
    pub signed_revision_end: Option<usize>,
    /// Unsigned tail after the signed revision, if any.
    pub uncovered_byte_ranges: Vec<[usize; 2]>,
    /// Field name (`/T`), if present.
    pub field_name: Option<String>,
    /// `/Name` (signer name as stated in the signature dict), if present.
    pub signer_name: Option<String>,
    /// `/M` signing time (raw PDF date string), if present.
    pub signing_time: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact_info: Option<String>,
    /// `/SubFilter` (e.g. adbe.pkcs7.detached, ETSI.CAdES.detached).
    pub sub_filter: Option<String>,
    /// Digest algorithm named in the CMS, e.g. "SHA-256".
    pub digest_algorithm: Option<String>,
    /// Cryptographic **integrity** of the signature: does the CMS signature
    /// verify against the embedded certificate's key over the signed bytes?
    /// This is *not* a trust/authenticity verdict — see [`SignatureReport::trust`]
    /// and [`SignatureReport::status`].
    pub validity: SignatureValidity,
    /// Whether the signer certificate is **trusted** (chains to a configured
    /// trust anchor and is in-validity / not revoked). `NotVerified` when no
    /// anchors were configured.
    pub trust: SignatureTrust,
    pub coverage: Coverage,
    /// The overall honest verdict combining integrity + trust + coverage.
    /// `Trusted` only when all three hold.
    pub status: SignatureStatus,
    /// Signer certificate details (when a cert was present and parsed).
    pub certificate: Option<CertInfo>,
    /// PAdES/LTV material discovered for this signature.
    pub ltv: LtvReport,
    /// Prompt 24 structured validation report. This keeps PDF container,
    /// CMS math, path trust, revocation, PAdES, and network posture separate.
    pub prompt24: Prompt24SignatureValidationReport,
    /// Prompt 25 timestamp, DSS/VRI/LTV, permission, and edit-preservation
    /// layer. It is derived from the same canonical PDF/CMS/PKIX/revocation
    /// pipeline and does not replace Prompt 24 trust decisions.
    pub prompt25: Prompt25SignatureLtvEditReport,
    /// Machine-readable check separation for security/enterprise reports.
    pub checks: SignatureCheckDetails,
    /// Human-readable note on what was/wasn't checked.
    pub note: String,
}

/// Verification reports together with the bounded, cryptographically accepted
/// evidence that can be replayed offline. Raw evidence is deliberately absent
/// from [`SignatureReport`]; callers must opt in to this result type before it
/// is retained or serialized.
#[derive(Debug, Clone, Serialize)]
pub struct SignatureValidationOutcome {
    pub reports: Vec<SignatureReport>,
    pub evidence_bundle: EvidenceBundle,
}

/// Verify every signature field in the document with default options (no trust
/// anchors). Integrity and coverage are established; trust is reported as
/// `NotVerified`. Use [`verify_signatures_with_options`] to evaluate trust.
pub fn verify_signatures(doc: &PdfDocument) -> Result<Vec<SignatureReport>> {
    verify_signatures_with_options(doc, &VerifyOptions::default())
}

/// Verify every signature field, evaluating signer trust against the trust
/// anchors in `options`. A signature is reported as `Trusted` only when its
/// integrity verifies, its signer chains to a configured anchor (in validity
/// and not revoked), and it covers the whole file.
pub fn verify_signatures_with_options(
    doc: &PdfDocument,
    options: &VerifyOptions,
) -> Result<Vec<SignatureReport>> {
    options.algorithm_policy.validate()?;
    options
        .cancellation
        .check("signature-validation-before-discovery")?;
    let reader = doc.reader();
    let file = reader.file_bytes();
    validate_evidence_bundle_document_binding(options, file)?;
    let dss = read_dss_index(reader);
    let mut reports = Vec::new();

    for (idx, field) in find_signature_fields(doc).into_iter().enumerate() {
        options
            .cancellation
            .check("signature-validation-before-signature")?;
        reports.extend(verify_one(&field, file, idx + 1, &dss, options));
        options
            .cancellation
            .check("signature-validation-after-signature")?;
    }
    Ok(reports)
}

/// Verify signatures and capture only evidence that the validation pipeline
/// actually accepted for path construction or revocation evaluation. The
/// returned bundle remains untrusted input on import and is revalidated during
/// every offline replay.
pub fn verify_signatures_with_options_and_evidence(
    doc: &PdfDocument,
    options: &VerifyOptions,
) -> Result<SignatureValidationOutcome> {
    options.algorithm_policy.validate()?;
    options
        .cancellation
        .check("signature-validation-before-discovery")?;
    let reader = doc.reader();
    let file = reader.file_bytes();
    validate_evidence_bundle_document_binding(options, file)?;
    let dss = read_dss_index(reader);
    let policy = effective_retrieval_policy(options);
    let mut evidence = match &options.evidence_bundle {
        Some(bundle) => EvidenceStore::import_bundle(
            bundle,
            policy.budget.max_cache_entries,
            policy.budget.max_cache_bytes,
        )
        .map_err(|error| OxideError::invalid_input(format!("evidence bundle: {error}")))?,
        None => EvidenceStore::new(
            policy.budget.max_cache_entries,
            policy.budget.max_cache_bytes,
        ),
    };
    let mut reports = Vec::new();

    for (idx, field) in find_signature_fields(doc).into_iter().enumerate() {
        options
            .cancellation
            .check("signature-validation-before-signature")?;
        let verification = verify_one_with_evidence(&field, file, idx + 1, &dss, options);
        for record in verification.evidence_records {
            evidence.insert(record).map_err(|error| {
                OxideError::invalid_input(format!("validated evidence: {error}"))
            })?;
        }
        reports.extend(verification.reports);
        options
            .cancellation
            .check("signature-validation-after-signature")?;
    }

    let (validation_time_unix, _) = validation_time(options);
    Ok(SignatureValidationOutcome {
        reports,
        evidence_bundle: evidence.export_bundle(
            Some(hex_lower(&Sha256::digest(file))),
            None,
            Some(validation_time_unix),
            Some(validation_policy_fingerprint(options)),
        ),
    })
}

/// Validate one RFC 3161 signature timestamp token against the exact CMS
/// signature value bytes it claims to timestamp.
///
/// The token is not accepted from structure alone: this verifies the
/// `messageImprint`, token CMS signature, TSA certificate EKU, and TSA path at
/// `genTime` using the same explicit trust and revocation options as PDF
/// signature validation. No network retrieval occurs unless `options`
/// explicitly enables the bounded Prompt 24 evidence transport.
pub fn verify_signature_timestamp_token_der(
    token_der: &[u8],
    signature_value: &[u8],
    options: &VerifyOptions,
) -> Result<TimestampValidationReport> {
    options.algorithm_policy.validate()?;
    options
        .cancellation
        .check("timestamp-validation-before-parse")?;
    Ok(validate_signature_timestamp_token(
        token_der,
        "caller_supplied_signature_timestamp_token".to_string(),
        signature_value,
        options,
        &options.algorithm_policy,
    ))
}

/// A portable evidence bundle is scoped to the source PDF when its producer
/// supplied that identity. Evidence still undergoes normal cryptographic and
/// freshness validation, but rejecting an explicit document mismatch avoids
/// accidentally replaying a bundle selected for another signing workflow.
fn validate_evidence_bundle_document_binding(options: &VerifyOptions, file: &[u8]) -> Result<()> {
    let Some(bundle) = &options.evidence_bundle else {
        return Ok(());
    };
    let Some(expected) = bundle.source_document_sha256.as_deref() else {
        return Ok(());
    };
    let actual = hex_lower(&Sha256::digest(file));
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(OxideError::invalid_input(
            "evidence bundle source_document_sha256 does not match the PDF being validated",
        ));
    }
    Ok(())
}

/// Embed PAdES long-term-validation material in a catalog `/DSS` dictionary.
///
/// This is an incremental update: original bytes stay untouched, existing
/// signatures remain cryptographically valid for their signed byte ranges, and
/// their coverage will correctly report `modified_after_signing` after the DSS
/// append. The writer embeds caller-supplied OCSP/CRL bytes as opaque streams
/// and adds signer certificates already present in CMS signatures.
pub fn add_ltv_material(doc: &PdfDocument, material: &LtvMaterial) -> Result<Vec<u8>> {
    let reader = doc.reader();
    if reader.is_encrypted() {
        return Err(OxideError::UnsupportedFeature(
            "embedding LTV/DSS material in encrypted inputs is not yet supported".to_string(),
        ));
    }
    if material.is_empty() {
        return Err(OxideError::MalformedPdf(
            "LTV/DSS material must include at least one cert, OCSP response, or CRL".to_string(),
        ));
    }

    let fields = find_signature_fields(doc);
    if fields.is_empty() {
        return Err(OxideError::MalformedPdf(
            "LTV/DSS embedding requires at least one signature field".to_string(),
        ));
    }

    if let Some(index) = material.signature_index {
        if index == 0 || index > fields.len() {
            return Err(OxideError::MalformedPdf(format!(
                "LTV signature_index {index} is out of range for {} signature(s)",
                fields.len()
            )));
        }
    }

    let selected = fields
        .iter()
        .enumerate()
        .filter(|(idx, _)| material.signature_index.is_none_or(|n| n == idx + 1))
        .collect::<Vec<_>>();

    let (root_number, root_generation) = reader.root_reference().ok_or_else(|| {
        OxideError::MalformedPdf("LTV/DSS writer: trailer is missing /Root".to_string())
    })?;
    let mut catalog = reader
        .get_object(root_number, root_generation)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("/Root is not a dictionary".to_string()))?;

    let mut certs = material.certificates_der.clone();
    for (_, field) in &selected {
        if let Some(contents) = field
            .sig_dict
            .get("Contents")
            .and_then(PdfObject::as_string)
        {
            for der in cms_certificate_der(contents) {
                push_unique_bytes(&mut certs, der);
            }
        }
    }
    for cert in &certs {
        Certificate::from_der(cert)
            .map_err(|e| OxideError::MalformedPdf(format!("LTV certificate DER: {e}")))?;
    }
    for crl in &material.crls_der {
        CertificateList::from_der(crl)
            .map_err(|e| OxideError::MalformedPdf(format!("LTV CRL DER: {e}")))?;
    }

    let next = next_free_object_number(reader);
    let dss_number = next;
    let mut number = next + 1;
    let mut raw_objects = Vec::new();

    let cert_refs = append_dss_streams(&mut raw_objects, &mut number, &certs);
    let ocsp_refs = append_dss_streams(&mut raw_objects, &mut number, &material.ocsp_responses_der);
    let crl_refs = append_dss_streams(&mut raw_objects, &mut number, &material.crls_der);

    let mut vri = PdfDictionary::empty();
    for (_, field) in selected {
        let Some(contents) = field
            .sig_dict
            .get("Contents")
            .and_then(PdfObject::as_string)
        else {
            continue;
        };
        let key = signature_vri_key(contents);
        let mut entry = PdfDictionary::empty();
        if !cert_refs.is_empty() {
            entry.insert("Cert", PdfObject::Array(cert_refs.clone()));
        }
        if !ocsp_refs.is_empty() {
            entry.insert("OCSP", PdfObject::Array(ocsp_refs.clone()));
        }
        if !crl_refs.is_empty() {
            entry.insert("CRL", PdfObject::Array(crl_refs.clone()));
        }
        vri.insert(key, PdfObject::Dictionary(entry));
    }

    let mut dss = PdfDictionary::empty();
    if !cert_refs.is_empty() {
        dss.insert("Certs", PdfObject::Array(cert_refs));
    }
    if !ocsp_refs.is_empty() {
        dss.insert("OCSPs", PdfObject::Array(ocsp_refs));
    }
    if !crl_refs.is_empty() {
        dss.insert("CRLs", PdfObject::Array(crl_refs));
    }
    dss.insert("VRI", PdfObject::Dictionary(vri));

    catalog.insert("DSS", reference(dss_number, 0));
    raw_objects.push(raw_object(
        root_number,
        root_generation,
        &PdfObject::Dictionary(catalog),
    ));
    raw_objects.push(raw_object(dss_number, 0, &PdfObject::Dictionary(dss)));

    write_incremental_update_raw(reader, raw_objects)
}

/// Apply an RSA/SHA-256 detached CMS signature as an incremental update.
///
/// The returned bytes preserve the original input as an exact prefix, append a
/// signature field/widget plus signature dictionary, patch `/ByteRange`, and
/// fill the `/Contents` placeholder with DER CMS. A caller-supplied timestamp
/// token can be embedded in CMS; DSS revocation/certificate material is added
/// afterward with [`add_ltv_material`]. Trust-chain and network policy remain
/// caller-owned.
pub fn sign_document(
    doc: &PdfDocument,
    signer: &PdfSigner,
    options: &SignatureOptions,
) -> Result<Vec<u8>> {
    let reader = doc.reader();
    if reader.is_encrypted() {
        return Err(OxideError::UnsupportedFeature(
            "digital signing encrypted inputs is not yet supported".to_string(),
        ));
    }
    if signer.certificates.is_empty() {
        return Err(OxideError::MalformedPdf(
            "digital signing requires a signer certificate".to_string(),
        ));
    }
    if options.contents_reserved_bytes == 0 {
        return Err(OxideError::MalformedPdf(
            "signature /Contents placeholder must reserve at least one byte".to_string(),
        ));
    }

    let page_index = options.page.checked_sub(1).ok_or_else(|| {
        OxideError::MalformedPdf("signature page numbers are 1-based".to_string())
    })?;
    let pages = doc.get_pages()?;
    let page = pages.get(page_index).ok_or_else(|| {
        OxideError::MalformedPdf(format!(
            "signature target page {} is out of range",
            options.page
        ))
    })?;

    let (root_number, root_generation) = reader.root_reference().ok_or_else(|| {
        OxideError::MalformedPdf("signature writer: trailer is missing /Root".to_string())
    })?;
    let mut catalog = reader
        .get_object(root_number, root_generation)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("/Root is not a dictionary".to_string()))?;
    let mut page_dict = reader
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("target page is not a dictionary".to_string()))?;

    let next = next_free_object_number(reader);
    let sig_number = next;
    let field_number = next + 1;
    let appearance_number = options.rect.map(|_| next + 2);
    let acroform_number = match catalog.get_reference("AcroForm") {
        Some((number, _)) => number,
        None => appearance_number.map_or(next + 2, |n| n + 1),
    };

    let sig_ref = reference(sig_number, 0);
    let field_ref = reference(field_number, 0);
    let page_ref = reference(page.object_number, page.generation_number);

    let (mut acroform, acroform_ref) = match catalog.get("AcroForm") {
        Some(PdfObject::Reference { number, generation }) => {
            let dict = reader
                .get_object(*number, *generation)?
                .as_dict()
                .cloned()
                .ok_or_else(|| OxideError::MalformedPdf("/AcroForm is not a dictionary".into()))?;
            (dict, reference(*number, *generation))
        }
        Some(PdfObject::Dictionary(dict)) => (dict.clone(), reference(acroform_number, 0)),
        Some(_) | None => (PdfDictionary::empty(), reference(acroform_number, 0)),
    };

    let mut fields = resolve_array(acroform.get("Fields"), reader).unwrap_or_default();
    fields.push(field_ref.clone());
    acroform.insert("Fields", PdfObject::Array(fields));
    acroform.insert("SigFlags", PdfObject::Integer(3));
    catalog.insert("AcroForm", acroform_ref.clone());

    let mut annots = resolve_array(page_dict.get("Annots"), reader).unwrap_or_default();
    annots.push(field_ref.clone());
    page_dict.insert("Annots", PdfObject::Array(annots));

    let rect = options.rect.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let mut field = PdfDictionary::empty();
    field.insert("Type", PdfObject::Name("Annot".to_string()));
    field.insert("Subtype", PdfObject::Name("Widget".to_string()));
    field.insert("FT", PdfObject::Name("Sig".to_string()));
    field.insert(
        "T",
        PdfObject::String(options.field_name.as_bytes().to_vec()),
    );
    field.insert("F", PdfObject::Integer(132));
    field.insert("Rect", rect_object(rect));
    field.insert("P", page_ref);
    field.insert("V", sig_ref.clone());
    if let Some(ap_number) = appearance_number {
        let mut ap = PdfDictionary::empty();
        ap.insert("N", reference(ap_number, 0));
        field.insert("AP", PdfObject::Dictionary(ap));
    }

    let mut raw_objects = vec![
        raw_object(
            root_number,
            root_generation,
            &PdfObject::Dictionary(catalog),
        ),
        raw_object(
            page.object_number,
            page.generation_number,
            &PdfObject::Dictionary(page_dict),
        ),
        raw_object(
            acroform_ref.as_reference().unwrap().0,
            acroform_ref.as_reference().unwrap().1,
            &PdfObject::Dictionary(acroform),
        ),
        raw_object(field_number, 0, &PdfObject::Dictionary(field)),
        RawIncrementalObject {
            number: sig_number,
            generation: 0,
            body: signature_dictionary_body(options, options.contents_reserved_bytes),
        },
    ];

    if let (Some(ap_number), Some(rect)) = (appearance_number, options.rect) {
        raw_objects.push(raw_object(ap_number, 0, &appearance_stream(options, rect)));
    }

    let mut staged = write_incremental_update_raw(reader, raw_objects)?;
    let byte_range_start = find_unique(&staged, BYTE_RANGE_PLACEHOLDER)?;
    let contents_marker = contents_placeholder(options.contents_reserved_bytes);
    let contents_marker_start = find_unique(&staged, &contents_marker)?;
    let contents_hex_start = contents_marker_start + 1;
    let contents_after = contents_marker_start + contents_marker.len();
    let byte_range = ByteRange {
        a: 0,
        b: contents_marker_start,
        c: contents_after,
        d: staged.len().saturating_sub(contents_after),
    };
    patch_byte_range(&mut staged, byte_range_start, &byte_range)?;

    let signed_bytes = extract_signed_bytes(&staged, &byte_range).ok_or_else(|| {
        OxideError::MalformedPdf("signature writer produced an invalid /ByteRange".to_string())
    })?;
    let digest = Sha256::digest(&signed_bytes);
    let cms = build_detached_cms(signer, &digest, options.timestamp_token_der.as_deref())?;
    if cms.len() > options.contents_reserved_bytes {
        return Err(OxideError::ResourceLimit(format!(
            "CMS signature is {} bytes but /Contents reserved only {} bytes",
            cms.len(),
            options.contents_reserved_bytes
        )));
    }
    patch_contents_hex(
        &mut staged,
        contents_hex_start,
        options.contents_reserved_bytes,
        &cms,
    );

    Ok(staged)
}

// ===========================================================================
// Prompt 26 — append-only incremental signing engine
// ===========================================================================

/// Signing intent: approval, or a certification (DocMDP) signature with an
/// exact permission level (1 = no changes, 2 = form fill + signing, 3 =
/// annotations + form fill + signing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningIntent {
    Approval,
    Certification { docmdp_permissions: u8 },
}

/// Status of an embedded document timestamp for a signing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentTimestampStatus {
    /// A caller-supplied RFC 3161 token was embedded as a signature timestamp.
    EmbeddedFromCaller,
    /// No timestamp was requested.
    NotRequested,
    /// A timestamp was requested but no TSA endpoint/policy is configured; the
    /// engine never performs a default network TSA call, so this is deferred.
    DeferredNoTsaConfigured,
}

/// Options for [`sign_incremental`].
#[derive(Debug, Clone)]
pub struct IncrementalSigningOptions {
    /// Base signature-dictionary options (field name, rect, /M, reserved bytes,
    /// caller-supplied timestamp token, ...).
    pub signature: SignatureOptions,
    /// Signing intent (approval vs certification/DocMDP).
    pub intent: SigningIntent,
    /// If the CMS does not fit the reserved placeholder, retry once with a
    /// larger placeholder instead of failing.
    pub retry_larger_placeholder: bool,
    /// Hard cap on placeholder growth during retry.
    pub max_placeholder_bytes: usize,
}

impl Default for IncrementalSigningOptions {
    fn default() -> Self {
        Self {
            signature: SignatureOptions::default(),
            intent: SigningIntent::Approval,
            retry_larger_placeholder: true,
            max_placeholder_bytes: 256 * 1024,
        }
    }
}

/// A signing request handed to an [`ExternalSigner`]. It carries only the
/// digest, algorithm, certificate identity, profile intent, an operation id,
/// and the placeholder size — never private keys, passwords, or document bytes.
#[derive(Debug, Clone, Serialize)]
pub struct CmsSigningRequest {
    pub algorithm: String,
    /// Lowercase hex SHA-256 digest of the exact signed bytes.
    pub digest_sha256_hex: String,
    /// Expected signer-certificate SHA-256 fingerprint (uppercase hex), if the
    /// caller pinned one.
    pub expected_certificate_sha256: Option<String>,
    /// Profile intent, e.g. `approval`, `certification`, `pades-b-b`.
    pub profile_intent: String,
    pub operation_id: String,
    pub reserved_bytes: usize,
}

/// An external signer's response. The negotiated mode here is "return complete
/// detached CMS `ContentInfo` DER"; HSM/KMS adapters commonly produce this.
#[derive(Debug, Clone)]
pub struct CmsSigningResult {
    pub cms_der: Vec<u8>,
    pub algorithm: String,
    /// SHA-256 fingerprint (uppercase hex) of the certificate the signer used.
    pub signer_certificate_sha256: String,
}

/// External signer callback (HSM/KMS-style). Implementations must not receive
/// document bytes or private keys through Oxide; they receive a structured
/// [`CmsSigningRequest`] and return a [`CmsSigningResult`].
pub trait ExternalSigner {
    fn sign_cms(
        &self,
        request: &CmsSigningRequest,
    ) -> std::result::Result<CmsSigningResult, String>;
}

/// Which signer produces the CMS.
pub enum IncrementalSigner<'a> {
    /// Local key-provider signing using an in-process [`PdfSigner`].
    Local(&'a PdfSigner),
    /// External CMS-producing signer (callback), with an optional pinned
    /// certificate fingerprint (uppercase hex SHA-256) the response must match.
    ExternalCms {
        signer: &'a dyn ExternalSigner,
        expected_certificate_sha256: Option<String>,
    },
}

/// Post-sign validation of a generated signed PDF, produced by reopening the
/// output and running the Prompt 24/25 validators.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PostSignValidationReport {
    pub structural_open: bool,
    pub byte_range_exact: bool,
    pub cms_parsed: bool,
    pub signature_valid: bool,
    pub coverage_whole_file: bool,
    pub docmdp_evaluated: bool,
    pub overall_pass: bool,
    /// Serialized last [`SignatureReport`] for evidence (contains no secrets).
    pub signature_report_json: String,
}

/// The result of an incremental signing run.
#[derive(Debug, Clone, Serialize)]
pub struct IncrementalSignResult {
    #[serde(skip_serializing)]
    pub signed_pdf: Vec<u8>,
    pub original_len: usize,
    pub signed_len: usize,
    pub reserved_bytes: usize,
    pub cms_len: usize,
    pub retried: bool,
    pub certification: bool,
    pub prefix_preserved: bool,
    pub timestamp_status: DocumentTimestampStatus,
    pub post_sign: PostSignValidationReport,
}

/// A staged, signer-agnostic incremental signature revision: the file bytes
/// with a patched `/ByteRange` and an empty `/Contents` placeholder, plus the
/// digest of the exact signed bytes. This is the clean CMS-insertion boundary.
struct StagedSignature {
    staged: Vec<u8>,
    contents_hex_start: usize,
    digest: Vec<u8>,
}

fn signature_dictionary_body_ext(
    options: &SignatureOptions,
    reserved_bytes: usize,
    docmdp_p: Option<u8>,
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"<<\n");
    body.extend_from_slice(b"/Type /Sig\n");
    body.extend_from_slice(b"/Filter /Adobe.PPKLite\n");
    body.extend_from_slice(b"/SubFilter /adbe.pkcs7.detached\n");
    body.extend_from_slice(b"/ByteRange ");
    body.extend_from_slice(BYTE_RANGE_PLACEHOLDER);
    body.extend_from_slice(b"\n/Contents ");
    body.extend_from_slice(&contents_placeholder(reserved_bytes));
    body.extend_from_slice(b"\n");
    push_optional_pdf_string(&mut body, "Name", options.signer_name.as_deref());
    push_optional_pdf_string(&mut body, "Reason", options.reason.as_deref());
    push_optional_pdf_string(&mut body, "Location", options.location.as_deref());
    push_optional_pdf_string(&mut body, "ContactInfo", options.contact_info.as_deref());
    push_optional_pdf_string(&mut body, "M", options.signing_time.as_deref());
    if let Some(p) = docmdp_p {
        body.extend_from_slice(
            format!(
                "/Reference [ << /Type /SigRef /TransformMethod /DocMDP /TransformParams << /Type /TransformParams /P {p} /V /1.2 >> /DigestMethod /SHA256 >> ]\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(b">>");
    body
}

/// Stage an append-only signature revision (no CMS yet). Mirrors the approval
/// [`sign_document`] staging and additionally supports certification (DocMDP)
/// signatures via `/Reference` + catalog `/Perms /DocMDP`.
fn stage_signature(
    doc: &PdfDocument,
    options: &SignatureOptions,
    reserved_bytes: usize,
    docmdp_p: Option<u8>,
) -> Result<StagedSignature> {
    let reader = doc.reader();
    if reader.is_encrypted() {
        return Err(OxideError::UnsupportedFeature(
            "digital signing encrypted inputs is not yet supported".to_string(),
        ));
    }
    if reserved_bytes == 0 {
        return Err(OxideError::MalformedPdf(
            "signature /Contents placeholder must reserve at least one byte".to_string(),
        ));
    }
    let page_index = options.page.checked_sub(1).ok_or_else(|| {
        OxideError::MalformedPdf("signature page numbers are 1-based".to_string())
    })?;
    let pages = doc.get_pages()?;
    let page = pages.get(page_index).ok_or_else(|| {
        OxideError::MalformedPdf(format!(
            "signature target page {} is out of range",
            options.page
        ))
    })?;

    let (root_number, root_generation) = reader.root_reference().ok_or_else(|| {
        OxideError::MalformedPdf("signature writer: trailer is missing /Root".to_string())
    })?;
    let mut catalog = reader
        .get_object(root_number, root_generation)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("/Root is not a dictionary".to_string()))?;
    let mut page_dict = reader
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("target page is not a dictionary".to_string()))?;

    let next = next_free_object_number(reader);
    let sig_number = next;
    let field_number = next + 1;
    let appearance_number = options.rect.map(|_| next + 2);
    let acroform_number = match catalog.get_reference("AcroForm") {
        Some((number, _)) => number,
        None => appearance_number.map_or(next + 2, |n| n + 1),
    };

    let sig_ref = reference(sig_number, 0);
    let field_ref = reference(field_number, 0);
    let page_ref = reference(page.object_number, page.generation_number);

    let (mut acroform, acroform_ref) = match catalog.get("AcroForm") {
        Some(PdfObject::Reference { number, generation }) => {
            let dict = reader
                .get_object(*number, *generation)?
                .as_dict()
                .cloned()
                .ok_or_else(|| OxideError::MalformedPdf("/AcroForm is not a dictionary".into()))?;
            (dict, reference(*number, *generation))
        }
        Some(PdfObject::Dictionary(dict)) => (dict.clone(), reference(acroform_number, 0)),
        Some(_) | None => (PdfDictionary::empty(), reference(acroform_number, 0)),
    };

    let mut fields = resolve_array(acroform.get("Fields"), reader).unwrap_or_default();
    fields.push(field_ref.clone());
    acroform.insert("Fields", PdfObject::Array(fields));
    acroform.insert("SigFlags", PdfObject::Integer(3));
    catalog.insert("AcroForm", acroform_ref.clone());

    if docmdp_p.is_some() {
        // Certification signatures record the modification-detection policy in
        // the catalog `/Perms /DocMDP` pointing at the signature dictionary.
        let mut perms = PdfDictionary::empty();
        perms.insert("DocMDP", sig_ref.clone());
        catalog.insert("Perms", PdfObject::Dictionary(perms));
    }

    let mut annots = resolve_array(page_dict.get("Annots"), reader).unwrap_or_default();
    annots.push(field_ref.clone());
    page_dict.insert("Annots", PdfObject::Array(annots));

    let rect = options.rect.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let mut field = PdfDictionary::empty();
    field.insert("Type", PdfObject::Name("Annot".to_string()));
    field.insert("Subtype", PdfObject::Name("Widget".to_string()));
    field.insert("FT", PdfObject::Name("Sig".to_string()));
    field.insert(
        "T",
        PdfObject::String(options.field_name.as_bytes().to_vec()),
    );
    field.insert("F", PdfObject::Integer(132));
    field.insert("Rect", rect_object(rect));
    field.insert("P", page_ref);
    field.insert("V", sig_ref.clone());
    if let Some(ap_number) = appearance_number {
        let mut ap = PdfDictionary::empty();
        ap.insert("N", reference(ap_number, 0));
        field.insert("AP", PdfObject::Dictionary(ap));
    }

    let mut raw_objects = vec![
        raw_object(
            root_number,
            root_generation,
            &PdfObject::Dictionary(catalog),
        ),
        raw_object(
            page.object_number,
            page.generation_number,
            &PdfObject::Dictionary(page_dict),
        ),
        raw_object(
            acroform_ref.as_reference().unwrap().0,
            acroform_ref.as_reference().unwrap().1,
            &PdfObject::Dictionary(acroform),
        ),
        raw_object(field_number, 0, &PdfObject::Dictionary(field)),
        RawIncrementalObject {
            number: sig_number,
            generation: 0,
            body: signature_dictionary_body_ext(options, reserved_bytes, docmdp_p),
        },
    ];

    if let (Some(ap_number), Some(rect)) = (appearance_number, options.rect) {
        raw_objects.push(raw_object(ap_number, 0, &appearance_stream(options, rect)));
    }

    let mut staged = write_incremental_update_raw(reader, raw_objects)?;
    let byte_range_start = find_unique(&staged, BYTE_RANGE_PLACEHOLDER)?;
    let contents_marker = contents_placeholder(reserved_bytes);
    let contents_marker_start = find_unique(&staged, &contents_marker)?;
    let contents_hex_start = contents_marker_start + 1;
    let contents_after = contents_marker_start + contents_marker.len();
    let byte_range = ByteRange {
        a: 0,
        b: contents_marker_start,
        c: contents_after,
        d: staged.len().saturating_sub(contents_after),
    };
    patch_byte_range(&mut staged, byte_range_start, &byte_range)?;

    let signed_bytes = extract_signed_bytes(&staged, &byte_range).ok_or_else(|| {
        OxideError::MalformedPdf("signature writer produced an invalid /ByteRange".to_string())
    })?;
    let digest = Sha256::digest(&signed_bytes).to_vec();
    Ok(StagedSignature {
        staged,
        contents_hex_start,
        digest,
    })
}

/// Reopen a generated signed PDF and validate it with the Prompt 24/25 engine.
fn post_sign_validate(signed: &[u8]) -> PostSignValidationReport {
    let mut report = PostSignValidationReport::default();
    let Ok(doc) = crate::document::PdfDocument::open_bytes(signed.to_vec()) else {
        return report;
    };
    report.structural_open = true;
    let Ok(reports) = verify_signatures(&doc) else {
        return report;
    };
    if let Some(rep) = reports.last() {
        report.byte_range_exact = rep.checks.byte_range_contents_gap_matches;
        report.cms_parsed = rep.checks.contents_present;
        report.signature_valid = matches!(rep.validity, SignatureValidity::Valid);
        report.coverage_whole_file = matches!(rep.coverage, Coverage::WholeFile);
        report.docmdp_evaluated = rep.checks.docmdp_evaluated;
        report.signature_report_json = serde_json::to_string(rep).unwrap_or_default();
    }
    report.overall_pass =
        report.structural_open && report.signature_valid && report.byte_range_exact;
    report
}

/// Append-only incremental signing engine entry point.
///
/// Produces a signed PDF whose original bytes are preserved as an exact prefix,
/// with a placeholder-planned `/Contents`, a patched `/ByteRange` computed over
/// the exact signed bytes, a CMS produced by a local or external signer, and a
/// mandatory post-sign reopen + validation. A generated signature is only
/// returned when post-sign validation confirms the CMS is mathematically valid
/// over the exact signed bytes.
pub fn sign_incremental(
    doc: &PdfDocument,
    signer: IncrementalSigner<'_>,
    options: &IncrementalSigningOptions,
) -> Result<IncrementalSignResult> {
    let original_len = doc.reader().file_bytes().len();

    let docmdp_p = match options.intent {
        SigningIntent::Approval => None,
        SigningIntent::Certification { docmdp_permissions } => {
            if !(1..=3).contains(&docmdp_permissions) {
                return Err(OxideError::MalformedPdf(format!(
                    "certification DocMDP permission must be 1, 2, or 3 (got {docmdp_permissions})"
                )));
            }
            Some(docmdp_permissions)
        }
    };
    if let IncrementalSigner::Local(local) = &signer {
        if local.certificates.is_empty() {
            return Err(OxideError::MalformedPdf(
                "digital signing requires a signer certificate".to_string(),
            ));
        }
    }

    let profile_intent = if docmdp_p.is_some() {
        "certification"
    } else {
        "approval"
    };
    let timestamp_status = if options.signature.timestamp_token_der.is_some() {
        DocumentTimestampStatus::EmbeddedFromCaller
    } else {
        DocumentTimestampStatus::NotRequested
    };

    let mut reserved = options.signature.contents_reserved_bytes.max(1);
    let mut retried = false;
    loop {
        let staged = stage_signature(doc, &options.signature, reserved, docmdp_p)?;
        let operation_id = format!(
            "op-{}",
            hex_upper(&staged.digest[..staged.digest.len().min(8)])
        );

        let cms = match &signer {
            IncrementalSigner::Local(local) => build_detached_cms(
                local,
                &staged.digest,
                options.signature.timestamp_token_der.as_deref(),
            )?,
            IncrementalSigner::ExternalCms {
                signer,
                expected_certificate_sha256,
            } => {
                let request = CmsSigningRequest {
                    algorithm: "RSASSA-PKCS1v1_5-SHA256".to_string(),
                    digest_sha256_hex: hex_lower(&staged.digest),
                    expected_certificate_sha256: expected_certificate_sha256.clone(),
                    profile_intent: profile_intent.to_string(),
                    operation_id,
                    reserved_bytes: reserved,
                };
                let result = signer.sign_cms(&request).map_err(|e| {
                    OxideError::MalformedPdf(format!("external signer returned an error: {e}"))
                })?;
                // Reject a non-CMS / malformed response before inserting it.
                ContentInfo::from_der(&result.cms_der).map_err(|e| {
                    OxideError::MalformedPdf(format!(
                        "external signer response is not a valid CMS ContentInfo: {e}"
                    ))
                })?;
                // Reject a response that used the wrong certificate.
                if let Some(expected) = expected_certificate_sha256 {
                    if !result
                        .signer_certificate_sha256
                        .eq_ignore_ascii_case(expected)
                    {
                        return Err(OxideError::MalformedPdf(
                            "external signer used a certificate that does not match the pinned fingerprint"
                                .to_string(),
                        ));
                    }
                }
                // Reject a wrong-algorithm response.
                if !result
                    .algorithm
                    .eq_ignore_ascii_case("RSASSA-PKCS1v1_5-SHA256")
                    && !result.algorithm.eq_ignore_ascii_case("RSA-SHA256")
                    && !result
                        .algorithm
                        .eq_ignore_ascii_case("sha256WithRSAEncryption")
                {
                    return Err(OxideError::UnsupportedFeature(format!(
                        "external signer negotiated an unsupported algorithm: {}",
                        result.algorithm
                    )));
                }
                result.cms_der
            }
        };

        if cms.len() > reserved {
            if options.retry_larger_placeholder && reserved < options.max_placeholder_bytes {
                let grown = cms.len().saturating_add(512);
                reserved = grown.min(options.max_placeholder_bytes).max(reserved + 1);
                retried = true;
                continue;
            }
            return Err(OxideError::ResourceLimit(format!(
                "CMS signature is {} bytes but /Contents reserved only {} bytes",
                cms.len(),
                reserved
            )));
        }

        let mut out = staged.staged;
        patch_contents_hex(&mut out, staged.contents_hex_start, reserved, &cms);

        let prefix_preserved = out
            .get(..original_len)
            .map(|prefix| prefix == doc.reader().file_bytes())
            .unwrap_or(false);

        let post_sign = post_sign_validate(&out);
        if !post_sign.signature_valid {
            return Err(OxideError::MalformedPdf(
                "post-sign validation failed: the generated signature is not mathematically valid over the signed bytes"
                    .to_string(),
            ));
        }

        return Ok(IncrementalSignResult {
            original_len,
            signed_len: out.len(),
            reserved_bytes: reserved,
            cms_len: cms.len(),
            retried,
            certification: docmdp_p.is_some(),
            prefix_preserved,
            timestamp_status,
            post_sign,
            signed_pdf: out,
        });
    }
}

/// A [`SignaturePlaceholderPlan`] describes the reserved capacity, the required
/// CMS size for a given signer, and whether it fits. This lets callers size the
/// `/Contents` placeholder before committing to a signing run.
#[derive(Debug, Clone, Serialize)]
pub struct SignaturePlaceholderPlan {
    pub reserved_bytes: usize,
    pub required_bytes: usize,
    pub fits: bool,
    pub byte_range: [usize; 4],
    pub signed_digest_sha256_hex: String,
}

/// Plan the `/Contents` placeholder for a local signer without producing a
/// final signed document: stages the revision, builds the CMS, and reports the
/// exact required vs. reserved capacity.
pub fn plan_signature_placeholder(
    doc: &PdfDocument,
    signer: &PdfSigner,
    options: &IncrementalSigningOptions,
) -> Result<SignaturePlaceholderPlan> {
    let docmdp_p = match options.intent {
        SigningIntent::Approval => None,
        SigningIntent::Certification { docmdp_permissions } => Some(docmdp_permissions),
    };
    let reserved = options.signature.contents_reserved_bytes.max(1);
    let staged = stage_signature(doc, &options.signature, reserved, docmdp_p)?;
    let cms = build_detached_cms(
        signer,
        &staged.digest,
        options.signature.timestamp_token_der.as_deref(),
    )?;
    // Recover the byte range from the staged bytes for reporting.
    let contents_marker = contents_placeholder(reserved);
    let contents_marker_start = find_unique(&staged.staged, &contents_marker)?;
    let contents_after = contents_marker_start + contents_marker.len();
    let byte_range = [
        0,
        contents_marker_start,
        contents_after,
        staged.staged.len().saturating_sub(contents_after),
    ];
    Ok(SignaturePlaceholderPlan {
        reserved_bytes: reserved,
        required_bytes: cms.len(),
        fits: cms.len() <= reserved,
        byte_range,
        signed_digest_sha256_hex: hex_lower(&staged.digest),
    })
}

/// A located signature field with its signature dictionary.
struct SigField {
    field_name: Option<String>,
    sig_dict: PdfDictionary,
    discovery_kind: SignatureDiscoveryKind,
    field_object: Option<(u32, u16)>,
    signature_object: Option<(u32, u16)>,
    contents_span: std::result::Result<(usize, usize), String>,
    discovery_issue: Option<String>,
}

/// Walk `/AcroForm /Fields` and then enumerate standalone `/Type /Sig`
/// dictionaries. A signature-like dictionary outside a field is retained as an
/// orphaned inventory entry rather than silently ignored or promoted to a form
/// signature. Inherited `/FT` and nested `/Kids` are handled for field-owned
/// signatures.
fn find_signature_fields(doc: &PdfDocument) -> Vec<SigField> {
    let reader = doc.reader();
    let mut out = Vec::new();
    let mut claimed_signature_objects = std::collections::HashSet::new();
    if let Ok(catalog) = doc.get_catalog() {
        if let Some(acroform) = resolve_dict(catalog.get("AcroForm"), reader) {
            if let Some(fields) = resolve_array(acroform.get("Fields"), reader) {
                let mut visited = std::collections::HashSet::new();
                for field in &fields {
                    walk_field(
                        field,
                        None,
                        reader,
                        &mut out,
                        &mut visited,
                        &mut claimed_signature_objects,
                        0,
                    );
                }
            }
        }
    }

    for (number, generation) in reader.object_ids() {
        if claimed_signature_objects.contains(&(number, generation)) {
            continue;
        }
        let Ok(PdfObject::Dictionary(sig_dict)) = reader.get_object(number, generation) else {
            continue;
        };
        if sig_dict.get_name("Type") != Some("Sig") {
            continue;
        }
        let object = (number, generation);
        out.push(SigField {
            field_name: None,
            sig_dict,
            discovery_kind: SignatureDiscoveryKind::OrphanedSignatureDictionary,
            field_object: None,
            signature_object: Some(object),
            contents_span: contents_span_for_object(reader, object),
            discovery_issue: None,
        });
    }
    out
}

fn walk_field(
    field_obj: &PdfObject,
    inherited_ft: Option<&str>,
    reader: &PdfReader,
    out: &mut Vec<SigField>,
    visited: &mut std::collections::HashSet<(u32, u16)>,
    claimed_signature_objects: &mut std::collections::HashSet<(u32, u16)>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let field_object = field_obj.as_reference();
    if let Some((number, generation)) = field_object {
        if !visited.insert((number, generation)) {
            return;
        }
    }
    let Ok(PdfObject::Dictionary(field)) = reader.resolve(field_obj.clone()) else {
        return;
    };
    let ft = field.get_name("FT").or(inherited_ft);

    // A signature field: /FT /Sig with a /V signature dictionary.
    if ft == Some("Sig") {
        if let Some(sig_dict) = resolve_dict(field.get("V"), reader) {
            let signature_object = field.get("V").and_then(PdfObject::as_reference);
            let source_object = signature_object.or(field_object);
            let discovery_issue = signature_object.and_then(|object| {
                (!claimed_signature_objects.insert(object)).then(|| {
                    format!(
                        "multiple AcroForm signature fields reference signature object {} {} R",
                        object.0, object.1
                    )
                })
            });
            out.push(SigField {
                field_name: field.get("T").and_then(decode_text_string),
                sig_dict,
                discovery_kind: SignatureDiscoveryKind::AcroformSignatureField,
                field_object,
                signature_object,
                contents_span: source_object
                    .map(|object| contents_span_for_object(reader, object))
                    .unwrap_or_else(|| {
                        Err("signature dictionary has no indirect source object".to_string())
                    }),
                discovery_issue,
            });
        }
    }

    // Recurse into /Kids.
    if let Some(kids) = resolve_array(field.get("Kids"), reader) {
        for kid in &kids {
            walk_field(
                kid,
                ft,
                reader,
                out,
                visited,
                claimed_signature_objects,
                depth + 1,
            );
        }
    }
}

fn contents_span_for_object(
    reader: &PdfReader,
    object: (u32, u16),
) -> std::result::Result<(usize, usize), String> {
    let range = reader
        .uncompressed_object_range(object.0, object.1)
        .ok_or_else(|| {
            format!(
                "signature object {} {} R has no uncompressed raw source range",
                object.0, object.1
            )
        })?;
    let span = find_raw_contents_span(&reader.file_bytes()[range.clone()])?;
    Ok((range.start + span.0, range.start + span.1))
}

/// Locate the single raw PDF string token assigned to `/Contents`. The parser
/// value alone is insufficient: ByteRange must exclude exactly this source
/// token, not an arbitrary gap that happens to leave a CMS blob parseable.
fn find_raw_contents_span(raw: &[u8]) -> std::result::Result<(usize, usize), String> {
    let mut offset = 0usize;
    let mut spans = Vec::new();
    while offset < raw.len() {
        match raw[offset] {
            b'%' => offset = skip_pdf_comment(raw, offset),
            b'(' => offset = skip_pdf_literal_string(raw, offset)?,
            b'<' if raw.get(offset + 1) == Some(&b'<') => offset += 2,
            b'>' if raw.get(offset + 1) == Some(&b'>') => offset += 2,
            b'<' if raw.get(offset + 1) != Some(&b'<') => {
                offset = skip_pdf_hex_string(raw, offset)?
            }
            b'/' => {
                let name_start = offset + 1;
                let mut name_end = name_start;
                while name_end < raw.len() && !is_pdf_token_delimiter(raw[name_end]) {
                    name_end += 1;
                }
                let is_contents = raw.get(name_start..name_end) == Some(b"Contents".as_slice());
                offset = name_end;
                if is_contents {
                    offset = skip_pdf_space_and_comments(raw, offset);
                    let span = match raw.get(offset).copied() {
                        Some(b'<') if raw.get(offset + 1) != Some(&b'<') => {
                            (offset, skip_pdf_hex_string(raw, offset)?)
                        }
                        Some(b'(') => (offset, skip_pdf_literal_string(raw, offset)?),
                        _ => {
                            return Err(
                                "/Contents does not contain a direct PDF string token".to_string()
                            )
                        }
                    };
                    spans.push(span);
                    offset = span.1;
                }
            }
            _ => offset += 1,
        }
    }
    match spans.len() {
        1 => Ok(spans[0]),
        0 => Err("raw signature source did not contain /Contents".to_string()),
        _ => Err("raw signature source contains duplicate /Contents keys".to_string()),
    }
}

fn skip_pdf_space_and_comments(raw: &[u8], mut offset: usize) -> usize {
    loop {
        while raw.get(offset).is_some_and(|byte| is_pdf_whitespace(*byte)) {
            offset += 1;
        }
        if raw.get(offset) == Some(&b'%') {
            offset = skip_pdf_comment(raw, offset);
            continue;
        }
        return offset;
    }
}

fn skip_pdf_comment(raw: &[u8], mut offset: usize) -> usize {
    while offset < raw.len() && raw[offset] != b'\n' && raw[offset] != b'\r' {
        offset += 1;
    }
    offset
}

fn skip_pdf_hex_string(raw: &[u8], start: usize) -> std::result::Result<usize, String> {
    let mut offset = start + 1;
    while offset < raw.len() {
        if raw[offset] == b'>' {
            return Ok(offset + 1);
        }
        offset += 1;
    }
    Err("unterminated PDF hexadecimal string in signature source".to_string())
}

fn skip_pdf_literal_string(raw: &[u8], start: usize) -> std::result::Result<usize, String> {
    let mut offset = start + 1;
    let mut depth = 1usize;
    while offset < raw.len() {
        match raw[offset] {
            b'\\' => {
                offset = offset.saturating_add(2);
            }
            b'(' => {
                depth = depth.saturating_add(1);
                offset += 1;
            }
            b')' => {
                depth -= 1;
                offset += 1;
                if depth == 0 {
                    return Ok(offset);
                }
            }
            _ => offset += 1,
        }
    }
    Err("unterminated PDF literal string in signature source".to_string())
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_pdf_token_delimiter(byte: u8) -> bool {
    is_pdf_whitespace(byte)
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

#[derive(Default)]
struct DssIndex {
    present: bool,
    certs: Vec<Vec<u8>>,
    ocsp: Vec<Vec<u8>>,
    crls: Vec<Vec<u8>>,
    vri: std::collections::BTreeMap<String, DssVriEntry>,
}

#[derive(Default)]
struct DssVriEntry {
    certs: Vec<Vec<u8>>,
    ocsp: Vec<Vec<u8>>,
    crls: Vec<Vec<u8>>,
}

fn read_dss_index(reader: &PdfReader) -> DssIndex {
    let mut index = DssIndex::default();
    let Some((root, root_generation)) = reader.root_reference() else {
        return index;
    };
    let Ok(PdfObject::Dictionary(catalog)) = reader.get_object(root, root_generation) else {
        return index;
    };
    let Some(dss) = resolve_dict(catalog.get("DSS"), reader) else {
        return index;
    };
    index.present = true;
    index.certs = resolve_stream_array(dss.get("Certs"), reader);
    index.ocsp = resolve_stream_array(dss.get("OCSPs"), reader);
    index.crls = resolve_stream_array(dss.get("CRLs"), reader);

    if let Some(vri_dict) = resolve_dict(dss.get("VRI"), reader) {
        for (key, value) in vri_dict.entries() {
            let Some(entry_dict) = resolve_dict(Some(value), reader) else {
                continue;
            };
            index.vri.insert(
                key.clone(),
                DssVriEntry {
                    certs: resolve_stream_array(entry_dict.get("Cert"), reader),
                    ocsp: resolve_stream_array(entry_dict.get("OCSP"), reader),
                    crls: resolve_stream_array(entry_dict.get("CRL"), reader),
                },
            );
        }
    }

    index
}

fn resolve_stream_array(obj: Option<&PdfObject>, reader: &PdfReader) -> Vec<Vec<u8>> {
    resolve_array(obj, reader)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|obj| resolve_stream_bytes(&obj, reader))
        .collect()
}

fn resolve_stream_bytes(obj: &PdfObject, reader: &PdfReader) -> Option<Vec<u8>> {
    match reader.resolve(obj.clone()).ok()? {
        PdfObject::Stream { raw, .. } => Some(raw),
        PdfObject::String(bytes) => Some(bytes),
        _ => None,
    }
}

fn build_ltv_report(
    contents: &[u8],
    dss: &DssIndex,
    cms: &CmsResult,
    cert: Option<&CertInfo>,
) -> LtvReport {
    let (vri_key, vri_entry) = dss_vri_entry_for_signature(contents, dss);
    let (embedded_certs, embedded_ocsp, embedded_crls, crl_bytes) = if let Some(entry) = vri_entry {
        (
            entry.certs.len(),
            entry.ocsp.len(),
            entry.crls.len(),
            entry.crls.as_slice(),
        )
    } else {
        (
            dss.certs.len(),
            dss.ocsp.len(),
            dss.crls.len(),
            dss.crls.as_slice(),
        )
    };

    let revocation_status = revocation_status_from_crls(crl_bytes, cert);
    let has_valid_timestamp = cms
        .timestamp_reports
        .iter()
        .any(TimestampValidationReport::is_valid);
    let has_validation_material =
        embedded_certs > 0 && (embedded_ocsp > 0 || embedded_crls > 0) && vri_entry.is_some();
    let pades_level = if has_valid_timestamp {
        PadesLevel::BaselineT
    } else {
        PadesLevel::BaselineB
    };

    let note = if has_valid_timestamp && has_validation_material {
        "PAdES B-T timestamp validated and matching DSS/VRI evidence is present; Prompt 25 report determines whether embedded evidence is sufficient for B-LT".to_string()
    } else {
        match pades_level {
            PadesLevel::BaselineLT => {
                "PAdES B-LT material validated by Prompt 25".to_string()
            }
            PadesLevel::BaselineT => {
                "PAdES B-T material validated: RFC 3161 signature timestamp imprint, token CMS signature, TSA EKU, and TSA path checks passed under the configured policy".to_string()
            }
            PadesLevel::BaselineB if dss.present => {
                if vri_entry.is_some() {
                    "DSS VRI material present, but no validated signature timestamp token; not promoted beyond PAdES B-B".to_string()
                } else {
                    "DSS present but no VRI entry matched this signature's /Contents hash".to_string()
                }
            }
            PadesLevel::BaselineB => {
                "no validated RFC 3161 signature timestamp token or matching DSS validation material found".to_string()
            }
            PadesLevel::BaselineLTA => {
                "PAdES B-LTA document timestamp material detected".to_string()
            }
        }
    };

    LtvReport {
        pades_level,
        timestamp_token_count: cms.timestamp_token_count,
        invalid_timestamp_token_count: cms.invalid_timestamp_token_count,
        dss_present: dss.present,
        vri_key: Some(vri_key),
        vri_matched: vri_entry.is_some(),
        embedded_certs,
        embedded_ocsp_responses: embedded_ocsp,
        embedded_crls,
        revocation_status,
        note,
    }
}

struct DssVriMaterial<'a> {
    vri_key: String,
    vri_entry: Option<&'a DssVriEntry>,
    certs: &'a [Vec<u8>],
    ocsp: &'a [Vec<u8>],
    crls: &'a [Vec<u8>],
}

fn dss_vri_material_for_signature<'a>(contents: &[u8], dss: &'a DssIndex) -> DssVriMaterial<'a> {
    let (vri_key, vri_entry) = dss_vri_entry_for_signature(contents, dss);
    if let Some(entry) = vri_entry {
        DssVriMaterial {
            vri_key,
            vri_entry: Some(entry),
            certs: &entry.certs,
            ocsp: &entry.ocsp,
            crls: &entry.crls,
        }
    } else {
        DssVriMaterial {
            vri_key,
            vri_entry: None,
            certs: &dss.certs,
            ocsp: &dss.ocsp,
            crls: &dss.crls,
        }
    }
}

fn options_with_dss_evidence(
    options: &VerifyOptions,
    contents: &[u8],
    dss: &DssIndex,
) -> VerifyOptions {
    let mut effective = options.clone();
    if !dss.present {
        return effective;
    }
    let material = dss_vri_material_for_signature(contents, dss);
    for cert in material.certs {
        push_unique_bytes(&mut effective.intermediates_der, cert.clone());
    }
    for response in material.ocsp {
        push_unique_bytes(&mut effective.ocsp_responses_der, response.clone());
    }
    for crl in material.crls {
        push_unique_bytes(&mut effective.crls_der, crl.clone());
    }
    effective
}

fn build_dss_validation_report(
    contents: &[u8],
    dss: &DssIndex,
    prompt24: &Prompt24SignatureValidationReport,
) -> DssValidationReport {
    let material = dss_vri_material_for_signature(contents, dss);
    let evidence_present =
        !material.certs.is_empty() || !material.ocsp.is_empty() || !material.crls.is_empty();
    let replayable = dss.present
        && material.vri_entry.is_some()
        && !material.certs.is_empty()
        && (!material.ocsp.is_empty() || !material.crls.is_empty());
    let validation_material_status = if replayable {
        match prompt24.revocation.status {
            SignatureValidationState::Valid => {
                if prompt24.path.status == SignatureValidationState::Valid {
                    SignatureValidationState::Valid
                } else {
                    SignatureValidationState::Indeterminate
                }
            }
            SignatureValidationState::Revoked => SignatureValidationState::Revoked,
            SignatureValidationState::EvidenceStale => SignatureValidationState::EvidenceStale,
            SignatureValidationState::EvidenceMissing => SignatureValidationState::EvidenceMissing,
            SignatureValidationState::NetworkDisabled => SignatureValidationState::NetworkDisabled,
            SignatureValidationState::NetworkFailure => SignatureValidationState::NetworkFailure,
            SignatureValidationState::UnsupportedAlgorithm => {
                SignatureValidationState::UnsupportedAlgorithm
            }
            SignatureValidationState::NotChecked if evidence_present => {
                SignatureValidationState::Indeterminate
            }
            SignatureValidationState::NotChecked => SignatureValidationState::NotChecked,
            _ => SignatureValidationState::Indeterminate,
        }
    } else if evidence_present {
        SignatureValidationState::Indeterminate
    } else {
        SignatureValidationState::NotChecked
    };
    let status = if !dss.present {
        SignatureValidationState::NotChecked
    } else if material.vri_entry.is_none() {
        SignatureValidationState::EvidenceMissing
    } else if replayable {
        validation_material_status.clone()
    } else {
        SignatureValidationState::EvidenceMissing
    };
    let mut warnings = Vec::new();
    if dss.present && material.vri_entry.is_none() {
        warnings.push(
            "DSS exists but no VRI key matched this signature's supported /Contents digest forms"
                .to_string(),
        );
    }
    if dss.present && material.vri_entry.is_some() && !replayable {
        warnings.push(
            "matched VRI entry lacks either certificate evidence or revocation evidence"
                .to_string(),
        );
    }
    DssValidationReport {
        status,
        dss_present: dss.present,
        vri_key: Some(material.vri_key),
        vri_matched: material.vri_entry.is_some(),
        global_cert_count: dss.certs.len(),
        global_ocsp_count: dss.ocsp.len(),
        global_crl_count: dss.crls.len(),
        matched_cert_count: material.certs.len(),
        matched_ocsp_count: material.ocsp.len(),
        matched_crl_count: material.crls.len(),
        evidence_replayable_offline: replayable,
        validation_material_status,
        warnings,
    }
}

fn build_prompt25_report(
    contents: &[u8],
    dss: &DssIndex,
    cms: &CmsResult,
    _ltv: &LtvReport,
    coverage: &Coverage,
    prompt24: &Prompt24SignatureValidationReport,
) -> Prompt25SignatureLtvEditReport {
    let timestamp_tokens = cms.timestamp_reports.clone();
    let signature_timestamp_status = if timestamp_tokens.is_empty() {
        SignatureValidationState::NotChecked
    } else if timestamp_tokens
        .iter()
        .any(TimestampValidationReport::is_valid)
    {
        SignatureValidationState::Valid
    } else if timestamp_tokens
        .iter()
        .any(|report| report.status == SignatureValidationState::UnsupportedAlgorithm)
    {
        SignatureValidationState::UnsupportedAlgorithm
    } else {
        SignatureValidationState::Invalid
    };
    let dss_report = build_dss_validation_report(contents, dss, prompt24);
    let lt_ready = signature_timestamp_status == SignatureValidationState::Valid
        && dss_report.dss_present
        && dss_report.vri_matched
        && dss_report.evidence_replayable_offline
        && prompt24.path.status == SignatureValidationState::Valid
        && prompt24.revocation.status == SignatureValidationState::Valid;
    let ltv_status = if lt_ready {
        SignatureValidationState::Valid
    } else if dss_report.dss_present
        || signature_timestamp_status == SignatureValidationState::Valid
    {
        SignatureValidationState::Indeterminate
    } else {
        SignatureValidationState::NotChecked
    };
    let achieved_pades_level = if lt_ready {
        PadesLevel::BaselineLT
    } else if signature_timestamp_status == SignatureValidationState::Valid {
        PadesLevel::BaselineT
    } else {
        PadesLevel::BaselineB
    };
    let (validation_indication, validation_subindication) = match ltv_status {
        SignatureValidationState::Valid => (
            SignatureValidationIndication::Passed,
            SignatureValidationSubindication::None,
        ),
        SignatureValidationState::NotChecked => (
            SignatureValidationIndication::NotEvaluated,
            SignatureValidationSubindication::NotEvaluated,
        ),
        SignatureValidationState::Revoked => (
            SignatureValidationIndication::Failed,
            SignatureValidationSubindication::CertificateRevoked,
        ),
        SignatureValidationState::UnsupportedAlgorithm => (
            SignatureValidationIndication::Indeterminate,
            SignatureValidationSubindication::UnsupportedAlgorithm,
        ),
        _ => (
            SignatureValidationIndication::Indeterminate,
            SignatureValidationSubindication::ValidationIndeterminate,
        ),
    };
    let mut report = Prompt25SignatureLtvEditReport {
        timestamp_tokens,
        signature_timestamp_status,
        dss: dss_report,
        ltv_status,
        achieved_pades_level,
        validation_indication,
        validation_subindication,
        post_signature_modification_status: match coverage {
            Coverage::WholeFile => SignatureValidationState::Valid,
            Coverage::ModifiedAfterSigning => SignatureValidationState::ModifiedAfterSigning,
        },
        ..Prompt25SignatureLtvEditReport::default()
    };
    report.warnings.extend(report.dss.warnings.clone());
    report
}

fn revocation_status_from_crls(crls: &[Vec<u8>], cert: Option<&CertInfo>) -> RevocationStatus {
    if crls.is_empty() {
        return RevocationStatus::NotChecked;
    }
    let Some(cert) = cert else {
        return RevocationStatus::EmbeddedMaterial;
    };
    let mut parsed_any = false;
    for crl in crls {
        let Ok(list) = CertificateList::from_der(crl) else {
            continue;
        };
        parsed_any = true;
        if let Some(revoked) = &list.tbs_cert_list.revoked_certificates {
            if revoked
                .iter()
                .any(|entry| hex_upper(entry.serial_number.as_bytes()) == cert.serial_hex)
            {
                return RevocationStatus::RevokedByEmbeddedCrl;
            }
        }
    }
    if parsed_any {
        RevocationStatus::GoodFromEmbeddedCrl
    } else {
        RevocationStatus::Unknown
    }
}

fn append_dss_streams(
    raw_objects: &mut Vec<RawIncrementalObject>,
    next_number: &mut u32,
    streams: &[Vec<u8>],
) -> Vec<PdfObject> {
    let mut refs = Vec::with_capacity(streams.len());
    for bytes in streams {
        let number = *next_number;
        *next_number += 1;
        refs.push(reference(number, 0));
        raw_objects.push(raw_object(number, 0, &dss_stream(bytes.clone())));
    }
    refs
}

fn dss_stream(raw: Vec<u8>) -> PdfObject {
    PdfObject::Stream {
        dict: PdfDictionary::empty(),
        raw,
    }
}

fn cms_certificate_der(contents: &[u8]) -> Vec<Vec<u8>> {
    let Ok(der) = exact_cms_der_object(contents) else {
        return Vec::new();
    };
    let Ok(ci) = ContentInfo::from_der(der) else {
        return Vec::new();
    };
    let Ok(signed) = ci.content.decode_as::<SignedData>() else {
        return Vec::new();
    };
    let Some(certs) = signed.certificates else {
        return Vec::new();
    };
    certs
        .0
        .iter()
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(cert) => cert.to_der().ok(),
            _ => None,
        })
        .collect()
}

fn push_unique_bytes(out: &mut Vec<Vec<u8>>, bytes: Vec<u8>) {
    if !out.iter().any(|existing| existing == &bytes) {
        out.push(bytes);
    }
}

#[derive(Clone)]
struct FetchedAiaCertificate {
    certificate_der: Vec<u8>,
    record: EvidenceRecord,
}

#[derive(Clone)]
struct FetchedNetworkEvidence {
    bytes: Vec<u8>,
    record: EvidenceRecord,
    expected_ocsp_nonce: Option<Vec<u8>>,
}

#[derive(Default)]
struct FetchedRevocationEvidence {
    ocsp: Vec<FetchedNetworkEvidence>,
    crls: Vec<FetchedNetworkEvidence>,
    rejected_ocsp: Vec<String>,
}

fn evidence_record_from_response(
    kind: EvidenceKind,
    response: &crate::signature_evidence::RetrievalResponse,
) -> EvidenceRecord {
    let retrieved_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs());
    let mut record = EvidenceRecord::from_bytes(
        kind,
        &response.bytes,
        Some(response.final_uri.clone()),
        retrieved_at_unix,
        response.content_type.clone(),
        response.trace.request_body_sha256.clone(),
        false,
    );
    record.source_uri_sha256 = response.source_uri_sha256.clone();
    record
}

fn push_unique_fetched_evidence(
    out: &mut Vec<FetchedNetworkEvidence>,
    item: FetchedNetworkEvidence,
) {
    if !out.iter().any(|existing| existing.bytes == item.bytes) {
        out.push(item);
    }
}

fn signature_vri_key(contents: &[u8]) -> String {
    // A VRI key is a digest of the exact CMS object, not the entire reserved
    // PDF `/Contents` slot. For malformed content retain a deterministic raw
    // digest for inventory only; it cannot produce a validated CMS result.
    let cms = exact_cms_der_object(contents).unwrap_or(contents);
    let digest = Sha1::digest(cms);
    hex_upper(&digest)
}

fn signature_vri_key_candidates(contents: &[u8]) -> Vec<String> {
    let cms = exact_cms_der_object(contents).unwrap_or(contents);
    let mut keys = vec![signature_vri_key(contents)];

    // pyHanko's DSS writer follows the common PAdES convention of hashing the
    // ASCII hex representation of the PDF signature contents. Accept both the
    // trimmed DER object and the full padded `/Contents` string as alternate
    // bindings while retaining the raw-CMS key for existing Oxide DSS output
    // and malformed inventory paths.
    let hex_contents = hex_lower(cms);
    let hex_key = hex_upper(&Sha1::digest(hex_contents.as_bytes()));
    if !keys.iter().any(|key| key == &hex_key) {
        keys.push(hex_key);
    }
    let padded_hex_contents = hex_lower(contents);
    let padded_hex_key = hex_upper(&Sha1::digest(padded_hex_contents.as_bytes()));
    if !keys.iter().any(|key| key == &padded_hex_key) {
        keys.push(padded_hex_key);
    }
    keys
}

fn dss_vri_entry_for_signature<'a>(
    contents: &[u8],
    dss: &'a DssIndex,
) -> (String, Option<&'a DssVriEntry>) {
    let mut candidates = signature_vri_key_candidates(contents).into_iter();
    let fallback = candidates
        .next()
        .unwrap_or_else(|| signature_vri_key(contents));
    if let Some(entry) = dss.vri.get(&fallback) {
        return (fallback, Some(entry));
    }
    for key in candidates {
        if let Some(entry) = dss.vri.get(&key) {
            return (key, Some(entry));
        }
    }
    (fallback, None)
}

struct VerifyOneOutcome {
    reports: Vec<SignatureReport>,
    evidence_records: Vec<EvidenceRecord>,
}

fn verify_one(
    field: &SigField,
    file: &[u8],
    index: usize,
    dss: &DssIndex,
    options: &VerifyOptions,
) -> Vec<SignatureReport> {
    verify_one_with_evidence(field, file, index, dss, options).reports
}

fn verify_one_with_evidence(
    field: &SigField,
    file: &[u8],
    index: usize,
    dss: &DssIndex,
    options: &VerifyOptions,
) -> VerifyOneOutcome {
    let sig = &field.sig_dict;
    let mut report = SignatureReport {
        index,
        cms_signer_index: None,
        cms_signer_count: 0,
        discovery_kind: field.discovery_kind.clone(),
        field_object: field
            .field_object
            .map(|(number, generation)| PdfSignatureObjectIdentity { number, generation }),
        signature_object: field
            .signature_object
            .map(|(number, generation)| PdfSignatureObjectIdentity { number, generation }),
        contents_span: field
            .contents_span
            .as_ref()
            .ok()
            .map(|(start, end)| [*start, *end]),
        signed_revision_end: None,
        uncovered_byte_ranges: Vec::new(),
        field_name: field.field_name.clone(),
        signer_name: sig.get("Name").and_then(decode_text_string),
        signing_time: sig.get("M").and_then(decode_text_string),
        reason: sig.get("Reason").and_then(decode_text_string),
        location: sig.get("Location").and_then(decode_text_string),
        contact_info: sig.get("ContactInfo").and_then(decode_text_string),
        sub_filter: sig.get_name("SubFilter").map(str::to_string),
        digest_algorithm: None,
        validity: SignatureValidity::Error,
        trust: SignatureTrust::NotVerified,
        coverage: Coverage::ModifiedAfterSigning,
        status: SignatureStatus::Error,
        certificate: None,
        ltv: LtvReport::default(),
        prompt24: Prompt24SignatureValidationReport::default(),
        prompt25: Prompt25SignatureLtvEditReport::default(),
        checks: SignatureCheckDetails::default(),
        note: String::new(),
    };

    if let Some(issue) = &field.discovery_issue {
        report.note = issue.clone();
        report.prompt24 =
            prompt24_error_report(options, SignatureValidationState::Malformed, issue);
        return VerifyOneOutcome {
            reports: vec![report],
            evidence_records: Vec::new(),
        };
    }

    // /ByteRange = [a b c d]; signed data = file[a..a+b] ++ file[c..c+d].
    let byte_range = match parse_byte_range(sig) {
        Some(br) => {
            report.checks.byte_range_present = true;
            report.checks.byte_range_well_formed = true;
            report.checks.byte_range = Some([br.a, br.b, br.c, br.d]);
            br
        }
        None => {
            report.note = "missing or malformed /ByteRange".to_string();
            report.prompt24 = prompt24_error_report(
                options,
                SignatureValidationState::ByteRangeInvalid,
                "missing or malformed /ByteRange",
            );
            return VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            };
        }
    };

    let signed_data_bytes = match extract_signed_bytes(file, &byte_range) {
        Some(b) => {
            report.checks.byte_range_in_bounds = true;
            report.checks.byte_range_non_overlapping = true;
            report.checks.signed_bytes = b.len();
            b
        }
        None => {
            report.note = "/ByteRange out of bounds for file".to_string();
            report.prompt24 = prompt24_error_report(
                options,
                SignatureValidationState::ByteRangeInvalid,
                "/ByteRange out of bounds for file",
            );
            return VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            };
        }
    };

    let signed_revision_end = match byte_range.c.checked_add(byte_range.d) {
        Some(end) if end <= file.len() => end,
        _ => {
            report.note = "/ByteRange signed revision end overflows or exceeds file".to_string();
            report.prompt24 = prompt24_error_report(
                options,
                SignatureValidationState::ByteRangeInvalid,
                "/ByteRange signed revision end overflows or exceeds file",
            );
            return VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            };
        }
    };
    report.signed_revision_end = Some(signed_revision_end);
    if signed_revision_end < file.len() {
        report
            .uncovered_byte_ranges
            .push([signed_revision_end, file.len()]);
    }

    // Coverage: do the ranges + the /Contents gap reach the end of the file?
    report.coverage = compute_coverage(&byte_range, file.len());
    report.checks.byte_range_covers_whole_file = report.coverage == Coverage::WholeFile;

    // /Contents = DER CMS blob (a hex/binary string).
    let contents = match sig.get("Contents").and_then(PdfObject::as_string) {
        Some(c) => {
            report.checks.contents_present = true;
            c.to_vec()
        }
        None => {
            report.note = "missing /Contents".to_string();
            report.prompt24 = prompt24_error_report(
                options,
                SignatureValidationState::Malformed,
                "missing /Contents",
            );
            return VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            };
        }
    };

    let contents_span = match &field.contents_span {
        Ok(span) => *span,
        Err(error) => {
            report.note = format!("cannot bind /ByteRange gap to raw /Contents: {error}");
            report.prompt24 = prompt24_error_report(
                options,
                SignatureValidationState::ByteRangeInvalid,
                &report.note,
            );
            return VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            };
        }
    };
    let gap_start = byte_range.a.checked_add(byte_range.b);
    if gap_start != Some(contents_span.0) || byte_range.c != contents_span.1 {
        report.note = format!(
            "/ByteRange excluded gap {}..{} does not exactly match raw /Contents {}..{}",
            gap_start.unwrap_or(usize::MAX),
            byte_range.c,
            contents_span.0,
            contents_span.1
        );
        report.prompt24 = prompt24_error_report(
            options,
            SignatureValidationState::ByteRangeInvalid,
            &report.note,
        );
        return VerifyOneOutcome {
            reports: vec![report],
            evidence_records: Vec::new(),
        };
    }
    report.checks.byte_range_contents_gap_matches = true;

    match verify_cms(&contents, &signed_data_bytes, options) {
        Ok(results) => {
            let signer_count = results.len();
            let mut reports = Vec::with_capacity(signer_count);
            let mut evidence_records = Vec::new();
            for (signer_index, result) in results.into_iter().enumerate() {
                let mut signer_report = report.clone();
                signer_report.cms_signer_index = Some(signer_index + 1);
                signer_report.cms_signer_count = signer_count;
                let effective_options = options_with_dss_evidence(options, &contents, dss);
                let ltv = build_ltv_report(&contents, dss, &result, result.certificate.as_ref());
                let validation = evaluate_prompt24_validation(Prompt24ValidationInput {
                    signer: result.signer_cert.as_ref(),
                    chain: &result.chain,
                    options: &effective_options,
                    ltv: &ltv,
                    cms: &result.cms,
                    sub_filter: signer_report.sub_filter.as_deref(),
                    signer_resolution: result.signer_resolution,
                    coverage: &signer_report.coverage,
                    validity: &result.validity,
                });
                signer_report.validity = result.validity.clone();
                signer_report.digest_algorithm = result.digest_algorithm.clone();
                signer_report.certificate = result.certificate.clone();
                evidence_records.extend(validation.evidence_records);
                signer_report.trust = validation.trust;
                signer_report.ltv = ltv;
                let mut prompt24 = validation.report;
                set_validation_indication(&mut prompt24);
                signer_report.prompt25 = build_prompt25_report(
                    &contents,
                    dss,
                    &result,
                    &signer_report.ltv,
                    &signer_report.coverage,
                    &prompt24,
                );
                signer_report.ltv.pades_level = signer_report.prompt25.achieved_pades_level.clone();
                signer_report.prompt24 = prompt24;
                signer_report.status = overall_status(
                    &signer_report.validity,
                    &signer_report.trust,
                    &signer_report.coverage,
                );
                signer_report.checks.digest_matches =
                    signer_report.validity == SignatureValidity::Valid;
                signer_report.checks.cms_verified =
                    signer_report.validity == SignatureValidity::Valid;
                signer_report.checks.chain_verified =
                    signer_report.trust == SignatureTrust::Trusted;
                signer_report.checks.revocation_checked = signer_report.prompt24.revocation.status
                    != SignatureValidationState::NotChecked;
                signer_report.checks.timestamp_present =
                    signer_report.ltv.timestamp_token_count > 0;
                signer_report.checks.timestamp_verified =
                    signer_report.prompt25.signature_timestamp_status
                        == SignatureValidationState::Valid;
                signer_report.checks.ltv_material_present = signer_report.ltv.dss_present
                    || signer_report.ltv.embedded_certs > 0
                    || signer_report.ltv.embedded_ocsp_responses > 0
                    || signer_report.ltv.embedded_crls > 0;
                signer_report.checks.ltv_verified =
                    signer_report.prompt25.ltv_status == SignatureValidationState::Valid;
                signer_report.note = format!(
                    "CMS SignerInfo {}/{}. {}. {}",
                    signer_index + 1,
                    signer_count,
                    status_note(&signer_report),
                    signer_report.ltv.note
                );
                reports.push(signer_report);
            }
            VerifyOneOutcome {
                reports,
                evidence_records,
            }
        }
        Err(msg) => {
            report.validity = SignatureValidity::Error;
            report.status = SignatureStatus::Error;
            let state = if msg.contains("ambiguous") {
                SignatureValidationState::SignerCertificateAmbiguous
            } else if msg.contains("signer certificate") {
                SignatureValidationState::SignerCertificateMissing
            } else {
                SignatureValidationState::Malformed
            };
            report.prompt24 = prompt24_error_report(options, state, &msg);
            report.note = msg;
            VerifyOneOutcome {
                reports: vec![report],
                evidence_records: Vec::new(),
            }
        }
    }
}

/// Combine integrity + trust + coverage into the overall honest verdict.
/// `Trusted` requires all three: integrity `Valid`, signer `Trusted`, and
/// whole-file coverage.
fn overall_status(
    validity: &SignatureValidity,
    trust: &SignatureTrust,
    coverage: &Coverage,
) -> SignatureStatus {
    match validity {
        SignatureValidity::Valid => {
            if *trust == SignatureTrust::Revoked {
                SignatureStatus::Revoked
            } else if *coverage == Coverage::ModifiedAfterSigning {
                SignatureStatus::ValidButModified
            } else if *trust == SignatureTrust::Trusted {
                SignatureStatus::Trusted
            } else {
                SignatureStatus::ValidUntrusted
            }
        }
        SignatureValidity::Invalid => SignatureStatus::Invalid,
        SignatureValidity::UnsupportedAlgorithm => SignatureStatus::UnsupportedAlgorithm,
        SignatureValidity::Error => SignatureStatus::Error,
    }
}

/// Evaluate signer trust against configured anchors. Returns `NotVerified` when
/// no anchors are configured — the safe default that never claims trust without
/// a verified chain. Revocation by embedded material is a hard failure
/// regardless of anchors.
#[allow(dead_code)]
fn evaluate_trust(
    signer: Option<&Certificate>,
    chain: &[Certificate],
    options: &VerifyOptions,
    revocation: &RevocationStatus,
) -> SignatureTrust {
    if *revocation == RevocationStatus::RevokedByEmbeddedCrl {
        return SignatureTrust::Revoked;
    }
    let anchors: Vec<Certificate> = options
        .trust_anchors_der
        .iter()
        .filter_map(|der| Certificate::from_der(der).ok())
        .collect();
    if anchors.is_empty() {
        return SignatureTrust::NotVerified;
    }
    let Some(signer) = signer else {
        return SignatureTrust::Untrusted;
    };
    if !cert_in_validity_period(signer) {
        return SignatureTrust::Expired;
    }
    if chains_to_anchor(signer, chain, &anchors) {
        SignatureTrust::Trusted
    } else {
        SignatureTrust::Untrusted
    }
}

/// True if `signer` chains to one of `anchors`, either by being a pinned anchor
/// itself or via a path through the embedded `chain`, with every link verified
/// by an actual certificate-signature check.
#[allow(dead_code)]
fn chains_to_anchor(signer: &Certificate, chain: &[Certificate], anchors: &[Certificate]) -> bool {
    const MAX_CHAIN_DEPTH: usize = 10;
    if anchors.iter().any(|anchor| same_cert(anchor, signer)) {
        return true;
    }
    let mut current = signer.clone();
    for _ in 0..MAX_CHAIN_DEPTH {
        if anchors.iter().any(|anchor| issued_by(&current, anchor)) {
            return true;
        }
        let Some(issuer) = chain
            .iter()
            .find(|cand| !same_cert(cand, &current) && issued_by(&current, cand))
        else {
            return false;
        };
        if anchors.iter().any(|anchor| same_cert(anchor, issuer)) {
            return true;
        }
        current = issuer.clone();
    }
    false
}

#[allow(dead_code)]
fn same_cert(a: &Certificate, b: &Certificate) -> bool {
    match (a.to_der(), b.to_der()) {
        (Ok(da), Ok(db)) => da == db,
        _ => false,
    }
}

/// True if `child`'s issuer name matches `issuer`'s subject AND `issuer`'s
/// public key actually verifies `child`'s certificate signature.
#[allow(dead_code)]
fn issued_by(child: &Certificate, issuer: &Certificate) -> bool {
    child.tbs_certificate.issuer == issuer.tbs_certificate.subject && cert_signed_by(child, issuer)
}

#[allow(dead_code)]
fn cert_signed_by(child: &Certificate, issuer: &Certificate) -> bool {
    let digest_oid = match child.signature_algorithm.oid {
        OID_SHA256_RSA => OID_SHA256,
        OID_SHA384_RSA => OID_SHA384,
        OID_SHA512_RSA => OID_SHA512,
        OID_SHA1_RSA => OID_SHA1,
        _ => return false,
    };
    let Ok(tbs) = child.tbs_certificate.to_der() else {
        return false;
    };
    let Some(signature) = child.signature.as_bytes() else {
        return false;
    };
    verify_rsa(
        issuer,
        &digest_oid,
        &tbs,
        signature,
        &SignatureAlgorithmPolicy::default(),
    )
    .unwrap_or(false)
}

#[allow(dead_code)]
fn cert_in_validity_period(cert: &Certificate) -> bool {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        // Clock error: don't fail closed to "expired" (that would be misleading);
        // trust is still gated by the chain check.
        return true;
    };
    let validity = &cert.tbs_certificate.validity;
    let not_before = validity.not_before.to_unix_duration();
    let not_after = validity.not_after.to_unix_duration();
    not_before <= now && now <= not_after
}

struct Prompt24TrustEvaluation {
    trust: SignatureTrust,
    report: Prompt24SignatureValidationReport,
    evidence_records: Vec<EvidenceRecord>,
}

struct Prompt24ValidationInput<'a> {
    signer: Option<&'a Certificate>,
    chain: &'a [Certificate],
    options: &'a VerifyOptions,
    ltv: &'a LtvReport,
    cms: &'a CmsValidationReport,
    sub_filter: Option<&'a str>,
    signer_resolution: SignerCertResolution,
    coverage: &'a Coverage,
    validity: &'a SignatureValidity,
}

fn evaluate_prompt24_validation(input: Prompt24ValidationInput<'_>) -> Prompt24TrustEvaluation {
    let signer = input.signer;
    let chain = input.chain;
    let options = input.options;
    let ltv = input.ltv;
    let cms = input.cms;
    let sub_filter = input.sub_filter;
    let signer_resolution = input.signer_resolution;
    let coverage = input.coverage;
    let validity = input.validity;
    let (validation_time_unix, validation_time_source) = validation_time(options);
    let mut report = Prompt24SignatureValidationReport {
        policy: policy_report(options, validation_time_unix, validation_time_source),
        cms: cms.clone(),
        signer_resolution: signer_resolution_state(signer_resolution),
        certificate_inventory_count: chain.len() + options.intermediates_der.len(),
        pades: pades_prompt24_report(ltv, coverage, validity, cms, sub_filter, None, None),
        network: network_prompt24_report(options),
        ..Prompt24SignatureValidationReport::default()
    };
    report.deferred_evidence = deferred_prompt24_evidence(ltv);

    if *validity != SignatureValidity::Valid {
        report.overall = match validity {
            SignatureValidity::Invalid => SignatureValidationState::SignatureMathInvalid,
            SignatureValidity::UnsupportedAlgorithm => {
                SignatureValidationState::UnsupportedAlgorithm
            }
            SignatureValidity::Error => SignatureValidationState::Malformed,
            SignatureValidity::Valid => SignatureValidationState::Valid,
        };
        report.overall_reason =
            "CMS signature math or digest did not establish a valid signature".to_string();
        return Prompt24TrustEvaluation {
            trust: SignatureTrust::NotVerified,
            report,
            evidence_records: Vec::new(),
        };
    }

    if signer_resolution != SignerCertResolution::Found || signer.is_none() {
        report.path.signer_certificate_status = report.signer_resolution.clone();
        report.path.status = report.signer_resolution.clone();
        report.pades.certificate_path_status = report.path.status.clone();
        report.overall = report.signer_resolution.clone();
        report.overall_reason = "SignerInfo did not resolve to exactly one certificate".to_string();
        return Prompt24TrustEvaluation {
            trust: SignatureTrust::Untrusted,
            report,
            evidence_records: Vec::new(),
        };
    }

    if options.trust_anchors_der.is_empty() {
        report.path.status = SignatureValidationState::EvidenceMissing;
        report.path.signer_certificate_status = SignatureValidationState::Valid;
        report.path.validation_error =
            Some("no explicit trust anchors were configured".to_string());
        report.pades.certificate_path_status = report.path.status.clone();
        report.overall = SignatureValidationState::Untrusted;
        report.overall_reason =
            "cryptographic signature is valid, but signer trust was not evaluated".to_string();
        return Prompt24TrustEvaluation {
            trust: SignatureTrust::NotVerified,
            report,
            evidence_records: Vec::new(),
        };
    }

    let signer = signer.expect("signer checked above");
    let mut path_report = CertificatePathValidationReport {
        signer_certificate_status: SignatureValidationState::Valid,
        implemented_checks: path_validation_checks(),
        ..CertificatePathValidationReport::default()
    };
    let (anchor_certs, anchors, anchor_errors) = parse_trust_anchors(&options.trust_anchors_der);
    path_report.anchor_parse_errors = anchor_errors;
    if anchors.is_empty() {
        path_report.status = SignatureValidationState::EvidenceMissing;
        path_report.validation_error = Some("no usable trust anchors were parsed".to_string());
        report.path = path_report;
        report.pades.certificate_path_status = report.path.status.clone();
        report.overall = SignatureValidationState::Untrusted;
        report.overall_reason = "no usable trust anchor was available".to_string();
        return Prompt24TrustEvaluation {
            trust: SignatureTrust::Untrusted,
            report,
            evidence_records: Vec::new(),
        };
    }

    let distrusted_fingerprints = match configured_distrust_set(options) {
        Ok(fingerprints) => fingerprints,
        Err(error) => {
            path_report.status = SignatureValidationState::PolicyRejected;
            path_report.validation_error = Some(error.to_string());
            report.path = path_report;
            report.pades.certificate_path_status = SignatureValidationState::PolicyRejected;
            report.overall = SignatureValidationState::PolicyRejected;
            report.overall_reason = error.to_string();
            return Prompt24TrustEvaluation {
                trust: SignatureTrust::Untrusted,
                report,
                evidence_records: Vec::new(),
            };
        }
    };

    let mut effective_options = options.clone();
    let (mut pool, intermediate_errors) = build_certificate_pool(signer, chain, &effective_options);
    path_report.intermediate_parse_errors = intermediate_errors;
    let mut builder_config = PathBuilderConfig::new();
    builder_config.max_depth = options.max_chain_depth;
    builder_config.dfs_budget = options.max_path_candidates;

    let mut policy = ValidationPolicy::new(validation_time_unix);
    policy.max_path_len = options.max_chain_depth.min(u8::MAX as usize) as u8;
    policy.min_rsa_key_bits = Some(options.algorithm_policy.min_rsa_key_bits.into());

    let (mut selected_path, initial_attempted, mut last_error) = validate_candidate_paths(
        signer,
        &pool,
        &anchors,
        &anchor_certs,
        &policy,
        &builder_config,
        &distrusted_fingerprints,
    );
    path_report.candidate_paths_tried += initial_attempted;
    let mut fetched_aia = Vec::new();

    let mut retrieval_session = if effective_retrieval_policy(options).enabled {
        match retrieval_session_for(options) {
            Ok(session) => Some(session),
            Err(error) => {
                report.network.status = SignatureValidationState::PolicyRejected;
                report.network.note = format!("controlled retrieval policy rejected: {error}");
                report
                    .warnings
                    .push(format!("controlled retrieval was not started: {error}"));
                None
            }
        }
    } else {
        None
    };

    if selected_path.is_none() {
        if let Some(session) = retrieval_session.as_mut() {
            let fetched = fetch_aia_intermediates(signer, chain, session, &mut report.warnings);
            if !fetched.is_empty() {
                effective_options
                    .intermediates_der
                    .extend(fetched.iter().map(|item| item.certificate_der.clone()));
                let (next_pool, extra_errors) =
                    build_certificate_pool(signer, chain, &effective_options);
                pool = next_pool;
                path_report.intermediate_parse_errors.extend(extra_errors);
                let (retrieved_path, attempted, error) = validate_candidate_paths(
                    signer,
                    &pool,
                    &anchors,
                    &anchor_certs,
                    &policy,
                    &builder_config,
                    &distrusted_fingerprints,
                );
                path_report.candidate_paths_tried += attempted;
                if retrieved_path.is_some() {
                    selected_path = retrieved_path;
                }
                if error.is_some() {
                    last_error = error;
                }
                report.certificate_inventory_count =
                    chain.len() + effective_options.intermediates_der.len();
                fetched_aia = fetched;
            }
            sync_network_report(&mut report.network, session);
        }
    }

    let Some((selected_path, selected_anchor_index)) = selected_path else {
        path_report.status = if path_report.candidate_paths_tried == 0 {
            SignatureValidationState::PathNotFound
        } else {
            SignatureValidationState::PathInvalid
        };
        path_report.validation_error = last_error;
        let trust = trust_from_path_failure(&path_report.validation_error);
        report.path = path_report;
        report.pades.certificate_path_status = report.path.status.clone();
        report.overall = SignatureValidationState::Untrusted;
        report.overall_reason =
            "no certificate path validated to a configured trust anchor".to_string();
        return Prompt24TrustEvaluation {
            trust,
            report,
            evidence_records: Vec::new(),
        };
    };

    path_report.selected_anchor_index = Some(selected_anchor_index);
    path_report.selected_path_subjects = selected_path
        .iter()
        .map(|cert| cert.tbs_certificate.subject.to_string())
        .collect();
    path_report.selected_path_serials = selected_path
        .iter()
        .map(|cert| hex_upper(cert.tbs_certificate.serial_number.as_bytes()))
        .collect();

    if let Err(error) = validate_signer_certificate_usage(signer) {
        path_report.status = SignatureValidationState::PolicyRejected;
        path_report.signer_certificate_status = SignatureValidationState::PolicyRejected;
        path_report.validation_error = Some(error.clone());
        report.path = path_report;
        report.pades.certificate_path_status = SignatureValidationState::PolicyRejected;
        report.overall = SignatureValidationState::PolicyRejected;
        report.overall_reason = error;
        return Prompt24TrustEvaluation {
            trust: SignatureTrust::Untrusted,
            report,
            evidence_records: Vec::new(),
        };
    }

    path_report.status = SignatureValidationState::Valid;

    if let Some(session) = retrieval_session.as_mut() {
        cache_selected_aia_evidence(session, &fetched_aia, &selected_path, &mut report.warnings);
        sync_network_report(&mut report.network, session);
    }

    let mut revocation_options = effective_options.clone();
    let mut fetched_revocation = FetchedRevocationEvidence::default();
    if revocation_options.revocation_mode.requires_evidence() {
        if let Some(session) = retrieval_session.as_mut() {
            fetched_revocation = fetch_online_revocation_evidence(
                &selected_path,
                anchor_certs.get(selected_anchor_index),
                session,
                &mut report.warnings,
            );
            revocation_options.ocsp_responses_der.extend(
                fetched_revocation
                    .ocsp
                    .iter()
                    .map(|item| item.bytes.clone()),
            );
            revocation_options.crls_der.extend(
                fetched_revocation
                    .crls
                    .iter()
                    .map(|item| item.bytes.clone()),
            );
            sync_network_report(&mut report.network, session);
        }
    }
    let crl_signer_candidates =
        crl_signer_candidates(chain, &selected_path, &anchor_certs, &revocation_options);
    let mut revocation = evaluate_revocation_prompt24(RevocationEvaluationContext {
        path: &selected_path,
        anchor: anchors.get(selected_anchor_index),
        anchor_certificate: anchor_certs.get(selected_anchor_index),
        crl_signer_candidates: &crl_signer_candidates,
        cms_chain: chain,
        anchors: &anchors,
        path_policy: &policy,
        builder_config: &builder_config,
        options: &revocation_options,
        validation_time_unix,
    });
    if !fetched_revocation.rejected_ocsp.is_empty() {
        revocation
            .errors
            .extend(fetched_revocation.rejected_ocsp.iter().cloned());
        if !matches!(
            revocation.status,
            SignatureValidationState::Valid | SignatureValidationState::Revoked
        ) {
            revocation.status = SignatureValidationState::NonceMismatch;
        }
    }
    if let Some(session) = retrieval_session.as_mut() {
        cache_validated_revocation_evidence(
            session,
            &fetched_revocation,
            &selected_path,
            anchors.get(selected_anchor_index),
            validation_time_unix,
            &mut report.warnings,
        );
        sync_network_report(&mut report.network, session);
    }
    if ltv.revocation_status == RevocationStatus::RevokedByEmbeddedCrl {
        revocation.status = SignatureValidationState::Revoked;
        revocation.errors.push(
            "legacy DSS CRL serial inventory listed the signer; Prompt 24 does not treat unverified DSS CRLs as proof of good status".to_string(),
        );
    }
    let online_revocation_failure = retrieval_session.as_ref().is_some_and(|session| {
        session.traces().iter().any(|trace| {
            matches!(trace.kind, RetrievalKind::Ocsp | RetrievalKind::Crl) && trace.error.is_some()
        })
    });
    if online_revocation_failure
        && !matches!(
            revocation.status,
            SignatureValidationState::Valid
                | SignatureValidationState::Revoked
                | SignatureValidationState::NonceMismatch
        )
    {
        revocation.status = SignatureValidationState::NetworkFailure;
        revocation.errors.push(
            "controlled OCSP/CRL retrieval did not establish usable revocation evidence"
                .to_string(),
        );
    }
    if revocation_options
        .revocation_mode
        .requires_online_retrieval()
        && retrieval_session.is_none()
        && matches!(
            revocation.status,
            SignatureValidationState::EvidenceMissing | SignatureValidationState::NotChecked
        )
    {
        revocation.status = SignatureValidationState::NetworkDisabled;
        revocation.errors.push(
            "online revocation policy requires controlled retrieval or supplied evidence, but retrieval is disabled or its policy was rejected"
                .to_string(),
        );
    }
    let revocation_blocks_trust = if options.revocation_mode.requires_evidence() {
        revocation.status != SignatureValidationState::Valid
    } else {
        revocation.status == SignatureValidationState::Revoked
    };

    let trust = if revocation.status == SignatureValidationState::Revoked {
        SignatureTrust::Revoked
    } else if revocation_blocks_trust {
        SignatureTrust::Untrusted
    } else {
        SignatureTrust::Trusted
    };

    report.path = path_report;
    report.revocation = revocation;
    report.pades = pades_prompt24_report(
        ltv,
        coverage,
        validity,
        cms,
        sub_filter,
        Some(&report.path),
        Some(&report.revocation),
    );
    report.overall = if trust == SignatureTrust::Trusted {
        if *coverage == Coverage::ModifiedAfterSigning {
            SignatureValidationState::ModifiedAfterSigning
        } else {
            SignatureValidationState::Valid
        }
    } else if trust == SignatureTrust::Revoked {
        SignatureValidationState::Revoked
    } else {
        SignatureValidationState::Indeterminate
    };
    report.overall_reason = match report.overall {
        SignatureValidationState::Valid => {
            "signature math, certificate path, and configured revocation policy passed".to_string()
        }
        SignatureValidationState::ModifiedAfterSigning => {
            "signature validates for a historical revision; later bytes are present".to_string()
        }
        SignatureValidationState::Revoked => "revocation evidence reports revoked".to_string(),
        _ => "signature math passed but trust/revocation policy did not establish trusted validity"
            .to_string(),
    };

    Prompt24TrustEvaluation {
        trust,
        report,
        evidence_records: retrieval_session
            .as_ref()
            .map(|session| session.cache().records().cloned().collect())
            .unwrap_or_default(),
    }
}

fn set_validation_indication(report: &mut Prompt24SignatureValidationReport) {
    use SignatureValidationIndication::{Failed, Indeterminate, NotEvaluated, Passed};
    use SignatureValidationState as State;
    use SignatureValidationSubindication as Sub;

    let (indication, subindication) = match &report.overall {
        State::Valid => (Passed, Sub::None),
        State::Indeterminate => (Indeterminate, Sub::ValidationIndeterminate),
        State::Invalid => (Failed, Sub::SignatureMathInvalid),
        State::Malformed => (Failed, Sub::CmsMalformed),
        State::ByteRangeInvalid => (Failed, Sub::PdfStructureInvalid),
        State::DigestMismatch => (Failed, Sub::DigestMismatch),
        State::SignatureMathInvalid => (Failed, Sub::SignatureMathInvalid),
        State::SignerCertificateMissing => (Indeterminate, Sub::SignerCertificateMissing),
        State::SignerCertificateAmbiguous => (Indeterminate, Sub::SignerCertificateAmbiguous),
        State::PathNotFound => (Indeterminate, Sub::PathNotFound),
        State::PathInvalid => (Failed, Sub::PathInvalid),
        State::Untrusted | State::Expired | State::NotYetValid => {
            (Indeterminate, Sub::CertificateUntrusted)
        }
        State::Revoked => (Failed, Sub::CertificateRevoked),
        State::EvidenceMissing => (Indeterminate, Sub::RevocationEvidenceMissing),
        State::EvidenceStale => (Indeterminate, Sub::RevocationEvidenceStale),
        State::RevocationUnknown | State::ConflictingEvidence | State::NonceMismatch => {
            (Indeterminate, Sub::RevocationUnknown)
        }
        State::NetworkDisabled => (Indeterminate, Sub::NetworkDisabled),
        State::NetworkFailure => (Indeterminate, Sub::NetworkFailure),
        State::UnsupportedAlgorithm => (Indeterminate, Sub::UnsupportedAlgorithm),
        State::UnsupportedProfile => (Indeterminate, Sub::UnsupportedProfile),
        State::PolicyRejected => (Failed, Sub::PolicyRejected),
        State::ModifiedAfterSigning | State::PartialDocumentCoverage => {
            (Indeterminate, Sub::DocumentModifiedAfterSigning)
        }
        State::DeferredToLaterPrompt => (Indeterminate, Sub::DeferredToLaterPrompt),
        State::NotChecked => (NotEvaluated, Sub::NotEvaluated),
    };
    report.indication = indication;
    report.subindication = subindication;
}

fn prompt24_error_report(
    options: &VerifyOptions,
    state: SignatureValidationState,
    reason: &str,
) -> Prompt24SignatureValidationReport {
    let (validation_time_unix, validation_time_source) = validation_time(options);
    let mut report = Prompt24SignatureValidationReport {
        policy: policy_report(options, validation_time_unix, validation_time_source),
        signer_resolution: SignatureValidationState::NotChecked,
        path: CertificatePathValidationReport {
            status: state.clone(),
            validation_error: Some(reason.to_string()),
            ..CertificatePathValidationReport::default()
        },
        network: network_prompt24_report(options),
        overall: state,
        overall_reason: reason.to_string(),
        ..Prompt24SignatureValidationReport::default()
    };
    set_validation_indication(&mut report);
    report
}

fn policy_report(
    options: &VerifyOptions,
    validation_time_unix: u64,
    validation_time_source: String,
) -> SignatureValidationPolicyReport {
    SignatureValidationPolicyReport {
        profile: options.policy_profile,
        revocation_mode: options.revocation_mode,
        algorithm_policy: options.algorithm_policy.clone(),
        validation_time_unix,
        validation_time_source,
        online_retrieval_enabled: options.allow_online_retrieval,
        evidence_cache_configured: effective_retrieval_policy(options)
            .cache_directory
            .is_some(),
        ocsp_nonce_policy: effective_retrieval_policy(options).ocsp_nonce_policy,
        max_chain_depth: options.max_chain_depth,
        max_path_candidates: options.max_path_candidates,
        trust_anchor_count: options.trust_anchors_der.len(),
        intermediate_count: options.intermediates_der.len(),
        distrust_entry_count: options.distrusted_certificate_sha256.len(),
        supplied_ocsp_count: options.ocsp_responses_der.len(),
        supplied_crl_count: options.crls_der.len(),
    }
}

fn validation_time(options: &VerifyOptions) -> (u64, String) {
    if let Some(unix) = options.validation_time_unix {
        return (unix, "caller_supplied".to_string());
    }
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    (unix, "system_clock".to_string())
}

fn signer_resolution_state(resolution: SignerCertResolution) -> SignatureValidationState {
    match resolution {
        SignerCertResolution::Found => SignatureValidationState::Valid,
        SignerCertResolution::Missing => SignatureValidationState::SignerCertificateMissing,
        SignerCertResolution::Ambiguous => SignatureValidationState::SignerCertificateAmbiguous,
    }
}

fn parse_trust_anchors(ders: &[Vec<u8>]) -> (Vec<Certificate>, Vec<TrustAnchor>, Vec<String>) {
    let mut certs = Vec::new();
    let mut anchors = Vec::new();
    let mut errors = Vec::new();
    for (idx, der) in ders.iter().enumerate() {
        match Certificate::from_der(der) {
            Ok(cert) => match TrustAnchor::try_from(cert.clone()) {
                Ok(anchor) => {
                    certs.push(cert);
                    anchors.push(anchor);
                }
                Err(err) => errors.push(format!("trust anchor #{idx}: {err}")),
            },
            Err(err) => errors.push(format!("trust anchor #{idx}: {err}")),
        }
    }
    (certs, anchors, errors)
}

fn build_certificate_pool(
    signer: &Certificate,
    chain: &[Certificate],
    options: &VerifyOptions,
) -> (CertPool, Vec<String>) {
    let mut pool = CertPool::new();
    let mut seen = Vec::new();
    let signer_der = signer.to_der().ok();
    for cert in chain {
        let cert_der = cert.to_der().ok();
        if signer_der.is_some() && cert_der == signer_der {
            continue;
        }
        if let Some(der) = cert_der {
            if seen.iter().any(|existing: &Vec<u8>| existing == &der) {
                continue;
            }
            seen.push(der);
        }
        pool.add(cert.clone());
    }
    let mut errors = Vec::new();
    for (idx, der) in options.intermediates_der.iter().enumerate() {
        match Certificate::from_der(der) {
            Ok(cert) => {
                if seen.iter().any(|existing| existing == der) {
                    continue;
                }
                seen.push(der.clone());
                pool.add(cert);
            }
            Err(err) => errors.push(format!("intermediate #{idx}: {err}")),
        }
    }
    (pool, errors)
}

fn retrieval_session_for(options: &VerifyOptions) -> std::result::Result<RetrievalSession, String> {
    let policy = effective_retrieval_policy(options);
    let mut session = RetrievalSession::new(policy.clone())
        .map_err(|error| error.to_string())?
        .with_cancellation_token(options.cancellation.clone());
    if let Some(bundle) = &options.evidence_bundle {
        let store = EvidenceStore::import_bundle(
            bundle,
            policy.budget.max_cache_entries,
            policy.budget.max_cache_bytes,
        )
        .map_err(|error| error.to_string())?;
        session = session.with_cache(store);
    }
    Ok(session)
}

fn sync_network_report(report: &mut NetworkValidationReport, session: &RetrievalSession) {
    report.fetch_traces = session.traces().to_vec();
    report.retrieved_evidence = report
        .fetch_traces
        .iter()
        .filter_map(|trace| {
            Some(NetworkEvidenceReport {
                kind: match trace.kind {
                    RetrievalKind::AiaIssuer => "aia_issuer".to_string(),
                    RetrievalKind::Ocsp => "ocsp".to_string(),
                    RetrievalKind::Crl => "crl".to_string(),
                },
                source_uri: trace
                    .final_uri
                    .clone()
                    .unwrap_or_else(|| trace.requested_uri.clone()),
                sha256: trace.response_sha256.clone()?,
                byte_count: trace.response_bytes,
                cache_hit: trace.cache_hit,
            })
        })
        .collect();
    report.aia_fetching = network_state_for_kind(&report.fetch_traces, RetrievalKind::AiaIssuer);
    report.ocsp_fetching = network_state_for_kind(&report.fetch_traces, RetrievalKind::Ocsp);
    report.crl_fetching = network_state_for_kind(&report.fetch_traces, RetrievalKind::Crl);
    let states = [
        report.aia_fetching.clone(),
        report.ocsp_fetching.clone(),
        report.crl_fetching.clone(),
    ];
    report.status = if states.contains(&SignatureValidationState::NetworkFailure) {
        SignatureValidationState::NetworkFailure
    } else if states.contains(&SignatureValidationState::PolicyRejected) {
        SignatureValidationState::PolicyRejected
    } else if states.contains(&SignatureValidationState::Valid) {
        SignatureValidationState::Valid
    } else {
        SignatureValidationState::NotChecked
    };
    report.note = format!(
        "controlled evidence retrieval recorded {} request trace(s), {} successful/cached response(s)",
        report.fetch_traces.len(),
        report.retrieved_evidence.len()
    );
}

fn network_state_for_kind(
    traces: &[RetrievalTrace],
    kind: RetrievalKind,
) -> SignatureValidationState {
    let matching = traces
        .iter()
        .filter(|trace| trace.kind == kind)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return SignatureValidationState::NotChecked;
    }
    if matching.iter().any(|trace| trace.response_sha256.is_some()) {
        return SignatureValidationState::Valid;
    }
    if matching.iter().any(|trace| {
        trace.error.as_deref().is_some_and(|error| {
            error.contains("forbidden")
                || error.contains("allowlisted")
                || error.contains("credentials")
                || error.contains("unsupported")
        })
    }) {
        SignatureValidationState::PolicyRejected
    } else {
        SignatureValidationState::NetworkFailure
    }
}

fn aia_access_urls(cert: &Certificate, method: ObjectIdentifier) -> Vec<String> {
    let Ok(Some((_critical, access))) = cert.tbs_certificate.get::<AuthorityInfoAccessSyntax>()
    else {
        return Vec::new();
    };
    let mut urls = access
        .0
        .iter()
        .filter(|description| description.access_method == method)
        .filter_map(|description| general_name_uri(&description.access_location))
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    urls
}

fn crl_distribution_urls(cert: &Certificate) -> Vec<String> {
    let Ok(Some((_critical, distribution_points))) =
        cert.tbs_certificate.get::<CrlDistributionPoints>()
    else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for point in distribution_points.0 {
        let Some(DistributionPointName::FullName(names)) = point.distribution_point else {
            continue;
        };
        for name in names {
            if let Some(uri) = general_name_uri(&name) {
                urls.push(uri);
            }
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

fn general_name_uri(name: &GeneralName) -> Option<String> {
    match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.as_str().to_string()),
        _ => None,
    }
}

fn certificates_from_aia_response(bytes: &[u8]) -> Vec<Vec<u8>> {
    if let Ok(cert) = Certificate::from_der(bytes) {
        return cert.to_der().ok().into_iter().collect();
    }
    if let Ok(cert) = Certificate::from_pem(bytes) {
        return cert.to_der().ok().into_iter().collect();
    }
    cms_certificate_der(bytes)
}

fn fetch_aia_intermediates(
    signer: &Certificate,
    chain: &[Certificate],
    session: &mut RetrievalSession,
    warnings: &mut Vec<String>,
) -> Vec<FetchedAiaCertificate> {
    let mut fetched = Vec::new();
    let mut pending = Vec::with_capacity(chain.len().saturating_add(1));
    pending.push(signer.clone());
    pending.extend(chain.iter().cloned());
    let mut visited_certificates = Vec::<Vec<u8>>::new();
    let mut visited_urls = Vec::<String>::new();

    // AIA may expose a chain one issuer at a time. Continue from each newly
    // fetched certificate, but leave the shared request budget as the hard
    // cap and bound the number of parsed candidates separately.
    while let Some(cert) = pending.pop() {
        let Ok(cert_der) = cert.to_der() else {
            warnings.push(
                "AIA traversal skipped a certificate that could not be DER-encoded".to_string(),
            );
            continue;
        };
        if visited_certificates
            .iter()
            .any(|existing| existing == &cert_der)
        {
            continue;
        }
        visited_certificates.push(cert_der);

        let mut urls = aia_access_urls(&cert, OID_AD_CA_ISSUERS);
        urls.sort();
        urls.dedup();
        for url in urls {
            if visited_urls.iter().any(|existing| existing == &url) {
                continue;
            }
            visited_urls.push(url.clone());
            match session.fetch(RetrievalKind::AiaIssuer, &url, RetrievalMethod::Get, None) {
                Ok(response) => {
                    let record =
                        evidence_record_from_response(EvidenceKind::Certificate, &response);
                    let certificates = certificates_from_aia_response(&response.bytes);
                    if certificates.is_empty() {
                        warnings.push(format!(
                            "AIA issuer response at {} did not contain a permitted certificate or PKCS#7 certificate collection",
                            response.final_uri
                        ));
                    }
                    for certificate_der in certificates {
                        if fetched.iter().any(|item: &FetchedAiaCertificate| {
                            item.certificate_der == certificate_der
                        }) || visited_certificates
                            .iter()
                            .any(|existing| existing == &certificate_der)
                        {
                            continue;
                        }
                        if fetched.len() >= MAX_AIA_RETRIEVED_CERTIFICATES {
                            warnings.push(format!(
                                "AIA traversal stopped after {MAX_AIA_RETRIEVED_CERTIFICATES} fetched certificates"
                            ));
                            return fetched;
                        }
                        match Certificate::from_der(&certificate_der) {
                            Ok(fetched_certificate) => {
                                pending.push(fetched_certificate);
                                fetched.push(FetchedAiaCertificate {
                                    certificate_der,
                                    record: record.clone(),
                                });
                            }
                            Err(error) => warnings.push(format!(
                                "AIA issuer response at {} contained malformed certificate data: {error}",
                                response.final_uri
                            )),
                        }
                    }
                }
                Err(error) => {
                    warnings.push(format!("AIA issuer retrieval rejected or failed: {error}"))
                }
            }
        }
    }
    fetched
}

struct OcspRequestMaterial {
    bytes: Vec<u8>,
    nonce: Option<Vec<u8>>,
}

fn build_ocsp_request(
    cert: &Certificate,
    issuer: &Certificate,
    nonce_policy: OcspNoncePolicy,
) -> std::result::Result<OcspRequestMaterial, String> {
    let request = OcspRequest::from_cert::<Sha1>(issuer, cert)
        .map_err(|error| format!("OCSP CertID construction: {error}"))?;
    let mut builder = OcspRequestBuilder::default().with_request(request);
    let nonce = if nonce_policy == OcspNoncePolicy::Required {
        let mut bytes = vec![0_u8; 16];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| format!("OCSP nonce generation: {error}"))?;
        builder = builder
            .with_extension(
                OcspNonce::new(bytes.clone())
                    .map_err(|error| format!("OCSP nonce encoding: {error}"))?,
            )
            .map_err(|error| format!("OCSP nonce extension: {error}"))?;
        Some(bytes)
    } else {
        None
    };
    let bytes = builder
        .build()
        .to_der()
        .map_err(|error| format!("OCSP request encoding: {error}"))?;
    Ok(OcspRequestMaterial { bytes, nonce })
}

fn validate_ocsp_response_nonce(
    response_der: &[u8],
    expected_nonce: Option<&[u8]>,
) -> std::result::Result<(), String> {
    let Some(expected_nonce) = expected_nonce else {
        return Ok(());
    };
    let response = OcspResponse::from_der(response_der)
        .map_err(|error| format!("OCSP response nonce parse: {error}"))?;
    if response.response_status != OcspResponseStatus::Successful {
        return Err(format!(
            "OCSP response nonce unavailable because response status was {:?}",
            response.response_status
        ));
    }
    let response_bytes = response.response_bytes.ok_or_else(|| {
        "OCSP response nonce unavailable: BasicOCSPResponse was absent".to_string()
    })?;
    if response_bytes.response_type != OID_OCSP_BASIC_RESPONSE {
        return Err(
            "OCSP response nonce unavailable: response type was not BasicOCSPResponse".to_string(),
        );
    }
    let basic = BasicOcspResponse::from_der(response_bytes.response.as_bytes())
        .map_err(|error| format!("OCSP BasicOCSPResponse nonce parse: {error}"))?;
    let received_nonce = basic
        .nonce()
        .ok_or_else(|| "OCSP response nonce was absent or malformed".to_string())?;
    if received_nonce.0.as_bytes() != expected_nonce {
        return Err("OCSP response nonce did not match the request".to_string());
    }
    Ok(())
}

fn fetch_online_revocation_evidence(
    path: &[Certificate],
    anchor_certificate: Option<&Certificate>,
    session: &mut RetrievalSession,
    warnings: &mut Vec<String>,
) -> FetchedRevocationEvidence {
    let mut evidence = FetchedRevocationEvidence::default();
    for (index, cert) in path.iter().enumerate() {
        let issuer = path.get(index + 1).or(anchor_certificate);
        let Some(issuer) = issuer else {
            warnings.push(format!(
                "no issuer certificate was available to construct revocation requests for path position {index}"
            ));
            continue;
        };
        match build_ocsp_request(cert, issuer, session.policy().ocsp_nonce_policy) {
            Ok(request) => {
                for url in aia_access_urls(cert, OID_AD_OCSP) {
                    match session.fetch(
                        RetrievalKind::Ocsp,
                        &url,
                        RetrievalMethod::Post,
                        Some(&request.bytes),
                    ) {
                        Ok(response) => {
                            match validate_ocsp_response_nonce(
                                &response.bytes,
                                request.nonce.as_deref(),
                            ) {
                                Ok(()) => {
                                    let record = evidence_record_from_response(
                                        EvidenceKind::Ocsp,
                                        &response,
                                    );
                                    push_unique_fetched_evidence(
                                        &mut evidence.ocsp,
                                        FetchedNetworkEvidence {
                                            bytes: response.bytes,
                                            record,
                                            expected_ocsp_nonce: request.nonce.clone(),
                                        },
                                    );
                                }
                                Err(error) => {
                                    let message = format!(
                                        "OCSP response from {} was rejected by nonce policy: {error}",
                                        response.final_uri
                                    );
                                    warnings.push(message.clone());
                                    evidence.rejected_ocsp.push(message);
                                }
                            }
                        }
                        Err(error) => {
                            warnings.push(format!("OCSP retrieval rejected or failed: {error}"))
                        }
                    }
                }
            }
            Err(error) => warnings.push(error),
        }
        for url in crl_distribution_urls(cert) {
            match session.fetch(RetrievalKind::Crl, &url, RetrievalMethod::Get, None) {
                Ok(response) => {
                    let record = evidence_record_from_response(EvidenceKind::Crl, &response);
                    push_unique_fetched_evidence(
                        &mut evidence.crls,
                        FetchedNetworkEvidence {
                            bytes: response.bytes,
                            record,
                            expected_ocsp_nonce: None,
                        },
                    );
                }
                Err(error) => warnings.push(format!("CRL retrieval rejected or failed: {error}")),
            }
        }
    }
    evidence
}

fn cache_selected_aia_evidence(
    session: &mut RetrievalSession,
    fetched: &[FetchedAiaCertificate],
    selected_path: &[Certificate],
    warnings: &mut Vec<String>,
) {
    for item in fetched {
        let used_by_selected_path = selected_path.iter().any(|cert| {
            cert.to_der()
                .map(|der| der == item.certificate_der)
                .unwrap_or(false)
        });
        if !used_by_selected_path {
            continue;
        }
        let mut record = item.record.clone();
        record.validated_at_acquisition = true;
        if let Err(error) = session.cache_validated(record) {
            warnings.push(format!("validated AIA evidence was not cached: {error}"));
        }
    }
}

fn cache_validated_revocation_evidence(
    session: &mut RetrievalSession,
    fetched: &FetchedRevocationEvidence,
    path: &[Certificate],
    anchor: Option<&TrustAnchor>,
    validation_time_unix: u64,
    warnings: &mut Vec<String>,
) {
    for item in &fetched.ocsp {
        if revocation_evidence_valid_for_path(
            EvidenceKind::Ocsp,
            &item.bytes,
            path,
            anchor,
            validation_time_unix,
            item.expected_ocsp_nonce.as_deref(),
        ) {
            let mut record = item.record.clone();
            record.validated_at_acquisition = true;
            if let Err(error) = session.cache_validated(record) {
                warnings.push(format!("validated OCSP evidence was not cached: {error}"));
            }
        }
    }
    for item in &fetched.crls {
        if revocation_evidence_valid_for_path(
            EvidenceKind::Crl,
            &item.bytes,
            path,
            anchor,
            validation_time_unix,
            None,
        ) {
            let mut record = item.record.clone();
            record.validated_at_acquisition = true;
            if let Err(error) = session.cache_validated(record) {
                warnings.push(format!("validated CRL evidence was not cached: {error}"));
            }
        }
    }
}

fn revocation_evidence_valid_for_path(
    kind: EvidenceKind,
    der: &[u8],
    path: &[Certificate],
    anchor: Option<&TrustAnchor>,
    validation_time_unix: u64,
    expected_ocsp_nonce: Option<&[u8]>,
) -> bool {
    if kind == EvidenceKind::Ocsp && validate_ocsp_response_nonce(der, expected_ocsp_nonce).is_err()
    {
        return false;
    }
    for (index, cert) in path.iter().enumerate() {
        let issuer = path.get(index + 1);
        let result = match kind {
            EvidenceKind::Ocsp => {
                match OcspChecker::new(der, validation_time_unix, DefaultVerifier) {
                    Ok(checker) => match issuer {
                        Some(issuer) => checker.check_revocation(cert, issuer),
                        None => anchor
                            .map(|anchor| checker.check_revocation_against_anchor(cert, anchor))
                            .unwrap_or_else(|| Err(pkix_revocation::Error::OcspStatusUnknown)),
                    },
                    Err(_) => continue,
                }
            }
            EvidenceKind::Crl => {
                match CrlChecker::new(der, validation_time_unix, DefaultVerifier) {
                    Ok(checker) => match issuer {
                        Some(issuer) => checker.check_revocation(cert, issuer),
                        None => anchor
                            .map(|anchor| checker.check_revocation_against_anchor(cert, anchor))
                            .unwrap_or_else(|| Err(pkix_revocation::Error::OcspStatusUnknown)),
                    },
                    Err(_) => continue,
                }
            }
            EvidenceKind::Certificate => continue,
        };
        if result.is_ok() || result.as_ref().err().is_some_and(is_revoked) {
            return true;
        }
    }
    false
}

type PathValidationSelection = (Option<(Vec<Certificate>, usize)>, usize, Option<String>);

fn validate_candidate_paths(
    signer: &Certificate,
    pool: &CertPool,
    anchors: &[TrustAnchor],
    anchor_certificates: &[Certificate],
    policy: &ValidationPolicy,
    builder_config: &PathBuilderConfig,
    distrusted_fingerprints: &std::collections::BTreeSet<String>,
) -> PathValidationSelection {
    let mut selected = None;
    let mut attempted = 0usize;
    let mut last_error = None;
    for candidate in
        pkix_path_builder::build_path_candidates_with_config(signer, pool, anchors, builder_config)
    {
        match candidate {
            Ok(path) => {
                attempted += 1;
                if let Some(fingerprint) = path.iter().find_map(|certificate| {
                    certificate_fingerprint_sha256(certificate)
                        .filter(|fingerprint| distrusted_fingerprints.contains(fingerprint))
                }) {
                    last_error = Some(format!(
                        "candidate path contains a caller-distrusted certificate {fingerprint}"
                    ));
                    continue;
                }
                match pkix_path::validate_path(&path, anchors, policy, &DefaultVerifier) {
                    Ok(validated) => {
                        if anchor_certificates
                            .get(validated.anchor_index)
                            .and_then(certificate_fingerprint_sha256)
                            .is_some_and(|fingerprint| {
                                distrusted_fingerprints.contains(&fingerprint)
                            })
                        {
                            last_error = Some(
                                "candidate path terminates at a caller-distrusted trust anchor"
                                    .to_string(),
                            );
                            continue;
                        }
                        selected = Some((path, validated.anchor_index));
                        break;
                    }
                    Err(error) => last_error = Some(error.to_string()),
                }
            }
            Err(error) => {
                last_error = Some(error.to_string());
                break;
            }
        }
    }
    (selected, attempted, last_error)
}

fn path_validation_checks() -> Vec<&'static str> {
    vec![
        "certificate_signature_chain",
        "trust_anchor_termination",
        "validity_period",
        "basic_constraints_ca",
        "path_len_constraint",
        "key_usage_key_cert_sign_when_present",
        "signer_key_usage_digital_signature_or_non_repudiation_when_present",
        "unknown_critical_extension_rejection",
        "name_constraints_dns_rfc822_uri_ip_directory_name",
        "certificate_policy_tree",
        "policy_mappings",
        "inhibit_any_policy",
        "policy_constraints",
        "algorithm_identifier_dispatch",
        "rsa_min_key_size_policy",
    ]
}

/// A KeyUsage extension limits all uses of the subject key. A PDF/CMS signing
/// certificate therefore needs either digitalSignature or nonRepudiation when
/// it declares KeyUsage. Absence remains permitted by RFC 5280; profiles that
/// require a particular extension are evaluated separately by PAdES policy.
fn validate_signer_certificate_usage(cert: &Certificate) -> std::result::Result<(), String> {
    let key_usage = cert.tbs_certificate.get::<KeyUsage>().map_err(|error| {
        format!("signer KeyUsage extension is malformed or duplicated: {error}")
    })?;
    let Some((_critical, key_usage)) = key_usage else {
        return Ok(());
    };
    if key_usage.digital_signature() || key_usage.non_repudiation() {
        Ok(())
    } else {
        Err("signer certificate KeyUsage does not permit digital signatures".to_string())
    }
}

fn trust_from_path_failure(error: &Option<String>) -> SignatureTrust {
    match error.as_deref() {
        Some(msg) if msg.contains("validity") || msg.contains("ValidityPeriod") => {
            SignatureTrust::Expired
        }
        _ => SignatureTrust::Untrusted,
    }
}

/// Candidate certificate set for CRL signer discovery. These are all
/// untrusted until the selected document path or a separately built RFC 5280
/// path validates them to an explicit anchor.
fn crl_signer_candidates(
    cms_chain: &[Certificate],
    selected_path: &[Certificate],
    anchor_certificates: &[Certificate],
    options: &VerifyOptions,
) -> Vec<Certificate> {
    let mut candidates = Vec::new();
    let mut seen = Vec::<Vec<u8>>::new();
    let mut add = |cert: Certificate| {
        let Ok(der) = cert.to_der() else {
            return;
        };
        if seen.iter().any(|existing| existing == &der) {
            return;
        }
        seen.push(der);
        candidates.push(cert);
    };
    for cert in selected_path {
        add(cert.clone());
    }
    for cert in cms_chain {
        add(cert.clone());
    }
    for der in &options.intermediates_der {
        if let Ok(cert) = Certificate::from_der(der) {
            add(cert);
        }
    }
    for cert in anchor_certificates {
        add(cert.clone());
    }
    candidates
}

struct RevocationEvaluationContext<'a> {
    path: &'a [Certificate],
    anchor: Option<&'a TrustAnchor>,
    anchor_certificate: Option<&'a Certificate>,
    crl_signer_candidates: &'a [Certificate],
    cms_chain: &'a [Certificate],
    anchors: &'a [TrustAnchor],
    path_policy: &'a ValidationPolicy,
    builder_config: &'a PathBuilderConfig,
    options: &'a VerifyOptions,
    validation_time_unix: u64,
}

fn evaluate_revocation_prompt24(
    ctx: RevocationEvaluationContext<'_>,
) -> RevocationValidationReport {
    let mut report = RevocationValidationReport {
        ocsp_responses_supplied: ctx.options.ocsp_responses_der.len(),
        crls_supplied: ctx.options.crls_der.len(),
        ..RevocationValidationReport::default()
    };
    let evidence_count = ctx.options.ocsp_responses_der.len() + ctx.options.crls_der.len();
    if evidence_count == 0 {
        report.status = if ctx.options.revocation_mode.requires_evidence() {
            SignatureValidationState::EvidenceMissing
        } else {
            SignatureValidationState::NotChecked
        };
        return report;
    }

    let mut any_good = false;
    let mut any_revoked = false;
    for (idx, cert) in ctx.path.iter().enumerate() {
        let decision = evaluate_certificate_revocation(idx, cert, ctx.path.get(idx + 1), &ctx);
        any_good |= decision.status == SignatureValidationState::Valid;
        any_revoked |= decision.status == SignatureValidationState::Revoked;
        report.certificate_decisions.push(decision);
    }
    report.status = if any_revoked {
        SignatureValidationState::Revoked
    } else if report
        .certificate_decisions
        .iter()
        .all(|decision| decision.status == SignatureValidationState::Valid)
    {
        SignatureValidationState::Valid
    } else if any_good
        && matches!(
            ctx.options.revocation_mode,
            SignatureRevocationMode::OfflineBestEffort | SignatureRevocationMode::OnlineBestEffort
        )
    {
        SignatureValidationState::Indeterminate
    } else {
        SignatureValidationState::RevocationUnknown
    };
    report
}

fn evaluate_certificate_revocation(
    path_index: usize,
    cert: &Certificate,
    issuer: Option<&Certificate>,
    ctx: &RevocationEvaluationContext<'_>,
) -> CertificateRevocationDecision {
    let mut errors = Vec::new();
    let mut good_evidence = Vec::new();
    let mut revoked_evidence = Vec::new();
    for (idx, der) in ctx.options.ocsp_responses_der.iter().enumerate() {
        match OcspChecker::new(der, ctx.validation_time_unix, DefaultVerifier) {
            Ok(checker) => {
                let result = match issuer {
                    Some(issuer) => checker.check_revocation(cert, issuer),
                    None => ctx
                        .anchor
                        .map(|anchor| checker.check_revocation_against_anchor(cert, anchor))
                        .unwrap_or_else(|| Err(pkix_revocation::Error::OcspStatusUnknown)),
                };
                match result {
                    Ok(()) => good_evidence.push(format!("ocsp_response_{idx}")),
                    Err(err) if is_revoked(&err) => {
                        revoked_evidence.push((format!("ocsp_response_{idx}"), err.to_string()));
                    }
                    Err(err) => errors.push(format!("ocsp_response_{idx}: {err}")),
                }
            }
            Err(err) => errors.push(format!("ocsp_response_{idx}: {err}")),
        }
    }
    let mut base_crls = Vec::new();
    let mut delta_crls = Vec::new();
    for (idx, der) in ctx.options.crls_der.iter().enumerate() {
        match crl_is_delta(der) {
            Ok(true) => delta_crls.push((idx, der.as_slice())),
            Ok(false) => base_crls.push((idx, der.as_slice())),
            Err(error) => errors.push(format!("crl_{idx}: {error}")),
        }
    }

    // A delta carries only changes from a base CRL. It is never evaluated on
    // its own, because an absent revoked entry in a delta does not establish a
    // certificate as good. Successful pair construction also binds issuer,
    // CRL number, freshness, signature, and removeFromCRL merge semantics.
    let mut base_used_by_delta = vec![false; base_crls.len()];
    for (delta_idx, delta_der) in delta_crls {
        let mut paired = false;
        let mut last_error = None;
        for (base_position, (base_idx, base_der)) in base_crls.iter().enumerate() {
            match crl_checker_for_certificate(
                base_der,
                Some(delta_der),
                issuer,
                ctx.anchor_certificate,
                ctx.path,
                ctx.crl_signer_candidates,
                ctx.cms_chain,
                ctx.anchors,
                ctx.path_policy,
                ctx.builder_config,
                ctx.options,
                ctx.validation_time_unix,
            ) {
                Ok(checker) => {
                    paired = true;
                    base_used_by_delta[base_position] = true;
                    let source = format!("crl_base_{base_idx}_delta_{delta_idx}");
                    record_crl_check_result(
                        source,
                        check_crl_for_certificate(&checker, cert, issuer, ctx.anchor),
                        &mut good_evidence,
                        &mut revoked_evidence,
                        &mut errors,
                    );
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if !paired {
            errors.push(format!(
                "delta CRL {delta_idx} had no valid supplied base CRL pair{}",
                last_error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ));
        }
    }

    for (base_position, (idx, der)) in base_crls.into_iter().enumerate() {
        if base_used_by_delta[base_position] {
            continue;
        }
        match crl_checker_for_certificate(
            der,
            None,
            issuer,
            ctx.anchor_certificate,
            ctx.path,
            ctx.crl_signer_candidates,
            ctx.cms_chain,
            ctx.anchors,
            ctx.path_policy,
            ctx.builder_config,
            ctx.options,
            ctx.validation_time_unix,
        ) {
            Ok(checker) => record_crl_check_result(
                format!("crl_{idx}"),
                check_crl_for_certificate(&checker, cert, issuer, ctx.anchor),
                &mut good_evidence,
                &mut revoked_evidence,
                &mut errors,
            ),
            Err(error) => errors.push(format!("crl_{idx}: {error}")),
        }
    }
    if let Some((source, reason)) = revoked_evidence.into_iter().next() {
        let conflict = (!good_evidence.is_empty()).then(|| {
            format!(
                "conflicting revocation evidence: revoked {source} takes precedence over good {}",
                good_evidence.join(", ")
            )
        });
        return revocation_decision(
            path_index,
            cert,
            SignatureValidationState::Revoked,
            Some(source),
            conflict.or(Some(reason)),
        );
    }
    if let Some(source) = good_evidence.into_iter().next() {
        return revocation_decision(
            path_index,
            cert,
            SignatureValidationState::Valid,
            Some(source),
            None,
        );
    }
    let state = if errors.iter().any(|err| {
        err.contains("expired") || err.contains("stale") || err.contains("validity window")
    }) {
        SignatureValidationState::EvidenceStale
    } else {
        SignatureValidationState::RevocationUnknown
    };
    revocation_decision(path_index, cert, state, None, Some(errors.join("; ")))
}

fn crl_is_delta(der: &[u8]) -> std::result::Result<bool, String> {
    let crl = CertificateList::from_der(der).map_err(|error| format!("CRL DER parse: {error}"))?;
    Ok(crl
        .tbs_cert_list
        .crl_extensions
        .as_deref()
        .is_some_and(|extensions| {
            extensions
                .iter()
                .any(|extension| extension.extn_id == OID_DELTA_CRL_INDICATOR)
        }))
}

/// Construct a CRL checker only after any non-direct cRLIssuer has been
/// proven to be an explicit anchor or to validate through the same bounded
/// PKIX machinery as the document signer. `pkix-revocation` performs the
/// CRL signature, IDP, cRLSign, scope, and entry checks; this layer supplies
/// the missing trust decision for a delegated CRL signer.
#[allow(clippy::too_many_arguments)]
fn crl_checker_for_certificate(
    base_der: &[u8],
    delta_der: Option<&[u8]>,
    issuer: Option<&Certificate>,
    anchor_certificate: Option<&Certificate>,
    selected_path: &[Certificate],
    signer_candidates: &[Certificate],
    cms_chain: &[Certificate],
    anchors: &[TrustAnchor],
    path_policy: &ValidationPolicy,
    builder_config: &PathBuilderConfig,
    options: &VerifyOptions,
    validation_time_unix: u64,
) -> std::result::Result<CrlChecker<DefaultVerifier>, String> {
    let crl =
        CertificateList::from_der(base_der).map_err(|error| format!("CRL DER parse: {error}"))?;
    let declares_indirect = crl_declares_indirect(&crl)?;
    let direct_issuer = issuer.or(anchor_certificate);

    // A direct CRL is scoped to the certificate issuer's name even when the
    // CRL signing key is discovered through a valid key-rollover bridge.
    // `pkix-revocation` intentionally relaxes this check in its discovery
    // constructor so it can model RFC 5280 key rollover. Enforce it here
    // before that constructor is reachable: otherwise a different CA's CRL
    // could be treated as evidence for this certificate merely because its
    // signer also happens to chain to a configured anchor.
    if !declares_indirect {
        if let Some(direct_issuer) = direct_issuer {
            if crl.tbs_cert_list.issuer != direct_issuer.tbs_certificate.subject {
                return Err("direct CRL issuer does not match the certificate issuer".to_string());
            }
        }
    }
    let discovered = discover_crl_signer(signer_candidates, &crl);
    let delegated = discovered.filter(|candidate| {
        !direct_issuer.is_some_and(|direct| certificates_equal(candidate, direct))
    });

    if declares_indirect {
        let crl_issuer = discovered.ok_or_else(|| {
            "indirect CRL signer was not found among supplied and path-validated certificate candidates"
                .to_string()
        })?;
        validate_crl_signer_path(
            crl_issuer,
            selected_path,
            cms_chain,
            anchors,
            path_policy,
            builder_config,
            options,
        )?;
        return match delta_der {
            Some(delta_der) => CrlChecker::with_delta_and_crl_issuer(
                base_der,
                delta_der,
                crl_issuer.clone(),
                validation_time_unix,
                DefaultVerifier,
            )
            .map_err(|error| format!("delegated delta CRL construction: {error}")),
            None => CrlChecker::new_with_crl_issuer(
                base_der,
                crl_issuer.clone(),
                validation_time_unix,
                DefaultVerifier,
            )
            .map_err(|error| format!("delegated CRL construction: {error}")),
        };
    }

    if let Some(crl_issuer) = delegated {
        // A direct CRL signed by a different key is only acceptable for a
        // validated key-rollover bridge. The dependency's signer-discovery
        // constructor models that RFC 5280/PKITS case; the explicit path
        // validation above supplies the trust proof it intentionally omits.
        validate_crl_signer_path(
            crl_issuer,
            selected_path,
            cms_chain,
            anchors,
            path_policy,
            builder_config,
            options,
        )?;
        if delta_der.is_some() {
            // The discovery constructor can authenticate a direct CRL signed
            // with a rollover key, but it has no corresponding delta-aware
            // constructor. Accepting the base alone here would make a delta
            // CRL appear to establish a good result. Require a common direct
            // signer (or an explicitly declared indirect CRL) for a delta
            // pair until that combination has a dedicated implementation.
            return Err(
                "delta CRL with a discovered direct rollover signer is unsupported; no base/delta pair was accepted"
                    .to_string(),
            );
        }
        return CrlChecker::new_with_signer_discovery(
            base_der,
            signer_candidates,
            issuer.or(anchor_certificate).ok_or_else(|| {
                "CRL signer discovery requires an issuer certificate or trust anchor".to_string()
            })?,
            validation_time_unix,
            DefaultVerifier,
        )
        .map_err(|error| format!("CRL signer discovery construction: {error}"));
    }

    match delta_der {
        Some(delta_der) => {
            CrlChecker::with_delta(base_der, delta_der, validation_time_unix, DefaultVerifier)
                .map_err(|error| format!("base/delta CRL construction: {error}"))
        }
        None => CrlChecker::new(base_der, validation_time_unix, DefaultVerifier)
            .map_err(|error| format!("CRL construction: {error}")),
    }
}

fn crl_declares_indirect(crl: &CertificateList) -> std::result::Result<bool, String> {
    let Some(extension) = crl
        .tbs_cert_list
        .crl_extensions
        .as_deref()
        .and_then(|extensions| {
            extensions
                .iter()
                .find(|extension| extension.extn_id == OID_ISSUING_DISTRIBUTION_POINT)
        })
    else {
        return Ok(false);
    };
    if !extension.critical {
        return Err("IssuingDistributionPoint extension must be critical".to_string());
    }
    let idp = IssuingDistributionPoint::from_der(extension.extn_value.as_bytes())
        .map_err(|error| format!("IssuingDistributionPoint DER parse: {error}"))?;
    Ok(idp.indirect_crl)
}

fn validate_crl_signer_path(
    crl_issuer: &Certificate,
    selected_path: &[Certificate],
    cms_chain: &[Certificate],
    anchors: &[TrustAnchor],
    path_policy: &ValidationPolicy,
    builder_config: &PathBuilderConfig,
    options: &VerifyOptions,
) -> std::result::Result<(), String> {
    let distrusted_fingerprints =
        configured_distrust_set(options).map_err(|error| error.to_string())?;
    if certificate_fingerprint_sha256(crl_issuer)
        .as_ref()
        .is_some_and(|fingerprint| distrusted_fingerprints.contains(fingerprint))
    {
        return Err("delegated CRL signer is caller-distrusted".to_string());
    }
    if selected_path
        .iter()
        .any(|candidate| certificates_equal(candidate, crl_issuer))
    {
        return Ok(());
    }
    if options.trust_anchors_der.iter().any(|der| {
        Certificate::from_der(der)
            .map(|candidate| certificates_equal(&candidate, crl_issuer))
            .unwrap_or(false)
    }) {
        return Ok(());
    }

    let (pool, parse_errors) = build_certificate_pool(crl_issuer, cms_chain, options);
    let (anchor_certificates, _, _) = parse_trust_anchors(&options.trust_anchors_der);
    let (path, attempts, error) = validate_candidate_paths(
        crl_issuer,
        &pool,
        anchors,
        &anchor_certificates,
        path_policy,
        builder_config,
        &distrusted_fingerprints,
    );
    if path.is_some() {
        return Ok(());
    }
    let parse_note = (!parse_errors.is_empty()).then(|| parse_errors.join(", "));
    Err(format!(
        "delegated CRL signer did not validate to an explicit trust anchor after {attempts} candidate path(s){}{}",
        error
            .as_deref()
            .map(|value| format!(": {value}"))
            .unwrap_or_default(),
        parse_note
            .as_deref()
            .map(|value| format!("; intermediate parse errors: {value}"))
            .unwrap_or_default(),
    ))
}

fn certificates_equal(left: &Certificate, right: &Certificate) -> bool {
    match (left.to_der(), right.to_der()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn check_crl_for_certificate(
    checker: &CrlChecker<DefaultVerifier>,
    cert: &Certificate,
    issuer: Option<&Certificate>,
    anchor: Option<&TrustAnchor>,
) -> std::result::Result<(), pkix_revocation::Error> {
    match issuer {
        Some(issuer) => checker.check_revocation(cert, issuer),
        None => anchor
            .map(|anchor| checker.check_revocation_against_anchor(cert, anchor))
            .unwrap_or_else(|| Err(pkix_revocation::Error::OcspStatusUnknown)),
    }
}

fn record_crl_check_result(
    source: String,
    result: std::result::Result<(), pkix_revocation::Error>,
    good_evidence: &mut Vec<String>,
    revoked_evidence: &mut Vec<(String, String)>,
    errors: &mut Vec<String>,
) {
    match result {
        Ok(()) => good_evidence.push(source),
        Err(error) if is_revoked(&error) => revoked_evidence.push((source, error.to_string())),
        Err(error) => errors.push(format!("{source}: {error}")),
    }
}

fn revocation_decision(
    path_index: usize,
    cert: &Certificate,
    status: SignatureValidationState,
    evidence_type: Option<String>,
    error: Option<String>,
) -> CertificateRevocationDecision {
    CertificateRevocationDecision {
        path_index,
        subject: cert.tbs_certificate.subject.to_string(),
        serial_hex: hex_upper(cert.tbs_certificate.serial_number.as_bytes()),
        status,
        evidence_type,
        error,
    }
}

fn is_revoked(err: &pkix_revocation::Error) -> bool {
    matches!(err, pkix_revocation::Error::Revoked { .. })
}

fn pades_prompt24_report(
    ltv: &LtvReport,
    coverage: &Coverage,
    validity: &SignatureValidity,
    cms: &CmsValidationReport,
    sub_filter: Option<&str>,
    path: Option<&CertificatePathValidationReport>,
    revocation: Option<&RevocationValidationReport>,
) -> PadesValidationReport {
    let higher_level = ltv.timestamp_token_count > 0 || ltv.dss_present;
    let mut missing = Vec::new();
    let sub_filter = sub_filter.unwrap_or_default();
    let detected_profile = match sub_filter {
        "ETSI.CAdES.detached" => "pades_baseline_b_candidate".to_string(),
        "ETSI.RFC3161" => "pades_document_timestamp_prompt25_classified".to_string(),
        "adbe.pkcs7.detached" | "adbe.pkcs7.sha1" => "generic_pdf_cms_detached".to_string(),
        "" => "missing_pdf_signature_subfilter".to_string(),
        _ => format!("unsupported_pdf_signature_subfilter:{sub_filter}"),
    };
    if sub_filter != "ETSI.CAdES.detached" {
        missing.push("PAdES baseline requires /SubFilter /ETSI.CAdES.detached".to_string());
    }
    if *validity != SignatureValidity::Valid {
        missing.push("valid CMS detached signature".to_string());
    }
    if cms.signed_attributes != SignatureValidationState::Valid {
        missing.push("valid CMS signed attributes".to_string());
    }
    if cms.content_type_attribute != SignatureValidationState::Valid {
        missing.push("CMS contentType=id-data signed attribute".to_string());
    }
    if cms.message_digest_attribute != SignatureValidationState::Valid {
        missing.push("CMS messageDigest signed attribute bound to PDF ByteRange".to_string());
    }
    if cms.signing_certificate_reference != SignatureValidationState::Valid {
        missing
            .push("validated ESS SigningCertificate or SigningCertificateV2 reference".to_string());
    }
    let required_cms_attributes_valid = cms.signed_attributes == SignatureValidationState::Valid
        && cms.content_type_attribute == SignatureValidationState::Valid
        && cms.message_digest_attribute == SignatureValidationState::Valid
        && cms.signing_certificate_reference == SignatureValidationState::Valid;
    let structural_status = if sub_filter == "ETSI.RFC3161" {
        SignatureValidationState::DeferredToLaterPrompt
    } else if sub_filter != "ETSI.CAdES.detached" {
        SignatureValidationState::UnsupportedProfile
    } else if *validity == SignatureValidity::Valid && required_cms_attributes_valid {
        SignatureValidationState::Valid
    } else if matches!(
        cms.signed_attributes,
        SignatureValidationState::NotChecked | SignatureValidationState::Indeterminate
    ) {
        SignatureValidationState::Indeterminate
    } else {
        SignatureValidationState::Invalid
    };
    let certificate_path_status = path
        .map(|report| report.status.clone())
        .unwrap_or(SignatureValidationState::NotChecked);
    let revocation_status = revocation
        .map(|report| report.status.clone())
        .unwrap_or(SignatureValidationState::NotChecked);
    let status = match structural_status.clone() {
        SignatureValidationState::Valid => match certificate_path_status.clone() {
            SignatureValidationState::Valid => match revocation_status.clone() {
                // A caller can explicitly select no revocation check. That
                // posture remains visible in the report, while a configured
                // strict or online policy must reach `valid` to produce a
                // final baseline pass.
                SignatureValidationState::Valid | SignatureValidationState::NotChecked => {
                    SignatureValidationState::Valid
                }
                SignatureValidationState::Revoked => SignatureValidationState::Revoked,
                _ => SignatureValidationState::Indeterminate,
            },
            SignatureValidationState::NotChecked | SignatureValidationState::EvidenceMissing => {
                SignatureValidationState::Indeterminate
            }
            other => other,
        },
        other => other,
    };
    PadesValidationReport {
        status,
        structural_status,
        detected_profile,
        validated_level: if sub_filter == "ETSI.CAdES.detached" {
            match ltv.pades_level {
                PadesLevel::BaselineLT => {
                    "pades_baseline_lt_with_validated_timestamp_and_replayable_evidence".to_string()
                }
                PadesLevel::BaselineT => {
                    "pades_baseline_t_with_validated_signature_timestamp".to_string()
                }
                PadesLevel::BaselineLTA => {
                    "pades_baseline_lta_archive_timestamp_classified".to_string()
                }
                PadesLevel::BaselineB => "pades_baseline_b_pdf_cms_ess_conformance".to_string(),
            }
        } else {
            "none".to_string()
        },
        signed_revision_coverage_status: SignatureValidationState::Valid,
        current_document_status: match coverage {
            Coverage::WholeFile => SignatureValidationState::Valid,
            Coverage::ModifiedAfterSigning => SignatureValidationState::ModifiedAfterSigning,
        },
        certificate_path_status,
        revocation_status,
        higher_level_evidence_present: higher_level,
        higher_level_evidence_status: if higher_level {
            match ltv.pades_level {
                PadesLevel::BaselineT | PadesLevel::BaselineLT => SignatureValidationState::Valid,
                PadesLevel::BaselineLTA => SignatureValidationState::DeferredToLaterPrompt,
                PadesLevel::BaselineB => SignatureValidationState::Indeterminate,
            }
        } else {
            SignatureValidationState::NotChecked
        },
        missing_requirements: missing,
    }
}

fn network_prompt24_report(options: &VerifyOptions) -> NetworkValidationReport {
    if effective_retrieval_policy(options).enabled {
        NetworkValidationReport {
            status: SignatureValidationState::NotChecked,
            aia_fetching: SignatureValidationState::NotChecked,
            ocsp_fetching: SignatureValidationState::NotChecked,
            crl_fetching: SignatureValidationState::NotChecked,
            fetch_traces: Vec::new(),
            retrieved_evidence: Vec::new(),
            note: "controlled online evidence retrieval is enabled; every fetched item remains untrusted until cryptographically validated".to_string(),
        }
    } else {
        NetworkValidationReport::default()
    }
}

fn effective_retrieval_policy(options: &VerifyOptions) -> RetrievalPolicy {
    let mut policy = options.retrieval_policy.clone();
    policy.enabled = options.allow_online_retrieval || policy.enabled;
    policy
}

fn deferred_prompt24_evidence(ltv: &LtvReport) -> Vec<String> {
    let mut deferred = Vec::new();
    if ltv.invalid_timestamp_token_count > 0 {
        deferred
            .push("one or more RFC 3161 timestamp tokens failed Prompt 25 validation".to_string());
    }
    if ltv.dss_present && !ltv.vri_matched {
        deferred.push(
            "DSS is present but no matching VRI entry was bound to this signature".to_string(),
        );
    }
    deferred
}

fn status_note(report: &SignatureReport) -> String {
    let integrity = match report.validity {
        SignatureValidity::Valid => "cryptographically valid over the signed byte ranges",
        SignatureValidity::Invalid => {
            "signature/digest did NOT verify (signed content changed or signature corrupt)"
        }
        SignatureValidity::UnsupportedAlgorithm => {
            "signature algorithm or parameters are not supported by the configured verifier"
        }
        SignatureValidity::Error => "could not verify",
    };
    let trust = match report.trust {
        SignatureTrust::NotVerified => {
            "signer trust NOT verified (no trust anchors configured) — a valid signature here is \
             not proof of a trusted signer"
        }
        SignatureTrust::Trusted => "signer chains to a configured trust anchor",
        SignatureTrust::Untrusted => {
            "signer does NOT chain to any configured trust anchor (self-signed or unknown issuer)"
        }
        SignatureTrust::Expired => "signer certificate is outside its validity period",
        SignatureTrust::Revoked => "signer certificate was revoked by embedded material",
    };
    let coverage = match report.coverage {
        Coverage::WholeFile => "covers the whole file",
        Coverage::ModifiedAfterSigning => "document was MODIFIED after signing (bytes appended)",
    };
    format!("{integrity}; {trust}; {coverage}")
}

struct CmsResult {
    validity: SignatureValidity,
    digest_algorithm: Option<String>,
    cms: CmsValidationReport,
    certificate: Option<CertInfo>,
    signer_resolution: SignerCertResolution,
    /// The signer certificate (raw), for trust-chain evaluation.
    signer_cert: Option<Certificate>,
    /// All certificates embedded in the CMS (the candidate chain).
    chain: Vec<Certificate>,
    timestamp_reports: Vec<TimestampValidationReport>,
    timestamp_token_count: usize,
    invalid_timestamp_token_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignerCertResolution {
    Found,
    Missing,
    Ambiguous,
}

struct SignerCertSelection {
    resolution: SignerCertResolution,
    cert: Option<Certificate>,
}

fn collect_chain(signed: &SignedData) -> Vec<Certificate> {
    signed
        .certificates
        .as_ref()
        .map(|certs| {
            certs
                .0
                .iter()
                .filter_map(|choice| match choice {
                    CertificateChoices::Certificate(cert) => Some(cert.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Verify every SignerInfo in a detached CMS SignedData blob over `content`
/// (the signed PDF byte ranges). Each signer is kept separate all the way to
/// the PDF report; a bad or ambiguous signer never causes an arbitrary
/// certificate fallback for another signer.
fn verify_cms(
    der: &[u8],
    content: &[u8],
    options: &VerifyOptions,
) -> std::result::Result<Vec<CmsResult>, String> {
    // PDF writers commonly reserve a larger /Contents slot and pad it with
    // zero bytes. Never remove zero bytes heuristically: a valid DER CMS
    // object can itself end in 0x00 (for example an RSA signature integer).
    // Determine the exact ContentInfo TLV length, then accept only zero
    // padding after that declared object.
    let der = exact_cms_der_object(der)?;

    let ci = ContentInfo::from_der(der).map_err(|e| format!("CMS parse: {e}"))?;
    if ci.content_type != OID_ID_SIGNED_DATA {
        return Err("CMS ContentInfo is not id-signedData".to_string());
    }
    let signed: SignedData = ci
        .content
        .decode_as()
        .map_err(|e| format!("SignedData decode: {e}"))?;
    if signed.encap_content_info.econtent_type != OID_ID_DATA {
        return Err("CMS detached content type is not id-data".to_string());
    }
    if signed.encap_content_info.econtent.is_some() {
        return Err("PDF CMS SignedData must be detached; eContent was present".to_string());
    }

    let signer_infos = signed.signer_infos.0.as_slice();
    if signer_infos.is_empty() {
        return Err("no SignerInfo in CMS".to_string());
    }
    let signer_info_count = signer_infos.len();
    signer_infos
        .iter()
        .map(|signer| verify_cms_signer(&signed, signer, content, signer_info_count, options))
        .collect()
}

fn verify_cms_signer(
    signed: &SignedData,
    signer: &SignerInfo,
    content: &[u8],
    signer_info_count: usize,
    options: &VerifyOptions,
) -> std::result::Result<CmsResult, String> {
    let algorithm_policy = &options.algorithm_policy;
    let digest_oid = signer.digest_alg.oid;
    let digest_name = digest_oid_name(&digest_oid);
    let timestamp_reports = cms_signature_timestamp_reports(
        signer,
        options,
        algorithm_policy,
        signer.signature.as_bytes(),
    );
    let timestamp_token_count = timestamp_reports
        .iter()
        .filter(|report| report.is_valid())
        .count();
    let invalid_timestamp_token_count = timestamp_reports
        .iter()
        .filter(|report| !report.is_valid())
        .count();
    let mut cms = CmsValidationReport {
        content_info: SignatureValidationState::Valid,
        detached_content: SignatureValidationState::Valid,
        signer_info_count,
        ..CmsValidationReport::default()
    };
    if !signed
        .digest_algorithms
        .iter()
        .any(|algorithm| algorithm.oid == digest_oid)
    {
        cms.digest_algorithm_declared = SignatureValidationState::Invalid;
        cms.status = SignatureValidationState::Invalid;
        return Ok(CmsResult {
            validity: SignatureValidity::Invalid,
            digest_algorithm: digest_name,
            cms,
            certificate: None,
            signer_resolution: SignerCertResolution::Missing,
            signer_cert: None,
            chain: collect_chain(signed),
            timestamp_reports: timestamp_reports.clone(),
            timestamp_token_count,
            invalid_timestamp_token_count,
        });
    }
    cms.digest_algorithm_declared = SignatureValidationState::Valid;

    // Find the signer certificate and collect the embedded chain.
    let selection = find_signer_cert(signed, signer);
    let cert = selection.cert;
    let cert_info = cert.as_ref().map(cert_to_info);
    let signer_cert = cert.clone();
    let chain = collect_chain(signed);

    if !algorithm_policy.allows_digest(&digest_oid) {
        cms.status = SignatureValidationState::UnsupportedAlgorithm;
        return Ok(CmsResult {
            validity: SignatureValidity::UnsupportedAlgorithm,
            digest_algorithm: digest_name,
            cms,
            certificate: cert_info,
            signer_resolution: selection.resolution,
            signer_cert,
            chain,
            timestamp_reports: timestamp_reports.clone(),
            timestamp_token_count,
            invalid_timestamp_token_count,
        });
    }

    // Compute the digest of the signed content.
    let content_digest = match digest_bytes(&digest_oid, content) {
        Some(d) => d,
        None => {
            cms.status = SignatureValidationState::UnsupportedAlgorithm;
            return Ok(CmsResult {
                validity: SignatureValidity::UnsupportedAlgorithm,
                digest_algorithm: digest_name,
                cms,
                certificate: cert_info,
                signer_resolution: selection.resolution,
                signer_cert: signer_cert.clone(),
                chain: chain.clone(),
                timestamp_reports: timestamp_reports.clone(),
                timestamp_token_count,
                invalid_timestamp_token_count,
            });
        }
    };

    // Determine what is actually signed:
    //  - with signed attributes: messageDigest attr must equal content_digest,
    //    and the signature is over DER(SET OF signed attributes);
    //  - without: the signature is over the content directly (its digest).
    let (_legacy_signed_payload_digest_input, attrs_ok) = match &signer.signed_attrs {
        Some(attrs) => {
            // messageDigest attribute check.
            let md_matches = signed_attr_message_digest(attrs)
                .map(|md| md == content_digest.as_slice())
                .unwrap_or(false);
            // The signature input is the DER re-encoding of the attributes as
            // an explicit SET OF (tag 0x31), per RFC 5652 §5.4.
            let der_attrs = match reencode_signed_attrs_as_set(attrs) {
                Some(b) => b,
                None => {
                    cms.signed_attributes = SignatureValidationState::Malformed;
                    cms.status = SignatureValidationState::Malformed;
                    return Ok(CmsResult {
                        validity: SignatureValidity::Error,
                        digest_algorithm: digest_name,
                        cms,
                        certificate: cert_info,
                        signer_resolution: selection.resolution,
                        signer_cert,
                        chain,
                        timestamp_reports: timestamp_reports.clone(),
                        timestamp_token_count,
                        invalid_timestamp_token_count,
                    });
                }
            };
            (der_attrs, md_matches)
        }
        None => {
            // No signed attrs: signature is over the content's digest directly.
            (content.to_vec(), true)
        }
    };

    if !attrs_ok {
        // messageDigest mismatch ⇒ the signed content doesn't match the bytes.
        cms.status = SignatureValidationState::Invalid;
        return Ok(CmsResult {
            validity: SignatureValidity::Invalid,
            digest_algorithm: digest_name,
            cms,
            certificate: cert_info,
            signer_resolution: selection.resolution,
            signer_cert: signer_cert.clone(),
            chain: chain.clone(),
            timestamp_reports: timestamp_reports.clone(),
            timestamp_token_count,
            invalid_timestamp_token_count,
        });
    }

    let signed_payload_digest_input = match &signer.signed_attrs {
        Some(attrs) => match validate_signed_attributes(
            attrs,
            &content_digest,
            &signed.encap_content_info.econtent_type,
            signer,
            cert.as_ref(),
            &mut cms,
        ) {
            Ok(payload) => payload,
            Err(_) => {
                cms.status = SignatureValidationState::Invalid;
                return Ok(CmsResult {
                    validity: SignatureValidity::Invalid,
                    digest_algorithm: digest_name,
                    cms,
                    certificate: cert_info,
                    signer_resolution: selection.resolution,
                    signer_cert: signer_cert.clone(),
                    chain: chain.clone(),
                    timestamp_reports: timestamp_reports.clone(),
                    timestamp_token_count,
                    invalid_timestamp_token_count,
                });
            }
        },
        None => {
            // Generic detached CMS may omit signedAttrs, but it cannot satisfy
            // PAdES baseline signed-attribute requirements.
            cms.signed_attributes = SignatureValidationState::EvidenceMissing;
            cms.content_type_attribute = SignatureValidationState::EvidenceMissing;
            cms.message_digest_attribute = SignatureValidationState::EvidenceMissing;
            cms.signing_certificate_reference = SignatureValidationState::EvidenceMissing;
            cms.cms_algorithm_protection = SignatureValidationState::NotChecked;
            content.to_vec()
        }
    };

    // Verify the signature over the exact CMS input using the certificate key.
    let Some(cert) = cert else {
        cms.status = match selection.resolution {
            SignerCertResolution::Ambiguous => SignatureValidationState::SignerCertificateAmbiguous,
            SignerCertResolution::Missing | SignerCertResolution::Found => {
                SignatureValidationState::SignerCertificateMissing
            }
        };
        return Ok(CmsResult {
            validity: SignatureValidity::Error,
            digest_algorithm: digest_name,
            cms,
            certificate: cert_info,
            signer_resolution: selection.resolution,
            signer_cert,
            chain,
            timestamp_reports: timestamp_reports.clone(),
            timestamp_token_count,
            invalid_timestamp_token_count,
        });
    };

    let validity = match verify_signature_algorithm(
        &cert,
        &digest_oid,
        signer,
        &signed_payload_digest_input,
        algorithm_policy,
    ) {
        Ok(true) => SignatureValidity::Valid,
        Ok(false) => SignatureValidity::Invalid,
        Err(error) if error.starts_with("unsupported algorithm:") => {
            SignatureValidity::UnsupportedAlgorithm
        }
        Err(_) => SignatureValidity::Invalid,
    };

    Ok(CmsResult {
        validity: validity.clone(),
        digest_algorithm: digest_name,
        cms: CmsValidationReport {
            status: match validity {
                SignatureValidity::Valid => SignatureValidationState::Valid,
                SignatureValidity::Invalid => SignatureValidationState::Invalid,
                SignatureValidity::UnsupportedAlgorithm => {
                    SignatureValidationState::UnsupportedAlgorithm
                }
                SignatureValidity::Error => SignatureValidationState::Malformed,
            },
            ..cms
        },
        certificate: cert_info,
        signer_resolution: selection.resolution,
        signer_cert,
        chain,
        timestamp_reports,
        timestamp_token_count,
        invalid_timestamp_token_count,
    })
}

// RSA-with-hash signature algorithm OIDs (some PDFs name these instead of plain rsaEncryption).
const OID_SHA1_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const OID_SHA256_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_SHA384_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const OID_SHA512_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const OID_SECP256R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
const OID_SECP384R1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");

fn verify_signature_algorithm(
    cert: &Certificate,
    digest_oid: &ObjectIdentifier,
    signer: &SignerInfo,
    payload: &[u8],
    policy: &SignatureAlgorithmPolicy,
) -> std::result::Result<bool, String> {
    let signature_oid = signer.signature_algorithm.oid;
    let signature = signer.signature.as_bytes();
    match signature_oid {
        OID_RSA_ENCRYPTION | OID_SHA1_RSA | OID_SHA256_RSA | OID_SHA384_RSA | OID_SHA512_RSA => {
            if !policy.allow_rsa_pkcs1v15 {
                return Err(
                    "unsupported algorithm: RSA PKCS#1 v1.5 is forbidden by policy".to_string(),
                );
            }
            if cert.tbs_certificate.subject_public_key_info.algorithm.oid != OID_RSA_ENCRYPTION {
                return Err(
                    "signature algorithm is incompatible with signer public key".to_string()
                );
            }
            if !algorithm_parameters_absent_or_null(&signer.signature_algorithm) {
                return Err(
                    "RSA PKCS#1 v1.5 signatureAlgorithm parameters are malformed".to_string(),
                );
            }
            if let Some(implied_digest) = rsa_signature_digest(signature_oid) {
                if implied_digest != *digest_oid {
                    return Err(
                        "RSA signature algorithm hash does not match SignerInfo digestAlgorithm"
                            .to_string(),
                    );
                }
            }
            verify_rsa(cert, digest_oid, payload, signature, policy)
        }
        OID_RSA_PSS => {
            if !policy.allow_rsa_pss {
                return Err("unsupported algorithm: RSA-PSS is forbidden by policy".to_string());
            }
            verify_rsa_pss(
                cert,
                digest_oid,
                &signer.signature_algorithm,
                payload,
                signature,
                policy,
            )
        }
        OID_ECDSA_SHA256 | OID_ECDSA_SHA384 | OID_ECDSA_SHA512 => verify_ecdsa(
            cert,
            digest_oid,
            &signer.signature_algorithm,
            payload,
            signature,
            policy,
        ),
        _ => Err(format!(
            "unsupported algorithm: CMS signature OID {signature_oid}"
        )),
    }
}

fn rsa_signature_digest(signature_oid: ObjectIdentifier) -> Option<ObjectIdentifier> {
    match signature_oid {
        OID_SHA1_RSA => Some(OID_SHA1),
        OID_SHA256_RSA => Some(OID_SHA256),
        OID_SHA384_RSA => Some(OID_SHA384),
        OID_SHA512_RSA => Some(OID_SHA512),
        OID_RSA_ENCRYPTION => None,
        _ => None,
    }
}

fn algorithm_parameters_absent_or_null(algorithm: &AlgorithmIdentifierOwned) -> bool {
    algorithm.parameters.as_ref().is_none_or(|parameters| {
        parameters
            .to_der()
            .is_ok_and(|encoded| encoded.as_slice() == [0x05, 0x00])
    })
}

fn verify_rsa_pss(
    cert: &Certificate,
    digest_oid: &ObjectIdentifier,
    algorithm: &AlgorithmIdentifierOwned,
    payload: &[u8],
    signature: &[u8],
    policy: &SignatureAlgorithmPolicy,
) -> std::result::Result<bool, String> {
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPublicKey;

    if cert.tbs_certificate.subject_public_key_info.algorithm.oid != OID_RSA_ENCRYPTION
        && cert.tbs_certificate.subject_public_key_info.algorithm.oid != OID_RSA_PSS
    {
        return Err("RSA-PSS signature is incompatible with signer public key".to_string());
    }
    let (pss_hash, mgf_hash, salt_len) = match &algorithm.parameters {
        Some(parameters) => {
            let encoded = parameters
                .to_der()
                .map_err(|error| format!("RSA-PSS parameters DER: {error}"))?;
            let params = rsa::pkcs1::RsaPssParams::try_from(encoded.as_slice())
                .map_err(|error| format!("RSA-PSS parameters: {error}"))?;
            if params.mask_gen.oid != OID_MGF1 {
                return Err("RSA-PSS mask generation algorithm is not MGF1".to_string());
            }
            let mgf_hash = params
                .mask_gen
                .parameters
                .as_ref()
                .ok_or_else(|| "RSA-PSS MGF1 hash parameters are absent".to_string())?
                .oid;
            (params.hash.oid, mgf_hash, params.salt_len as usize)
        }
        // The RFC 8017 default parameter set is SHA-1/MGF1-SHA-1/saltLen=20.
        None => (OID_SHA1, OID_SHA1, 20),
    };
    if pss_hash != *digest_oid || mgf_hash != *digest_oid {
        return Err(
            "RSA-PSS hash or MGF1 hash does not match SignerInfo digestAlgorithm".to_string(),
        );
    }

    let spki = &cert.tbs_certificate.subject_public_key_info;
    let spki_der = spki
        .to_der()
        .map_err(|error| format!("spki encode: {error}"))?;
    let pubkey = RsaPublicKey::try_from(
        spki::SubjectPublicKeyInfoRef::try_from(spki_der.as_slice())
            .map_err(|error| format!("spki parse: {error}"))?,
    )
    .map_err(|error| format!("rsa key: {error}"))?;
    if pubkey.n().bits() < usize::from(policy.min_rsa_key_bits) {
        return Err("RSA-PSS signer key is smaller than the configured minimum".to_string());
    }
    let digest_len = digest_output_len(digest_oid)
        .ok_or_else(|| "unsupported algorithm: RSA-PSS digest".to_string())?;
    let max_salt_len = pubkey.size().saturating_sub(digest_len.saturating_add(2));
    if salt_len > max_salt_len {
        return Err("RSA-PSS salt length exceeds the signer key encoding capacity".to_string());
    }
    let verified = match *digest_oid {
        OID_SHA1 => pubkey
            .verify(
                rsa::Pss::new_with_salt::<Sha1>(salt_len),
                &Sha1::digest(payload),
                signature,
            )
            .is_ok(),
        OID_SHA256 => pubkey
            .verify(
                rsa::Pss::new_with_salt::<Sha256>(salt_len),
                &Sha256::digest(payload),
                signature,
            )
            .is_ok(),
        OID_SHA384 => pubkey
            .verify(
                rsa::Pss::new_with_salt::<Sha384>(salt_len),
                &Sha384::digest(payload),
                signature,
            )
            .is_ok(),
        OID_SHA512 => pubkey
            .verify(
                rsa::Pss::new_with_salt::<Sha512>(salt_len),
                &Sha512::digest(payload),
                signature,
            )
            .is_ok(),
        _ => return Err("unsupported algorithm: RSA-PSS digest".to_string()),
    };
    Ok(verified)
}

fn verify_ecdsa(
    cert: &Certificate,
    digest_oid: &ObjectIdentifier,
    algorithm: &AlgorithmIdentifierOwned,
    payload: &[u8],
    signature: &[u8],
    policy: &SignatureAlgorithmPolicy,
) -> std::result::Result<bool, String> {
    if cert.tbs_certificate.subject_public_key_info.algorithm.oid != OID_EC_PUBLIC_KEY {
        return Err("ECDSA signature is incompatible with signer public key".to_string());
    }
    if algorithm.parameters.is_some() {
        return Err("ECDSA signatureAlgorithm parameters must be absent".to_string());
    }
    let curve = cert
        .tbs_certificate
        .subject_public_key_info
        .algorithm
        .parameters
        .as_ref()
        .ok_or_else(|| "EC public key is missing a named-curve parameter".to_string())?
        .to_der()
        .map_err(|error| format!("EC named-curve DER: {error}"))?;
    let curve = ObjectIdentifier::from_der(&curve)
        .map_err(|error| format!("EC named-curve parameter: {error}"))?;
    let spki_der = cert
        .tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|error| format!("spki encode: {error}"))?;

    match (algorithm.oid, *digest_oid, curve) {
        (OID_ECDSA_SHA256, OID_SHA256, OID_SECP256R1) => {
            if !policy.allow_ecdsa_p256 {
                return Err("unsupported algorithm: ECDSA P-256 is forbidden by policy".to_string());
            }
            use p256::ecdsa::signature::Verifier as _;
            use p256::pkcs8::DecodePublicKey as _;
            let key = p256::ecdsa::VerifyingKey::from_public_key_der(&spki_der)
                .map_err(|error| format!("P-256 public key: {error}"))?;
            let signature = p256::ecdsa::DerSignature::from_der(signature)
                .map_err(|error| format!("P-256 ECDSA DER signature: {error}"))?;
            Ok(key.verify(payload, &signature).is_ok())
        }
        (OID_ECDSA_SHA384, OID_SHA384, OID_SECP384R1) => {
            if !policy.allow_ecdsa_p384 {
                return Err("unsupported algorithm: ECDSA P-384 is forbidden by policy".to_string());
            }
            use p384::ecdsa::signature::Verifier as _;
            use p384::pkcs8::DecodePublicKey as _;
            let key = p384::ecdsa::VerifyingKey::from_public_key_der(&spki_der)
                .map_err(|error| format!("P-384 public key: {error}"))?;
            let signature = p384::ecdsa::DerSignature::from_der(signature)
                .map_err(|error| format!("P-384 ECDSA DER signature: {error}"))?;
            Ok(key.verify(payload, &signature).is_ok())
        }
        (OID_ECDSA_SHA512, OID_SHA512, _) => {
            Err("unsupported algorithm: ECDSA-SHA-512 requires an unsupported named curve".to_string())
        }
        (signature_oid, digest_oid, curve_oid) => Err(format!(
            "ECDSA signature algorithm, digest, and named curve are incompatible ({signature_oid}, {digest_oid}, {curve_oid})"
        )),
    }
}

fn digest_output_len(oid: &ObjectIdentifier) -> Option<usize> {
    Some(match *oid {
        OID_SHA1 => 20,
        OID_SHA256 => 32,
        OID_SHA384 => 48,
        OID_SHA512 => 64,
        _ => return None,
    })
}

/// Verify an RSA PKCS#1 v1.5 signature: `RSA_verify(pubkey, H(payload), sig)`.
fn verify_rsa(
    cert: &Certificate,
    digest_oid: &ObjectIdentifier,
    payload: &[u8],
    signature: &[u8],
    policy: &SignatureAlgorithmPolicy,
) -> std::result::Result<bool, String> {
    use rsa::pkcs1v15::Pkcs1v15Sign;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPublicKey;

    let spki = &cert.tbs_certificate.subject_public_key_info;
    let spki_der = spki.to_der().map_err(|e| format!("spki encode: {e}"))?;
    let pubkey = RsaPublicKey::try_from(
        spki::SubjectPublicKeyInfoRef::try_from(spki_der.as_slice())
            .map_err(|e| format!("spki parse: {e}"))?,
    )
    .map_err(|e| format!("rsa key: {e}"))?;
    if pubkey.n().bits() < usize::from(policy.min_rsa_key_bits) {
        return Err("RSA signer key is smaller than the configured minimum".to_string());
    }

    // The signature is over H(payload); pick the scheme matching the digest OID
    // (it prepends the correct DigestInfo prefix internally).
    let ok = match *digest_oid {
        OID_SHA256 => {
            let h = Sha256::digest(payload);
            pubkey
                .verify(Pkcs1v15Sign::new::<Sha256>(), &h, signature)
                .is_ok()
        }
        OID_SHA384 => {
            let h = Sha384::digest(payload);
            pubkey
                .verify(Pkcs1v15Sign::new::<Sha384>(), &h, signature)
                .is_ok()
        }
        OID_SHA512 => {
            let h = Sha512::digest(payload);
            pubkey
                .verify(Pkcs1v15Sign::new::<Sha512>(), &h, signature)
                .is_ok()
        }
        OID_SHA1 => {
            let h = Sha1::digest(payload);
            pubkey
                .verify(Pkcs1v15Sign::new::<Sha1>(), &h, signature)
                .is_ok()
        }
        _ => return Err("unsupported digest".to_string()),
    };
    Ok(ok)
}

fn digest_bytes(oid: &ObjectIdentifier, data: &[u8]) -> Option<Vec<u8>> {
    Some(match *oid {
        OID_SHA256 => Sha256::digest(data).to_vec(),
        OID_SHA384 => Sha384::digest(data).to_vec(),
        OID_SHA512 => Sha512::digest(data).to_vec(),
        OID_SHA1 => Sha1::digest(data).to_vec(),
        _ => return None,
    })
}

fn digest_oid_name(oid: &ObjectIdentifier) -> Option<String> {
    Some(
        match *oid {
            OID_SHA256 => "SHA-256",
            OID_SHA384 => "SHA-384",
            OID_SHA512 => "SHA-512",
            OID_SHA1 => "SHA-1",
            _ => return None,
        }
        .to_string(),
    )
}

/// RFC 6211 CMSAlgorithmProtection. When a signer includes this attribute,
/// it must bind the same digest and signature algorithms that appear in the
/// enclosing SignerInfo; otherwise it is an algorithm-confusion attempt.
#[derive(Clone, Debug, Sequence)]
struct CmsAlgorithmProtection {
    digest_algorithm: AlgorithmIdentifierOwned,
    #[asn1(context_specific = "1", tag_mode = "IMPLICIT", optional = "true")]
    signature_algorithm: Option<AlgorithmIdentifierOwned>,
}

fn validate_signed_attributes(
    attrs: &x509_cert::attr::Attributes,
    content_digest: &[u8],
    expected_content_type: &ObjectIdentifier,
    signer: &SignerInfo,
    signer_certificate: Option<&Certificate>,
    cms: &mut CmsValidationReport,
) -> std::result::Result<Vec<u8>, String> {
    const MAX_SIGNED_ATTRIBUTES: usize = 64;
    if attrs.iter().count() > MAX_SIGNED_ATTRIBUTES {
        cms.signed_attributes = SignatureValidationState::Malformed;
        return Err("CMS signed attribute count exceeds the configured limit".to_string());
    }

    let content_type = match exactly_one_signed_attribute(attrs, OID_CONTENT_TYPE, "contentType") {
        Ok(attribute) => attribute,
        Err(error) => {
            cms.signed_attributes = SignatureValidationState::Invalid;
            cms.content_type_attribute = SignatureValidationState::Invalid;
            return Err(error);
        }
    };
    let content_type_der = content_type.values.as_slice()[0]
        .to_der()
        .map_err(|error| format!("CMS contentType DER: {error}"))?;
    let content_type_value = ObjectIdentifier::from_der(&content_type_der)
        .map_err(|error| format!("CMS contentType value: {error}"))?;
    if content_type_value != *expected_content_type {
        cms.signed_attributes = SignatureValidationState::Invalid;
        cms.content_type_attribute = SignatureValidationState::Invalid;
        return Err("CMS contentType signed attribute does not match detached content".to_string());
    }
    cms.content_type_attribute = SignatureValidationState::Valid;

    let message_digest =
        match exactly_one_signed_attribute(attrs, OID_MESSAGE_DIGEST, "messageDigest") {
            Ok(attribute) => attribute,
            Err(error) => {
                cms.signed_attributes = SignatureValidationState::Invalid;
                cms.message_digest_attribute = SignatureValidationState::Invalid;
                return Err(error);
            }
        };
    let message_digest_der = message_digest.values.as_slice()[0]
        .to_der()
        .map_err(|error| format!("CMS messageDigest DER: {error}"))?;
    let message_digest_value = der::asn1::OctetString::from_der(&message_digest_der)
        .map_err(|error| format!("CMS messageDigest value: {error}"))?;
    if message_digest_value.as_bytes() != content_digest {
        cms.signed_attributes = SignatureValidationState::Invalid;
        cms.message_digest_attribute = SignatureValidationState::DigestMismatch;
        return Err(
            "CMS messageDigest signed attribute does not match PDF ByteRange digest".to_string(),
        );
    }
    cms.message_digest_attribute = SignatureValidationState::Valid;

    let signing_certificate_attributes = attrs
        .iter()
        .filter(|attribute| {
            attribute.oid == OID_SIGNING_CERTIFICATE || attribute.oid == OID_SIGNING_CERTIFICATE_V2
        })
        .collect::<Vec<_>>();
    cms.signing_certificate_reference = match signing_certificate_attributes.len() {
        0 => SignatureValidationState::EvidenceMissing,
        1 if signing_certificate_attributes[0].values.as_slice().len() == 1 => {
            match signer_certificate {
                Some(certificate) => match validate_ess_signing_certificate(
                    signing_certificate_attributes[0],
                    certificate,
                ) {
                    Ok(()) => SignatureValidationState::Valid,
                    Err(error) if error.starts_with("unsupported algorithm:") => {
                        SignatureValidationState::UnsupportedAlgorithm
                    }
                    Err(_) => SignatureValidationState::Invalid,
                },
                None => SignatureValidationState::EvidenceMissing,
            }
        }
        _ => {
            cms.signed_attributes = SignatureValidationState::Invalid;
            return Err("CMS SigningCertificate attribute is duplicate or malformed".to_string());
        }
    };

    let algorithm_protection = attrs
        .iter()
        .filter(|attribute| attribute.oid == OID_CMS_ALGORITHM_PROTECTION)
        .collect::<Vec<_>>();
    cms.cms_algorithm_protection = match algorithm_protection.len() {
        0 => SignatureValidationState::NotChecked,
        1 if algorithm_protection[0].values.as_slice().len() == 1 => {
            let encoded = algorithm_protection[0].values.as_slice()[0]
                .to_der()
                .map_err(|error| format!("CMSAlgorithmProtection DER: {error}"))?;
            let protected = CmsAlgorithmProtection::from_der(&encoded)
                .map_err(|error| format!("CMSAlgorithmProtection value: {error}"))?;
            if protected.digest_algorithm.oid != signer.digest_alg.oid
                || protected
                    .signature_algorithm
                    .as_ref()
                    .is_some_and(|algorithm| algorithm.oid != signer.signature_algorithm.oid)
            {
                cms.signed_attributes = SignatureValidationState::Invalid;
                return Err(
                    "CMSAlgorithmProtection does not match SignerInfo algorithms".to_string(),
                );
            }
            SignatureValidationState::Valid
        }
        _ => {
            cms.signed_attributes = SignatureValidationState::Invalid;
            return Err("CMSAlgorithmProtection attribute is duplicate or malformed".to_string());
        }
    };

    cms.signed_attributes = SignatureValidationState::Valid;
    reencode_signed_attrs_as_set(attrs)
        .ok_or_else(|| "CMS signed attributes are not canonical DER SET encoding".to_string())
}

fn exactly_one_signed_attribute<'a>(
    attrs: &'a x509_cert::attr::Attributes,
    oid: ObjectIdentifier,
    name: &str,
) -> std::result::Result<&'a Attribute, String> {
    let matches = attrs
        .iter()
        .filter(|attribute| attribute.oid == oid)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "CMS {name} signed attribute must occur exactly once, found {}",
            matches.len()
        ));
    }
    if matches[0].values.as_slice().len() != 1 {
        return Err(format!(
            "CMS {name} signed attribute must contain exactly one value"
        ));
    }
    Ok(matches[0])
}

/// Validate the ESS signing-certificate reference against the exact DER
/// certificate selected from the SignerIdentifier. This intentionally does not
/// fall back to subject-name comparisons or a different CMS certificate.
fn validate_ess_signing_certificate(
    attribute: &Attribute,
    signer_certificate: &Certificate,
) -> std::result::Result<(), String> {
    let value = attribute.values.as_slice()[0]
        .to_der()
        .map_err(|error| format!("ESS signing-certificate DER: {error}"))?;
    let outer = der_sequence_children(&value, 2, "ESS SigningCertificate")?;
    let certs = outer
        .first()
        .ok_or_else(|| "ESS SigningCertificate is missing certs".to_string())?;
    if certs.tag != 0x30 {
        return Err("ESS SigningCertificate certs is not a DER SEQUENCE".to_string());
    }
    let cert_ids = der_children(certs.value, 16, "ESS certs")?;
    if cert_ids.is_empty() {
        return Err("ESS SigningCertificate certs is empty".to_string());
    }
    let certificate_der = signer_certificate
        .to_der()
        .map_err(|error| format!("signer certificate DER: {error}"))?;
    let version_two = attribute.oid == OID_SIGNING_CERTIFICATE_V2;
    let matches = cert_ids
        .iter()
        .map(|cert_id| {
            ess_cert_id_matches(cert_id, version_two, &certificate_der, signer_certificate)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if matches.iter().filter(|matched| **matched).count() != 1 {
        return Err(
            "ESS signing-certificate reference does not identify exactly one signer certificate"
                .to_string(),
        );
    }
    Ok(())
}

fn ess_cert_id_matches(
    cert_id: &DerTlv<'_>,
    version_two: bool,
    certificate_der: &[u8],
    signer_certificate: &Certificate,
) -> std::result::Result<bool, String> {
    if cert_id.tag != 0x30 {
        return Err("ESSCertID is not a DER SEQUENCE".to_string());
    }
    let fields = der_children(cert_id.value, 3, "ESSCertID")?;
    let (hash_oid, hash_index) =
        if version_two && fields.first().is_some_and(|field| field.tag == 0x30) {
            let algorithm = AlgorithmIdentifierOwned::from_der(fields[0].encoded)
                .map_err(|error| format!("ESSCertIDv2 hashAlgorithm: {error}"))?;
            (algorithm.oid, 1)
        } else if version_two {
            (OID_SHA256, 0)
        } else {
            (OID_SHA1, 0)
        };
    let hash = fields
        .get(hash_index)
        .ok_or_else(|| "ESSCertID is missing certHash".to_string())?;
    if hash.tag != 0x04 {
        return Err("ESSCertID certHash is not an OCTET STRING".to_string());
    }
    use subtle::ConstantTimeEq;
    let expected_hash = digest_bytes(&hash_oid, certificate_der)
        .ok_or_else(|| format!("unsupported algorithm: ESS certificate hash {hash_oid}"))?;
    let hash_matches = hash.value.ct_eq(expected_hash.as_slice()).unwrap_u8() == 1;
    let issuer_serial = fields.get(hash_index + 1);
    if fields.len() > hash_index + 2 {
        return Err("ESSCertID has trailing fields".to_string());
    }
    let issuer_serial_matches = match issuer_serial {
        Some(issuer_serial) => ess_issuer_serial_matches(issuer_serial, signer_certificate)?,
        None => true,
    };
    Ok(hash_matches && issuer_serial_matches)
}

fn ess_issuer_serial_matches(
    issuer_serial: &DerTlv<'_>,
    signer_certificate: &Certificate,
) -> std::result::Result<bool, String> {
    if issuer_serial.tag != 0x30 {
        return Err("ESS issuerSerial is not a DER SEQUENCE".to_string());
    }
    let fields = der_children(issuer_serial.value, 2, "ESS issuerSerial")?;
    if fields.len() != 2 || fields[0].tag != 0x30 || fields[1].tag != 0x02 {
        return Err("ESS issuerSerial is malformed".to_string());
    }
    let expected_issuer = signer_certificate
        .tbs_certificate
        .issuer
        .to_der()
        .map_err(|error| format!("signer issuer DER: {error}"))?;
    let issuer_names = der_children(fields[0].value, 16, "ESS issuer GeneralNames")?;
    let issuer_matches = issuer_names.iter().any(|name| {
        // directoryName is [4] EXPLICIT Name. Other GeneralName variants do
        // not provide a safe substitute for an X.509 issuer Name comparison.
        name.tag == 0xa4
            && der_children(name.value, 1, "ESS directoryName")
                .ok()
                .is_some_and(|children| {
                    children.len() == 1 && children[0].encoded == expected_issuer
                })
    });
    Ok(issuer_matches
        && fields[1].value == signer_certificate.tbs_certificate.serial_number.as_bytes())
}

#[derive(Clone, Copy)]
struct DerTlv<'a> {
    tag: u8,
    value: &'a [u8],
    encoded: &'a [u8],
}

fn der_sequence_children<'a>(
    input: &'a [u8],
    max_children: usize,
    context: &str,
) -> std::result::Result<Vec<DerTlv<'a>>, String> {
    let (outer, rest) = der_take_tlv(input, context)?;
    if outer.tag != 0x30 || !rest.is_empty() {
        return Err(format!("{context} is not one complete DER SEQUENCE"));
    }
    der_children(outer.value, max_children, context)
}

fn der_children<'a>(
    mut input: &'a [u8],
    max_children: usize,
    context: &str,
) -> std::result::Result<Vec<DerTlv<'a>>, String> {
    let mut children = Vec::new();
    while !input.is_empty() {
        if children.len() == max_children {
            return Err(format!("{context} exceeds the configured DER child limit"));
        }
        let (child, rest) = der_take_tlv(input, context)?;
        children.push(child);
        input = rest;
    }
    Ok(children)
}

fn der_take_tlv<'a>(
    input: &'a [u8],
    context: &str,
) -> std::result::Result<(DerTlv<'a>, &'a [u8]), String> {
    if input.len() < 2 {
        return Err(format!("{context} is truncated"));
    }
    let tag = input[0];
    if tag & 0x1f == 0x1f {
        return Err(format!(
            "{context} uses an unsupported high-tag-number form"
        ));
    }
    let first_length = input[1];
    let (length, length_bytes) = if first_length < 0x80 {
        (first_length as usize, 1)
    } else {
        let count = (first_length & 0x7f) as usize;
        if count == 0 || count > std::mem::size_of::<usize>() || input.len() < 2 + count {
            return Err(format!("{context} has an invalid DER length"));
        }
        if input[2] == 0 {
            return Err(format!("{context} has a non-canonical DER length"));
        }
        let mut length = 0usize;
        for byte in &input[2..2 + count] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(*byte as usize))
                .ok_or_else(|| format!("{context} length overflows"))?;
        }
        if length < 0x80 {
            return Err(format!("{context} has a non-canonical DER long length"));
        }
        (length, 1 + count)
    };
    let header_len = 1usize
        .checked_add(length_bytes)
        .ok_or_else(|| format!("{context} header length overflows"))?;
    let total_len = header_len
        .checked_add(length)
        .ok_or_else(|| format!("{context} value length overflows"))?;
    if total_len > input.len() {
        return Err(format!("{context} value is truncated"));
    }
    Ok((
        DerTlv {
            tag,
            value: &input[header_len..total_len],
            encoded: &input[..total_len],
        },
        &input[total_len..],
    ))
}

/// Extract the `messageDigest` signed-attribute value (an OCTET STRING).
fn signed_attr_message_digest(attrs: &x509_cert::attr::Attributes) -> Option<Vec<u8>> {
    for attr in attrs.iter() {
        if attr.oid == OID_MESSAGE_DIGEST {
            let any = attr.values.as_slice().first()?;
            // The value is an OCTET STRING; its inner bytes are the digest.
            let octets = der::asn1::OctetString::from_der(&any.to_der().ok()?).ok()?;
            return Some(octets.as_bytes().to_vec());
        }
    }
    None
}

#[derive(Debug, Clone)]
struct ParsedTstInfo {
    policy_oid: ObjectIdentifier,
    serial_hex: String,
    gen_time_unix: u64,
    gen_time: String,
    hash_algorithm: AlgorithmIdentifierOwned,
    message_imprint: Vec<u8>,
}

fn cms_signature_timestamp_reports(
    signer: &SignerInfo,
    options: &VerifyOptions,
    algorithm_policy: &SignatureAlgorithmPolicy,
    signature_value: &[u8],
) -> Vec<TimestampValidationReport> {
    let mut reports = Vec::new();
    let Some(attrs) = &signer.unsigned_attrs else {
        return reports;
    };
    let mut attribute_index = 0usize;
    for attr in attrs.iter() {
        if attr.oid != OID_SIGNATURE_TIMESTAMP_TOKEN {
            continue;
        }
        attribute_index += 1;
        for value in attr.values.iter() {
            let Ok(der) = value.to_der() else {
                let mut report = TimestampValidationReport::new(
                    TimestampTokenType::SignatureTimestamp,
                    format!("cms_unsigned_attribute:{attribute_index}"),
                    &[],
                );
                report.status = SignatureValidationState::Malformed;
                report.errors.push(
                    "timestamp-token unsigned attribute value was not DER encodable".to_string(),
                );
                reports.push(report);
                continue;
            };
            reports.push(validate_signature_timestamp_token(
                &der,
                format!("cms_unsigned_attribute:{attribute_index}"),
                signature_value,
                options,
                algorithm_policy,
            ));
        }
    }
    reports
}

fn validate_signature_timestamp_token(
    token_der: &[u8],
    location: String,
    signature_value: &[u8],
    options: &VerifyOptions,
    algorithm_policy: &SignatureAlgorithmPolicy,
) -> TimestampValidationReport {
    let mut report =
        TimestampValidationReport::new(TimestampTokenType::SignatureTimestamp, location, token_der);
    let ci = match ContentInfo::from_der(token_der) {
        Ok(ci) => {
            report.content_info_status = SignatureValidationState::Valid;
            ci
        }
        Err(error) => {
            report.content_info_status = SignatureValidationState::Malformed;
            report.status = SignatureValidationState::Malformed;
            report
                .errors
                .push(format!("timestamp ContentInfo parse failed: {error}"));
            return report;
        }
    };
    if ci.content_type != OID_ID_SIGNED_DATA {
        report.content_info_status = SignatureValidationState::UnsupportedProfile;
        report.status = SignatureValidationState::UnsupportedProfile;
        report.errors.push(format!(
            "timestamp ContentInfo type was {}, expected id-signedData",
            ci.content_type
        ));
        return report;
    }
    let signed: SignedData = match ci.content.decode_as() {
        Ok(signed) => {
            report.signed_data_status = SignatureValidationState::Valid;
            signed
        }
        Err(error) => {
            report.signed_data_status = SignatureValidationState::Malformed;
            report.status = SignatureValidationState::Malformed;
            report
                .errors
                .push(format!("timestamp SignedData decode failed: {error}"));
            return report;
        }
    };
    if signed.encap_content_info.econtent_type != OID_ID_CT_TST_INFO {
        report.signed_data_status = SignatureValidationState::UnsupportedProfile;
        report.status = SignatureValidationState::UnsupportedProfile;
        report.errors.push(format!(
            "timestamp encapsulated content type was {}, expected id-ct-TSTInfo",
            signed.encap_content_info.econtent_type
        ));
        return report;
    }
    let Some(tst_info_any) = signed.encap_content_info.econtent.as_ref() else {
        report.signed_data_status = SignatureValidationState::EvidenceMissing;
        report.status = SignatureValidationState::Malformed;
        report
            .errors
            .push("timestamp SignedData did not embed TSTInfo".to_string());
        return report;
    };
    let tst_info_der = tst_info_any.value();
    let tst_info = match parse_tst_info(tst_info_der) {
        Ok(tst_info) => {
            report.tst_info_status = SignatureValidationState::Valid;
            report.policy_oid = Some(tst_info.policy_oid.to_string());
            report.serial_hex = Some(tst_info.serial_hex.clone());
            report.gen_time_unix = Some(tst_info.gen_time_unix);
            report.gen_time = Some(tst_info.gen_time.clone());
            report.hash_algorithm = digest_oid_name(&tst_info.hash_algorithm.oid);
            report.message_imprint_digest_hex = Some(hex_upper(&tst_info.message_imprint));
            tst_info
        }
        Err(error) => {
            report.tst_info_status = SignatureValidationState::Malformed;
            report.status = SignatureValidationState::Malformed;
            report.errors.push(error);
            return report;
        }
    };
    match digest_bytes(&tst_info.hash_algorithm.oid, signature_value) {
        Some(expected) => {
            report.expected_imprint_digest_hex = Some(hex_upper(&expected));
            if expected == tst_info.message_imprint {
                report.message_imprint_status = SignatureValidationState::Valid;
            } else {
                report.message_imprint_status = SignatureValidationState::DigestMismatch;
                report.status = SignatureValidationState::DigestMismatch;
                report.errors.push(
                    "timestamp messageImprint does not match the CMS signature value".to_string(),
                );
                return report;
            }
        }
        None => {
            report.message_imprint_status = SignatureValidationState::UnsupportedAlgorithm;
            report.status = SignatureValidationState::UnsupportedAlgorithm;
            report.errors.push(format!(
                "timestamp messageImprint hash algorithm {} is unsupported",
                tst_info.hash_algorithm.oid
            ));
            return report;
        }
    }

    let signer_infos = signed.signer_infos.0.as_slice();
    if signer_infos.len() != 1 {
        report.cms_signature_status = SignatureValidationState::SignerCertificateAmbiguous;
        report.status = SignatureValidationState::SignerCertificateAmbiguous;
        report.errors.push(format!(
            "timestamp token must contain exactly one SignerInfo, found {}",
            signer_infos.len()
        ));
        return report;
    }
    let token_signer = &signer_infos[0];
    let mut cms = CmsValidationReport {
        content_info: SignatureValidationState::Valid,
        detached_content: SignatureValidationState::Valid,
        signer_info_count: 1,
        ..CmsValidationReport::default()
    };
    if !signed
        .digest_algorithms
        .iter()
        .any(|algorithm| algorithm.oid == token_signer.digest_alg.oid)
    {
        report.cms_signature_status = SignatureValidationState::Invalid;
        report.status = SignatureValidationState::Invalid;
        report
            .errors
            .push("timestamp SignerInfo digestAlgorithm was absent from SignedData".to_string());
        return report;
    }
    cms.digest_algorithm_declared = SignatureValidationState::Valid;
    if !algorithm_policy.allows_digest(&token_signer.digest_alg.oid) {
        report.cms_signature_status = SignatureValidationState::UnsupportedAlgorithm;
        report.status = SignatureValidationState::UnsupportedAlgorithm;
        report.errors.push(format!(
            "timestamp digest algorithm {} is rejected by policy",
            token_signer.digest_alg.oid
        ));
        return report;
    }
    let token_digest = match digest_bytes(&token_signer.digest_alg.oid, tst_info_der) {
        Some(digest) => digest,
        None => {
            report.cms_signature_status = SignatureValidationState::UnsupportedAlgorithm;
            report.status = SignatureValidationState::UnsupportedAlgorithm;
            report.errors.push(format!(
                "timestamp signer digest algorithm {} is unsupported",
                token_signer.digest_alg.oid
            ));
            return report;
        }
    };
    let signed_payload = match token_signer.signed_attrs.as_ref() {
        Some(attrs) => match validate_signed_attributes(
            attrs,
            &token_digest,
            &OID_ID_CT_TST_INFO,
            token_signer,
            find_signer_cert(&signed, token_signer).cert.as_ref(),
            &mut cms,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                report.cms_signature_status = SignatureValidationState::Invalid;
                report.status = SignatureValidationState::Invalid;
                report.errors.push(format!(
                    "timestamp signed attributes failed validation: {error}"
                ));
                return report;
            }
        },
        None => {
            report.cms_signature_status = SignatureValidationState::EvidenceMissing;
            report.status = SignatureValidationState::Malformed;
            report
                .errors
                .push("timestamp token signer is missing signed attributes".to_string());
            return report;
        }
    };
    let selection = find_signer_cert(&signed, token_signer);
    let Some(tsa_cert) = selection.cert else {
        report.tsa_certificate_status = signer_resolution_state(selection.resolution);
        report.status = report.tsa_certificate_status.clone();
        report.errors.push(
            "timestamp SignerInfo did not resolve to exactly one TSA certificate".to_string(),
        );
        return report;
    };
    report.tsa_subject = Some(tsa_cert.tbs_certificate.subject.to_string());
    match verify_signature_algorithm(
        &tsa_cert,
        &token_signer.digest_alg.oid,
        token_signer,
        &signed_payload,
        algorithm_policy,
    ) {
        Ok(true) => {
            report.cms_signature_status = SignatureValidationState::Valid;
        }
        Ok(false) => {
            report.cms_signature_status = SignatureValidationState::SignatureMathInvalid;
            report.status = SignatureValidationState::SignatureMathInvalid;
            report
                .errors
                .push("timestamp token CMS signature did not verify".to_string());
            return report;
        }
        Err(error) if error.starts_with("unsupported algorithm:") => {
            report.cms_signature_status = SignatureValidationState::UnsupportedAlgorithm;
            report.status = SignatureValidationState::UnsupportedAlgorithm;
            report.errors.push(error);
            return report;
        }
        Err(error) => {
            report.cms_signature_status = SignatureValidationState::Invalid;
            report.status = SignatureValidationState::Invalid;
            report.errors.push(error);
            return report;
        }
    }

    match validate_tsa_certificate_usage(&tsa_cert) {
        Ok(()) => {
            report.tsa_certificate_status = SignatureValidationState::Valid;
            report.tsa_eku_status = SignatureValidationState::Valid;
        }
        Err(error) => {
            report.tsa_certificate_status = SignatureValidationState::PolicyRejected;
            report.tsa_eku_status = SignatureValidationState::PolicyRejected;
            report.status = SignatureValidationState::PolicyRejected;
            report.errors.push(error);
            return report;
        }
    }

    let (path_status, warnings) =
        validate_tsa_path_at_gen_time(&tsa_cert, &collect_chain(&signed), options, &cms, &tst_info);
    report.tsa_path_status = path_status;
    report.warnings.extend(warnings);
    report.status = if report.tsa_path_status == SignatureValidationState::Valid {
        SignatureValidationState::Valid
    } else {
        report.tsa_path_status.clone()
    };
    report
}

fn parse_tst_info(der: &[u8]) -> std::result::Result<ParsedTstInfo, String> {
    let fields = der_sequence_children(der, 16, "TSTInfo")?;
    if fields.len() < 5 {
        return Err("TSTInfo is missing mandatory fields".to_string());
    }
    if fields[0].tag != 0x02 || fields[0].value != [0x01] {
        return Err("TSTInfo version is not v1".to_string());
    }
    let policy_oid = ObjectIdentifier::from_der(fields[1].encoded)
        .map_err(|error| format!("TSTInfo policy OID: {error}"))?;
    if fields[2].tag != 0x30 {
        return Err("TSTInfo messageImprint is not a SEQUENCE".to_string());
    }
    let imprint = der_children(fields[2].value, 2, "TSTInfo messageImprint")?;
    if imprint.len() != 2 || imprint[1].tag != 0x04 {
        return Err("TSTInfo messageImprint is malformed".to_string());
    }
    let hash_algorithm = AlgorithmIdentifierOwned::from_der(imprint[0].encoded)
        .map_err(|error| format!("TSTInfo messageImprint hashAlgorithm: {error}"))?;
    if fields[3].tag != 0x02 {
        return Err("TSTInfo serialNumber is not an INTEGER".to_string());
    }
    let gen_time = GeneralizedTime::from_der(fields[4].encoded)
        .map_err(|error| format!("TSTInfo genTime: {error}"))?;
    Ok(ParsedTstInfo {
        policy_oid,
        serial_hex: hex_upper(fields[3].value),
        gen_time_unix: gen_time.to_unix_duration().as_secs(),
        gen_time: gen_time.to_date_time().to_string(),
        hash_algorithm,
        message_imprint: imprint[1].value.to_vec(),
    })
}

fn validate_tsa_certificate_usage(cert: &Certificate) -> std::result::Result<(), String> {
    match cert.tbs_certificate.get::<ExtendedKeyUsage>() {
        Ok(Some((_critical, eku))) => {
            if !eku.0.contains(&OID_KP_TIME_STAMPING) {
                return Err(
                    "TSA certificate ExtendedKeyUsage does not contain id-kp-timeStamping"
                        .to_string(),
                );
            }
        }
        Ok(None) => {
            return Err(
                "TSA certificate is missing ExtendedKeyUsage id-kp-timeStamping".to_string(),
            );
        }
        Err(error) => return Err(format!("TSA ExtendedKeyUsage parse failed: {error}")),
    }
    match cert.tbs_certificate.get::<KeyUsage>() {
        Ok(Some((_critical, key_usage))) if !key_usage.digital_signature() => {
            Err("TSA certificate KeyUsage does not permit digitalSignature".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) => Err(format!("TSA KeyUsage parse failed: {error}")),
    }
}

fn validate_tsa_path_at_gen_time(
    tsa_cert: &Certificate,
    chain: &[Certificate],
    options: &VerifyOptions,
    cms: &CmsValidationReport,
    tst_info: &ParsedTstInfo,
) -> (SignatureValidationState, Vec<String>) {
    let mut tsa_options = options.clone();
    tsa_options.validation_time_unix = Some(tst_info.gen_time_unix);
    let ltv = LtvReport::default();
    let coverage = Coverage::WholeFile;
    let validity = SignatureValidity::Valid;
    let evaluation = evaluate_prompt24_validation(Prompt24ValidationInput {
        signer: Some(tsa_cert),
        chain,
        options: &tsa_options,
        ltv: &ltv,
        cms,
        sub_filter: None,
        signer_resolution: SignerCertResolution::Found,
        coverage: &coverage,
        validity: &validity,
    });
    let path_status = evaluation.report.path.status.clone();
    let revocation_status = evaluation.report.revocation.status.clone();
    let mut warnings = evaluation.report.warnings;
    warnings.extend(evaluation.report.revocation.errors);
    let status = if path_status != SignatureValidationState::Valid {
        path_status
    } else if revocation_status == SignatureValidationState::Revoked {
        SignatureValidationState::Revoked
    } else if tsa_options.revocation_mode.requires_evidence()
        && revocation_status != SignatureValidationState::Valid
    {
        revocation_status
    } else {
        SignatureValidationState::Valid
    };
    (status, warnings)
}

/// Re-encode the signed attributes as an explicit `SET OF Attribute` (tag
/// 0x31) for signature verification. In the CMS structure they are stored
/// IMPLICIT [0]; the signature is computed over the EXPLICIT SET encoding.
fn reencode_signed_attrs_as_set(attrs: &x509_cert::attr::Attributes) -> Option<Vec<u8>> {
    // `Attributes` is a SetOfVec<Attribute>; DER-encoding it yields the SET OF
    // body. der's encoder writes it with the SET tag (0x31) already.
    let der = attrs.to_der().ok()?;
    // Ensure the leading tag is SET (0x31), not [0] (0xA0). `to_der` on the
    // SetOfVec emits SET, which is exactly what we need.
    (der.first() == Some(&0x31)).then_some(der)
}

/// Find the certificate in the SignedData whose issuer+serial or SKI matches
/// the SignerInfo's `sid`. Never falls back to an arbitrary certificate.
fn find_signer_cert(signed: &SignedData, signer: &SignerInfo) -> SignerCertSelection {
    let Some(certs) = signed.certificates.as_ref() else {
        return SignerCertSelection {
            resolution: SignerCertResolution::Missing,
            cert: None,
        };
    };
    use cms::cert::CertificateChoices;
    use cms::signed_data::SignerIdentifier;

    let mut matches: Vec<Certificate> = Vec::new();
    for choice in certs.0.iter() {
        if let CertificateChoices::Certificate(cert) = choice {
            let matched = match &signer.sid {
                SignerIdentifier::IssuerAndSerialNumber(ias) => {
                    cert.tbs_certificate.serial_number == ias.serial_number
                        && cert.tbs_certificate.issuer == ias.issuer
                }
                SignerIdentifier::SubjectKeyIdentifier(ski) => cert_subject_key_identifier(cert)
                    .map(|cert_ski| cert_ski == ski.0.as_bytes())
                    .unwrap_or(false),
            };
            if matched {
                matches.push(cert.clone());
            }
        }
    }
    match matches.len() {
        0 => SignerCertSelection {
            resolution: SignerCertResolution::Missing,
            cert: None,
        },
        1 => SignerCertSelection {
            resolution: SignerCertResolution::Found,
            cert: matches.into_iter().next(),
        },
        _ => SignerCertSelection {
            resolution: SignerCertResolution::Ambiguous,
            cert: None,
        },
    }
}

fn cert_subject_key_identifier(cert: &Certificate) -> Option<Vec<u8>> {
    let (_, ski) = cert
        .tbs_certificate
        .get::<x509_cert::ext::pkix::SubjectKeyIdentifier>()
        .ok()??;
    Some(ski.0.as_bytes().to_vec())
}

fn cert_to_info(cert: &Certificate) -> CertInfo {
    let tbs = &cert.tbs_certificate;
    CertInfo {
        subject: tbs.subject.to_string(),
        issuer: tbs.issuer.to_string(),
        serial_hex: hex_upper(tbs.serial_number.as_bytes()),
        not_before: tbs.validity.not_before.to_string(),
        not_after: tbs.validity.not_after.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Signing helpers
// ---------------------------------------------------------------------------

fn next_free_object_number(reader: &PdfReader) -> u32 {
    let max_seen = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0);
    let trailer_size = reader.size().unwrap_or(0).max(0) as u32;
    max_seen.max(trailer_size.saturating_sub(1)) + 1
}

fn reference(number: u32, generation: u16) -> PdfObject {
    PdfObject::Reference { number, generation }
}

fn raw_object(number: u32, generation: u16, object: &PdfObject) -> RawIncrementalObject {
    let mut body = Vec::new();
    serialize_object(object, &mut body);
    RawIncrementalObject {
        number,
        generation,
        body,
    }
}

fn rect_object(rect: [f64; 4]) -> PdfObject {
    PdfObject::Array(rect.into_iter().map(PdfObject::Real).collect::<Vec<_>>())
}

fn signature_dictionary_body(options: &SignatureOptions, reserved_bytes: usize) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"<<\n");
    body.extend_from_slice(b"/Type /Sig\n");
    body.extend_from_slice(b"/Filter /Adobe.PPKLite\n");
    body.extend_from_slice(b"/SubFilter /adbe.pkcs7.detached\n");
    body.extend_from_slice(b"/ByteRange ");
    body.extend_from_slice(BYTE_RANGE_PLACEHOLDER);
    body.extend_from_slice(b"\n/Contents ");
    body.extend_from_slice(&contents_placeholder(reserved_bytes));
    body.extend_from_slice(b"\n");
    push_optional_pdf_string(&mut body, "Name", options.signer_name.as_deref());
    push_optional_pdf_string(&mut body, "Reason", options.reason.as_deref());
    push_optional_pdf_string(&mut body, "Location", options.location.as_deref());
    push_optional_pdf_string(&mut body, "ContactInfo", options.contact_info.as_deref());
    push_optional_pdf_string(&mut body, "M", options.signing_time.as_deref());
    body.extend_from_slice(b">>");
    body
}

fn push_optional_pdf_string(out: &mut Vec<u8>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        out.extend_from_slice(format!("/{key} ").as_bytes());
        out.extend_from_slice(pdf_literal_string(value).as_bytes());
        out.extend_from_slice(b"\n");
    }
}

fn pdf_literal_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('(');
    for byte in value.as_bytes() {
        match *byte {
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(*byte as char),
            b => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out.push(')');
    out
}

fn appearance_stream(options: &SignatureOptions, rect: [f64; 4]) -> PdfObject {
    let width = (rect[2] - rect[0]).abs().max(1.0);
    let height = (rect[3] - rect[1]).abs().max(1.0);
    let signer = options
        .signer_name
        .as_deref()
        .unwrap_or(options.field_name.as_str());
    let reason = options.reason.as_deref().unwrap_or("Signed");
    let raw = format!(
        "q\n1 1 1 rg 0 0 {} {} re f\n0 0 0 RG 0.75 w 0 0 {} {} re S\nBT /Helv 10 Tf 8 {} Td {} Tj\n0 -14 Td {} Tj\nET\nQ",
        pdf_number(width),
        pdf_number(height),
        pdf_number(width),
        pdf_number(height),
        pdf_number((height - 16.0).max(8.0)),
        pdf_literal_string(&format!("Digitally signed by {signer}")),
        pdf_literal_string(reason),
    )
    .into_bytes();

    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));

    let mut fonts = PdfDictionary::empty();
    fonts.insert("Helv", PdfObject::Dictionary(font));

    let mut resources = PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));

    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".to_string()));
    dict.insert("Subtype", PdfObject::Name("Form".to_string()));
    dict.insert("BBox", rect_object([0.0, 0.0, width, height]));
    dict.insert("Resources", PdfObject::Dictionary(resources));
    PdfObject::Stream { dict, raw }
}

fn pdf_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut s = format!("{value:.3}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

fn contents_placeholder(reserved_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(reserved_bytes * 2 + 2);
    out.push(b'<');
    out.resize(reserved_bytes * 2 + 1, b'0');
    out.push(b'>');
    out
}

fn find_unique(haystack: &[u8], needle: &[u8]) -> Result<usize> {
    let mut found = None;
    for (idx, window) in haystack.windows(needle.len()).enumerate() {
        if window == needle {
            if found.is_some() {
                return Err(OxideError::MalformedPdf(
                    "signature writer found a non-unique placeholder".to_string(),
                ));
            }
            found = Some(idx);
        }
    }
    found.ok_or_else(|| {
        OxideError::MalformedPdf("signature writer placeholder was not found".to_string())
    })
}

fn patch_byte_range(out: &mut [u8], start: usize, br: &ByteRange) -> Result<()> {
    for value in [br.a, br.b, br.c, br.d] {
        if value as u64 > MAX_BYTE_RANGE_FIELD {
            return Err(OxideError::ResourceLimit(
                "signature ByteRange exceeds fixed 10-digit placeholder".to_string(),
            ));
        }
    }
    let replacement = format!("[{:>10} {:>10} {:>10} {:>10}]", br.a, br.b, br.c, br.d);
    debug_assert_eq!(replacement.len(), BYTE_RANGE_PLACEHOLDER.len());
    out[start..start + BYTE_RANGE_PLACEHOLDER.len()].copy_from_slice(replacement.as_bytes());
    Ok(())
}

fn patch_contents_hex(out: &mut [u8], hex_start: usize, reserved_bytes: usize, cms: &[u8]) {
    let hex_len = reserved_bytes * 2;
    for byte in &mut out[hex_start..hex_start + hex_len] {
        *byte = b'0';
    }
    for (idx, byte) in cms.iter().enumerate() {
        out[hex_start + idx * 2] = b"0123456789ABCDEF"[(byte >> 4) as usize];
        out[hex_start + idx * 2 + 1] = b"0123456789ABCDEF"[(byte & 0x0f) as usize];
    }
}

fn build_detached_cms(
    signer: &PdfSigner,
    content_digest: &[u8],
    timestamp_token_der: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let content = EncapsulatedContentInfo {
        econtent_type: const_oid::db::rfc5911::ID_DATA,
        econtent: None,
    };
    let digest_algorithm = AlgorithmIdentifierOwned {
        oid: OID_SHA256,
        parameters: None,
    };
    let signing_key = SigningKey::<Sha256>::new(signer.private_key.clone());
    let cert = signer.signer_certificate();
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: cert.tbs_certificate.issuer.clone(),
        serial_number: cert.tbs_certificate.serial_number.clone(),
    });
    let mut signer_info = SignerInfoBuilder::new(
        &signing_key,
        sid,
        digest_algorithm.clone(),
        &content,
        Some(content_digest),
    )
    .map_err(|e| OxideError::MalformedPdf(format!("CMS signer info: {e}")))?;
    signer_info
        .add_signed_attribute(
            create_signing_time_attribute()
                .map_err(|e| OxideError::MalformedPdf(format!("CMS signing time: {e}")))?,
        )
        .map_err(|e| OxideError::MalformedPdf(format!("CMS signed attribute: {e}")))?;
    if let Some(token_der) = timestamp_token_der {
        signer_info
            .add_unsigned_attribute(signature_timestamp_attribute(token_der)?)
            .map_err(|e| OxideError::MalformedPdf(format!("CMS unsigned attribute: {e}")))?;
    }

    let mut builder = SignedDataBuilder::new(&content);
    builder
        .add_digest_algorithm(digest_algorithm)
        .map_err(|e| OxideError::MalformedPdf(format!("CMS digest algorithm: {e}")))?;
    for cert in &signer.certificates {
        builder
            .add_certificate(CertificateChoices::Certificate(cert.clone()))
            .map_err(|e| OxideError::MalformedPdf(format!("CMS certificate: {e}")))?;
    }
    builder
        .add_signer_info::<SigningKey<Sha256>, rsa::pkcs1v15::Signature>(signer_info)
        .map_err(|e| OxideError::MalformedPdf(format!("CMS signature: {e}")))?;
    let content_info = builder
        .build()
        .map_err(|e| OxideError::MalformedPdf(format!("CMS build: {e}")))?;
    content_info
        .to_der()
        .map_err(|e| OxideError::MalformedPdf(format!("CMS encode: {e}")))
}

fn signature_timestamp_attribute(token_der: &[u8]) -> Result<Attribute> {
    ContentInfo::from_der(token_der)
        .map_err(|e| OxideError::MalformedPdf(format!("timestamp token ContentInfo: {e}")))?;
    let value = AttributeValue::from_der(token_der)
        .map_err(|e| OxideError::MalformedPdf(format!("timestamp token attribute: {e}")))?;
    let mut values = SetOfVec::new();
    values
        .insert(value)
        .map_err(|e| OxideError::MalformedPdf(format!("timestamp token set: {e}")))?;
    Ok(Attribute {
        oid: OID_SIGNATURE_TIMESTAMP_TOKEN,
        values,
    })
}

// ---------------------------------------------------------------------------
// ByteRange + coverage
// ---------------------------------------------------------------------------

/// `/ByteRange` as four offsets `[a, b, c, d]`.
struct ByteRange {
    a: usize,
    b: usize,
    c: usize,
    d: usize,
}

fn parse_byte_range(sig: &PdfDictionary) -> Option<ByteRange> {
    let arr = sig.get_array("ByteRange")?;
    if arr.len() != 4 {
        return None;
    }
    let n = |i: usize| -> Option<usize> {
        let value = arr[i].as_integer()?;
        usize::try_from(value).ok()
    };
    Some(ByteRange {
        a: n(0)?,
        b: n(1)?,
        c: n(2)?,
        d: n(3)?,
    })
}

fn extract_signed_bytes(file: &[u8], br: &ByteRange) -> Option<Vec<u8>> {
    // A detached signature must start at byte zero and leave one nonempty
    // excluded span for /Contents. Arbitrary leading or empty gaps can make a
    // malformed range present a subset as a complete signed revision.
    if br.a != 0 || br.b == 0 {
        return None;
    }
    let end1 = br.a.checked_add(br.b)?;
    let end2 = br.c.checked_add(br.d)?;
    if end1 > file.len() || end2 > file.len() || br.c <= end1 {
        return None;
    }
    let mut out = Vec::with_capacity(br.b.checked_add(br.d)?);
    out.extend_from_slice(&file[br.a..end1]);
    out.extend_from_slice(&file[br.c..end2]);
    Some(out)
}

/// The signature covers the whole file only when the final signed range ends
/// exactly at EOF. Even trailing whitespace is unsigned and must remain a
/// post-signature change in the report.
fn compute_coverage(br: &ByteRange, file_len: usize) -> Coverage {
    let signed_end = br.c.checked_add(br.d);
    if signed_end == Some(file_len) {
        Coverage::WholeFile
    } else {
        Coverage::ModifiedAfterSigning
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return exactly one definite-length DER CMS ContentInfo object from a PDF
/// `/Contents` byte string.
///
/// PDF `/Contents` padding is outside the CMS object and must be zero. The
/// parser deliberately rejects nonzero trailing bytes instead of allowing a
/// second ASN.1 object or arbitrary smuggling data after the signed CMS.
fn exact_cms_der_object(bytes: &[u8]) -> std::result::Result<&[u8], String> {
    if bytes.len() < 2 {
        return Err("CMS ContentInfo is truncated before its DER length".to_string());
    }
    // CMS ContentInfo is a universal constructed SEQUENCE and therefore has
    // the one-octet DER tag 0x30. Requiring it here keeps length parsing
    // simple, bounded, and profile-specific.
    if bytes[0] != 0x30 {
        return Err("CMS ContentInfo must start with DER SEQUENCE".to_string());
    }
    let first_length = bytes[1];
    let (header_len, content_len) = if first_length & 0x80 == 0 {
        (2usize, usize::from(first_length))
    } else {
        let length_octets = usize::from(first_length & 0x7f);
        if length_octets == 0 {
            return Err("CMS ContentInfo uses forbidden indefinite DER length".to_string());
        }
        if length_octets > std::mem::size_of::<usize>() || bytes.len() < 2 + length_octets {
            return Err("CMS ContentInfo has a truncated DER length".to_string());
        }
        // DER requires a minimal long-form length encoding.
        if bytes[2] == 0 {
            return Err("CMS ContentInfo has a non-canonical DER length".to_string());
        }
        let mut length = 0usize;
        for byte in &bytes[2..2 + length_octets] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or_else(|| "CMS ContentInfo length overflows platform bounds".to_string())?;
        }
        if length < 128 {
            return Err("CMS ContentInfo has a non-canonical DER long length".to_string());
        }
        (2 + length_octets, length)
    };
    let object_len = header_len
        .checked_add(content_len)
        .ok_or_else(|| "CMS ContentInfo length overflows platform bounds".to_string())?;
    if object_len > bytes.len() {
        return Err(format!(
            "CMS ContentInfo is truncated: declared {object_len} bytes, available {}",
            bytes.len()
        ));
    }
    if bytes[object_len..].iter().any(|byte| *byte != 0) {
        return Err("CMS ContentInfo has nonzero trailing bytes after DER object".to_string());
    }
    Ok(&bytes[..object_len])
}

fn resolve_dict(obj: Option<&PdfObject>, reader: &PdfReader) -> Option<PdfDictionary> {
    match obj? {
        PdfObject::Dictionary(d) => Some(d.clone()),
        r @ PdfObject::Reference { .. } => match reader.resolve(r.clone()).ok()? {
            PdfObject::Dictionary(d) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

fn resolve_array(obj: Option<&PdfObject>, reader: &PdfReader) -> Option<Vec<PdfObject>> {
    match obj? {
        PdfObject::Array(a) => Some(a.clone()),
        r @ PdfObject::Reference { .. } => match reader.resolve(r.clone()).ok()? {
            PdfObject::Array(a) => Some(a),
            _ => None,
        },
        _ => None,
    }
}

fn decode_text_string(obj: &PdfObject) -> Option<String> {
    match obj {
        PdfObject::String(bytes) => {
            let s = crate::info::decode_pdf_text_string(bytes);
            (!s.is_empty()).then_some(s)
        }
        _ => None,
    }
}

fn hex_upper(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn normalize_certificate_fingerprint(value: &str) -> Result<String> {
    let normalized = value
        .bytes()
        .filter(|byte| !matches!(byte, b':' | b'-' | b'_' | b' ' | b'\t' | b'\r' | b'\n'))
        .collect::<Vec<_>>();
    if normalized.len() != 64 || !normalized.iter().all(u8::is_ascii_hexdigit) {
        return Err(OxideError::invalid_input(
            "certificate SHA-256 fingerprint must contain exactly 64 hexadecimal digits",
        ));
    }
    Ok(normalized
        .into_iter()
        .map(|byte| byte.to_ascii_uppercase() as char)
        .collect())
}

fn certificate_fingerprint_sha256(cert: &Certificate) -> Option<String> {
    cert.to_der()
        .ok()
        .map(|der| hex_upper(&Sha256::digest(&der)))
}

fn configured_distrust_set(options: &VerifyOptions) -> Result<std::collections::BTreeSet<String>> {
    options
        .distrusted_certificate_sha256
        .iter()
        .map(|fingerprint| normalize_certificate_fingerprint(fingerprint))
        .collect()
}

fn validation_policy_fingerprint(options: &VerifyOptions) -> String {
    let mut anchor_hashes = options
        .trust_anchors_der
        .iter()
        .map(|der| hex_lower(&Sha256::digest(der)))
        .collect::<Vec<_>>();
    anchor_hashes.sort();
    let mut intermediate_hashes = options
        .intermediates_der
        .iter()
        .map(|der| hex_lower(&Sha256::digest(der)))
        .collect::<Vec<_>>();
    intermediate_hashes.sort();
    let mut distrust_hashes = options.distrusted_certificate_sha256.clone();
    distrust_hashes.sort();
    let policy = effective_retrieval_policy(options);
    let mut allowed_hosts = policy.allowed_hosts.clone();
    allowed_hosts.sort();
    let mut denied_hosts = policy.denied_hosts.clone();
    denied_hosts.sort();
    let mut allowed_ports = policy.allowed_ports.clone();
    allowed_ports.sort();
    let cache_directory_hash = policy
        .cache_directory
        .as_deref()
        .map(|directory| hex_lower(&Sha256::digest(directory.as_bytes())))
        .unwrap_or_default();
    let text = format!(
        "schema={PROMPT24_SIGNATURE_VALIDATION_SCHEMA_VERSION}\nprofile={:?}\nrevocation={:?}\nalgorithm_policy={:?}\nvalidation_time={:?}\nmax_depth={}\nmax_candidates={}\nanchors={}\nintermediates={}\ndistrust={}\nnetwork_enabled={}\nhttp={}\nhttps={}\nallow_private={}\nallow_non_default_ports={}\nallow_cross_origin_redirects={}\ncache_directory_sha256={}\nocsp_nonce_policy={:?}\nallowed_hosts={}\ndenied_hosts={}\nallowed_ports={}\nbudget={:?}",
        options.policy_profile,
        options.revocation_mode,
        options.algorithm_policy,
        options.validation_time_unix,
        options.max_chain_depth,
        options.max_path_candidates,
        anchor_hashes.join(","),
        intermediate_hashes.join(","),
        distrust_hashes.join(","),
        policy.enabled,
        policy.allow_http,
        policy.allow_https,
        policy.allow_private_network,
        policy.allow_non_default_ports,
        policy.allow_cross_origin_redirects,
        cache_directory_hash,
        policy.ocsp_nonce_policy,
        allowed_hosts.join(","),
        denied_hosts.join(","),
        allowed_ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(","),
        policy.budget,
    );
    hex_lower(&Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::EncodePrivateKey as _;

    fn br(a: usize, b: usize, c: usize, d: usize) -> ByteRange {
        ByteRange { a, b, c, d }
    }

    fn runtime_test_signer() -> PdfSigner {
        let (cert, private_key) = crate::pubsec::test_support::ephemeral_identity();
        let key_der = private_key.to_pkcs8_der().expect("private key DER");
        let cert_der = cert.to_der().expect("certificate DER");
        PdfSigner::from_der(key_der.as_bytes(), &cert_der, &[]).expect("runtime signer parses")
    }

    // ---- Prompt 26 incremental signing engine tests ----

    fn signable_pdf() -> Vec<u8> {
        use crate::authoring::{PageSize, PdfBuilder};
        let mut builder = PdfBuilder::new();
        builder.add_page(PageSize::custom(300.0, 300.0));
        builder.to_bytes().expect("authored signable pdf")
    }

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn signer_fingerprint(signer: &PdfSigner) -> String {
        hex_upper(&Sha256::digest(signer.signer_certificate_der().unwrap()))
    }

    struct DelegatingExternalSigner {
        signer: PdfSigner,
        reported_fingerprint: String,
        algorithm: String,
    }

    impl ExternalSigner for DelegatingExternalSigner {
        fn sign_cms(
            &self,
            request: &CmsSigningRequest,
        ) -> std::result::Result<CmsSigningResult, String> {
            let digest = hex_to_bytes(&request.digest_sha256_hex);
            let cms = build_detached_cms(&self.signer, &digest, None).map_err(|e| e.to_string())?;
            Ok(CmsSigningResult {
                cms_der: cms,
                algorithm: self.algorithm.clone(),
                signer_certificate_sha256: self.reported_fingerprint.clone(),
            })
        }
    }

    struct MalformedExternalSigner;
    impl ExternalSigner for MalformedExternalSigner {
        fn sign_cms(
            &self,
            _request: &CmsSigningRequest,
        ) -> std::result::Result<CmsSigningResult, String> {
            Ok(CmsSigningResult {
                cms_der: vec![0xDE, 0xAD, 0xBE, 0xEF],
                algorithm: "RSASSA-PKCS1v1_5-SHA256".to_string(),
                signer_certificate_sha256: "00".to_string(),
            })
        }
    }

    fn approval_options(reserved: usize) -> IncrementalSigningOptions {
        IncrementalSigningOptions {
            signature: SignatureOptions {
                field_name: "OxideEngineSig".to_string(),
                signer_name: Some("Oxide Prompt 26".to_string()),
                reason: Some("engine test".to_string()),
                contents_reserved_bytes: reserved,
                ..SignatureOptions::default()
            },
            intent: SigningIntent::Approval,
            retry_larger_placeholder: true,
            max_placeholder_bytes: 256 * 1024,
        }
    }

    #[test]
    fn incremental_approval_signature_reopens_and_validates() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf.clone()).unwrap();
        let signer = runtime_test_signer();
        let result = sign_incremental(
            &doc,
            IncrementalSigner::Local(&signer),
            &approval_options(8192),
        )
        .expect("approval signing");
        assert!(result.prefix_preserved, "original bytes must be a prefix");
        assert!(result.signed_pdf.starts_with(&pdf));
        assert!(result.post_sign.structural_open);
        assert!(result.post_sign.signature_valid);
        assert!(result.post_sign.byte_range_exact);
        assert!(result.post_sign.overall_pass);
        assert!(!result.certification);
        assert!(!result.retried);
    }

    #[test]
    fn incremental_certification_signature_sets_docmdp() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let mut options = approval_options(8192);
        options.intent = SigningIntent::Certification {
            docmdp_permissions: 2,
        };
        let result = sign_incremental(&doc, IncrementalSigner::Local(&signer), &options)
            .expect("certification signing");
        assert!(result.certification);
        assert!(result.post_sign.signature_valid);
        // The DocMDP transform + catalog /Perms must be present and recognized.
        let text = String::from_utf8_lossy(&result.signed_pdf);
        assert!(text.contains("/DocMDP"), "DocMDP transform must be encoded");
        assert!(text.contains("/Perms"), "catalog /Perms must be encoded");
        // The Prompt 25/18 permission engine must recognize the created DocMDP.
        let engine = crate::engine::ContentEngine::open_bytes(result.signed_pdf.clone()).unwrap();
        let policy = crate::prompt18::analyze_edit_policy(
            &engine,
            crate::prompt18::EditOperation::FormValueUpdate,
        )
        .unwrap();
        assert!(
            policy
                .structural_policies
                .iter()
                .any(|p| p.certification_signature && p.docmdp_p == Some(2)),
            "Prompt 25 permission engine must recognize the created DocMDP P=2 certification signature"
        );
    }

    #[test]
    fn incremental_certification_rejects_out_of_range_permission() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let mut options = approval_options(8192);
        options.intent = SigningIntent::Certification {
            docmdp_permissions: 9,
        };
        assert!(sign_incremental(&doc, IncrementalSigner::Local(&signer), &options).is_err());
    }

    #[test]
    fn external_signer_happy_path_matches_local() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let fingerprint = signer_fingerprint(&signer);
        let external = DelegatingExternalSigner {
            signer,
            reported_fingerprint: fingerprint.clone(),
            algorithm: "RSASSA-PKCS1v1_5-SHA256".to_string(),
        };
        let result = sign_incremental(
            &doc,
            IncrementalSigner::ExternalCms {
                signer: &external,
                expected_certificate_sha256: Some(fingerprint),
            },
            &approval_options(8192),
        )
        .expect("external signing");
        assert!(result.post_sign.signature_valid);
        assert!(result.post_sign.overall_pass);
    }

    #[test]
    fn external_signer_wrong_certificate_rejected() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let real_fingerprint = signer_fingerprint(&signer);
        let external = DelegatingExternalSigner {
            signer,
            reported_fingerprint: real_fingerprint,
            algorithm: "RSASSA-PKCS1v1_5-SHA256".to_string(),
        };
        // Pin a different fingerprint than the signer reports/uses.
        let pinned = "AA".repeat(32);
        let err = sign_incremental(
            &doc,
            IncrementalSigner::ExternalCms {
                signer: &external,
                expected_certificate_sha256: Some(pinned),
            },
            &approval_options(8192),
        );
        assert!(err.is_err(), "wrong certificate must be rejected");
    }

    #[test]
    fn external_signer_wrong_algorithm_rejected() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let fingerprint = signer_fingerprint(&signer);
        let external = DelegatingExternalSigner {
            signer,
            reported_fingerprint: fingerprint.clone(),
            algorithm: "ECDSA-P521-SHA3-512".to_string(),
        };
        let err = sign_incremental(
            &doc,
            IncrementalSigner::ExternalCms {
                signer: &external,
                expected_certificate_sha256: Some(fingerprint),
            },
            &approval_options(8192),
        );
        assert!(err.is_err(), "wrong algorithm must be rejected");
    }

    #[test]
    fn external_signer_malformed_cms_rejected() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let external = MalformedExternalSigner;
        let err = sign_incremental(
            &doc,
            IncrementalSigner::ExternalCms {
                signer: &external,
                expected_certificate_sha256: None,
            },
            &approval_options(8192),
        );
        assert!(
            err.is_err(),
            "malformed CMS must be rejected before insertion"
        );
    }

    #[test]
    fn placeholder_too_small_retries_and_succeeds() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        // 8 bytes cannot hold an RSA-2048 CMS; retry must grow the placeholder.
        let result = sign_incremental(
            &doc,
            IncrementalSigner::Local(&signer),
            &approval_options(8),
        )
        .expect("retry signing");
        assert!(result.retried, "placeholder must have been grown");
        assert!(result.post_sign.signature_valid);
    }

    #[test]
    fn placeholder_too_small_without_retry_fails_before_producing_output() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let mut options = approval_options(8);
        options.retry_larger_placeholder = false;
        let err = sign_incremental(&doc, IncrementalSigner::Local(&signer), &options);
        assert!(err.is_err(), "too-small placeholder must fail, not lie");
    }

    #[test]
    fn placeholder_plan_reports_required_vs_reserved() {
        let pdf = signable_pdf();
        let doc = crate::document::PdfDocument::open_bytes(pdf).unwrap();
        let signer = runtime_test_signer();
        let tight = plan_signature_placeholder(&doc, &signer, &approval_options(8)).unwrap();
        assert!(!tight.fits, "8-byte placeholder cannot fit the CMS");
        assert!(tight.required_bytes > 8);
        let roomy = plan_signature_placeholder(&doc, &signer, &approval_options(8192)).unwrap();
        assert!(roomy.fits);
        assert_eq!(
            roomy.byte_range[0], 0,
            "ByteRange must start at file offset 0"
        );
    }

    #[test]
    fn extract_signed_bytes_concatenates_two_ranges() {
        let file = b"AAAA<SIG>BBBB".to_vec();
        // range1 = [0,4) "AAAA"; gap [4,9) is the <SIG>; range2 = [9,13) "BBBB"
        let bytes = extract_signed_bytes(&file, &br(0, 4, 9, 4)).unwrap();
        assert_eq!(bytes, b"AAAABBBB");
    }

    #[test]
    fn extract_signed_bytes_rejects_out_of_bounds() {
        let file = b"short".to_vec();
        assert!(extract_signed_bytes(&file, &br(0, 100, 0, 0)).is_none());
    }

    #[test]
    fn coverage_whole_file_vs_modified() {
        // Signed end reaches EOF -> whole file.
        assert_eq!(compute_coverage(&br(0, 4, 9, 4), 13), Coverage::WholeFile);
        // Trailing bytes after signed end -> modified after signing.
        assert_eq!(
            compute_coverage(&br(0, 4, 9, 4), 100),
            Coverage::ModifiedAfterSigning
        );
    }

    #[test]
    fn byte_range_requires_file_start_and_a_nonempty_contents_gap() {
        let file = b"AAAA<SIG>BBBB".to_vec();
        assert!(extract_signed_bytes(&file, &br(1, 3, 9, 4)).is_none());
        assert!(extract_signed_bytes(&file, &br(0, 4, 4, 9)).is_none());
    }

    #[test]
    fn raw_contents_locator_skips_dictionary_delimiters_and_strings() {
        let raw = b"8 0 obj\n<< /Note (/Contents <not-a-value>) /Contents <30 00> >>\nendobj";
        assert_eq!(find_raw_contents_span(raw).unwrap(), (53, 60));
        assert!(find_raw_contents_span(b"<< /Contents <00> /Contents <01> >>").is_err());
    }

    #[test]
    fn pades_baseline_remains_conformant_for_a_signed_historical_revision() {
        let cms = CmsValidationReport {
            status: SignatureValidationState::Valid,
            content_info: SignatureValidationState::Valid,
            detached_content: SignatureValidationState::Valid,
            signer_info_count: 1,
            digest_algorithm_declared: SignatureValidationState::Valid,
            signed_attributes: SignatureValidationState::Valid,
            content_type_attribute: SignatureValidationState::Valid,
            message_digest_attribute: SignatureValidationState::Valid,
            signing_certificate_reference: SignatureValidationState::Valid,
            cms_algorithm_protection: SignatureValidationState::Valid,
        };
        let report = pades_prompt24_report(
            &LtvReport::default(),
            &Coverage::ModifiedAfterSigning,
            &SignatureValidity::Valid,
            &cms,
            Some("ETSI.CAdES.detached"),
            None,
            None,
        );
        assert_eq!(report.status, SignatureValidationState::Indeterminate);
        assert_eq!(report.structural_status, SignatureValidationState::Valid);
        assert_eq!(
            report.signed_revision_coverage_status,
            SignatureValidationState::Valid
        );
        assert_eq!(
            report.current_document_status,
            SignatureValidationState::ModifiedAfterSigning
        );
        assert!(report.missing_requirements.is_empty());
    }

    #[test]
    fn pades_final_status_requires_path_but_respects_an_explicit_no_revocation_policy() {
        let cms = CmsValidationReport {
            status: SignatureValidationState::Valid,
            content_info: SignatureValidationState::Valid,
            detached_content: SignatureValidationState::Valid,
            signer_info_count: 1,
            digest_algorithm_declared: SignatureValidationState::Valid,
            signed_attributes: SignatureValidationState::Valid,
            content_type_attribute: SignatureValidationState::Valid,
            message_digest_attribute: SignatureValidationState::Valid,
            signing_certificate_reference: SignatureValidationState::Valid,
            cms_algorithm_protection: SignatureValidationState::Valid,
        };
        let path = CertificatePathValidationReport {
            status: SignatureValidationState::Valid,
            ..CertificatePathValidationReport::default()
        };
        let revocation = RevocationValidationReport::default();
        let report = pades_prompt24_report(
            &LtvReport::default(),
            &Coverage::WholeFile,
            &SignatureValidity::Valid,
            &cms,
            Some("ETSI.CAdES.detached"),
            Some(&path),
            Some(&revocation),
        );
        assert_eq!(report.structural_status, SignatureValidationState::Valid);
        assert_eq!(report.status, SignatureValidationState::Valid);
        assert_eq!(
            report.revocation_status,
            SignatureValidationState::NotChecked
        );
    }

    #[test]
    fn ess_signing_certificate_v2_binds_the_exact_selected_certificate() {
        let certificate =
            Certificate::from_pem(include_bytes!("../tests/fixtures/sign_test_rsa_cert.pem"))
                .expect("test certificate parses");
        let certificate_der = certificate.to_der().unwrap();
        let cert_hash = Sha256::digest(&certificate_der);
        let cert_id = der_sequence(&[der_octet_string(&cert_hash)]);
        let value_der = der_sequence(&[der_sequence(&[cert_id])]);
        let attribute = attribute_with_single_value(OID_SIGNING_CERTIFICATE_V2, &value_der);
        assert!(validate_ess_signing_certificate(&attribute, &certificate).is_ok());

        let mut altered_hash = cert_hash.to_vec();
        altered_hash[0] ^= 0x80;
        let altered = attribute_with_single_value(
            OID_SIGNING_CERTIFICATE_V2,
            &der_sequence(&[der_sequence(&[der_sequence(&[der_octet_string(
                &altered_hash,
            )])])]),
        );
        assert!(validate_ess_signing_certificate(&altered, &certificate).is_err());
    }

    #[test]
    fn rsa_pss_parameters_are_bound_to_the_declared_hash_and_salt_length() {
        use rsa::signature::{RandomizedSigner as _, SignatureEncoding};

        let (certificate, private_key) = crate::pubsec::test_support::ephemeral_identity();
        let payload = b"prompt24 rsa-pss parameter regression";
        let signing_key = rsa::pss::SigningKey::<Sha256>::new_with_salt_len(private_key, 32);
        let signature = signing_key
            .sign_with_rng(&mut rsa::rand_core::OsRng, payload)
            .to_vec();
        let params = rsa::pkcs1::RsaPssParams::new::<Sha256>(32)
            .to_der()
            .unwrap();
        let algorithm = AlgorithmIdentifierOwned {
            oid: OID_RSA_PSS,
            parameters: Some(der::asn1::Any::from_der(&params).unwrap()),
        };
        assert!(verify_rsa_pss(
            &certificate,
            &OID_SHA256,
            &algorithm,
            payload,
            &signature,
            &SignatureAlgorithmPolicy::default(),
        )
        .unwrap());
        assert!(verify_rsa_pss(
            &certificate,
            &OID_SHA384,
            &algorithm,
            payload,
            &signature,
            &SignatureAlgorithmPolicy::default(),
        )
        .is_err());
    }

    #[test]
    fn required_ocsp_nonce_is_fresh_and_the_response_must_echo_it() {
        let certificate =
            Certificate::from_pem(include_bytes!("../tests/fixtures/sign_test_rsa_cert.pem"))
                .expect("test certificate parses");
        let request = build_ocsp_request(&certificate, &certificate, OcspNoncePolicy::Required)
            .expect("OCSP request with nonce builds");
        let nonce = request.nonce.expect("required policy creates a nonce");
        assert_eq!(nonce.len(), 16);
        let decoded = x509_ocsp::OcspRequest::from_der(&request.bytes).expect("request decodes");
        assert_eq!(
            decoded.nonce().expect("request nonce").0.as_bytes(),
            nonce.as_slice()
        );

        let nonce_response = include_bytes!("../tests/fixtures/aia_leaf_nonce_good.ocsp");
        let expected = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        assert!(validate_ocsp_response_nonce(nonce_response, Some(&expected)).is_ok());
        assert!(validate_ocsp_response_nonce(nonce_response, Some(b"wrong nonce")).is_err());
        assert!(validate_ocsp_response_nonce(
            include_bytes!("../tests/fixtures/aia_leaf_good.ocsp"),
            Some(&expected),
        )
        .is_err());
    }

    #[test]
    fn rfc3161_signature_timestamp_validates_imprint_tsa_eku_and_path() {
        let tsa = runtime_test_tsa_signer();
        let signature_value = b"prompt25 cms signature value";
        let token = build_timestamp_token_for_test(&tsa, signature_value).unwrap();
        let options =
            VerifyOptions::default().with_trust_anchor_der(tsa.signer_certificate_der().unwrap());

        let report = validate_signature_timestamp_token(
            &token,
            "unit-test".to_string(),
            signature_value,
            &options,
            &SignatureAlgorithmPolicy::default(),
        );

        assert_eq!(
            report.status,
            SignatureValidationState::Valid,
            "{report:#?}"
        );
        assert_eq!(
            report.message_imprint_status,
            SignatureValidationState::Valid
        );
        assert_eq!(report.tsa_eku_status, SignatureValidationState::Valid);
        assert_eq!(report.tsa_path_status, SignatureValidationState::Valid);
    }

    #[test]
    fn rfc3161_signature_timestamp_rejects_wrong_imprint() {
        let tsa = runtime_test_tsa_signer();
        let token = build_timestamp_token_for_test(&tsa, b"other signature").unwrap();
        let options =
            VerifyOptions::default().with_trust_anchor_der(tsa.signer_certificate_der().unwrap());

        let report = validate_signature_timestamp_token(
            &token,
            "unit-test".to_string(),
            b"prompt25 cms signature value",
            &options,
            &SignatureAlgorithmPolicy::default(),
        );

        assert_eq!(report.status, SignatureValidationState::DigestMismatch);
        assert_eq!(
            report.message_imprint_status,
            SignatureValidationState::DigestMismatch
        );
        assert_ne!(report.tsa_path_status, SignatureValidationState::Valid);
    }

    #[test]
    fn cms_multiple_signer_infos_are_validated_independently() {
        let signer = runtime_test_signer();
        let content = b"prompt24 multi-signer detached CMS regression";
        let digest = Sha256::digest(content);
        let cms = build_two_signer_cms_for_test(&signer, digest.as_slice()).unwrap();

        let results = verify_cms(&cms, content, &VerifyOptions::default()).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| {
            result.validity == SignatureValidity::Valid
                && result.signer_resolution == SignerCertResolution::Found
                && result.cms.signer_info_count == 2
        }));
    }

    #[test]
    fn one_pdf_signature_emits_one_report_per_cms_signer() {
        let signer = runtime_test_signer();
        let prefix = b"%PDF-synthetic-signed-prefix\n";
        let suffix = b"\n%%EOF\n";
        let mut signed = prefix.to_vec();
        signed.extend_from_slice(suffix);
        let digest = Sha256::digest(&signed);
        let cms = build_two_signer_cms_for_test(&signer, digest.as_slice()).unwrap();
        let encoded_contents = hex_upper(&cms).into_bytes();
        let mut file = prefix.to_vec();
        let contents_start = file.len();
        file.push(b'<');
        file.extend_from_slice(&encoded_contents);
        file.push(b'>');
        let contents_end = file.len();
        file.extend_from_slice(suffix);

        let mut sig_dict = PdfDictionary::empty();
        sig_dict.insert(
            "ByteRange",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(prefix.len() as i64),
                PdfObject::Integer(contents_end as i64),
                PdfObject::Integer(suffix.len() as i64),
            ]),
        );
        sig_dict.insert("Contents", PdfObject::String(cms));
        sig_dict.insert(
            "SubFilter",
            PdfObject::Name("ETSI.CAdES.detached".to_string()),
        );
        let field = SigField {
            field_name: Some("MultiSigner".to_string()),
            sig_dict,
            discovery_kind: SignatureDiscoveryKind::AcroformSignatureField,
            field_object: Some((1, 0)),
            signature_object: Some((2, 0)),
            contents_span: Ok((contents_start, contents_end)),
            discovery_issue: None,
        };

        let outcome = verify_one_with_evidence(
            &field,
            &file,
            1,
            &DssIndex::default(),
            &VerifyOptions::default(),
        );
        assert_eq!(outcome.reports.len(), 2);
        for (position, report) in outcome.reports.iter().enumerate() {
            assert_eq!(report.index, 1);
            assert_eq!(report.cms_signer_index, Some(position + 1));
            assert_eq!(report.cms_signer_count, 2);
            assert_eq!(report.validity, SignatureValidity::Valid, "{report:#?}");
        }
    }

    #[test]
    fn signature_algorithm_policy_rejects_a_recognized_but_forbidden_scheme() {
        let signer = runtime_test_signer();
        let content = b"prompt24 algorithm policy regression";
        let digest = Sha256::digest(content);
        let cms = build_two_signer_cms_for_test(&signer, digest.as_slice()).unwrap();
        let policy = SignatureAlgorithmPolicy {
            allow_rsa_pkcs1v15: false,
            ..SignatureAlgorithmPolicy::default()
        };

        let options = VerifyOptions {
            algorithm_policy: policy,
            ..VerifyOptions::default()
        };
        let results = verify_cms(&cms, content, &options).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| {
            result.validity == SignatureValidity::UnsupportedAlgorithm
                && result.cms.status == SignatureValidationState::UnsupportedAlgorithm
        }));
        assert!(!SignatureAlgorithmPolicy::default().allows_digest(&OID_SHA1));
    }

    fn runtime_test_tsa_signer() -> PdfSigner {
        use rsa::pkcs8::EncodePublicKey as _;
        use rsa::rand_core::OsRng;
        use rsa::RsaPublicKey;
        use spki::SubjectPublicKeyInfoOwned;
        use std::str::FromStr;
        use x509_cert::builder::{Builder, CertificateBuilder, Profile};
        use x509_cert::name::Name;
        use x509_cert::serial_number::SerialNumber;
        use x509_cert::time::{Time, Validity};

        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key");
        let signing_key = SigningKey::<Sha256>::new(private_key.clone());
        let public_key = RsaPublicKey::from(&private_key);
        let spki_der = public_key.to_public_key_der().expect("SPKI DER");
        let spki = SubjectPublicKeyInfoOwned::try_from(spki_der.as_bytes()).expect("SPKI parse");
        let subject = Name::from_str("CN=Oxide Prompt25 TSA,O=Oxide,C=US").expect("name");
        let validity = Validity {
            not_before: Time::from(
                GeneralizedTime::from_unix_duration(std::time::Duration::from_secs(1_704_067_200))
                    .expect("notBefore"),
            ),
            not_after: Time::from(
                GeneralizedTime::from_unix_duration(std::time::Duration::from_secs(1_893_456_000))
                    .expect("notAfter"),
            ),
        };
        let mut builder = CertificateBuilder::new(
            Profile::Leaf {
                issuer: subject.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: false,
            },
            SerialNumber::from(25u32),
            validity,
            subject,
            spki,
            &signing_key,
        )
        .expect("cert builder");
        builder
            .add_extension(&ExtendedKeyUsage(vec![OID_KP_TIME_STAMPING]))
            .expect("timestamping EKU");
        let cert = builder.build::<rsa::pkcs1v15::Signature>().expect("cert");
        let key_der = private_key.to_pkcs8_der().expect("private key DER");
        let cert_der = cert.to_der().expect("certificate DER");
        PdfSigner::from_der(key_der.as_bytes(), &cert_der, &[]).expect("TSA signer parses")
    }

    fn build_timestamp_token_for_test(
        signer: &PdfSigner,
        imprint_input: &[u8],
    ) -> std::result::Result<Vec<u8>, String> {
        let tst_info = build_tst_info_for_test(imprint_input);
        let content = EncapsulatedContentInfo {
            econtent_type: OID_ID_CT_TST_INFO,
            econtent: Some(
                der::Any::new(der::Tag::OctetString, tst_info.clone())
                    .map_err(|error| error.to_string())?,
            ),
        };
        let digest_algorithm = AlgorithmIdentifierOwned {
            oid: OID_SHA256,
            parameters: None,
        };
        let signing_key = SigningKey::<Sha256>::new(signer.private_key.clone());
        let cert = signer.signer_certificate();
        let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate.issuer.clone(),
            serial_number: cert.tbs_certificate.serial_number.clone(),
        });
        let mut builder = SignedDataBuilder::new(&content);
        builder
            .add_digest_algorithm(digest_algorithm.clone())
            .map_err(|error| error.to_string())?;
        builder
            .add_certificate(CertificateChoices::Certificate(cert.clone()))
            .map_err(|error| error.to_string())?;
        let signer_info =
            SignerInfoBuilder::new(&signing_key, sid, digest_algorithm, &content, None)
                .map_err(|error| error.to_string())?;
        builder
            .add_signer_info::<SigningKey<Sha256>, rsa::pkcs1v15::Signature>(signer_info)
            .map_err(|error| error.to_string())?;
        builder
            .build()
            .map_err(|error| error.to_string())?
            .to_der()
            .map_err(|error| error.to_string())
    }

    fn build_tst_info_for_test(imprint_input: &[u8]) -> Vec<u8> {
        let digest_algorithm = AlgorithmIdentifierOwned {
            oid: OID_SHA256,
            parameters: None,
        }
        .to_der()
        .expect("algorithm DER");
        let imprint = der_sequence(&[
            digest_algorithm,
            der_octet_string(&Sha256::digest(imprint_input)),
        ]);
        let gen_time =
            GeneralizedTime::from_unix_duration(std::time::Duration::from_secs(1_767_225_600))
                .expect("genTime")
                .to_der()
                .expect("genTime DER");
        der_sequence(&[
            der_integer(&[0x01]),
            OID_SHA256.to_der().expect("policy OID"),
            imprint,
            der_integer(&[0x25]),
            gen_time,
        ])
    }

    fn build_two_signer_cms_for_test(
        signer: &PdfSigner,
        content_digest: &[u8],
    ) -> std::result::Result<Vec<u8>, String> {
        let content = EncapsulatedContentInfo {
            econtent_type: const_oid::db::rfc5911::ID_DATA,
            econtent: None,
        };
        let digest_algorithm = AlgorithmIdentifierOwned {
            oid: OID_SHA256,
            parameters: None,
        };
        let certificate = signer.signer_certificate();
        let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: certificate.tbs_certificate.issuer.clone(),
            serial_number: certificate.tbs_certificate.serial_number.clone(),
        });
        let mut builder = SignedDataBuilder::new(&content);
        builder
            .add_digest_algorithm(digest_algorithm.clone())
            .map_err(|error| error.to_string())?;
        for certificate in &signer.certificates {
            builder
                .add_certificate(CertificateChoices::Certificate(certificate.clone()))
                .map_err(|error| error.to_string())?;
        }

        for marker in [1_u8, 2_u8] {
            let signing_key = SigningKey::<Sha256>::new(signer.private_key.clone());
            let mut signer_info = SignerInfoBuilder::new(
                &signing_key,
                sid.clone(),
                digest_algorithm.clone(),
                &content,
                Some(content_digest),
            )
            .map_err(|error| error.to_string())?;
            signer_info
                .add_signed_attribute(
                    create_signing_time_attribute().map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;

            // Different opaque unsigned attributes make the two SignerInfo
            // values distinct while remaining outside the signed-attribute
            // semantics under test.
            let mut values = SetOfVec::new();
            values
                .insert(AttributeValue::from_der(&der_octet_string(&[marker])).unwrap())
                .unwrap();
            signer_info
                .add_unsigned_attribute(Attribute {
                    oid: ObjectIdentifier::new_unwrap("1.3.6.1.4.1.57264.24.1"),
                    values,
                })
                .map_err(|error| error.to_string())?;
            builder
                .add_signer_info::<SigningKey<Sha256>, rsa::pkcs1v15::Signature>(signer_info)
                .map_err(|error| error.to_string())?;
        }
        builder
            .build()
            .map_err(|error| error.to_string())?
            .to_der()
            .map_err(|error| error.to_string())
    }

    #[test]
    fn cms_der_object_length_preserves_terminal_zero_and_rejects_smuggling() {
        // A valid ASN.1 INTEGER inside the CMS object may legitimately end in
        // zero. PDF padding starts only after the declared outer sequence.
        let padded = [0x30, 0x03, 0x02, 0x01, 0x00, 0x00, 0x00];
        assert_eq!(
            exact_cms_der_object(&padded).unwrap(),
            &[0x30, 0x03, 0x02, 0x01, 0x00]
        );
        assert!(exact_cms_der_object(&[0x30, 0x81, 0x01, 0x00]).is_err());
        assert!(exact_cms_der_object(&[0x30, 0x03, 0x02, 0x01]).is_err());
        assert!(exact_cms_der_object(&[0x30, 0x00, 0x01]).is_err());
    }

    #[test]
    fn vri_key_candidates_accept_padded_pdf_contents_hash() {
        let padded = [0x30, 0x03, 0x02, 0x01, 0x00, 0x00, 0x00];
        let padded_hex_key = hex_upper(&Sha1::digest(hex_lower(&padded).as_bytes()));
        let candidates = signature_vri_key_candidates(&padded);
        assert_eq!(candidates[0], signature_vri_key(&padded));
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == &padded_hex_key),
            "pyHanko-compatible padded /Contents VRI key should be accepted"
        );
    }

    #[test]
    fn hex_upper_formats() {
        assert_eq!(hex_upper(&[0x0a, 0xff, 0x10]), "0AFF10");
    }

    fn attribute_with_single_value(oid: ObjectIdentifier, der: &[u8]) -> Attribute {
        let mut values = SetOfVec::new();
        values
            .insert(AttributeValue::from_der(der).unwrap())
            .unwrap();
        Attribute { oid, values }
    }

    fn der_octet_string(value: &[u8]) -> Vec<u8> {
        let mut result = vec![0x04];
        der_length(value.len(), &mut result);
        result.extend_from_slice(value);
        result
    }

    fn der_integer(value: &[u8]) -> Vec<u8> {
        let mut normalized = value.to_vec();
        while normalized.len() > 1 && normalized[0] == 0 && normalized[1] < 0x80 {
            normalized.remove(0);
        }
        if normalized.first().is_some_and(|byte| byte & 0x80 != 0) {
            normalized.insert(0, 0);
        }
        let mut result = vec![0x02];
        der_length(normalized.len(), &mut result);
        result.extend_from_slice(&normalized);
        result
    }

    fn der_sequence(parts: &[Vec<u8>]) -> Vec<u8> {
        let content_len = parts.iter().map(Vec::len).sum();
        let mut result = vec![0x30];
        der_length(content_len, &mut result);
        for part in parts {
            result.extend_from_slice(part);
        }
        result
    }

    fn der_length(length: usize, result: &mut Vec<u8>) {
        if length < 0x80 {
            result.push(length as u8);
            return;
        }
        let bytes = length.to_be_bytes();
        let start = bytes.iter().position(|byte| *byte != 0).unwrap();
        result.push(0x80 | (bytes.len() - start) as u8);
        result.extend_from_slice(&bytes[start..]);
    }
}
