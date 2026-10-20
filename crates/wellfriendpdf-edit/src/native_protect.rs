//! Native R6/AES-256 password protection. Go retains job and publication authority.
use crate::{
    PdfEditError, SignatureEffect, is_writer_housekeeping_object, native_unlock,
    pdf_objects_semantically_equal, save_new_pdf, sha256_file, signature_summaries,
};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, LoadOptions, Object, Permissions,
};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

pub const MAX_PAGE_COUNT: usize = 2_000;

#[derive(Clone, Copy, Serialize)]
pub struct ProtectionPermissions {
    pub print: bool,
    pub copy: bool,
    pub edit: bool,
    pub annotate: bool,
}

impl ProtectionPermissions {
    fn native(self) -> Permissions {
        let mut permissions = Permissions::COPYABLE_FOR_ACCESSIBILITY;
        if self.print {
            permissions |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY;
        }
        if self.copy {
            permissions |= Permissions::COPYABLE;
        }
        // The original modify=annotate level also permits assembly and filling
        // existing fields. modify=all includes all of these even when the
        // separate reported allowAnnotate option was false.
        if self.edit || self.annotate {
            permissions |=
                Permissions::ANNOTABLE | Permissions::FILLABLE | Permissions::ASSEMBLABLE;
        }
        if self.edit {
            permissions |= Permissions::MODIFIABLE;
        }
        permissions
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectResult {
    pub output_sha256: String,
    pub output_bytes: u64,
    pub page_count: usize,
    pub signature_effect: SignatureEffect,
    pub page_content_preserved: bool,
    pub decrypted_objects_preserved: bool,
    pub encryption_verified: bool,
    pub permission_bits: i64,
}

fn version_and_identifier(document: &mut Document) -> Result<(), PdfEditError> {
    if !document.trailer.has(b"ID") {
        // Identifier randomness is independent of the private encryption key.
        let mut identifier = [0_u8; 32];
        getrandom::fill(&mut identifier).map_err(|_| PdfEditError::OutputWriteFailed)?;
        document.trailer.set(
            "ID",
            Object::Array(vec![
                Object::string_literal(identifier[..16].to_vec()),
                Object::string_literal(identifier[16..].to_vec()),
            ]),
        );
    }
    let mut version = document.version.split('.');
    let major = version
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(PdfEditError::InvalidPdf)?;
    let minor = version
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(PdfEditError::InvalidPdf)?;
    if major < 1 || major == 1 && minor < 7 {
        document.version = "1.7".into();
    }
    if major < 2 {
        let catalog = document.catalog().map_err(|_| PdfEditError::InvalidPdf)?;
        let mut extensions = if catalog.has(b"Extensions") {
            document
                .get_dict_in_dict(catalog, b"Extensions")
                .map_err(|_| PdfEditError::InvalidPdf)?
                .clone()
        } else {
            Dictionary::new()
        };
        let mut adobe = if extensions.has(b"ADBE") {
            document
                .get_dict_in_dict(&extensions, b"ADBE")
                .map_err(|_| PdfEditError::InvalidPdf)?
                .clone()
        } else {
            Dictionary::new()
        };
        let level = adobe
            .get(b"ExtensionLevel")
            .and_then(Object::as_i64)
            .unwrap_or(0);
        if level < 8 {
            adobe.set("BaseVersion", Object::Name(b"1.7".to_vec()));
            adobe.set("ExtensionLevel", 8_i64);
        }
        extensions.set("ADBE", adobe);
        document
            .catalog_mut()
            .map_err(|_| PdfEditError::InvalidPdf)?
            .set("Extensions", extensions);
    }
    Ok(())
}

pub fn write(
    document: &mut Document,
    output: &Path,
    password: &str,
    permissions: ProtectionPermissions,
) -> Result<ProtectResult, PdfEditError> {
    if document.is_encrypted() || password.is_empty() || password.chars().count() > 256 {
        return Err(PdfEditError::EncryptedPdf);
    }
    let page_count = document.get_pages().len();
    if page_count == 0 || page_count > MAX_PAGE_COUNT {
        return Err(PdfEditError::PageLimitExceeded);
    }
    let signature_effect = if signature_summaries(document).is_empty() {
        SignatureEffect::None
    } else {
        SignatureEffect::InvalidatesExistingSignatures
    };
    version_and_identifier(document)?;
    let source = document.objects.clone();
    let trailer = document.trailer.clone();
    let pages = document.get_pages();
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), crypt_filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: password,
        user_password: password,
        permissions: permissions.native(),
    });
    key.fill(0);
    let state = state.map_err(|_| PdfEditError::ValidationFailed)?;
    let encoded = state.encode().map_err(|_| PdfEditError::ValidationFailed)?;
    let permission_bits = encoded
        .get(b"P")
        .and_then(Object::as_i64)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    document
        .encrypt(&state)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let parent = output.parent().ok_or(PdfEditError::OutputWriteFailed)?;
    let scratch = tempfile::Builder::new()
        .prefix("native-pdf-protect-")
        .tempdir_in(parent)
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let staged = scratch.path().join("protected.pdf");
    save_new_pdf(document, &staged)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    // A published protected output must reject empty access and authenticate the
    // supplied password. Authentication alone does not prove preserved content.
    if native_unlock::load(&staged, "").is_ok() {
        return Err(PdfEditError::ValidationFailed);
    }
    let mut options = LoadOptions::with_max_decompressed_size(64 * 1024 * 1024);
    options.password = Some(password.to_owned());
    let encrypted_read = Document::load_with_options(&staged, options)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let verified_encryption = encrypted_read
        .encryption_state
        .as_ref()
        .ok_or(PdfEditError::ValidationFailed)?
        .encode()
        .map_err(|_| PdfEditError::ValidationFailed)?;
    for field in [
        b"V".as_slice(),
        b"R".as_slice(),
        b"Length".as_slice(),
        b"P".as_slice(),
        b"CF".as_slice(),
        b"StmF".as_slice(),
        b"StrF".as_slice(),
        b"EncryptMetadata".as_slice(),
    ] {
        if encoded.get(field).ok() != verified_encryption.get(field).ok() {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    let verified = native_unlock::load(&staged, password)?;
    if verified.get_pages() != pages
        || source.iter().any(|(id, object)| {
            !is_writer_housekeeping_object(object)
                && !object.type_name().is_ok_and(|kind| kind == b"Linearized")
                && !verified
                    .objects
                    .get(id)
                    .is_some_and(|after| pdf_objects_semantically_equal(object, after))
        })
    {
        return Err(PdfEditError::ValidationFailed);
    }
    for (name, object) in trailer.iter() {
        let packing = name.as_slice() == b"Size"
            || trailer
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|kind| kind == b"XRef")
                && matches!(
                    name.as_slice(),
                    b"Type" | b"W" | b"Index" | b"Length" | b"Filter"
                );
        if !packing
            && !verified
                .trailer
                .get(name)
                .is_ok_and(|after| pdf_objects_semantically_equal(object, after))
        {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    let output_sha256 = sha256_file(&staged)?;
    let output_bytes = fs::metadata(&staged)
        .map_err(|_| PdfEditError::OutputWriteFailed)?
        .len();
    fs::hard_link(&staged, output).map_err(|_| PdfEditError::OutputWriteFailed)?;
    Ok(ProtectResult {
        output_sha256,
        output_bytes,
        page_count,
        signature_effect,
        page_content_preserved: true,
        decrypted_objects_preserved: true,
        encryption_verified: true,
        permission_bits,
    })
}
