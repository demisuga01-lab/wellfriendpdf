//! PDF public-key security-handler support for scoped Prompt 23B decryption.
//!
//! This module implements the document-level `/Filter /Adobe.PubSec`
//! decryption path for CMS `EnvelopedData` recipients using RSA key transport.
//! It intentionally does not perform certificate trust-chain validation.

use std::collections::HashMap;

use cms::builder::{
    ContentEncryptionAlgorithm, EnvelopedDataBuilder, KeyEncryptionInfo,
    KeyTransRecipientInfoBuilder,
};
use cms::cert::IssuerAndSerialNumber;
use cms::content_info::ContentInfo;
use cms::enveloped_data::{
    EnvelopedData, KeyTransRecipientInfo, RecipientIdentifier, RecipientInfo,
};
use der::asn1::{Any, OctetString};
use der::{AnyRef, Decode, DecodePem, Encode, Tagged};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::Pkcs1v15Encrypt;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::rand_core::OsRng;
use rsa::traits::PublicKeyParts;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use spki::AlgorithmIdentifierOwned;
use x509_cert::ext::pkix::SubjectKeyIdentifier;
use x509_cert::Certificate;
use zeroize::Zeroize;

use crate::crypto::{secret_bytes, CryptMethod, SecretBytes};
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::writer::{rewrite_document_objects, PdfWriter, PdfWriterCustomEncryption, WriterMode};

const MAX_CMS_BYTES: usize = 1_048_576;
const MAX_RECIPIENTS: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PFX_BYTES: usize = 1_048_576;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PFX_CERTS: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PFX_KEYS: usize = 16;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PFX_PASSWORD_BYTES: usize = 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_CERTIFICATE_BYTES: usize = 131_072;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PRIVATE_KEY_BYTES: usize = 131_072;
const PUBSEC_SEED_LEN: usize = 20;
const PUBSEC_PERMISSIONS_LEN: usize = 4;

/// One in-memory certificate/private-key identity for public-key PDF opening.
///
/// The private key is never serialized or logged by this type. Certificate
/// trust validation is deliberately outside this layer; matching establishes
/// only that this identity can attempt recipient decryption.
pub struct PubSecIdentity {
    certificate: Certificate,
    certificate_der_sha256: [u8; 32],
    subject_key_identifier: Option<Vec<u8>>,
    private_key: RsaPrivateKey,
}

/// Public recipient certificate used when writing `/Adobe.PubSec` PDFs.
///
/// This type contains only public certificate material and a parsed RSA public
/// key. Certificate trust is not evaluated here.
#[derive(Clone)]
pub struct PubSecRecipientCertificate {
    certificate: Certificate,
    certificate_der_sha256: [u8; 32],
    public_key: RsaPublicKey,
    subject_key_identifier: Option<Vec<u8>>,
}

impl PubSecRecipientCertificate {
    /// Build a recipient from DER or PEM X.509 certificate bytes.
    pub fn from_bytes(certificate_bytes: &[u8]) -> Result<Self> {
        let (certificate, der) = parse_certificate_bytes(certificate_bytes)?;
        Self::from_certificate_and_der(certificate, der)
    }

    /// Build a recipient from a DER-encoded X.509 certificate.
    pub fn from_der(certificate_der: &[u8]) -> Result<Self> {
        let certificate = Certificate::from_der(certificate_der)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate DER: {e}")))?;
        Self::from_certificate_and_der(certificate, certificate_der.to_vec())
    }

    /// Build a recipient from a PEM-encoded X.509 certificate.
    pub fn from_pem(certificate_pem: &str) -> Result<Self> {
        let certificate = Certificate::from_pem(certificate_pem.as_bytes())
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate PEM: {e}")))?;
        let der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        Self::from_certificate_and_der(certificate, der)
    }

    pub fn from_certificate(certificate: Certificate) -> Result<Self> {
        let der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        Self::from_certificate_and_der(certificate, der)
    }

    fn from_certificate_and_der(
        certificate: Certificate,
        certificate_der: Vec<u8>,
    ) -> Result<Self> {
        let public_key = RsaPublicKey::from_public_key_der(
            &certificate
                .tbs_certificate
                .subject_public_key_info
                .to_der()
                .map_err(|e| {
                    WellfriendError::MalformedPdf(format!("PubSec certificate SPKI encode: {e}"))
                })?,
        )
        .map_err(|e| {
            WellfriendError::UnsupportedFeature(format!(
                "PubSec recipient certificate public key is not supported RSA: {e}"
            ))
        })?;
        let digest = Sha256::digest(&certificate_der);
        let mut certificate_der_sha256 = [0u8; 32];
        certificate_der_sha256.copy_from_slice(&digest);
        let subject_key_identifier = certificate
            .tbs_certificate
            .get::<SubjectKeyIdentifier>()
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate SKI: {e}")))?
            .map(|(_, ski)| ski.0.as_bytes().to_vec());
        Ok(Self {
            certificate,
            certificate_der_sha256,
            public_key,
            subject_key_identifier,
        })
    }

    pub fn certificate_fingerprint_sha256(&self) -> [u8; 32] {
        self.certificate_der_sha256
    }

    pub fn subject_key_identifier(&self) -> Option<&[u8]> {
        self.subject_key_identifier.as_deref()
    }
}

/// Recipient identifier emitted in generated CMS KeyTransRecipientInfo values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PubSecRecipientIdMode {
    IssuerAndSerial,
    SubjectKeyIdentifier,
}

/// Full-rewrite public-key encryption options for the supported Prompt 23B
/// profile: `/Filter /Adobe.PubSec`, `/SubFilter /adbe.pkcs7.s5`,
/// KeyTransRecipientInfo recipients, AES-CBC CMS content protection, and
/// AESV2/AESV3 object crypt filters.
pub struct PubSecEncryptOptions {
    pub recipients: Vec<PubSecRecipientCertificate>,
    pub permissions: u32,
    pub encrypt_metadata: bool,
    pub method: CryptMethod,
    pub recipient_id_mode: PubSecRecipientIdMode,
}

impl Default for PubSecEncryptOptions {
    fn default() -> Self {
        Self {
            recipients: Vec::new(),
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: CryptMethod::AesV2,
            recipient_id_mode: PubSecRecipientIdMode::IssuerAndSerial,
        }
    }
}

pub struct PubSecEncryptReport {
    pub recipient_count: usize,
    pub crypt_filter: String,
    pub method: CryptMethod,
    pub key_length_bits: usize,
    pub permissions: u32,
    pub encrypt_metadata: bool,
}

impl PubSecIdentity {
    /// Build an identity from certificate and RSA private-key bytes. Each input
    /// may independently be DER or PEM.
    pub fn from_bytes(certificate_bytes: &[u8], private_key_bytes: &[u8]) -> Result<Self> {
        let (certificate, certificate_der) = parse_certificate_bytes(certificate_bytes)?;
        let private_key = parse_rsa_private_key_bytes(private_key_bytes)?;
        Self::from_parts(certificate, &certificate_der, private_key)
    }

    /// Build an identity from DER certificate and RSA private key bytes.
    ///
    /// `private_key_der` may be PKCS#8 or PKCS#1. Encrypted private keys and
    /// non-RSA keys are rejected with exact unsupported diagnostics.
    pub fn from_der(certificate_der: &[u8], private_key_der: &[u8]) -> Result<Self> {
        let certificate = Certificate::from_der(certificate_der)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate DER: {e}")))?;
        let private_key = RsaPrivateKey::from_pkcs8_der(private_key_der)
            .or_else(|_| RsaPrivateKey::from_pkcs1_der(private_key_der))
            .map_err(|e| {
                WellfriendError::UnsupportedFeature(format!(
                    "PubSec RSA private key parse failed or encrypted key unsupported: {e}"
                ))
            })?;
        Self::from_parts(certificate, certificate_der, private_key)
    }

    /// Build an identity from a DER certificate and encrypted PKCS#8 private
    /// key bytes using an explicit password. The password is operation-scoped;
    /// it is not stored in the provider and is never serialized into reports.
    pub fn from_encrypted_pkcs8_der(
        certificate_der: &[u8],
        encrypted_private_key_der: &[u8],
        password: &[u8],
    ) -> Result<Self> {
        let certificate = Certificate::from_der(certificate_der)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate DER: {e}")))?;
        let private_key =
            RsaPrivateKey::from_pkcs8_encrypted_der(encrypted_private_key_der, password).map_err(
                |e| {
                    WellfriendError::EncryptedPdf(format!(
                        "PubSec encrypted PKCS#8 private key could not be decrypted or parsed: {e}"
                    ))
                },
            )?;
        Self::from_parts(certificate, certificate_der, private_key)
    }

    /// Build an identity from PEM certificate and RSA private key text.
    pub fn from_pem(certificate_pem: &str, private_key_pem: &str) -> Result<Self> {
        let certificate = Certificate::from_pem(certificate_pem.as_bytes())
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate PEM: {e}")))?;
        let certificate_der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
            .map_err(|e| {
                WellfriendError::UnsupportedFeature(format!(
                    "PubSec RSA private key PEM parse failed or encrypted key unsupported: {e}"
                ))
            })?;
        Self::from_parts(certificate, &certificate_der, private_key)
    }

    /// Build an identity from PEM certificate text and encrypted PKCS#8 PEM
    /// private-key text using an explicit password.
    pub fn from_encrypted_pkcs8_pem(
        certificate_pem: &str,
        encrypted_private_key_pem: &str,
        password: &[u8],
    ) -> Result<Self> {
        let certificate = Certificate::from_pem(certificate_pem.as_bytes())
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate PEM: {e}")))?;
        let certificate_der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        let private_key =
            RsaPrivateKey::from_pkcs8_encrypted_pem(encrypted_private_key_pem, password).map_err(
                |e| {
                    WellfriendError::EncryptedPdf(format!(
                "PubSec encrypted PKCS#8 private key PEM could not be decrypted or parsed: {e}"
            ))
                },
            )?;
        Self::from_parts(certificate, &certificate_der, private_key)
    }

    /// Build an identity from a bounded PKCS #12/PFX bundle. The supported
    /// provider profile is narrow by design: one unambiguous matching X.509
    /// certificate plus RSA private key. Unsupported bags or algorithms fail
    /// closed without exposing password, key, or recovered bytes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_pkcs12_der(pfx_der: &[u8], password: &[u8]) -> Result<Self> {
        if pfx_der.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "PubSec PKCS#12/PFX input is empty".to_string(),
            ));
        }
        if pfx_der.len() > MAX_PFX_BYTES {
            return Err(WellfriendError::ResourceLimit(format!(
                "PubSec PKCS#12/PFX input exceeds {MAX_PFX_BYTES} bytes"
            )));
        }
        if password.len() > MAX_PFX_PASSWORD_BYTES {
            return Err(WellfriendError::ResourceLimit(format!(
                "PubSec PKCS#12/PFX password exceeds {MAX_PFX_PASSWORD_BYTES} bytes"
            )));
        }
        let password_str = std::str::from_utf8(password).map_err(|_| {
            WellfriendError::UnsupportedFeature(
                "PubSec PKCS#12/PFX provider currently requires a UTF-8 password".to_string(),
            )
        })?;
        let pfx = p12::PFX::parse(pfx_der).map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec PKCS#12/PFX parse failed: {e}"))
        })?;
        if !pfx.verify_mac(password_str) {
            return Err(WellfriendError::EncryptedPdf(
                "PubSec PKCS#12/PFX password or MAC verification failed".to_string(),
            ));
        }
        let certs = pfx.cert_x509_bags(password_str).map_err(|e| {
            WellfriendError::MalformedPdf(format!(
                "PubSec PKCS#12/PFX certificate extraction failed: {e}"
            ))
        })?;
        let mut keys = pfx.key_bags(password_str).map_err(|e| {
            WellfriendError::MalformedPdf(format!(
                "PubSec PKCS#12/PFX private-key extraction failed: {e}"
            ))
        })?;
        if certs.is_empty() || keys.is_empty() {
            zeroize_key_bags(&mut keys);
            return Err(WellfriendError::EncryptedPdf(
                "PubSec PKCS#12/PFX contains no supported certificate/private-key pair".to_string(),
            ));
        }
        if certs.len() > MAX_PFX_CERTS {
            zeroize_key_bags(&mut keys);
            return Err(WellfriendError::ResourceLimit(format!(
                "PubSec PKCS#12/PFX certificate count exceeds {MAX_PFX_CERTS}"
            )));
        }
        if keys.len() > MAX_PFX_KEYS {
            zeroize_key_bags(&mut keys);
            return Err(WellfriendError::ResourceLimit(format!(
                "PubSec PKCS#12/PFX private-key count exceeds {MAX_PFX_KEYS}"
            )));
        }

        let mut matches = Vec::new();
        for cert_der in &certs {
            if cert_der.len() > MAX_CERTIFICATE_BYTES {
                continue;
            }
            for key_der in &keys {
                if key_der.len() > MAX_PRIVATE_KEY_BYTES {
                    continue;
                }
                if let Ok(identity) = PubSecIdentity::from_der(cert_der, key_der) {
                    matches.push(identity);
                }
            }
        }
        zeroize_key_bags(&mut keys);
        match matches.len() {
            0 => Err(WellfriendError::EncryptedPdf(
                "PubSec PKCS#12/PFX did not contain an unambiguous matching RSA identity"
                    .to_string(),
            )),
            1 => Ok(matches.remove(0)),
            _ => Err(WellfriendError::MalformedPdf(
                "PubSec PKCS#12/PFX contains multiple matching identities; provide explicit certificate/private key to avoid ambiguity".to_string(),
            )),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn from_pkcs12_der(_pfx_der: &[u8], _password: &[u8]) -> Result<Self> {
        Err(WellfriendError::UnsupportedFeature(
            "PubSec PKCS#12/PFX provider is unavailable in WASM builds".to_string(),
        ))
    }

    /// Build from already parsed values. Used by in-process tests and by
    /// future HSM/keystore adapters that never expose private-key bytes.
    pub fn from_rsa_private_key(
        certificate: Certificate,
        private_key: RsaPrivateKey,
    ) -> Result<Self> {
        let certificate_der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        Self::from_parts(certificate, &certificate_der, private_key)
    }

    fn from_parts(
        certificate: Certificate,
        certificate_der: &[u8],
        private_key: RsaPrivateKey,
    ) -> Result<Self> {
        let digest = Sha256::digest(certificate_der);
        let mut certificate_der_sha256 = [0u8; 32];
        certificate_der_sha256.copy_from_slice(&digest);
        let certificate_public_key = certificate_rsa_public_key(&certificate)?;
        let private_public_key = RsaPublicKey::from(&private_key);
        if certificate_public_key.n() != private_public_key.n()
            || certificate_public_key.e() != private_public_key.e()
        {
            return Err(WellfriendError::EncryptedPdf(
                "PubSec certificate and RSA private key do not match".to_string(),
            ));
        }
        let subject_key_identifier = certificate
            .tbs_certificate
            .get::<SubjectKeyIdentifier>()
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate SKI: {e}")))?
            .map(|(_, ski)| ski.0.as_bytes().to_vec());
        Ok(Self {
            certificate,
            certificate_der_sha256,
            subject_key_identifier,
            private_key,
        })
    }

    /// SHA-256 fingerprint of the certificate DER bytes.
    pub fn certificate_fingerprint_sha256(&self) -> [u8; 32] {
        self.certificate_der_sha256
    }

    fn matches_recipient(&self, rid: &RecipientIdentifier) -> bool {
        match rid {
            RecipientIdentifier::IssuerAndSerialNumber(iasn) => issuer_serial_match(
                iasn,
                &self.certificate.tbs_certificate.issuer,
                &self.certificate.tbs_certificate.serial_number,
            ),
            RecipientIdentifier::SubjectKeyIdentifier(ski) => self
                .subject_key_identifier
                .as_deref()
                .is_some_and(|candidate| candidate == ski.0.as_bytes()),
        }
    }
}

fn certificate_rsa_public_key(certificate: &Certificate) -> Result<RsaPublicKey> {
    RsaPublicKey::from_public_key_der(
        &certificate
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .map_err(|e| {
                WellfriendError::MalformedPdf(format!("PubSec certificate SPKI encode: {e}"))
            })?,
    )
    .map_err(|e| {
        WellfriendError::UnsupportedFeature(format!(
            "PubSec certificate public key is not supported RSA: {e}"
        ))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn zeroize_key_bags(keys: &mut [Vec<u8>]) {
    for key in keys {
        key.zeroize();
    }
}

fn parse_certificate_bytes(bytes: &[u8]) -> Result<(Certificate, Vec<u8>)> {
    if looks_like_pem(bytes) {
        let pem = std::str::from_utf8(bytes).map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate PEM UTF-8: {e}"))
        })?;
        let certificate = Certificate::from_pem(pem.as_bytes())
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate PEM: {e}")))?;
        let der = certificate.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec certificate encode: {e}"))
        })?;
        Ok((certificate, der))
    } else {
        let certificate = Certificate::from_der(bytes)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec certificate DER: {e}")))?;
        Ok((certificate, bytes.to_vec()))
    }
}

fn parse_rsa_private_key_bytes(bytes: &[u8]) -> Result<RsaPrivateKey> {
    if looks_like_pem(bytes) {
        let pem = std::str::from_utf8(bytes).map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec private-key PEM UTF-8: {e}"))
        })?;
        RsaPrivateKey::from_pkcs8_pem(pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
            .map_err(|e| {
                WellfriendError::UnsupportedFeature(format!(
                    "PubSec RSA private key PEM parse failed or encrypted key unsupported: {e}"
                ))
            })
    } else {
        RsaPrivateKey::from_pkcs8_der(bytes)
            .or_else(|_| RsaPrivateKey::from_pkcs1_der(bytes))
            .map_err(|e| {
                WellfriendError::UnsupportedFeature(format!(
                    "PubSec RSA private key DER parse failed or encrypted key unsupported: {e}"
                ))
            })
    }
}

fn looks_like_pem(bytes: &[u8]) -> bool {
    bytes.starts_with(b"-----BEGIN ")
}

fn issuer_serial_match(
    iasn: &IssuerAndSerialNumber,
    issuer: &x509_cert::name::Name,
    serial: &x509_cert::serial_number::SerialNumber,
) -> bool {
    &iasn.issuer == issuer && &iasn.serial_number == serial
}

/// Explicit public-key provider used when opening `/Adobe.PubSec` PDFs.
pub struct PubSecKeyProvider {
    identities: Vec<PubSecIdentity>,
}

impl PubSecKeyProvider {
    pub fn new(identities: Vec<PubSecIdentity>) -> Result<Self> {
        if identities.is_empty() {
            return Err(WellfriendError::EncryptedPdf(
                "PubSec key provider has no candidate identities".to_string(),
            ));
        }
        if identities.len() > MAX_RECIPIENTS {
            return Err(WellfriendError::ResourceLimit(format!(
                "PubSec key provider candidate count exceeds {MAX_RECIPIENTS}"
            )));
        }
        Ok(Self { identities })
    }

    pub fn single(identity: PubSecIdentity) -> Self {
        Self {
            identities: vec![identity],
        }
    }

    pub fn identities(&self) -> &[PubSecIdentity] {
        &self.identities
    }
}

/// Parsed public-key encryption dictionary subset needed to create the reader
/// encryption context after CMS recipient recovery.
#[derive(Debug, Clone)]
pub struct PubSecEncryptionInfo {
    pub v: u8,
    pub subfilter: String,
    pub key_length: usize,
    pub encrypt_metadata: bool,
    pub stream_method: CryptMethod,
    pub string_method: CryptMethod,
    pub embedded_file_method: CryptMethod,
    pub crypt_filters: HashMap<String, CryptMethod>,
    recipient_sets: Vec<PubSecRecipientSet>,
}

#[derive(Debug, Clone)]
struct PubSecRecipientSet {
    source: PubSecRecipientSource,
    cms_der: Vec<u8>,
    digest_recipients: Vec<Vec<u8>>,
    includes_permissions: bool,
    encrypt_metadata: bool,
}

#[derive(Debug, Clone)]
enum PubSecRecipientSource {
    EncryptDictionary,
    CryptFilter(String),
}

/// Recovered document file key plus public-key permission metadata.
pub struct PubSecRecoveredKey {
    pub file_key: SecretBytes,
    pub permissions: Option<u32>,
    pub matched_recipient_source: String,
    pub key_length: usize,
}

/// Parse `/Filter /Adobe.PubSec` encryption dictionaries.
pub fn parse_pubsec_encryption_info(dict: &PdfDictionary) -> Result<PubSecEncryptionInfo> {
    let filter = dict.get_name("Filter").unwrap_or("");
    if filter != "Adobe.PubSec" {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec parser requires /Filter /Adobe.PubSec, got /{filter}"
        )));
    }
    let subfilter = dict
        .get_name("SubFilter")
        .ok_or_else(|| WellfriendError::MalformedPdf("PubSec /SubFilter is required".to_string()))?
        .to_string();
    let v = dict.get_integer("V").unwrap_or(0) as u8;
    let key_length = dict.get_integer("Length").unwrap_or(128) as usize;
    if key_length != 128 && key_length != 256 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec /Length {key_length} unsupported; supported lengths are 128 and 256 bits"
        )));
    }
    let encrypt_metadata = dict
        .get("EncryptMetadata")
        .and_then(PdfObject::as_bool)
        .unwrap_or(true);

    let mut crypt_filters = HashMap::new();
    let mut recipient_sets = Vec::new();
    match subfilter.as_str() {
        "adbe.pkcs7.s3" | "adbe.pkcs7.s4" => {
            let recipients = recipient_array(dict, "Recipients")?;
            for cms_der in recipients.iter().cloned() {
                recipient_sets.push(PubSecRecipientSet {
                    source: PubSecRecipientSource::EncryptDictionary,
                    cms_der,
                    digest_recipients: recipients.clone(),
                    includes_permissions: true,
                    encrypt_metadata,
                });
            }
            let method = if v == 4 {
                CryptMethod::AesV2
            } else {
                CryptMethod::V2
            };
            Ok(PubSecEncryptionInfo {
                v,
                subfilter,
                key_length,
                encrypt_metadata,
                stream_method: method.clone(),
                string_method: method.clone(),
                embedded_file_method: method,
                crypt_filters,
                recipient_sets,
            })
        }
        "adbe.pkcs7.s5" => {
            let (stream_name, string_name, embedded_name) = crypt_filter_names(dict);
            let cf = dict.get_dict("CF").ok_or_else(|| {
                WellfriendError::MalformedPdf("PubSec adbe.pkcs7.s5 requires /CF".to_string())
            })?;
            let stream_method = parse_pubsec_filter(
                cf,
                stream_name,
                key_length,
                encrypt_metadata,
                &mut crypt_filters,
                &mut recipient_sets,
            )?;
            let string_method = parse_pubsec_filter(
                cf,
                string_name,
                key_length,
                encrypt_metadata,
                &mut crypt_filters,
                &mut recipient_sets,
            )?;
            let embedded_file_method = parse_pubsec_filter(
                cf,
                embedded_name,
                key_length,
                encrypt_metadata,
                &mut crypt_filters,
                &mut recipient_sets,
            )?;
            if recipient_sets.is_empty() {
                return Err(WellfriendError::MalformedPdf(
                    "PubSec adbe.pkcs7.s5 contains no document-level recipient set".to_string(),
                ));
            }
            Ok(PubSecEncryptionInfo {
                v,
                subfilter,
                key_length,
                encrypt_metadata,
                stream_method,
                string_method,
                embedded_file_method,
                crypt_filters,
                recipient_sets,
            })
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec /SubFilter /{other} is not supported"
        ))),
    }
}

fn crypt_filter_names(dict: &PdfDictionary) -> (&str, &str, &str) {
    let stream = dict.get_name("StmF").unwrap_or("DefaultCryptFilter");
    let string = dict.get_name("StrF").unwrap_or(stream);
    let embedded = dict.get_name("EFF").unwrap_or(stream);
    (stream, string, embedded)
}

fn parse_pubsec_filter(
    cf: &PdfDictionary,
    name: &str,
    key_length: usize,
    inherited_encrypt_metadata: bool,
    crypt_filters: &mut HashMap<String, CryptMethod>,
    recipient_sets: &mut Vec<PubSecRecipientSet>,
) -> Result<CryptMethod> {
    if name == "Identity" {
        crypt_filters.insert(name.to_string(), CryptMethod::None);
        return Ok(CryptMethod::None);
    }
    let filter = cf.get_dict(name).ok_or_else(|| {
        WellfriendError::MalformedPdf(format!("PubSec crypt filter /{name} is missing"))
    })?;
    let method = match filter.get_name("CFM") {
        Some("V2") | None => CryptMethod::V2,
        Some("AESV2") => CryptMethod::AesV2,
        Some("AESV3") => CryptMethod::AesV3,
        Some(other) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "PubSec crypt filter /{name} has unsupported /CFM /{other}"
            )));
        }
    };
    if matches!(method, CryptMethod::AesV3) && key_length != 256 {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec /CFM /AESV3 requires 256-bit /Length, got {key_length}"
        )));
    }
    if !matches!(method, CryptMethod::AesV3) && key_length != 128 {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec /CFM /V2 or /AESV2 requires 128-bit /Length, got {key_length}"
        )));
    }
    let filter_encrypt_metadata = filter
        .get("EncryptMetadata")
        .and_then(PdfObject::as_bool)
        .unwrap_or(inherited_encrypt_metadata);
    let recipients = recipient_array(filter, "Recipients")?;
    for cms_der in recipients.iter().cloned() {
        recipient_sets.push(PubSecRecipientSet {
            source: PubSecRecipientSource::CryptFilter(name.to_string()),
            cms_der,
            digest_recipients: recipients.clone(),
            includes_permissions: true,
            encrypt_metadata: filter_encrypt_metadata,
        });
    }
    crypt_filters.insert(name.to_string(), method.clone());
    Ok(method)
}

fn recipient_array(dict: &PdfDictionary, key: &str) -> Result<Vec<Vec<u8>>> {
    match dict.get(key) {
        Some(PdfObject::Array(items)) => {
            if items.is_empty() {
                return Err(WellfriendError::MalformedPdf(format!(
                    "PubSec /{key} is empty"
                )));
            }
            if items.len() > MAX_RECIPIENTS {
                return Err(WellfriendError::ResourceLimit(format!(
                    "PubSec /{key} recipient count exceeds {MAX_RECIPIENTS}"
                )));
            }
            items
                .iter()
                .map(|item| match item {
                    PdfObject::String(bytes) => {
                        if bytes.len() > MAX_CMS_BYTES {
                            return Err(WellfriendError::ResourceLimit(format!(
                                "PubSec CMS recipient object exceeds {MAX_CMS_BYTES} bytes"
                            )));
                        }
                        Ok(bytes.clone())
                    }
                    other => Err(WellfriendError::MalformedPdf(format!(
                        "PubSec /{key} entry must be a CMS byte string, got {}",
                        other.variant_name()
                    ))),
                })
                .collect()
        }
        Some(PdfObject::String(bytes)) => {
            if bytes.len() > MAX_CMS_BYTES {
                return Err(WellfriendError::ResourceLimit(format!(
                    "PubSec CMS recipient object exceeds {MAX_CMS_BYTES} bytes"
                )));
            }
            Ok(vec![bytes.clone()])
        }
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "PubSec /{key} must be an array or string, got {}",
            other.variant_name()
        ))),
        None => Err(WellfriendError::MalformedPdf(format!(
            "PubSec /{key} is required"
        ))),
    }
}

/// Recover the PDF file encryption key from one matching CMS recipient.
pub fn recover_pubsec_file_key(
    info: &PubSecEncryptionInfo,
    provider: &PubSecKeyProvider,
) -> Result<PubSecRecoveredKey> {
    let mut unsupported = Vec::new();
    for set in &info.recipient_sets {
        match recover_cms_enveloped_payload(&set.cms_der, provider) {
            Ok(mut payload) => {
                let required = PUBSEC_SEED_LEN
                    + if set.includes_permissions {
                        PUBSEC_PERMISSIONS_LEN
                    } else {
                        0
                    };
                if payload.len() != required {
                    payload.zeroize();
                    return Err(WellfriendError::EncryptedPdf(format!(
                        "PubSec CMS payload length {} does not match expected {required}",
                        payload.len()
                    )));
                }
                let seed = &payload[..PUBSEC_SEED_LEN];
                let permissions = if set.includes_permissions {
                    let p = u32::from_be_bytes([
                        payload[PUBSEC_SEED_LEN],
                        payload[PUBSEC_SEED_LEN + 1],
                        payload[PUBSEC_SEED_LEN + 2],
                        payload[PUBSEC_SEED_LEN + 3],
                    ]);
                    Some(p)
                } else {
                    None
                };
                let file_key = derive_pubsec_file_key(
                    seed,
                    &set.digest_recipients,
                    info.key_length,
                    set.encrypt_metadata,
                )?;
                payload.zeroize();
                return Ok(PubSecRecoveredKey {
                    file_key,
                    permissions,
                    matched_recipient_source: match &set.source {
                        PubSecRecipientSource::EncryptDictionary => {
                            "encrypt_dictionary".to_string()
                        }
                        PubSecRecipientSource::CryptFilter(name) => {
                            format!("crypt_filter:{name}")
                        }
                    },
                    key_length: info.key_length,
                });
            }
            Err(err) if err.kind() == crate::error::ErrorKind::UnsupportedFeature => {
                unsupported.push(err.to_string());
            }
            Err(err) if err.kind() == crate::error::ErrorKind::Encrypted => {}
            Err(err) => return Err(err),
        }
    }
    if !unsupported.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec CMS recipient unsupported: {}",
            unsupported.join("; ")
        )));
    }
    Err(WellfriendError::EncryptedPdf(
        "PubSec recipient not found for supplied key provider".to_string(),
    ))
}

/// Write a full-rewrite `/Adobe.PubSec` encrypted PDF using the supported s5
/// KeyTrans profile.
pub fn encrypt_pdf_pubsec(
    reader: &PdfReader,
    options: &PubSecEncryptOptions,
) -> Result<(Vec<u8>, PubSecEncryptReport)> {
    let state = build_pubsec_writer_encryption(options)?;
    let report = PubSecEncryptReport {
        recipient_count: options.recipients.len(),
        crypt_filter: "DefaultCryptFilter".to_string(),
        method: options.method.clone(),
        key_length_bits: match options.method {
            CryptMethod::AesV3 => 256,
            CryptMethod::AesV2 => 128,
            _ => 0,
        },
        permissions: options.permissions,
        encrypt_metadata: options.encrypt_metadata,
    };
    let mut noop = |_orig: u32, _obj: &mut PdfObject| {};
    let (objects, root, info) = rewrite_document_objects(reader, &mut noop)?;
    let bytes = PdfWriter::new(objects, root)
        .with_info(info)
        .with_mode(WriterMode::ClassicXref)
        .with_custom_encryption(state)
        .write()?;
    Ok((bytes, report))
}

/// Re-encrypt a document to a new recipient set. The file key is deliberately
/// rotated because retaining the old key during recipient removal would keep
/// accidental access alive in downstream key caches and artifacts.
pub fn reencrypt_pdf_pubsec(
    reader: &PdfReader,
    options: &PubSecEncryptOptions,
) -> Result<(Vec<u8>, PubSecEncryptReport)> {
    encrypt_pdf_pubsec(reader, options)
}

fn build_pubsec_writer_encryption(
    options: &PubSecEncryptOptions,
) -> Result<PdfWriterCustomEncryption> {
    if options.recipients.is_empty() {
        return Err(WellfriendError::EncryptedPdf(
            "PubSec encryption requires at least one recipient certificate".to_string(),
        ));
    }
    if options.recipients.len() > MAX_RECIPIENTS {
        return Err(WellfriendError::ResourceLimit(format!(
            "PubSec recipient count exceeds {MAX_RECIPIENTS}"
        )));
    }
    let (method_name, key_length_bits, filter_length_bytes) = match options.method {
        CryptMethod::AesV2 => ("AESV2", 128usize, 16usize),
        CryptMethod::AesV3 => ("AESV3", 256usize, 32usize),
        _ => {
            return Err(WellfriendError::UnsupportedFeature(
                "PubSec writer supports only AESV2 and AESV3 crypt filters".to_string(),
            ));
        }
    };

    let mut seen = std::collections::BTreeSet::new();
    for recipient in &options.recipients {
        if !seen.insert(recipient.certificate_der_sha256) {
            return Err(WellfriendError::MalformedPdf(
                "PubSec recipient list contains duplicate certificate fingerprints".to_string(),
            ));
        }
        if matches!(
            options.recipient_id_mode,
            PubSecRecipientIdMode::SubjectKeyIdentifier
        ) && recipient.subject_key_identifier.is_none()
        {
            return Err(WellfriendError::MalformedPdf(
                "PubSec SKI recipient mode requires every certificate to contain a subjectKeyIdentifier".to_string(),
            ));
        }
    }

    let mut seed = secret_bytes(crate::crypto::random_bytes(PUBSEC_SEED_LEN));
    let mut payload = secret_bytes(Vec::with_capacity(PUBSEC_SEED_LEN + PUBSEC_PERMISSIONS_LEN));
    payload.extend_from_slice(&seed);
    payload.extend_from_slice(&options.permissions.to_be_bytes());

    let mut recipient_blobs = Vec::with_capacity(options.recipients.len());
    for recipient in &options.recipients {
        recipient_blobs.push(build_cms_enveloped_data_for_recipient(
            recipient,
            &payload,
            options.recipient_id_mode,
            if key_length_bits == 256 {
                ContentEncryptionAlgorithm::Aes256Cbc
            } else {
                ContentEncryptionAlgorithm::Aes128Cbc
            },
        )?);
    }
    let file_key = derive_pubsec_file_key(
        &seed,
        &recipient_blobs,
        key_length_bits,
        options.encrypt_metadata,
    )?;
    seed.zeroize();
    payload.zeroize();

    let mut crypt_filter = PdfDictionary::empty();
    crypt_filter.insert("Type", PdfObject::Name("CryptFilter".to_string()));
    crypt_filter.insert("CFM", PdfObject::Name(method_name.to_string()));
    crypt_filter.insert("Length", PdfObject::Integer(filter_length_bytes as i64));
    if !options.encrypt_metadata {
        crypt_filter.insert("EncryptMetadata", PdfObject::Boolean(false));
    }
    crypt_filter.insert(
        "Recipients",
        PdfObject::Array(
            recipient_blobs
                .iter()
                .map(|blob| PdfObject::String(blob.clone()))
                .collect(),
        ),
    );

    let mut cf = PdfDictionary::empty();
    cf.insert("DefaultCryptFilter", PdfObject::Dictionary(crypt_filter));

    let mut encrypt_dict = PdfDictionary::empty();
    encrypt_dict.insert("Filter", PdfObject::Name("Adobe.PubSec".to_string()));
    encrypt_dict.insert("SubFilter", PdfObject::Name("adbe.pkcs7.s5".to_string()));
    encrypt_dict.insert("V", PdfObject::Integer(4));
    encrypt_dict.insert("Length", PdfObject::Integer(key_length_bits as i64));
    if !options.encrypt_metadata {
        encrypt_dict.insert("EncryptMetadata", PdfObject::Boolean(false));
    }
    encrypt_dict.insert("CF", PdfObject::Dictionary(cf));
    encrypt_dict.insert("StmF", PdfObject::Name("DefaultCryptFilter".to_string()));
    encrypt_dict.insert("StrF", PdfObject::Name("DefaultCryptFilter".to_string()));

    Ok(PdfWriterCustomEncryption {
        file_key,
        encrypt_dict,
        string_method: options.method.clone(),
        stream_method: options.method.clone(),
        pdf_version: "1.7".to_string(),
    })
}

fn build_cms_enveloped_data_for_recipient(
    recipient: &PubSecRecipientCertificate,
    payload: &[u8],
    id_mode: PubSecRecipientIdMode,
    content_alg: ContentEncryptionAlgorithm,
) -> Result<Vec<u8>> {
    let rid = match id_mode {
        PubSecRecipientIdMode::IssuerAndSerial => {
            RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: recipient.certificate.tbs_certificate.issuer.clone(),
                serial_number: recipient.certificate.tbs_certificate.serial_number.clone(),
            })
        }
        PubSecRecipientIdMode::SubjectKeyIdentifier => {
            let ski = recipient.subject_key_identifier.as_ref().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "PubSec recipient certificate has no subjectKeyIdentifier".to_string(),
                )
            })?;
            RecipientIdentifier::SubjectKeyIdentifier(SubjectKeyIdentifier(
                OctetString::new(ski.clone()).map_err(|e| {
                    WellfriendError::MalformedPdf(format!("PubSec CMS SKI encode: {e}"))
                })?,
            ))
        }
    };
    let mut recipient_rng = OsRng;
    let recipient_info = KeyTransRecipientInfoBuilder::new(
        rid,
        KeyEncryptionInfo::Rsa(recipient.public_key.clone()),
        &mut recipient_rng,
    )
    .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS recipient builder: {e}")))?;
    let mut builder = EnvelopedDataBuilder::new(None, payload, content_alg, None)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS builder: {e}")))?;
    builder
        .add_recipient_info(recipient_info)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS add recipient: {e}")))?;
    let mut content_rng = OsRng;
    let enveloped = builder
        .build_with_rng(&mut content_rng)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS build: {e}")))?;
    let enveloped_der = enveloped
        .to_der()
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS DER: {e}")))?;
    let content = AnyRef::try_from(enveloped_der.as_slice())
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS ContentInfo Any: {e}")))?;
    let ci = ContentInfo {
        content_type: const_oid::db::rfc5911::ID_ENVELOPED_DATA,
        content: Any::from(content),
    };
    ci.to_der()
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS ContentInfo DER: {e}")))
}

fn derive_pubsec_file_key(
    seed: &[u8],
    recipients: &[Vec<u8>],
    key_length: usize,
    encrypt_metadata: bool,
) -> Result<SecretBytes> {
    if seed.len() != PUBSEC_SEED_LEN {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec seed must be {PUBSEC_SEED_LEN} bytes, got {}",
            seed.len()
        )));
    }
    let key_bytes = key_length / 8;
    let mut key = match key_length {
        128 => {
            let mut h = Sha1::new();
            h.update(seed);
            for recipient in recipients {
                h.update(recipient);
            }
            if !encrypt_metadata {
                h.update([0xFFu8; 4]);
            }
            h.finalize().to_vec()
        }
        256 => {
            let mut h = Sha256::new();
            h.update(seed);
            for recipient in recipients {
                h.update(recipient);
            }
            if !encrypt_metadata {
                h.update([0xFFu8; 4]);
            }
            h.finalize().to_vec()
        }
        other => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "PubSec file key length {other} is unsupported"
            )));
        }
    };
    key.truncate(key_bytes);
    Ok(secret_bytes(key))
}

fn recover_cms_enveloped_payload(cms_der: &[u8], provider: &PubSecKeyProvider) -> Result<Vec<u8>> {
    if cms_der.len() > MAX_CMS_BYTES {
        return Err(WellfriendError::ResourceLimit(format!(
            "PubSec CMS object exceeds {MAX_CMS_BYTES} bytes"
        )));
    }
    let ci = ContentInfo::from_der(cms_der)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS ContentInfo: {e}")))?;
    if ci.content_type != const_oid::db::rfc5911::ID_ENVELOPED_DATA {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec CMS content type {} is not EnvelopedData",
            ci.content_type
        )));
    }
    let enveloped_der = ci
        .content
        .to_der()
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS content DER: {e}")))?;
    let enveloped = EnvelopedData::from_der(&enveloped_der)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS EnvelopedData: {e}")))?;
    if enveloped.encrypted_content.content_type != const_oid::db::rfc5911::ID_DATA {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec CMS encrypted content type {} is unsupported",
            enveloped.encrypted_content.content_type
        )));
    }
    if enveloped.recip_infos.0.len() > MAX_RECIPIENTS {
        return Err(WellfriendError::ResourceLimit(format!(
            "PubSec CMS recipient count exceeds {MAX_RECIPIENTS}"
        )));
    }

    let encrypted_content = enveloped
        .encrypted_content
        .encrypted_content
        .as_ref()
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("PubSec CMS encrypted content is absent".to_string())
        })?;

    let mut unsupported = Vec::new();
    for recipient in enveloped.recip_infos.0.iter() {
        let RecipientInfo::Ktri(ktri) = recipient else {
            unsupported.push(format!("recipient type {}", recipient_type_name(recipient)));
            continue;
        };
        for identity in provider.identities() {
            if !identity.matches_recipient(&ktri.rid) {
                continue;
            }
            let mut cek = decrypt_key_transport(ktri, identity)?;
            let result = decrypt_cms_content(
                &enveloped.encrypted_content.content_enc_alg,
                &cek,
                encrypted_content.as_bytes(),
            );
            cek.zeroize();
            return result;
        }
    }
    if !unsupported.is_empty()
        && enveloped
            .recip_infos
            .0
            .iter()
            .all(|ri| !matches!(ri, RecipientInfo::Ktri(_)))
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec CMS unsupported recipient types: {}",
            unsupported.join(", ")
        )));
    }
    Err(WellfriendError::EncryptedPdf(
        "PubSec no matching CMS recipient".to_string(),
    ))
}

fn recipient_type_name(recipient: &RecipientInfo) -> &'static str {
    match recipient {
        RecipientInfo::Ktri(_) => "KeyTransRecipientInfo",
        RecipientInfo::Kari(_) => "KeyAgreeRecipientInfo",
        RecipientInfo::Kekri(_) => "KEKRecipientInfo",
        RecipientInfo::Pwri(_) => "PasswordRecipientInfo",
        RecipientInfo::Ori(_) => "OtherRecipientInfo",
    }
}

fn decrypt_key_transport(
    ktri: &KeyTransRecipientInfo,
    identity: &PubSecIdentity,
) -> Result<Vec<u8>> {
    let encrypted_key = ktri.enc_key.as_bytes();
    if encrypted_key.len() != identity.private_key.size() {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec RSA encrypted key length {} does not match private key size {}",
            encrypted_key.len(),
            identity.private_key.size()
        )));
    }
    let mut rng = OsRng;
    if ktri.key_enc_alg.oid == const_oid::db::rfc5912::RSA_ENCRYPTION {
        return identity
            .private_key
            .decrypt_blinded(&mut rng, Pkcs1v15Encrypt, encrypted_key)
            .map_err(|_| {
                WellfriendError::EncryptedPdf("PubSec RSA key transport failed".to_string())
            });
    }
    if ktri.key_enc_alg.oid == const_oid::db::rfc5912::ID_RSAES_OAEP {
        let padding = oaep_padding_from_params(&ktri.key_enc_alg)?;
        return identity
            .private_key
            .decrypt_blinded(&mut rng, padding, encrypted_key)
            .map_err(|_| {
                WellfriendError::EncryptedPdf("PubSec RSA-OAEP key transport failed".to_string())
            });
    }
    Err(WellfriendError::UnsupportedFeature(format!(
        "PubSec key transport algorithm {} is unsupported",
        ktri.key_enc_alg.oid
    )))
}

#[derive(Debug, Clone, der::Sequence)]
struct RsaesOaepParams {
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT", optional = "true")]
    hash_algorithm: Option<AlgorithmIdentifierOwned>,
    #[asn1(context_specific = "1", tag_mode = "EXPLICIT", optional = "true")]
    mask_gen_algorithm: Option<AlgorithmIdentifierOwned>,
    #[asn1(context_specific = "2", tag_mode = "EXPLICIT", optional = "true")]
    p_source_algorithm: Option<AlgorithmIdentifierOwned>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OaepDigest {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

fn oaep_padding_from_params(alg: &AlgorithmIdentifierOwned) -> Result<Oaep> {
    let params = match alg.parameters.as_ref() {
        None => RsaesOaepParams {
            hash_algorithm: None,
            mask_gen_algorithm: None,
            p_source_algorithm: None,
        },
        Some(params) if params.tag() == der::Tag::Sequence => {
            RsaesOaepParams::from_der(&params.to_der().map_err(|e| {
                WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP params DER: {e}"))
            })?)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP params: {e}")))?
        }
        Some(params) => {
            return Err(WellfriendError::MalformedPdf(format!(
                "PubSec RSA-OAEP parameters must be SEQUENCE, got {:?}",
                params.tag()
            )));
        }
    };
    let hash = match params.hash_algorithm.as_ref() {
        None => OaepDigest::Sha1,
        Some(alg) => parse_oaep_hash_algorithm(alg, "hashAlgorithm")?,
    };
    let mgf_hash = match params.mask_gen_algorithm.as_ref() {
        None => OaepDigest::Sha1,
        Some(alg) => parse_mgf1_hash_algorithm(alg)?,
    };
    validate_oaep_p_source(params.p_source_algorithm.as_ref())?;
    Ok(Oaep {
        digest: oaep_digest_box(hash),
        mgf_digest: oaep_digest_box(mgf_hash),
        label: None,
    })
}

fn parse_oaep_hash_algorithm(
    alg: &AlgorithmIdentifierOwned,
    field: &'static str,
) -> Result<OaepDigest> {
    validate_absent_or_null_params(alg, field)?;
    if alg.oid == const_oid::db::rfc5912::ID_SHA_1 {
        Ok(OaepDigest::Sha1)
    } else if alg.oid == const_oid::db::rfc5912::ID_SHA_256 {
        Ok(OaepDigest::Sha256)
    } else if alg.oid == const_oid::db::rfc5912::ID_SHA_384 {
        Ok(OaepDigest::Sha384)
    } else if alg.oid == const_oid::db::rfc5912::ID_SHA_512 {
        Ok(OaepDigest::Sha512)
    } else {
        Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec RSA-OAEP {field} digest {} is unsupported",
            alg.oid
        )))
    }
}

fn parse_mgf1_hash_algorithm(alg: &AlgorithmIdentifierOwned) -> Result<OaepDigest> {
    if alg.oid != const_oid::db::rfc5912::ID_MGF_1 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec RSA-OAEP maskGenAlgorithm {} is unsupported; only id-mgf1 is supported",
            alg.oid
        )));
    }
    let Some(params) = alg.parameters.as_ref() else {
        return Err(WellfriendError::MalformedPdf(
            "PubSec RSA-OAEP id-mgf1 parameters must contain a hash AlgorithmIdentifier"
                .to_string(),
        ));
    };
    let hash_alg = AlgorithmIdentifierOwned::from_der(&params.to_der().map_err(|e| {
        WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP MGF1 params DER: {e}"))
    })?)
    .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP MGF1 hash params: {e}")))?;
    parse_oaep_hash_algorithm(&hash_alg, "maskGenAlgorithm parameters")
}

fn validate_oaep_p_source(alg: Option<&AlgorithmIdentifierOwned>) -> Result<()> {
    let Some(alg) = alg else {
        return Ok(());
    };
    if alg.oid != const_oid::db::rfc5912::ID_P_SPECIFIED {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "PubSec RSA-OAEP pSourceAlgorithm {} is unsupported; only id-pSpecified is supported",
            alg.oid
        )));
    }
    let Some(params) = alg.parameters.as_ref() else {
        return Err(WellfriendError::MalformedPdf(
            "PubSec RSA-OAEP id-pSpecified parameters are missing".to_string(),
        ));
    };
    if params.tag() != der::Tag::OctetString {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec RSA-OAEP pSpecified parameters must be OCTET STRING, got {:?}",
            params.tag()
        )));
    }
    let label =
        OctetString::from_der(&params.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP label DER: {e}"))
        })?)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec RSA-OAEP label: {e}")))?;
    if !label.as_bytes().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "PubSec RSA-OAEP non-empty pSpecified labels are unsupported".to_string(),
        ));
    }
    Ok(())
}

fn validate_absent_or_null_params(
    alg: &AlgorithmIdentifierOwned,
    field: &'static str,
) -> Result<()> {
    match alg.parameters.as_ref() {
        None => Ok(()),
        Some(params) if params.tag() == der::Tag::Null && params.value().is_empty() => Ok(()),
        Some(params) => Err(WellfriendError::MalformedPdf(format!(
            "PubSec RSA-OAEP {field} digest parameters must be absent or NULL, got {:?}",
            params.tag()
        ))),
    }
}

fn oaep_digest_box(digest: OaepDigest) -> Box<dyn sha2::digest::DynDigest + Send + Sync> {
    match digest {
        OaepDigest::Sha1 => Box::new(Sha1::new()),
        OaepDigest::Sha256 => Box::new(Sha256::new()),
        OaepDigest::Sha384 => Box::new(Sha384::new()),
        OaepDigest::Sha512 => Box::new(Sha512::new()),
    }
}

fn decrypt_cms_content(
    alg: &AlgorithmIdentifierOwned,
    content_key: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let iv = cms_algorithm_iv(alg)?;
    if alg.oid == const_oid::db::rfc5911::ID_AES_128_CBC {
        return decrypt_aes_cbc::<aes::Aes128>(content_key, &iv, ciphertext, 16, "AES-128-CBC");
    }
    if alg.oid == const_oid::db::rfc5911::ID_AES_192_CBC {
        return decrypt_aes_cbc::<aes::Aes192>(content_key, &iv, ciphertext, 24, "AES-192-CBC");
    }
    if alg.oid == const_oid::db::rfc5911::ID_AES_256_CBC {
        return decrypt_aes_cbc::<aes::Aes256>(content_key, &iv, ciphertext, 32, "AES-256-CBC");
    }
    Err(WellfriendError::UnsupportedFeature(format!(
        "PubSec CMS content-encryption algorithm {} is unsupported",
        alg.oid
    )))
}

fn cms_algorithm_iv(alg: &AlgorithmIdentifierOwned) -> Result<Vec<u8>> {
    let Some(params) = alg.parameters.as_ref() else {
        return Err(WellfriendError::MalformedPdf(
            "PubSec CMS AES-CBC parameters are missing".to_string(),
        ));
    };
    if params.tag() != der::Tag::OctetString {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec CMS AES-CBC parameters must be OCTET STRING, got {:?}",
            params.tag()
        )));
    }
    let iv =
        OctetString::from_der(&params.to_der().map_err(|e| {
            WellfriendError::MalformedPdf(format!("PubSec CMS AES-CBC IV DER: {e}"))
        })?)
        .map_err(|e| WellfriendError::MalformedPdf(format!("PubSec CMS AES-CBC IV: {e}")))?;
    if iv.as_bytes().len() != 16 {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec CMS AES-CBC IV must be 16 bytes, got {}",
            iv.as_bytes().len()
        )));
    }
    Ok(iv.as_bytes().to_vec())
}

fn decrypt_aes_cbc<C>(
    content_key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    expected_key_len: usize,
    label: &str,
) -> Result<Vec<u8>>
where
    C: aes::cipher::BlockCipher + aes::cipher::BlockDecryptMut + aes::cipher::KeyInit,
{
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    if content_key.len() != expected_key_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec CMS {label} content key must be {expected_key_len} bytes, got {}",
            content_key.len()
        )));
    }
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
        return Err(WellfriendError::MalformedPdf(format!(
            "PubSec CMS {label} ciphertext length is invalid"
        )));
    }
    let mut buf = ciphertext.to_vec();
    let decryptor = cbc::Decryptor::<C>::new_from_slices(content_key, iv).map_err(|_| {
        WellfriendError::MalformedPdf(format!("PubSec CMS {label} invalid key or IV"))
    })?;
    let plaintext = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| {
            WellfriendError::EncryptedPdf(format!("PubSec CMS {label} content decryption failed"))
        })?
        .to_vec();
    Ok(plaintext)
}

#[cfg(test)]
pub(crate) mod test_support {
    use cms::builder::{
        ContentEncryptionAlgorithm, EnvelopedDataBuilder, KeyEncryptionInfo,
        KeyTransRecipientInfoBuilder,
    };
    use cms::cert::IssuerAndSerialNumber;
    use cms::content_info::ContentInfo;
    use cms::enveloped_data::RecipientIdentifier;
    use der::asn1::Any;
    use der::{AnyRef, Encode};
    use rsa::pkcs1v15::SigningKey;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::rand_core::OsRng;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use sha2::Sha256;
    use spki::SubjectPublicKeyInfoOwned;
    use std::str::FromStr;
    use std::time::Duration;
    use x509_cert::builder::{Builder, CertificateBuilder, Profile};
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::Validity;
    use x509_cert::Certificate;

    pub(crate) fn ephemeral_identity() -> (Certificate, RsaPrivateKey) {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA key");
        let signing_key = SigningKey::<Sha256>::new(private_key.clone());
        let public_key = RsaPublicKey::from(&private_key);
        let spki_der = public_key.to_public_key_der().expect("SPKI DER");
        let spki = SubjectPublicKeyInfoOwned::try_from(spki_der.as_bytes()).expect("SPKI parse");
        let subject = Name::from_str("CN=Wellfriend PubSec Test,O=Wellfriend,C=US").expect("name");
        let validity = Validity::from_now(Duration::from_secs(3600)).expect("validity");
        let builder = CertificateBuilder::new(
            Profile::Leaf {
                issuer: subject.clone(),
                enable_key_agreement: false,
                enable_key_encipherment: true,
            },
            SerialNumber::from(23u32),
            validity,
            subject,
            spki,
            &signing_key,
        )
        .expect("cert builder");
        let cert = builder.build::<rsa::pkcs1v15::Signature>().expect("cert");
        (cert, private_key)
    }

    pub(crate) fn build_cms_enveloped_data(
        cert: &Certificate,
        public_key: RsaPublicKey,
        payload: &[u8],
    ) -> Vec<u8> {
        let rid = RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate.issuer.clone(),
            serial_number: cert.tbs_certificate.serial_number.clone(),
        });
        let mut recipient_rng = OsRng;
        let recipient = KeyTransRecipientInfoBuilder::new(
            rid,
            KeyEncryptionInfo::Rsa(public_key),
            &mut recipient_rng,
        )
        .expect("recipient");
        let mut builder =
            EnvelopedDataBuilder::new(None, payload, ContentEncryptionAlgorithm::Aes128Cbc, None)
                .expect("enveloped builder");
        let mut content_rng = OsRng;
        let enveloped = builder
            .add_recipient_info(recipient)
            .expect("add recipient")
            .build_with_rng(&mut content_rng)
            .expect("build cms");
        let enveloped_der = enveloped.to_der().expect("enveloped der");
        let content = AnyRef::try_from(enveloped_der.as_slice()).expect("any ref");
        let ci = ContentInfo {
            content_type: const_oid::db::rfc5911::ID_ENVELOPED_DATA,
            content: Any::from(content),
        };
        ci.to_der().expect("content info der")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::encrypt_bytes_by_method;
    use crate::ContentEngine;
    use crate::PdfReader;
    use cms::content_info::CmsVersion;
    use cms::enveloped_data::EncryptedKey;
    use rsa::pkcs8::EncodePrivateKey;
    use rsa::RsaPublicKey;

    #[test]
    fn cms_recovers_seed_permissions_and_matches_issuer_serial() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let mut payload = vec![0x42; PUBSEC_SEED_LEN];
        payload.extend_from_slice(&0xFFFF_FFFCu32.to_be_bytes());
        let cms = test_support::build_cms_enveloped_data(
            &cert,
            RsaPublicKey::from(&private_key),
            &payload,
        );
        let provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(cert, private_key).unwrap(),
        );
        let recovered = recover_cms_enveloped_payload(&cms, &provider).unwrap();
        assert_eq!(recovered, payload);
    }

    #[test]
    fn wrong_private_key_does_not_release_payload() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let (wrong_cert, wrong_key) = test_support::ephemeral_identity();
        let mut payload = vec![0x24; PUBSEC_SEED_LEN];
        payload.extend_from_slice(&0xFFFF_FFFCu32.to_be_bytes());
        let cms = test_support::build_cms_enveloped_data(
            &cert,
            RsaPublicKey::from(&private_key),
            &payload,
        );
        let provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(wrong_cert, wrong_key).unwrap(),
        );
        let err = recover_cms_enveloped_payload(&cms, &provider).unwrap_err();
        assert_eq!(err.kind(), crate::error::ErrorKind::Encrypted);
        assert!(!err.to_string().contains("242424"));
    }

    #[test]
    fn rsa_oaep_explicit_sha256_mgf1_sha1_parameters_decrypt_key_transport() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let public_key = RsaPublicKey::from(&private_key);
        let cek = b"0123456789ABCDEF";
        let mut rng = OsRng;
        let encrypted_key = public_key
            .encrypt(&mut rng, Oaep::new_with_mgf_hash::<Sha256, Sha1>(), cek)
            .unwrap();
        let key_enc_alg = oaep_algorithm_identifier(
            Some(const_oid::db::rfc5912::ID_SHA_256),
            Some(const_oid::db::rfc5912::ID_SHA_1),
            true,
        );
        let ktri = KeyTransRecipientInfo {
            version: CmsVersion::V0,
            rid: RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: cert.tbs_certificate.issuer.clone(),
                serial_number: cert.tbs_certificate.serial_number.clone(),
            }),
            key_enc_alg,
            enc_key: EncryptedKey::new(encrypted_key).unwrap(),
        };
        let identity = PubSecIdentity::from_rsa_private_key(cert, private_key).unwrap();
        let decrypted = decrypt_key_transport(&ktri, &identity).unwrap();
        assert_eq!(decrypted, cek);
    }

    #[test]
    fn pubsec_s5_pdf_opens_with_matching_key_and_wrong_key_fails() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let mut seed = vec![0x31; PUBSEC_SEED_LEN];
        let mut payload = seed.clone();
        payload.extend_from_slice(&0xFFFF_FFFCu32.to_be_bytes());
        let cms = test_support::build_cms_enveloped_data(
            &cert,
            RsaPublicKey::from(&private_key),
            &payload,
        );
        let file_key =
            derive_pubsec_file_key(&seed, std::slice::from_ref(&cms), 128, true).unwrap();
        let pdf = build_pubsec_s5_pdf(&cms, &file_key);
        seed.zeroize();

        let provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(cert, private_key).unwrap(),
        );
        let engine = ContentEngine::open_bytes_with_pubsec_provider(pdf.clone(), &provider)
            .expect("matching PubSec key opens document");
        assert!(engine.is_encrypted());
        let text = engine.get_page_text(1).expect("text extracts");
        assert!(
            text.contains("PubSec OK"),
            "decrypted page text missing: {text}"
        );

        let no_provider = match ContentEngine::open_bytes(pdf.clone()) {
            Ok(_) => panic!("PubSec document must not open without provider"),
            Err(err) => err,
        };
        assert_eq!(no_provider.kind(), crate::error::ErrorKind::Encrypted);

        let (wrong_cert, wrong_key) = test_support::ephemeral_identity();
        let wrong_provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(wrong_cert, wrong_key).unwrap(),
        );
        let wrong = match ContentEngine::open_bytes_with_pubsec_provider(pdf, &wrong_provider) {
            Ok(_) => panic!("wrong key must fail"),
            Err(err) => err,
        };
        assert_eq!(wrong.kind(), crate::error::ErrorKind::Encrypted);
        assert!(!wrong.to_string().contains("PubSec OK"));
    }

    #[test]
    fn pubsec_writer_creates_multi_recipient_pdf_and_reopens_each_key() {
        let plain = build_plain_pdf();
        let reader = PdfReader::from_bytes(plain).unwrap();
        let (cert_a, key_a) = test_support::ephemeral_identity();
        let (cert_b, key_b) = test_support::ephemeral_identity();
        let recipient_a = PubSecRecipientCertificate::from_certificate(cert_a.clone()).unwrap();
        let recipient_b = PubSecRecipientCertificate::from_certificate(cert_b.clone()).unwrap();
        let options = PubSecEncryptOptions {
            recipients: vec![recipient_a, recipient_b],
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: CryptMethod::AesV2,
            recipient_id_mode: PubSecRecipientIdMode::IssuerAndSerial,
        };
        let (encrypted, report) = encrypt_pdf_pubsec(&reader, &options).unwrap();
        assert_eq!(report.recipient_count, 2);
        assert!(encrypted
            .windows(b"Adobe.PubSec".len())
            .any(|w| w == b"Adobe.PubSec"));

        let provider_a =
            PubSecKeyProvider::single(PubSecIdentity::from_rsa_private_key(cert_a, key_a).unwrap());
        let text_a = ContentEngine::open_bytes_with_pubsec_provider(encrypted.clone(), &provider_a)
            .unwrap()
            .get_page_text(1)
            .unwrap();
        assert!(text_a.contains("PubSec OK"));

        let provider_b =
            PubSecKeyProvider::single(PubSecIdentity::from_rsa_private_key(cert_b, key_b).unwrap());
        let text_b = ContentEngine::open_bytes_with_pubsec_provider(encrypted.clone(), &provider_b)
            .unwrap()
            .get_page_text(1)
            .unwrap();
        assert!(text_b.contains("PubSec OK"));

        let (wrong_cert, wrong_key) = test_support::ephemeral_identity();
        let wrong_provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(wrong_cert, wrong_key).unwrap(),
        );
        let err = match ContentEngine::open_bytes_with_pubsec_provider(encrypted, &wrong_provider) {
            Ok(_) => panic!("wrong recipient must not open PubSec output"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), crate::error::ErrorKind::Encrypted);
    }

    #[test]
    fn pubsec_writer_rejects_duplicate_recipient_certificate() {
        let plain = build_plain_pdf();
        let reader = PdfReader::from_bytes(plain).unwrap();
        let (cert, _) = test_support::ephemeral_identity();
        let recipient = PubSecRecipientCertificate::from_certificate(cert).unwrap();
        let options = PubSecEncryptOptions {
            recipients: vec![recipient.clone(), recipient],
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: CryptMethod::AesV2,
            recipient_id_mode: PubSecRecipientIdMode::IssuerAndSerial,
        };
        let err = match encrypt_pdf_pubsec(&reader, &options) {
            Ok(_) => panic!("duplicate recipient fails"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), crate::error::ErrorKind::MalformedPdf);
        assert!(!err.to_string().contains("PRIVATE KEY"));
    }

    #[test]
    fn pubsec_provider_loads_encrypted_pkcs8_and_rejects_wrong_password() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let cert_der = cert.to_der().unwrap();
        let password = b"pkcs8 test password";
        let mut rng = OsRng;
        let encrypted = private_key
            .to_pkcs8_encrypted_der(&mut rng, password)
            .unwrap();
        let identity =
            PubSecIdentity::from_encrypted_pkcs8_der(&cert_der, encrypted.as_bytes(), password)
                .unwrap();
        assert_eq!(
            identity.certificate_fingerprint_sha256(),
            PubSecRecipientCertificate::from_certificate(cert)
                .unwrap()
                .certificate_fingerprint_sha256()
        );
        let err = match PubSecIdentity::from_encrypted_pkcs8_der(
            &cert_der,
            encrypted.as_bytes(),
            b"wrong password",
        ) {
            Ok(_) => panic!("wrong encrypted PKCS#8 password must fail"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), crate::error::ErrorKind::Encrypted);
        assert!(!err.to_string().contains("pkcs8 test password"));
        assert!(!err.to_string().contains("wrong password"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pubsec_provider_loads_pkcs12_and_rejects_wrong_password() {
        let (cert, private_key) = test_support::ephemeral_identity();
        let cert_der = cert.to_der().unwrap();
        let key_der = private_key.to_pkcs8_der().unwrap();
        let password = "pfx test password";
        let pfx = p12::PFX::new(
            &cert_der,
            key_der.as_bytes(),
            None,
            password,
            "wellfriendpdf-pubsec",
        )
        .unwrap()
        .to_der();

        let identity = PubSecIdentity::from_pkcs12_der(&pfx, password.as_bytes()).unwrap();
        assert_eq!(
            identity.certificate_fingerprint_sha256(),
            PubSecRecipientCertificate::from_certificate(cert)
                .unwrap()
                .certificate_fingerprint_sha256()
        );

        let err = match PubSecIdentity::from_pkcs12_der(&pfx, b"wrong password") {
            Ok(_) => panic!("wrong PKCS#12 password must fail"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), crate::error::ErrorKind::Encrypted);
        assert!(!err.to_string().contains(password));
        assert!(!err.to_string().contains("wrong password"));
    }

    #[test]
    fn pubsec_identity_rejects_mismatched_certificate_private_key() {
        let (cert, _) = test_support::ephemeral_identity();
        let (_, wrong_key) = test_support::ephemeral_identity();
        let cert_der = cert.to_der().unwrap();
        let key_der = wrong_key.to_pkcs8_der().unwrap();
        let err = match PubSecIdentity::from_der(&cert_der, key_der.as_bytes()) {
            Ok(_) => panic!("mismatched certificate/key pair must fail"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), crate::error::ErrorKind::Encrypted);
        assert!(!err.to_string().contains("PRIVATE KEY"));
    }

    #[test]
    fn pubsec_reencrypt_rotates_recipient_access_on_removal() {
        let plain = build_plain_pdf();
        let reader = PdfReader::from_bytes(plain).unwrap();
        let (old_cert, old_key) = test_support::ephemeral_identity();
        let (new_cert, new_key) = test_support::ephemeral_identity();
        let old_recipient = PubSecRecipientCertificate::from_certificate(old_cert.clone()).unwrap();
        let new_recipient = PubSecRecipientCertificate::from_certificate(new_cert.clone()).unwrap();
        let initial = PubSecEncryptOptions {
            recipients: vec![old_recipient],
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: CryptMethod::AesV2,
            recipient_id_mode: PubSecRecipientIdMode::IssuerAndSerial,
        };
        let (encrypted, _) = encrypt_pdf_pubsec(&reader, &initial).unwrap();
        let old_provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(old_cert.clone(), old_key.clone()).unwrap(),
        );
        let opened =
            ContentEngine::open_bytes_with_pubsec_provider(encrypted, &old_provider).unwrap();
        let rotated = PubSecEncryptOptions {
            recipients: vec![new_recipient],
            permissions: 0xFFFF_FFFCu32,
            encrypt_metadata: true,
            method: CryptMethod::AesV2,
            recipient_id_mode: PubSecRecipientIdMode::IssuerAndSerial,
        };
        let (reencrypted, _) = reencrypt_pdf_pubsec(opened.document().reader(), &rotated).unwrap();

        let old_err = match ContentEngine::open_bytes_with_pubsec_provider(
            reencrypted.clone(),
            &old_provider,
        ) {
            Ok(_) => panic!("removed PubSec recipient must not open re-encrypted output"),
            Err(err) => err,
        };
        assert_eq!(old_err.kind(), crate::error::ErrorKind::Encrypted);

        let new_provider = PubSecKeyProvider::single(
            PubSecIdentity::from_rsa_private_key(new_cert, new_key).unwrap(),
        );
        let text = ContentEngine::open_bytes_with_pubsec_provider(reencrypted, &new_provider)
            .unwrap()
            .get_page_text(1)
            .unwrap();
        assert!(text.contains("PubSec OK"));
    }

    fn build_pubsec_s5_pdf(cms: &[u8], file_key: &[u8]) -> Vec<u8> {
        let content = b"BT /F1 12 Tf 72 720 Td (PubSec OK) Tj ET";
        let encrypted_content =
            encrypt_bytes_by_method(content, file_key, 4, 0, &CryptMethod::AesV2)
                .expect("encrypt content");
        let cms_hex = hex(cms);
        let mut objects = Vec::new();
        objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
        objects.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec());
        let mut stream =
            format!("<< /Length {} >>\nstream\n", encrypted_content.len()).into_bytes();
        stream.extend_from_slice(&encrypted_content);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
        objects.push(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .as_bytes()
                .to_vec(),
        );
        objects.push(format!(
            "<< /Filter /Adobe.PubSec /SubFilter /adbe.pkcs7.s5 /V 4 /Length 128 /CF << /DefaultCryptFilter << /Type /CryptFilter /CFM /AESV2 /Length 16 /Recipients [<{}>] >> >> /StmF /DefaultCryptFilter /StrF /DefaultCryptFilter >>",
            cms_hex
        ).into_bytes());

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let mut offsets = vec![0usize];
        for (idx, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Encrypt 6 0 R /ID [<0123456789ABCDEF0123456789ABCDEF><0123456789ABCDEF0123456789ABCDEF>] >>\nstartxref\n{}\n%%EOF\n",
                objects.len() + 1,
                xref
            )
            .as_bytes(),
        );
        out
    }

    fn build_plain_pdf() -> Vec<u8> {
        let content = b"BT /F1 12 Tf 72 720 Td (PubSec OK) Tj ET";
        let mut objects = Vec::new();
        objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
        objects.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec());
        let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend_from_slice(content);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
        objects.push(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .as_bytes()
                .to_vec(),
        );

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let mut offsets = vec![0usize];
        for (idx, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                objects.len() + 1,
                xref
            )
            .as_bytes(),
        );
        out
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

    fn oaep_algorithm_identifier(
        hash_oid: Option<const_oid::ObjectIdentifier>,
        mgf_hash_oid: Option<const_oid::ObjectIdentifier>,
        include_empty_psource: bool,
    ) -> AlgorithmIdentifierOwned {
        let hash_algorithm = hash_oid.map(hash_algorithm_identifier);
        let mask_gen_algorithm = mgf_hash_oid.map(|oid| {
            let inner = hash_algorithm_identifier(oid);
            let inner_der = inner.to_der().unwrap();
            AlgorithmIdentifierOwned {
                oid: const_oid::db::rfc5912::ID_MGF_1,
                parameters: Some(Any::from(
                    AnyRef::try_from(inner_der.as_slice()).expect("hash any"),
                )),
            }
        });
        let p_source_algorithm = include_empty_psource.then(|| AlgorithmIdentifierOwned {
            oid: const_oid::db::rfc5912::ID_P_SPECIFIED,
            parameters: Some(Any::new(der::Tag::OctetString, Vec::new()).unwrap()),
        });
        let params = RsaesOaepParams {
            hash_algorithm,
            mask_gen_algorithm,
            p_source_algorithm,
        };
        let params_der = params.to_der().unwrap();
        AlgorithmIdentifierOwned {
            oid: const_oid::db::rfc5912::ID_RSAES_OAEP,
            parameters: Some(Any::from(
                AnyRef::try_from(params_der.as_slice()).expect("oaep params any"),
            )),
        }
    }

    fn hash_algorithm_identifier(oid: const_oid::ObjectIdentifier) -> AlgorithmIdentifierOwned {
        AlgorithmIdentifierOwned {
            oid,
            parameters: None,
        }
    }
}
