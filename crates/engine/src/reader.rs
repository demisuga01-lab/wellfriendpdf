use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::crypto::{
    aes256_gcm_decrypt_pdf_object, compute_encryption_key, decrypt_string,
    derive_v5_file_key_from_owner, derive_v5_file_key_from_user, verify_user_password,
    verify_v5_owner_password, verify_v5_perms, verify_v5_user_password, CryptMethod,
    EncryptionInfo, SecretBytes,
};
use crate::decode_scanner::{find_marker_accelerated, rfind_marker_accelerated};
use crate::error::{OxideError, Result};
use crate::filters::decode_stream_from_dict;
use crate::object::{PdfDictionary, PdfObject};
use crate::parser::{ParserResolver, PdfParser};
use crate::parser_report::{ParserCategory, ParserDiagnostic, ParserSeverity, ParserSourceMetrics};
use crate::pubsec::{parse_pubsec_encryption_info, recover_pubsec_file_key, PubSecKeyProvider};

const MAX_FALLBACK_XREF_OBJECTS: usize = 200_000;
const STREAMING_TAIL_READ_LIMIT: usize = 16 * 1024 * 1024;
const STREAMING_XREF_READ_LIMIT: usize = 64 * 1024 * 1024;
const STREAMING_FULL_READ_FALLBACK_LIMIT: u64 = 128 * 1024 * 1024;
const STREAMING_STREAM_HEADER_READ_LIMIT: usize = 1024 * 1024;
const DEFAULT_OBJECT_STREAM_CACHE_LIMIT: usize = 32;
const MAX_XREF_CHAIN_DEPTH: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XrefEntry {
    Free,
    Uncompressed { offset: usize },
    Compressed { stream_obj: u32, index: u32 },
}

/// Active decryption state for a Standard-Security-Handler encrypted PDF.
///
/// Built once during [`PdfReader::from_bytes_with_password`] after the user
/// password is verified. Every object read through [`PdfReader::get_object`]
/// has its strings and stream bytes decrypted transparently.
#[derive(Clone, Debug)]
pub struct EncryptionContext {
    /// The file-wide encryption key (5 bytes for 40-bit, 16 bytes for 128-bit,
    /// or 32 bytes for V5/AES-256).
    pub file_key: SecretBytes,
    /// True when streams and strings are encrypted with AES-128 (`/CFM /AESV2`).
    pub is_aes: bool,
    /// True when this is a V5 (AES-256) document. For V5 the file key is used
    /// directly for every object — no per-object key derivation.
    pub is_v5: bool,
    /// Crypt filter method for ordinary streams (`/StmF`).
    pub stream_method: CryptMethod,
    /// Crypt filter method for strings (`/StrF`).
    pub string_method: CryptMethod,
    /// Crypt filter method for embedded-file streams (`/EFF`).
    pub embedded_file_method: CryptMethod,
    /// Named crypt filters from `/CF`, used by explicit `/Filter /Crypt`
    /// stream filters.
    pub crypt_filters: HashMap<String, CryptMethod>,
    /// Mirrors `/EncryptMetadata`; when false, `/Type /Metadata` streams are
    /// left as plaintext.
    pub encrypt_metadata: bool,
}

type ParsedObjectStream = HashMap<u32, (u32, PdfObject)>;

struct BoundedObjectStreamCache {
    streams: HashMap<u32, ParsedObjectStream>,
    order: VecDeque<u32>,
    max_streams: usize,
}

impl BoundedObjectStreamCache {
    fn new(max_streams: usize) -> Self {
        Self {
            streams: HashMap::new(),
            order: VecDeque::new(),
            max_streams: max_streams.max(1),
        }
    }

    fn contains_key(&self, stream_obj: &u32) -> bool {
        self.streams.contains_key(stream_obj)
    }

    fn get(&self, stream_obj: &u32) -> Option<&ParsedObjectStream> {
        self.streams.get(stream_obj)
    }

    fn insert(&mut self, stream_obj: u32, objects: ParsedObjectStream) {
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            self.streams.entry(stream_obj)
        {
            entry.insert(objects);
            return;
        }
        while self.streams.len() >= self.max_streams {
            let Some(victim) = self.order.pop_front() else {
                break;
            };
            self.streams.remove(&victim);
        }
        self.order.push_back(stream_obj);
        self.streams.insert(stream_obj, objects);
    }
}

enum PdfSource {
    Memory(Vec<u8>),
    File(SeekableFileSource),
}

struct SeekableFileSource {
    path: PathBuf,
    file: File,
    len: usize,
    raw_cache: OnceLock<Vec<u8>>,
}

pub(crate) struct PdfRangeReader<'a> {
    source: &'a PdfSource,
    offset: usize,
    remaining: usize,
}

pub(crate) struct ContentStreamRange<'a> {
    pub dict: PdfDictionary,
    pub reader: PdfRangeReader<'a>,
}

impl PdfSource {
    fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let len = file.metadata()?.len();
        let len = usize::try_from(len).map_err(|_| {
            OxideError::ResourceLimit("input file is too large for this platform".to_string())
        })?;
        Ok(Self::File(SeekableFileSource {
            path,
            file,
            len,
            raw_cache: OnceLock::new(),
        }))
    }

    fn len(&self) -> usize {
        match self {
            Self::Memory(data) => data.len(),
            Self::File(source) => source.len,
        }
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Memory(data) => Some(data),
            Self::File(_) => None,
        }
    }

    fn file_bytes(&self) -> &[u8] {
        match self {
            Self::Memory(data) => data,
            Self::File(source) => source.raw_cache.get_or_init(|| {
                source.read_all().unwrap_or_else(|err| {
                    log::warn!(
                        "could not materialize original file bytes for {}: {}",
                        source.path.display(),
                        err
                    );
                    Vec::new()
                })
            }),
        }
    }

    fn read_prefix(&self, max_len: usize) -> Result<Vec<u8>> {
        self.read_at(0, self.len().min(max_len))
    }

    fn read_tail(&self, max_len: usize) -> Result<Vec<u8>> {
        let len = self.len();
        let read_len = len.min(max_len);
        self.read_at(len - read_len, read_len)
    }

    fn read_from(&self, offset: usize, max_len: usize) -> Result<Vec<u8>> {
        if offset > self.len() {
            return Err(OxideError::ParseError(format!(
                "offset {offset} is beyond input length {}",
                self.len()
            )));
        }
        let len = (self.len() - offset).min(max_len);
        self.read_at(offset, len)
    }

    fn read_at(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| OxideError::MalformedPdf("read range overflows".to_string()))?;
        if end > self.len() {
            return Err(OxideError::ParseError(format!(
                "read range {offset}..{end} exceeds input length {}",
                self.len()
            )));
        }
        match self {
            Self::Memory(data) => Ok(data[offset..end].to_vec()),
            Self::File(source) => source.read_at(offset, len),
        }
    }
}

impl SeekableFileSource {
    fn read_at(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; len];
        read_exact_at(&self.file, &mut out, offset as u64)?;
        Ok(out)
    }

    fn read_all(&self) -> Result<Vec<u8>> {
        self.read_at(0, self.len)
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = file.read_at(buf, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "short read from PDF source",
            ));
        }
        offset += n as u64;
        let (_, rest) = buf.split_at_mut(n);
        buf = rest;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = file.seek_read(buf, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "short read from PDF source",
            ));
        }
        offset += n as u64;
        let (_, rest) = buf.split_at_mut(n);
        buf = rest;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom};

    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buf)
}

impl Read for PdfRangeReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let len = self.remaining.min(buf.len());
        let bytes = self
            .source
            .read_at(self.offset, len)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        self.offset += bytes.len();
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
}

pub struct PdfReader {
    source: PdfSource,
    version: String,
    xref: HashMap<(u32, u16), XrefEntry>,
    trailer: PdfDictionary,
    /// Cache of decoded object streams (`/Type /ObjStm`). Wrapped in an
    /// `RwLock` rather than a `RefCell` so the whole `PdfReader` — and therefore
    /// `ContentEngine` — is `Send + Sync`. This lets a single parsed engine be
    /// shared across rayon threads via `Arc` for parallel page extraction and
    /// rendering instead of cloning/reparsing the PDF per
    /// thread. Reads dominate; the lock is only taken for writing the first time
    /// a given object stream is decoded.
    object_stream_cache: RwLock<BoundedObjectStreamCache>,
    encryption: Option<EncryptionContext>,
    startxref: usize,
    diagnostics: Vec<ParserDiagnostic>,
}

impl PdfReader {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_path_with_password(path, b"")
    }

    /// Open a PDF from a file path, supplying a user password for encrypted
    /// documents. For non-encrypted PDFs the password is ignored.
    pub fn from_path_with_password(path: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let path = path.as_ref();
        let metadata_len = fs::metadata(path)?.len();
        let source = PdfSource::from_path(path)?;
        match Self::from_seekable_source_with_password(source, password) {
            Ok(reader) => Ok(reader),
            Err(primary) if metadata_len <= STREAMING_FULL_READ_FALLBACK_LIMIT => {
                match Self::from_bytes_with_password(fs::read(path)?, password) {
                    Ok(mut reader) => {
                        reader.diagnostics.push(
                            ParserDiagnostic::new(
                                ParserSeverity::RecoverableError,
                                ParserCategory::Source,
                                "streaming_open_fell_back_to_full_read",
                                "file-backed range open failed; reopened through bounded full-read fallback",
                            )
                            .with_recovery("materialized file because it was below the configured fallback limit")
                            .incomplete(),
                        );
                        Ok(reader)
                    }
                    Err(_) => Err(primary),
                }
            }
            Err(primary) => Err(primary),
        }
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with_password(data, b"")
    }

    /// Open a PDF from bytes in strict parser mode.
    ///
    /// Strict mode requires a discoverable `startxref`, a readable xref chain,
    /// and a trailer dictionary. It intentionally does not run the object-scan
    /// repair fallback that [`Self::from_bytes`] uses for damaged files.
    pub fn from_bytes_strict(data: Vec<u8>) -> Result<Self> {
        Self::from_bytes_strict_with_password(data, b"")
    }

    /// Strict parser-mode variant of [`Self::from_bytes_with_password`].
    pub fn from_bytes_strict_with_password(data: Vec<u8>, password: &[u8]) -> Result<Self> {
        let diagnostics = crate::parser_report::diagnose_pdf_bytes(&data);
        let version = parse_header_version(&data)?;
        let mut xref = HashMap::new();
        let mut trailer = None;
        let mut visited = HashSet::new();
        let startxref = find_startxref(&data)?;
        read_xref_chain(&data, startxref, &mut xref, &mut trailer, &mut visited)?;

        let trailer = trailer.ok_or_else(|| {
            OxideError::MalformedPdf("PDF did not contain a trailer dictionary".to_string())
        })?;

        let source = PdfSource::Memory(data);
        let encryption = setup_encryption(&source, &xref, &trailer, password)?;

        Ok(Self {
            source,
            version,
            xref,
            trailer,
            object_stream_cache: RwLock::new(BoundedObjectStreamCache::new(
                DEFAULT_OBJECT_STREAM_CACHE_LIMIT,
            )),
            encryption,
            startxref,
            diagnostics,
        })
    }

    /// Open a PDF from bytes, supplying a user password for encrypted
    /// documents. For non-encrypted PDFs the password is ignored.
    ///
    /// For encrypted PDFs the password is verified against the `/U` entry; the
    /// supplied password is tried first, then the empty password as a fallback
    /// (the most common case in the wild — permission-only encryption). If no
    /// password verifies, [`OxideError::EncryptedPdf`] is returned.
    pub fn from_bytes_with_password(data: Vec<u8>, password: &[u8]) -> Result<Self> {
        let mut diagnostics = crate::parser_report::diagnose_pdf_bytes(&data);
        let version = parse_header_version(&data)?;
        let mut xref = HashMap::new();
        let mut trailer = None;
        let mut visited = HashSet::new();

        let startxref = match find_startxref(&data) {
            Ok(startxref) => {
                if let Err(primary) =
                    read_xref_chain(&data, startxref, &mut xref, &mut trailer, &mut visited)
                {
                    xref.clear();
                    trailer = None;
                    if rebuild_xref_from_object_scan(&data, &mut xref, &mut trailer).is_err() {
                        return Err(primary);
                    }
                    diagnostics.push(
                        ParserDiagnostic::new(
                            ParserSeverity::RecoverableError,
                            ParserCategory::Repair,
                            "xref_chain_rebuilt_from_object_scan",
                            "xref chain could not be trusted and was rebuilt from a bounded indirect-object scan",
                        )
                        .at_offset(startxref)
                        .with_recovery("discarded damaged xref chain and used recovered object offsets")
                        .incomplete(),
                    );
                }
                startxref
            }
            Err(primary) => {
                if rebuild_xref_from_object_scan(&data, &mut xref, &mut trailer).is_err() {
                    return Err(primary);
                }
                diagnostics.push(
                    ParserDiagnostic::new(
                        ParserSeverity::RecoverableError,
                        ParserCategory::Repair,
                        "missing_startxref_repaired",
                        "missing startxref was repaired by bounded indirect-object scan",
                    )
                    .with_recovery("built xref table from indirect-object headers")
                    .incomplete(),
                );
                0
            }
        };
        let repaired_offsets = repair_uncompressed_xref_offsets(&data, &mut xref);
        if repaired_offsets > 0 {
            diagnostics.push(
                ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::Repair,
                    "xref_offsets_repaired",
                    format!(
                        "{repaired_offsets} xref offset(s) were corrected by nearby object headers"
                    ),
                )
                .with_recovery(
                    "replaced damaged uncompressed xref offsets with scanned object offsets",
                )
                .incomplete(),
            );
        }

        let trailer = trailer.ok_or_else(|| {
            OxideError::MalformedPdf("PDF did not contain a trailer dictionary".to_string())
        })?;

        let source = PdfSource::Memory(data);
        let encryption = setup_encryption(&source, &xref, &trailer, password)?;

        Ok(Self {
            source,
            version,
            xref,
            trailer,
            object_stream_cache: RwLock::new(BoundedObjectStreamCache::new(
                DEFAULT_OBJECT_STREAM_CACHE_LIMIT,
            )),
            encryption,
            startxref,
            diagnostics,
        })
    }

    /// Open a PDF from bytes using an explicit public-key security-handler
    /// provider. This path is used only for `/Filter /Adobe.PubSec` documents;
    /// it does not scan the filesystem or validate certificate trust.
    pub fn from_bytes_with_pubsec_provider(
        data: Vec<u8>,
        provider: &PubSecKeyProvider,
    ) -> Result<Self> {
        let mut diagnostics = crate::parser_report::diagnose_pdf_bytes(&data);
        let version = parse_header_version(&data)?;
        let mut xref = HashMap::new();
        let mut trailer = None;
        let mut visited = HashSet::new();

        let startxref = match find_startxref(&data) {
            Ok(startxref) => {
                if let Err(primary) =
                    read_xref_chain(&data, startxref, &mut xref, &mut trailer, &mut visited)
                {
                    xref.clear();
                    trailer = None;
                    if rebuild_xref_from_object_scan(&data, &mut xref, &mut trailer).is_err() {
                        return Err(primary);
                    }
                    diagnostics.push(
                        ParserDiagnostic::new(
                            ParserSeverity::RecoverableError,
                            ParserCategory::Repair,
                            "xref_chain_rebuilt_from_object_scan",
                            "xref chain could not be trusted and was rebuilt from a bounded indirect-object scan",
                        )
                        .at_offset(startxref)
                        .with_recovery("discarded damaged xref chain and used recovered object offsets")
                        .incomplete(),
                    );
                }
                startxref
            }
            Err(primary) => {
                if rebuild_xref_from_object_scan(&data, &mut xref, &mut trailer).is_err() {
                    return Err(primary);
                }
                diagnostics.push(
                    ParserDiagnostic::new(
                        ParserSeverity::RecoverableError,
                        ParserCategory::Repair,
                        "missing_startxref_repaired",
                        "missing startxref was repaired by bounded indirect-object scan",
                    )
                    .with_recovery("built xref table from indirect-object headers")
                    .incomplete(),
                );
                0
            }
        };
        let repaired_offsets = repair_uncompressed_xref_offsets(&data, &mut xref);
        if repaired_offsets > 0 {
            diagnostics.push(
                ParserDiagnostic::new(
                    ParserSeverity::RecoverableError,
                    ParserCategory::Repair,
                    "xref_offsets_repaired",
                    format!(
                        "{repaired_offsets} xref offset(s) were corrected by nearby object headers"
                    ),
                )
                .with_recovery(
                    "replaced damaged uncompressed xref offsets with scanned object offsets",
                )
                .incomplete(),
            );
        }

        let trailer = trailer.ok_or_else(|| {
            OxideError::MalformedPdf("PDF did not contain a trailer dictionary".to_string())
        })?;

        let source = PdfSource::Memory(data);
        let encryption = setup_encryption_pubsec(&source, &xref, &trailer, provider)?;

        Ok(Self {
            source,
            version,
            xref,
            trailer,
            object_stream_cache: RwLock::new(BoundedObjectStreamCache::new(
                DEFAULT_OBJECT_STREAM_CACHE_LIMIT,
            )),
            encryption,
            startxref,
            diagnostics,
        })
    }

    fn from_seekable_source_with_password(source: PdfSource, password: &[u8]) -> Result<Self> {
        let diagnostics = Vec::new();
        let prefix = source.read_prefix(1024)?;
        let version = parse_header_version(&prefix)?;
        let tail = source.read_tail(STREAMING_TAIL_READ_LIMIT)?;
        let mut xref = HashMap::new();
        let mut trailer = None;
        let mut visited = HashSet::new();
        let startxref = find_startxref(&tail)?;

        read_xref_chain_from_source(&source, startxref, &mut xref, &mut trailer, &mut visited)?;

        let trailer = trailer.ok_or_else(|| {
            OxideError::MalformedPdf("PDF did not contain a trailer dictionary".to_string())
        })?;
        let encryption = setup_encryption(&source, &xref, &trailer, password)?;

        Ok(Self {
            source,
            version,
            xref,
            trailer,
            object_stream_cache: RwLock::new(BoundedObjectStreamCache::new(
                DEFAULT_OBJECT_STREAM_CACHE_LIMIT,
            )),
            encryption,
            startxref,
            diagnostics,
        })
    }

    /// The active encryption context, if this document is encrypted and was
    /// successfully unlocked.
    pub fn encryption(&self) -> Option<&EncryptionContext> {
        self.encryption.as_ref()
    }

    /// True when the document is encrypted and a decryption context is active.
    pub fn is_encrypted(&self) -> bool {
        self.encryption.is_some()
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Total size of the input PDF in bytes (the length of the parsed buffer).
    /// Reported by the `info` tool.
    pub fn file_size(&self) -> usize {
        self.source.len()
    }

    /// Structured diagnostics collected during parser open/repair.
    pub fn parser_diagnostics(&self) -> &[ParserDiagnostic] {
        &self.diagnostics
    }

    /// Source/laziness metrics that can be reported without forcing object parsing.
    pub fn source_metrics(&self) -> ParserSourceMetrics {
        ParserSourceMetrics {
            file_size_bytes: self.source.len(),
            file_backed: matches!(self.source, PdfSource::File(_)),
            startxref: Some(self.startxref),
            xref_entries: self.xref.len(),
            objects_known: self.object_ids().len(),
            objects_parsed_during_open: 0,
            object_streams_decoded_during_open: 0,
            bytes_read_during_open: match self.source {
                PdfSource::Memory(_) => Some(self.source.len()),
                PdfSource::File(_) => None,
            },
        }
    }

    /// The exact original file bytes, as opened. Digital-signature verification
    /// hashes the bytes selected by a signature's `/ByteRange` against these —
    /// it must use the raw bytes, never a re-serialization.
    pub fn file_bytes(&self) -> &[u8] {
        self.source.file_bytes()
    }

    /// Byte offset recorded by the latest `startxref` marker.
    pub fn startxref_offset(&self) -> usize {
        self.startxref
    }

    /// The raw, resolved `/Encrypt` dictionary, if the document is encrypted.
    ///
    /// The `/Encrypt` dictionary's own verifier strings (`/O`, `/U`, `/OE`,
    /// `/UE`, `/Perms`) are **not** encrypted (PDF 32000-1 §7.6.1). They must
    /// therefore be read WITHOUT the per-object decryption pass that
    /// [`Self::get_object`] applies — otherwise the reader would AES/RC4-decrypt
    /// the plaintext verifiers and corrupt them (e.g. a 16-byte `/Perms`
    /// decrypts to empty). We parse the `/Encrypt` object straight from the file
    /// bytes, exactly as encryption setup does. Returns `None` for unencrypted
    /// documents and on any parse failure.
    pub fn encrypt_dictionary(&self) -> Option<PdfDictionary> {
        let encrypt = self.trailer.get("Encrypt")?;
        resolve_encrypt_dict(&self.source, &self.xref, encrypt)
            .ok()
            .flatten()
    }

    pub fn trailer(&self) -> &PdfDictionary {
        &self.trailer
    }

    pub fn size(&self) -> Option<i64> {
        self.trailer.get_integer("Size")
    }

    pub fn root_reference(&self) -> Option<(u32, u16)> {
        self.trailer.get_reference("Root")
    }

    /// The first element of the trailer `/ID` array, if present. The PDF
    /// writer copies this into manipulated output so the produced file keeps a
    /// stable identifier derived from a source document.
    pub fn first_file_id(&self) -> Option<Vec<u8>> {
        match self.trailer.get("ID") {
            Some(PdfObject::Array(arr)) => match arr.first() {
                Some(PdfObject::String(bytes)) => Some(bytes.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// The trailer `/Info` reference, if present. The document information
    /// dictionary holds metadata (Title, Author, Producer, …); the PDF writer
    /// copies it into rewritten/manipulated output when available.
    pub fn info_reference(&self) -> Option<(u32, u16)> {
        self.trailer.get_reference("Info")
    }

    /// Enumerate every in-use indirect object id `(number, generation)` known
    /// to the cross-reference table, in ascending object-number order.
    ///
    /// Free entries (object 0 and any other `f` entries) are excluded, as are
    /// compressed *container* streams' sub-objects' duplicates — each logical
    /// object appears once. Objects stored inside an object stream
    /// (`XrefEntry::Compressed`) are reported with the generation `0` the xref
    /// stream assigns them, so they can be fetched via [`Self::get_object`].
    ///
    /// This is the enumeration the [`crate::writer`] uses for a faithful
    /// whole-document round-trip (copy every object, identity-renumber, emit a
    /// fresh file). For page-level manipulation (merge/split/extract) the
    /// writer instead walks a dependency closure and never needs this.
    pub fn object_ids(&self) -> Vec<(u32, u16)> {
        let mut ids: Vec<(u32, u16)> = self
            .xref
            .iter()
            .filter(|(_, entry)| !matches!(entry, XrefEntry::Free))
            .map(|((number, generation), _)| (*number, *generation))
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn get_object(&self, number: u32, generation: u16) -> Result<PdfObject> {
        let entry = self
            .xref
            .get(&(number, generation))
            .cloned()
            .ok_or(OxideError::MissingObject { number, generation })?;

        match entry {
            XrefEntry::Free => Err(OxideError::MissingObject { number, generation }),
            XrefEntry::Uncompressed { offset } => {
                let parsed = self.parse_uncompressed_object_at(offset)?;
                if parsed.number != number || parsed.generation != generation {
                    return Err(OxideError::MissingObject { number, generation });
                }
                self.decrypt_object(parsed.object, number, generation)
            }
            XrefEntry::Compressed { stream_obj, index } => {
                // Objects stored inside an object stream are decrypted as part
                // of decrypting the containing ObjStm, so they must NOT be
                // decrypted again here (PDF 32000-1 §7.6.2 note).
                self.ensure_object_stream_cached(stream_obj)?;
                let cache = self
                    .object_stream_cache
                    .read()
                    .expect("object stream cache lock poisoned");
                let objects = cache
                    .get(&stream_obj)
                    .ok_or(OxideError::MissingObject { number, generation })?;
                let (actual_index, object) = objects
                    .get(&number)
                    .ok_or(OxideError::MissingObject { number, generation })?;
                if *actual_index != index {
                    return Err(OxideError::MissingObject { number, generation });
                }
                Ok(object.clone())
            }
        }
    }

    pub(crate) fn content_stream_range(
        &self,
        number: u32,
        generation: u16,
    ) -> Result<Option<ContentStreamRange<'_>>> {
        if self.encryption.is_some() {
            return Ok(None);
        }

        let Some(XrefEntry::Uncompressed { offset }) =
            self.xref.get(&(number, generation)).cloned()
        else {
            return Ok(None);
        };
        let header_bytes = self
            .source
            .read_from(offset, STREAMING_STREAM_HEADER_READ_LIMIT)?;
        let mut parser = PdfParser::with_resolver(&header_bytes, 0, Some(self))?;
        let header = match parser.parse_indirect_stream_header() {
            Ok(header) => header,
            Err(_) => return Ok(None),
        };
        if header.number != number || header.generation != generation {
            return Ok(None);
        }
        let Some(length) = header.length else {
            return Ok(None);
        };
        let length = usize::try_from(length)
            .map_err(|_| OxideError::MalformedPdf("stream Length is too large".to_string()))?;
        let stream_start = offset
            .checked_add(header.stream_start)
            .ok_or_else(|| OxideError::MalformedPdf("stream start offset overflows".to_string()))?;
        let stream_end = stream_start
            .checked_add(length)
            .ok_or_else(|| OxideError::MalformedPdf("stream Length overflows".to_string()))?;
        if stream_end > self.source.len() {
            return Err(OxideError::ParseError(format!(
                "stream range {stream_start}..{stream_end} exceeds input length {}",
                self.source.len()
            )));
        }
        if let Some(boundary) = self.next_object_boundary(offset) {
            if stream_end > boundary {
                return Err(OxideError::ParseError(format!(
                    "stream range for object {number} {generation} crosses next object boundary"
                )));
            }
        }
        Ok(Some(ContentStreamRange {
            dict: header.dict,
            reader: PdfRangeReader {
                source: &self.source,
                offset: stream_start,
                remaining: length,
            },
        }))
    }

    fn parse_uncompressed_object_at(&self, offset: usize) -> Result<crate::parser::IndirectObject> {
        if let Some(data) = self.source.as_bytes() {
            let mut parser = PdfParser::with_resolver(data, offset, Some(self))?;
            return parser.parse_indirect_object();
        }

        let bytes = self.read_object_window(offset)?;
        let mut parser = PdfParser::with_resolver(&bytes, 0, Some(self))?;
        parser.parse_indirect_object()
    }

    fn read_object_window(&self, offset: usize) -> Result<Vec<u8>> {
        if offset > self.source.len() {
            return Err(OxideError::ParseError(format!(
                "object offset {offset} is beyond input length {}",
                self.source.len()
            )));
        }
        let end = self
            .next_object_boundary(offset)
            .unwrap_or_else(|| self.source.len());
        if end <= offset {
            return Err(OxideError::ParseError(format!(
                "object offset {offset} has no readable range"
            )));
        }
        self.source.read_at(offset, end - offset)
    }

    fn next_object_boundary(&self, offset: usize) -> Option<usize> {
        self.xref
            .values()
            .filter_map(|entry| match entry {
                XrefEntry::Uncompressed { offset: candidate } if *candidate > offset => {
                    Some(*candidate)
                }
                _ => None,
            })
            .chain((self.startxref > offset).then_some(self.startxref))
            .min()
    }

    /// Recursively decrypt the strings and stream bytes inside a freshly-parsed
    /// top-level (uncompressed) object.
    ///
    /// No-op when the document is not encrypted, so the non-encrypted code path
    /// is unchanged. Structural cross-reference streams (`/Type /XRef`) are
    /// never encrypted and are left untouched; object streams (`/Type /ObjStm`)
    /// ARE encrypted and are decrypted here (their sub-objects are then parsed
    /// from the already-decrypted bytes and not decrypted again).
    fn decrypt_object(&self, obj: PdfObject, obj_num: u32, gen_num: u16) -> Result<PdfObject> {
        let ctx = match &self.encryption {
            None => return Ok(obj),
            Some(ctx) => ctx,
        };
        self.decrypt_object_inner(obj, obj_num, gen_num, ctx)
    }

    fn decrypt_object_inner(
        &self,
        obj: PdfObject,
        obj_num: u32,
        gen_num: u16,
        ctx: &EncryptionContext,
    ) -> Result<PdfObject> {
        Ok(match obj {
            PdfObject::String(bytes) => PdfObject::String(decrypt_bytes_by_method(
                &bytes,
                ctx,
                obj_num,
                gen_num,
                &ctx.string_method,
            )?),
            PdfObject::Stream { dict, raw } => {
                match dict.get_name("Type") {
                    // Cross-reference streams are never encrypted.
                    Some("XRef") => PdfObject::Stream { dict, raw },
                    // Metadata streams stay plaintext when /EncryptMetadata is false.
                    Some("Metadata") if !ctx.encrypt_metadata => PdfObject::Stream { dict, raw },
                    _ => {
                        let method = stream_crypt_method(&dict, ctx);
                        let decrypted =
                            decrypt_bytes_by_method(&raw, ctx, obj_num, gen_num, &method)?;
                        // String values inside the stream dictionary are also
                        // encrypted; decrypt them too.
                        let dict = match self.decrypt_object_inner(
                            PdfObject::Dictionary(dict),
                            obj_num,
                            gen_num,
                            ctx,
                        )? {
                            PdfObject::Dictionary(d) => d,
                            // decrypt_object_inner on a Dictionary always yields
                            // a Dictionary; this arm is unreachable in practice.
                            _ => PdfDictionary::empty(),
                        };
                        PdfObject::Stream {
                            dict,
                            raw: decrypted,
                        }
                    }
                }
            }
            PdfObject::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.decrypt_object_inner(item, obj_num, gen_num, ctx)?);
                }
                PdfObject::Array(out)
            }
            PdfObject::Dictionary(dict) => {
                let mut out = PdfDictionary::empty();
                for (key, value) in dict.iter() {
                    out.insert(
                        key.clone(),
                        self.decrypt_object_inner(value.clone(), obj_num, gen_num, ctx)?,
                    );
                }
                PdfObject::Dictionary(out)
            }
            // Integers, reals, booleans, names, references, null: unchanged.
            other => other,
        })
    }

    pub fn resolve(&self, object: PdfObject) -> Result<PdfObject> {
        let mut visited = HashSet::new();
        self.resolve_inner(object, &mut visited, 0)
    }

    pub fn get_and_resolve(&self, number: u32, generation: u16) -> Result<PdfObject> {
        let object = self.get_object(number, generation)?;
        self.resolve(object)
    }

    fn resolve_inner(
        &self,
        object: PdfObject,
        visited: &mut HashSet<(u32, u16)>,
        depth: usize,
    ) -> Result<PdfObject> {
        if depth > 64 {
            return Err(OxideError::MalformedPdf(
                "reference resolution exceeded depth limit".to_string(),
            ));
        }
        match object {
            PdfObject::Reference { number, generation } => {
                if !visited.insert((number, generation)) {
                    return Err(OxideError::MalformedPdf(format!(
                        "reference cycle at {number} {generation}"
                    )));
                }
                let resolved = self.get_object(number, generation)?;
                self.resolve_inner(resolved, visited, depth + 1)
            }
            other => Ok(other),
        }
    }

    fn ensure_object_stream_cached(&self, stream_obj: u32) -> Result<()> {
        // Fast path: already cached. Release the read lock before doing any
        // parsing work.
        if self
            .object_stream_cache
            .read()
            .expect("object stream cache lock poisoned")
            .contains_key(&stream_obj)
        {
            return Ok(());
        }
        // Parse WITHOUT holding the lock: `parse_object_stream` calls back into
        // `get_object`, which may itself acquire this lock for a *different*
        // object stream. Holding the write lock across that recursion would
        // deadlock. Parsing the same stream twice under a race is harmless and
        // idempotent (the result is value-identical), so we accept that and let
        // the last writer win.
        let objects = self.parse_object_stream(stream_obj)?;
        self.object_stream_cache
            .write()
            .expect("object stream cache lock poisoned")
            .insert(stream_obj, objects);
        Ok(())
    }

    fn parse_object_stream(&self, stream_obj: u32) -> Result<HashMap<u32, (u32, PdfObject)>> {
        let stream = self.get_object(stream_obj, 0)?;
        let (dict, raw) = stream.as_stream().ok_or_else(|| {
            OxideError::MalformedPdf(format!("object {stream_obj} 0 is not an object stream"))
        })?;
        if dict.get_name("Type") != Some("ObjStm") {
            return Err(OxideError::MalformedPdf(format!(
                "object {stream_obj} 0 is not /Type /ObjStm"
            )));
        }
        let decoded = crate::filters::decode_stream(&stream, self)?;
        let n = required_positive_usize(dict, "N")?;
        let first = required_nonnegative_usize(dict, "First")?;
        let _ = raw;
        parse_object_stream_data(&decoded, n, first, Some(self))
    }
}

fn setup_encryption(
    source: &PdfSource,
    xref: &HashMap<(u32, u16), XrefEntry>,
    trailer: &PdfDictionary,
    password: &[u8],
) -> Result<Option<EncryptionContext>> {
    let Some(encrypt_obj) = trailer.get("Encrypt") else {
        return Ok(None); // not encrypted
    };

    let encrypt_dict = match resolve_encrypt_dict(source, xref, encrypt_obj)? {
        Some(dict) => dict,
        None => return Err(OxideError::EncryptedDocument),
    };

    let info = match EncryptionInfo::from_dict(&encrypt_dict) {
        Ok(info) => info,
        Err(_) => return Err(OxideError::EncryptedDocument),
    };

    // V5 (AES-256, R5/R6) — entirely different key derivation path.
    if info.is_v5() {
        return setup_encryption_v5(password, &info);
    }

    let file_id = extract_file_id(trailer);

    // Try the supplied password first, then the empty password (permission-only
    // encryption, the common case).
    let candidates: Vec<&[u8]> = if password.is_empty() {
        vec![b""]
    } else {
        vec![password, b""]
    };

    let make_ctx = |file_key: SecretBytes| EncryptionContext {
        file_key,
        is_aes: info.is_aes(),
        is_v5: false,
        stream_method: info.stream_method.clone(),
        string_method: info.string_method.clone(),
        embedded_file_method: info.embedded_file_method.clone(),
        crypt_filters: info.crypt_filters.clone(),
        encrypt_metadata: info.encrypt_metadata,
    };

    for pwd in &candidates {
        if verify_user_password(pwd, &info, &file_id) {
            let file_key = compute_encryption_key(pwd, &info, &file_id);
            return Ok(Some(make_ctx(file_key)));
        }
    }

    // Try the supplied password as an OWNER password: recover the user-password
    // equivalent from /O (Algorithm 3 reverse), then derive the file key from it.
    if !password.is_empty() {
        let recovered = crate::crypto::recover_user_password_from_owner(password, &info);
        if verify_user_password(&recovered, &info, &file_id) {
            let file_key = compute_encryption_key(&recovered, &info, &file_id);
            return Ok(Some(make_ctx(file_key)));
        }
    }

    if info.stream_method == CryptMethod::None && info.string_method == CryptMethod::None {
        return Ok(None);
    }

    Err(OxideError::EncryptedPdf(
        "PDF is password-protected; provide the correct password".to_string(),
    ))
}

fn setup_encryption_pubsec(
    source: &PdfSource,
    xref: &HashMap<(u32, u16), XrefEntry>,
    trailer: &PdfDictionary,
    provider: &PubSecKeyProvider,
) -> Result<Option<EncryptionContext>> {
    let Some(encrypt_obj) = trailer.get("Encrypt") else {
        return Ok(None);
    };
    let encrypt_dict = match resolve_encrypt_dict(source, xref, encrypt_obj)? {
        Some(dict) => dict,
        None => return Err(OxideError::EncryptedDocument),
    };
    let filter = encrypt_dict.get_name("Filter").unwrap_or("");
    if filter != "Adobe.PubSec" {
        return Err(OxideError::UnsupportedFeature(format!(
            "public-key provider open requires /Filter /Adobe.PubSec, got /{filter}"
        )));
    }
    let info = parse_pubsec_encryption_info(&encrypt_dict)?;
    let recovered = recover_pubsec_file_key(&info, provider)?;
    let is_v5 = matches!(info.stream_method, CryptMethod::AesV3)
        || matches!(info.string_method, CryptMethod::AesV3)
        || matches!(info.embedded_file_method, CryptMethod::AesV3);
    Ok(Some(EncryptionContext {
        file_key: recovered.file_key,
        is_aes: matches!(info.stream_method, CryptMethod::AesV2),
        is_v5,
        stream_method: info.stream_method,
        string_method: info.string_method,
        embedded_file_method: info.embedded_file_method,
        crypt_filters: info.crypt_filters,
        encrypt_metadata: info.encrypt_metadata,
    }))
}

/// Set up decryption for a V5 (AES-256, R5/R6) document.
///
/// Tries the supplied password as a user password, then as an owner password,
/// then both again with the empty password as a fallback.
fn setup_encryption_v5(
    password: &[u8],
    info: &EncryptionInfo,
) -> Result<Option<EncryptionContext>> {
    // Build candidate list: supplied pwd first (user then owner), then empty pwd fallback.
    struct Candidate<'a> {
        pwd: &'a [u8],
        is_owner: bool,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    if !password.is_empty() {
        candidates.push(Candidate {
            pwd: password,
            is_owner: false,
        });
        candidates.push(Candidate {
            pwd: password,
            is_owner: true,
        });
    }
    // Always try empty password as fallback (permission-only encryption).
    candidates.push(Candidate {
        pwd: b"",
        is_owner: false,
    });
    candidates.push(Candidate {
        pwd: b"",
        is_owner: true,
    });

    for c in &candidates {
        let verified = if c.is_owner {
            verify_v5_owner_password(c.pwd, info)
        } else {
            verify_v5_user_password(c.pwd, info)
        };

        if !verified {
            continue;
        }

        let file_key_result = if c.is_owner {
            derive_v5_file_key_from_owner(c.pwd, info)
        } else {
            derive_v5_file_key_from_user(c.pwd, info)
        };

        let file_key = match file_key_result {
            Ok(k) => k,
            Err(_) => continue,
        };

        // /Perms verification: confirms the file key is correct and
        // permissions haven't been tampered with. Log a warning on failure
        // but don't reject — some writers produce slightly non-conformant
        // /Perms blocks while the key itself is correct.
        if !verify_v5_perms(&file_key, info) {
            log::warn!("V5 /Perms magic-byte check failed; proceeding with derived key");
        }

        return Ok(Some(EncryptionContext {
            file_key,
            is_aes: false, // V5 uses AES-256 directly, not the is_aes (AES-128) flag
            is_v5: true,
            stream_method: info.stream_method.clone(),
            string_method: info.string_method.clone(),
            embedded_file_method: info.embedded_file_method.clone(),
            crypt_filters: info.crypt_filters.clone(),
            encrypt_metadata: info.encrypt_metadata,
        }));
    }

    if info.stream_method == CryptMethod::None && info.string_method == CryptMethod::None {
        return Ok(None);
    }

    Err(OxideError::EncryptedPdf(
        "PDF is password-protected; provide the correct password".to_string(),
    ))
}

/// Extract the first element of the trailer `/ID` array (used in key
/// derivation). Returns an empty vector when `/ID` is absent.
fn extract_file_id(trailer: &PdfDictionary) -> Vec<u8> {
    match trailer.get("ID") {
        Some(PdfObject::Array(arr)) => match arr.first() {
            Some(PdfObject::String(bytes)) => bytes.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn resolve_encrypt_dict(
    source: &PdfSource,
    xref: &HashMap<(u32, u16), XrefEntry>,
    object: &PdfObject,
) -> Result<Option<PdfDictionary>> {
    match object {
        PdfObject::Dictionary(dict) => Ok(Some(dict.clone())),
        PdfObject::Reference { number, generation } => {
            let Some(XrefEntry::Uncompressed { offset }) = xref.get(&(*number, *generation)) else {
                return Ok(None);
            };
            let bytes;
            let (data, parser_offset): (&[u8], usize) = if let Some(data) = source.as_bytes() {
                (data, *offset)
            } else {
                bytes = read_object_window_from_source(source, xref, *offset, None)?;
                (&bytes, 0)
            };
            let mut parser = PdfParser::new(data, parser_offset)?;
            let parsed = parser.parse_indirect_object()?;
            match parsed.object {
                PdfObject::Dictionary(dict) => Ok(Some(dict)),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn read_object_window_from_source(
    source: &PdfSource,
    xref: &HashMap<(u32, u16), XrefEntry>,
    offset: usize,
    startxref: Option<usize>,
) -> Result<Vec<u8>> {
    if offset > source.len() {
        return Err(OxideError::ParseError(format!(
            "object offset {offset} is beyond input length {}",
            source.len()
        )));
    }
    let end = xref
        .values()
        .filter_map(|entry| match entry {
            XrefEntry::Uncompressed { offset: candidate } if *candidate > offset => {
                Some(*candidate)
            }
            _ => None,
        })
        .chain(startxref.filter(|candidate| *candidate > offset))
        .min()
        .unwrap_or_else(|| source.len());
    if end <= offset {
        return Err(OxideError::ParseError(format!(
            "object offset {offset} has no readable range"
        )));
    }
    source.read_at(offset, end - offset)
}

impl ParserResolver for PdfReader {
    fn resolve_for_parser(&self, object: &PdfObject) -> Result<PdfObject> {
        self.resolve(object.clone())
    }
}

fn stream_crypt_method(dict: &PdfDictionary, ctx: &EncryptionContext) -> CryptMethod {
    if let Some(name) = explicit_crypt_filter_name(dict) {
        return if name == "Identity" {
            CryptMethod::None
        } else {
            ctx.crypt_filters.get(name).cloned().unwrap_or_else(|| {
                if dict.get_name("Type") == Some("EmbeddedFile") {
                    ctx.embedded_file_method.clone()
                } else {
                    ctx.stream_method.clone()
                }
            })
        };
    }
    if dict.get_name("Type") == Some("EmbeddedFile") {
        ctx.embedded_file_method.clone()
    } else {
        ctx.stream_method.clone()
    }
}

fn explicit_crypt_filter_name(dict: &PdfDictionary) -> Option<&str> {
    let filter_obj = dict.get("Filter").or_else(|| dict.get("F"))?;
    let has_crypt = match filter_obj {
        PdfObject::Name(name) => name == "Crypt",
        PdfObject::Array(items) => items
            .iter()
            .any(|item| matches!(item, PdfObject::Name(name) if name == "Crypt")),
        _ => false,
    };
    if !has_crypt {
        return None;
    }

    let params_obj = dict.get("DecodeParms").or_else(|| dict.get("DP"));
    match params_obj {
        Some(PdfObject::Dictionary(params)) => params.get_name("Name"),
        Some(PdfObject::Array(items)) => {
            let idx = crypt_filter_index(filter_obj)?;
            items
                .get(idx)
                .and_then(PdfObject::as_dict)?
                .get_name("Name")
        }
        _ => None,
    }
}

fn crypt_filter_index(filter_obj: &PdfObject) -> Option<usize> {
    match filter_obj {
        PdfObject::Name(name) if name == "Crypt" => Some(0),
        PdfObject::Array(items) => items
            .iter()
            .position(|item| matches!(item, PdfObject::Name(name) if name == "Crypt")),
        _ => None,
    }
}

fn method_is_aes128(method: &CryptMethod) -> bool {
    matches!(method, CryptMethod::AesV2)
}

fn method_is_aes256(method: &CryptMethod) -> bool {
    matches!(method, CryptMethod::AesV3)
}

fn decrypt_bytes_by_method(
    data: &[u8],
    ctx: &EncryptionContext,
    obj_num: u32,
    gen_num: u16,
    method: &CryptMethod,
) -> Result<Vec<u8>> {
    match method {
        CryptMethod::None => Ok(data.to_vec()),
        CryptMethod::V2 | CryptMethod::AesV2 | CryptMethod::AesV3 => Ok(decrypt_string(
            data,
            &ctx.file_key,
            obj_num,
            gen_num,
            method_is_aes128(method),
            method_is_aes256(method),
        )),
        CryptMethod::AesV4 => aes256_gcm_decrypt_pdf_object(data, &ctx.file_key),
    }
}

fn read_xref_chain(
    data: &[u8],
    startxref: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
    trailer: &mut Option<PdfDictionary>,
    visited: &mut HashSet<usize>,
) -> Result<()> {
    let mut next = Some(startxref);
    let mut depth = 0usize;
    while let Some(offset) = next {
        if depth >= MAX_XREF_CHAIN_DEPTH {
            return Err(OxideError::ResourceLimit(format!(
                "xref /Prev chain exceeded depth limit of {MAX_XREF_CHAIN_DEPTH}"
            )));
        }
        depth += 1;
        if !visited.insert(offset) {
            return Err(OxideError::MalformedPdf(format!(
                "cyclic xref chain at offset {offset}"
            )));
        }
        let section = read_xref_section(data, offset, xref)?;
        if trailer.is_none() {
            *trailer = Some(section.trailer.clone());
        }

        if let Some(xref_stm) = section.xref_stm {
            if xref_stm != offset && !visited.contains(&xref_stm) {
                let _ = read_xref_section(data, xref_stm, xref)?;
            }
        }

        next = section.prev;
    }
    Ok(())
}

fn read_xref_chain_from_source(
    source: &PdfSource,
    startxref: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
    trailer: &mut Option<PdfDictionary>,
    visited: &mut HashSet<usize>,
) -> Result<()> {
    let mut next = Some(startxref);
    let mut depth = 0usize;
    while let Some(offset) = next {
        if depth >= MAX_XREF_CHAIN_DEPTH {
            return Err(OxideError::ResourceLimit(format!(
                "xref /Prev chain exceeded depth limit of {MAX_XREF_CHAIN_DEPTH}"
            )));
        }
        depth += 1;
        if !visited.insert(offset) {
            return Err(OxideError::MalformedPdf(format!(
                "cyclic xref chain at offset {offset}"
            )));
        }
        let section = read_xref_section_from_source(source, offset, xref)?;
        if trailer.is_none() {
            *trailer = Some(section.trailer.clone());
        }

        if let Some(xref_stm) = section.xref_stm {
            if xref_stm != offset && !visited.contains(&xref_stm) {
                let _ = read_xref_section_from_source(source, xref_stm, xref)?;
            }
        }

        next = section.prev;
    }
    Ok(())
}

fn repair_uncompressed_xref_offsets(
    data: &[u8],
    xref: &mut HashMap<(u32, u16), XrefEntry>,
) -> usize {
    let needs_repair = xref.iter().any(|(&(number, generation), entry)| {
        matches!(
            entry,
            XrefEntry::Uncompressed { offset }
                if !indirect_object_header_at_matches(data, *offset, number, generation)
        )
    });
    if !needs_repair {
        return 0;
    }

    let scanned = scan_indirect_object_headers(data);
    let mut repaired_count = 0usize;
    for (&(number, generation), entry) in xref.iter_mut() {
        let XrefEntry::Uncompressed { offset } = entry else {
            continue;
        };
        if indirect_object_header_at_matches(data, *offset, number, generation) {
            continue;
        }
        if let Some(repaired) = scanned.get(&(number, generation)) {
            *offset = *repaired;
            repaired_count += 1;
        }
    }
    repaired_count
}

fn rebuild_xref_from_object_scan(
    data: &[u8],
    xref: &mut HashMap<(u32, u16), XrefEntry>,
    trailer: &mut Option<PdfDictionary>,
) -> Result<()> {
    let scanned = scan_indirect_object_headers(data);
    if scanned.is_empty() {
        return Err(OxideError::MalformedPdf(
            "fallback object scan found no indirect objects".to_string(),
        ));
    }
    if scanned.len() > MAX_FALLBACK_XREF_OBJECTS {
        return Err(OxideError::ResourceLimit(format!(
            "fallback object scan found more than {MAX_FALLBACK_XREF_OBJECTS} objects"
        )));
    }

    xref.clear();
    let mut max_object = 0u32;
    for (&(number, generation), &offset) in &scanned {
        if number == 0 {
            continue;
        }
        max_object = max_object.max(number);
        xref.insert((number, generation), XrefEntry::Uncompressed { offset });
    }

    let mut rebuilt = find_last_trailer_dictionary(data).unwrap_or_else(PdfDictionary::empty);
    if rebuilt.get("Root").is_none() {
        if let Some((number, generation)) = find_catalog_reference(data, &scanned) {
            rebuilt.insert("Root", PdfObject::Reference { number, generation });
        }
    }
    if rebuilt.get("Size").is_none() {
        rebuilt.insert("Size", PdfObject::Integer(i64::from(max_object) + 1));
    }
    if rebuilt.get("Root").is_none() {
        return Err(OxideError::MalformedPdf(
            "fallback object scan could not recover trailer /Root".to_string(),
        ));
    }
    *trailer = Some(rebuilt);
    Ok(())
}

fn find_last_trailer_dictionary(data: &[u8]) -> Option<PdfDictionary> {
    let marker = b"trailer";
    let mut positions = Vec::new();
    let mut start = 0usize;
    while start <= data.len() {
        let Some(rel) = find_marker_accelerated(&data[start..], marker) else {
            break;
        };
        let pos = start + rel;
        positions.push(pos);
        start = pos + 1;
    }
    for pos in positions.into_iter().rev() {
        let dict_pos = pos + marker.len();
        let Ok(mut parser) = PdfParser::new(data, dict_pos) else {
            continue;
        };
        let Ok(PdfObject::Dictionary(dict)) = parser.parse_object() else {
            continue;
        };
        return Some(dict);
    }
    None
}

fn find_catalog_reference(data: &[u8], scanned: &HashMap<(u32, u16), usize>) -> Option<(u32, u16)> {
    let mut candidates: Vec<((u32, u16), usize)> =
        scanned.iter().map(|(id, offset)| (*id, *offset)).collect();
    candidates
        .sort_unstable_by_key(|((number, generation), offset)| (*number, *generation, *offset));
    for ((number, generation), offset) in candidates {
        let Ok(mut parser) = PdfParser::new(data, offset) else {
            continue;
        };
        let Ok(parsed) = parser.parse_indirect_object() else {
            continue;
        };
        let Some(dict) = parsed.object.as_dict() else {
            continue;
        };
        if dict.get_name("Type") == Some("Catalog") && dict.get_reference("Pages").is_some() {
            return Some((number, generation));
        }
    }
    None
}

fn scan_indirect_object_headers(data: &[u8]) -> HashMap<(u32, u16), usize> {
    let mut offsets = HashMap::new();
    let stream_spans = stream_data_spans(data);
    let marker = b" obj";
    let mut rel = 0usize;
    while rel <= data.len() {
        let Some(found) = find_marker_accelerated(&data[rel..], marker) else {
            break;
        };
        rel += found;
        let line_start = data[..rel]
            .iter()
            .rposition(|byte| *byte == b'\r' || *byte == b'\n')
            .map_or(0, |pos| pos + 1);
        let object_start = skip_ws_and_comments(data, line_start);
        if offset_in_spans(object_start, &stream_spans) {
            rel += 1;
            continue;
        }
        let Some((number, generation)) = parse_indirect_object_header(data, object_start) else {
            rel += 1;
            continue;
        };
        offsets.insert((number, generation), object_start);
        rel += 1;
    }
    offsets
}

fn stream_data_spans(data: &[u8]) -> Vec<(usize, usize)> {
    let marker = b"stream";
    let mut spans = Vec::new();
    let mut pos = 0usize;
    while pos <= data.len() {
        let Some(rel) = find_marker_accelerated(&data[pos..], marker) else {
            break;
        };
        let stream_pos = pos + rel;
        let after_marker = stream_pos + marker.len();
        let Some(data_start) = stream_data_start(data, after_marker) else {
            pos = after_marker;
            continue;
        };
        let data_end = find_endstream(data, data_start).unwrap_or(data.len());
        spans.push((data_start, data_end));
        pos = data_end.saturating_add(b"endstream".len()).min(data.len());
    }
    spans
}

fn stream_data_start(data: &[u8], pos: usize) -> Option<usize> {
    match data.get(pos).copied() {
        Some(b'\r') if data.get(pos + 1).copied() == Some(b'\n') => Some(pos + 2),
        Some(b'\r' | b'\n') => Some(pos + 1),
        _ => None,
    }
}

fn find_endstream(data: &[u8], start: usize) -> Option<usize> {
    if start > data.len() {
        return None;
    }
    find_marker_accelerated(&data[start..], b"endstream").map(|rel| start + rel)
}

fn offset_in_spans(offset: usize, spans: &[(usize, usize)]) -> bool {
    spans
        .iter()
        .any(|(start, end)| (*start..*end).contains(&offset))
}

fn indirect_object_header_at_matches(
    data: &[u8],
    offset: usize,
    number: u32,
    generation: u16,
) -> bool {
    parse_indirect_object_header(data, offset) == Some((number, generation))
}

fn parse_indirect_object_header(data: &[u8], offset: usize) -> Option<(u32, u16)> {
    let mut pos = offset;
    let number = u32::try_from(read_u64_token(data, &mut pos).ok()?).ok()?;
    let generation = u16::try_from(read_u64_token(data, &mut pos).ok()?).ok()?;
    let token = read_token(data, &mut pos).ok()?;
    (token == b"obj").then_some((number, generation))
}

#[derive(Clone, Debug)]
struct XrefSection {
    trailer: PdfDictionary,
    prev: Option<usize>,
    xref_stm: Option<usize>,
}

fn read_xref_section(
    data: &[u8],
    offset: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
) -> Result<XrefSection> {
    let offset = skip_ws_and_comments(data, offset);
    if bytes_at(data, offset, b"xref") {
        read_classic_xref(data, offset, xref)
    } else if let Ok(section) = read_xref_stream(data, offset, xref) {
        Ok(section)
    } else if let Some(repaired) = nearby_classic_xref_offset(data, offset) {
        read_classic_xref(data, repaired, xref)
    } else {
        read_xref_stream(data, offset, xref)
    }
}

fn read_xref_section_from_source(
    source: &PdfSource,
    offset: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
) -> Result<XrefSection> {
    let base = offset.saturating_sub(64);
    let data = source.read_from(base, STREAMING_XREF_READ_LIMIT)?;
    let rel_offset = offset - base;
    let rel_offset = skip_ws_and_comments(&data, rel_offset);
    if bytes_at(&data, rel_offset, b"xref") {
        read_classic_xref(&data, rel_offset, xref)
    } else if let Ok(section) = read_xref_stream(&data, rel_offset, xref) {
        Ok(section)
    } else if let Some(repaired) = nearby_classic_xref_offset(&data, rel_offset) {
        read_classic_xref(&data, repaired, xref)
    } else {
        read_xref_stream(&data, rel_offset, xref)
    }
}

fn nearby_classic_xref_offset(data: &[u8], offset: usize) -> Option<usize> {
    let start = offset.saturating_sub(64);
    let end = offset.saturating_add(1024).min(data.len());
    data.get(start..end).and_then(|slice| {
        let mut candidates = Vec::new();
        let mut rel_start = 0usize;
        while rel_start <= slice.len() {
            let Some(rel) = find_marker_accelerated(&slice[rel_start..], b"xref") else {
                break;
            };
            let rel = rel_start + rel;
            let pos = start + rel;
            let is_word = pos == 0
                || data
                    .get(pos - 1)
                    .copied()
                    .is_none_or(|b| !b.is_ascii_alphabetic());
            if is_word {
                candidates.push(pos);
            }
            rel_start = rel + 1;
        }
        candidates
            .into_iter()
            .min_by_key(|candidate| candidate.abs_diff(offset))
    })
}

fn read_classic_xref(
    data: &[u8],
    mut pos: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
) -> Result<XrefSection> {
    if !bytes_at(data, pos, b"xref") {
        return Err(OxideError::MalformedPdf(format!(
            "xref table expected at offset {pos}"
        )));
    }
    pos += b"xref".len();

    loop {
        pos = skip_ws_and_comments(data, pos);
        if bytes_at(data, pos, b"trailer") {
            pos += b"trailer".len();
            break;
        }

        let start = read_u64_token(data, &mut pos)?;
        let count = read_u64_token(data, &mut pos)?;
        for i in 0..count {
            let object_number = u32::try_from(start + i).map_err(|_| {
                OxideError::MalformedPdf("xref object number does not fit in u32".to_string())
            })?;
            let byte_offset = read_u64_token(data, &mut pos)?;
            let generation = read_u64_token(data, &mut pos)?;
            let status = read_token(data, &mut pos)?;
            let entry = match status.as_slice() {
                b"n" => XrefEntry::Uncompressed {
                    offset: usize::try_from(byte_offset).map_err(|_| {
                        OxideError::MalformedPdf(
                            "xref offset is too large for this platform".to_string(),
                        )
                    })?,
                },
                b"f" => XrefEntry::Free,
                other => {
                    return Err(OxideError::MalformedPdf(format!(
                        "invalid xref entry status {}",
                        String::from_utf8_lossy(other)
                    )));
                }
            };
            let generation = match status.as_slice() {
                b"f" => u16::try_from(generation).unwrap_or(u16::MAX),
                _ => u16::try_from(generation).map_err(|_| {
                    OxideError::MalformedPdf("xref generation does not fit in u16".to_string())
                })?,
            };
            xref.entry((object_number, generation)).or_insert(entry);
        }
    }

    let mut parser = PdfParser::new(data, pos)?;
    let trailer_obj = parser.parse_object()?;
    let trailer = match trailer_obj {
        PdfObject::Dictionary(dict) => dict,
        other => {
            return Err(OxideError::MalformedPdf(format!(
                "classic xref trailer must be a dictionary, got {}",
                other.variant_name()
            )));
        }
    };
    Ok(XrefSection {
        prev: optional_offset(&trailer, "Prev")?,
        xref_stm: optional_offset(&trailer, "XRefStm")?,
        trailer,
    })
}

fn read_xref_stream(
    data: &[u8],
    offset: usize,
    xref: &mut HashMap<(u32, u16), XrefEntry>,
) -> Result<XrefSection> {
    let mut parser = PdfParser::new(data, offset)?;
    let parsed = parser.parse_indirect_object()?;
    let PdfObject::Stream { dict, raw } = parsed.object else {
        return Err(OxideError::MalformedPdf(format!(
            "xref stream offset {offset} did not point to a stream"
        )));
    };
    if dict.get_name("Type") != Some("XRef") {
        return Err(OxideError::MalformedPdf(format!(
            "xref stream object {} {} is not /Type /XRef",
            parsed.number, parsed.generation
        )));
    }
    let decoded = decode_stream_from_dict(&dict, &raw)?;
    for (object_number, generation, entry) in parse_xref_stream_entries(&dict, &decoded)? {
        xref.entry((object_number, generation)).or_insert(entry);
    }
    Ok(XrefSection {
        prev: optional_offset(&dict, "Prev")?,
        xref_stm: optional_offset(&dict, "XRefStm")?,
        trailer: dict,
    })
}

pub(crate) fn parse_xref_stream_entries(
    dict: &PdfDictionary,
    raw: &[u8],
) -> Result<Vec<(u32, u16, XrefEntry)>> {
    let widths = required_integer_array(dict, "W")?;
    if widths.len() != 3 {
        return Err(OxideError::MalformedPdf(
            "xref stream /W must contain three integers".to_string(),
        ));
    }
    let w0 = nonnegative_usize(widths[0], "xref W[0]")?;
    let w1 = nonnegative_usize(widths[1], "xref W[1]")?;
    let w2 = nonnegative_usize(widths[2], "xref W[2]")?;
    let entry_len = w0
        .checked_add(w1)
        .and_then(|v| v.checked_add(w2))
        .ok_or_else(|| OxideError::MalformedPdf("xref entry width overflows".to_string()))?;
    if entry_len == 0 {
        return Err(OxideError::MalformedPdf(
            "xref stream entry width cannot be zero".to_string(),
        ));
    }

    let ranges = if let Some(index) = dict.get_array("Index") {
        parse_index_array(index)?
    } else {
        let size = required_positive_usize(dict, "Size")?;
        vec![(0u32, size)]
    };

    let mut entries = Vec::new();
    let mut seen_object_numbers = HashSet::new();
    let mut cursor = 0usize;
    for (start, count) in ranges {
        for relative in 0..count {
            let end = cursor.checked_add(entry_len).ok_or_else(|| {
                OxideError::MalformedPdf("xref stream cursor overflows".to_string())
            })?;
            if end > raw.len() {
                return Err(OxideError::MalformedPdf(
                    "xref stream ended before all entries were read".to_string(),
                ));
            }
            let entry_bytes = &raw[cursor..end];
            let field0 = if w0 == 0 {
                1
            } else {
                read_big_endian_field(&entry_bytes[0..w0])?
            };
            let field1_start = w0;
            let field2_start = w0 + w1;
            let field1 = read_big_endian_field(&entry_bytes[field1_start..field2_start])?;
            let field2 = read_big_endian_field(&entry_bytes[field2_start..])?;
            let relative = u32::try_from(relative).map_err(|_| {
                OxideError::MalformedPdf("xref stream /Index count exceeds u32".to_string())
            })?;
            let object_number = start.checked_add(relative).ok_or_else(|| {
                OxideError::MalformedPdf("xref stream object number overflows".to_string())
            })?;
            if !seen_object_numbers.insert(object_number) {
                return Err(OxideError::MalformedPdf(format!(
                    "xref stream /Index contains duplicate object {object_number}"
                )));
            }
            match field0 {
                0 => {
                    let generation = u16::try_from(field2).unwrap_or(u16::MAX);
                    entries.push((object_number, generation, XrefEntry::Free));
                }
                1 => {
                    let generation = u16::try_from(field2).map_err(|_| {
                        OxideError::MalformedPdf("xref generation does not fit in u16".to_string())
                    })?;
                    entries.push((
                        object_number,
                        generation,
                        XrefEntry::Uncompressed {
                            offset: usize::try_from(field1).map_err(|_| {
                                OxideError::MalformedPdf(
                                    "xref offset is too large for this platform".to_string(),
                                )
                            })?,
                        },
                    ));
                }
                2 => {
                    entries.push((
                        object_number,
                        0,
                        XrefEntry::Compressed {
                            stream_obj: u32::try_from(field1).map_err(|_| {
                                OxideError::MalformedPdf(
                                    "object stream number does not fit in u32".to_string(),
                                )
                            })?,
                            index: u32::try_from(field2).map_err(|_| {
                                OxideError::MalformedPdf(
                                    "object stream index does not fit in u32".to_string(),
                                )
                            })?,
                        },
                    ));
                }
                other => {
                    return Err(OxideError::MalformedPdf(format!(
                        "unsupported xref stream entry type {other}"
                    )));
                }
            }
            cursor = end;
        }
    }
    Ok(entries)
}

pub(crate) fn parse_object_stream_data(
    decoded: &[u8],
    n: usize,
    first: usize,
    resolver: Option<&dyn ParserResolver>,
) -> Result<HashMap<u32, (u32, PdfObject)>> {
    if first > decoded.len() {
        return Err(OxideError::MalformedPdf(
            "object stream /First exceeds decoded length".to_string(),
        ));
    }
    let header = &decoded[..first];
    let mut pos = 0usize;
    // `n` is the attacker-controlled `/N` count. Each table entry consumes at
    // least one byte of the `first`-byte header (two whitespace-separated
    // integer tokens), so a genuine stream can hold at most `first` entries.
    // Cap the preallocation hint at that bound: a crafted `/N 4000000000` in a
    // tiny stream must not reserve gigabytes before the per-entry loop rejects
    // the truncated header. The loop below still reads exactly `n` entries and
    // errors cleanly once the header runs out of tokens.
    let mut table = Vec::with_capacity(n.min(first));
    let mut seen_object_numbers = HashSet::new();
    for index in 0..n {
        let object_number = read_u64_token(header, &mut pos)?;
        let offset = read_u64_token(header, &mut pos)?;
        let object_number = u32::try_from(object_number).map_err(|_| {
            OxideError::MalformedPdf("object stream object number does not fit in u32".to_string())
        })?;
        if !seen_object_numbers.insert(object_number) {
            return Err(OxideError::MalformedPdf(format!(
                "object stream contains duplicate object {object_number}"
            )));
        }
        table.push((
            object_number,
            u32::try_from(index).map_err(|_| {
                OxideError::MalformedPdf("object stream index does not fit in u32".to_string())
            })?,
            usize::try_from(offset).map_err(|_| {
                OxideError::MalformedPdf(
                    "object stream offset is too large for this platform".to_string(),
                )
            })?,
        ));
    }

    let mut objects = HashMap::new();
    for (object_number, index, relative_offset) in table {
        let object_offset = first.checked_add(relative_offset).ok_or_else(|| {
            OxideError::MalformedPdf("object stream offset overflows".to_string())
        })?;
        if object_offset >= decoded.len() {
            return Err(OxideError::MalformedPdf(format!(
                "object stream offset for object {object_number} exceeds decoded length"
            )));
        }
        let mut parser = PdfParser::with_resolver(decoded, object_offset, resolver)?;
        let object = parser.parse_object()?;
        if objects.insert(object_number, (index, object)).is_some() {
            return Err(OxideError::MalformedPdf(format!(
                "object stream contains duplicate object {object_number}"
            )));
        }
    }
    Ok(objects)
}

fn required_integer_array(dict: &PdfDictionary, key: &str) -> Result<Vec<i64>> {
    let array = dict.get_array(key).ok_or_else(|| {
        OxideError::MalformedPdf(format!("required dictionary key /{key} is missing"))
    })?;
    let mut values = Vec::with_capacity(array.len());
    for object in array {
        match object {
            PdfObject::Integer(value) => values.push(*value),
            other => {
                return Err(OxideError::MalformedPdf(format!(
                    "/{key} array contains {}",
                    other.variant_name()
                )));
            }
        }
    }
    Ok(values)
}

fn parse_index_array(index: &[PdfObject]) -> Result<Vec<(u32, usize)>> {
    if !index.len().is_multiple_of(2) {
        return Err(OxideError::MalformedPdf(
            "xref stream /Index must contain pairs".to_string(),
        ));
    }
    let mut ranges = Vec::new();
    for pair in index.chunks(2) {
        let start = pair[0].as_integer().ok_or_else(|| {
            OxideError::MalformedPdf("xref /Index start must be an integer".to_string())
        })?;
        let count = pair[1].as_integer().ok_or_else(|| {
            OxideError::MalformedPdf("xref /Index count must be an integer".to_string())
        })?;
        if start < 0 || count < 0 {
            return Err(OxideError::MalformedPdf(
                "xref /Index values must be nonnegative".to_string(),
            ));
        }
        ranges.push((
            u32::try_from(start).map_err(|_| {
                OxideError::MalformedPdf("xref /Index start does not fit in u32".to_string())
            })?,
            usize::try_from(count).map_err(|_| {
                OxideError::MalformedPdf("xref /Index count is too large".to_string())
            })?,
        ));
    }
    Ok(ranges)
}

fn read_big_endian_field(bytes: &[u8]) -> Result<u64> {
    if bytes.len() > 8 {
        return Err(OxideError::UnsupportedFeature(
            "xref field wider than 64 bits".to_string(),
        ));
    }
    let mut value = 0u64;
    for &byte in bytes {
        value = (value << 8) | u64::from(byte);
    }
    Ok(value)
}

fn optional_offset(dict: &PdfDictionary, key: &str) -> Result<Option<usize>> {
    match dict.get(key) {
        Some(PdfObject::Integer(value)) => {
            if *value < 0 {
                return Err(OxideError::MalformedPdf(format!(
                    "/{key} offset cannot be negative"
                )));
            }
            Ok(Some(usize::try_from(*value).map_err(|_| {
                OxideError::MalformedPdf(format!("/{key} offset is too large"))
            })?))
        }
        Some(PdfObject::Null) | None => Ok(None),
        Some(other) => Err(OxideError::MalformedPdf(format!(
            "/{key} offset must be an integer, got {}",
            other.variant_name()
        ))),
    }
}

fn required_nonnegative_usize(dict: &PdfDictionary, key: &str) -> Result<usize> {
    let value = dict.get_integer(key).ok_or_else(|| {
        OxideError::MalformedPdf(format!("required dictionary key /{key} is missing"))
    })?;
    nonnegative_usize(value, key)
}

fn required_positive_usize(dict: &PdfDictionary, key: &str) -> Result<usize> {
    let value = required_nonnegative_usize(dict, key)?;
    if value == 0 {
        return Err(OxideError::MalformedPdf(format!("/{key} must be positive")));
    }
    Ok(value)
}

fn nonnegative_usize(value: i64, label: &str) -> Result<usize> {
    if value < 0 {
        return Err(OxideError::MalformedPdf(format!(
            "{label} must be nonnegative"
        )));
    }
    usize::try_from(value).map_err(|_| OxideError::MalformedPdf(format!("{label} is too large")))
}

fn parse_header_version(data: &[u8]) -> Result<String> {
    let search_len = data.len().min(1024);
    let header_offset = data[..search_len]
        .windows(b"%PDF-".len())
        .position(|window| window == b"%PDF-")
        .ok_or_else(|| OxideError::MalformedPdf("missing PDF header".to_string()))?;
    let version_start = header_offset + b"%PDF-".len();
    let mut version_end = version_start;
    while let Some(byte) = data.get(version_end).copied() {
        if is_pdf_whitespace(byte) {
            break;
        }
        version_end += 1;
    }
    let version = std::str::from_utf8(&data[version_start..version_end])
        .map_err(|err| OxideError::MalformedPdf(format!("PDF version is not UTF-8: {err}")))?;
    let valid = (version.len() == 3
        && version.as_bytes()[0] == b'1'
        && version.as_bytes()[1] == b'.'
        && version.as_bytes()[2].is_ascii_digit())
        || version == "2.0";
    if !valid {
        return Err(OxideError::MalformedPdf(format!(
            "unsupported PDF version header {version}"
        )));
    }
    Ok(version.to_string())
}

fn find_startxref(data: &[u8]) -> Result<usize> {
    let marker = b"startxref";
    let marker_pos = rfind_marker_accelerated(data, marker)
        .ok_or_else(|| OxideError::MalformedPdf("missing startxref".to_string()))?;
    let mut pos = marker_pos + marker.len();
    pos = skip_ws_and_comments(data, pos);
    let offset = read_u64_token(data, &mut pos)?;
    usize::try_from(offset)
        .map_err(|_| OxideError::MalformedPdf("startxref is too large".to_string()))
}

fn read_u64_token(data: &[u8], pos: &mut usize) -> Result<u64> {
    let token = read_token(data, pos)?;
    let text = std::str::from_utf8(&token)
        .map_err(|err| OxideError::ParseError(format!("invalid integer token: {err}")))?;
    text.parse::<u64>()
        .map_err(|err| OxideError::ParseError(format!("invalid unsigned integer: {err}")))
}

fn read_token(data: &[u8], pos: &mut usize) -> Result<Vec<u8>> {
    *pos = skip_ws_and_comments(data, *pos);
    let start = *pos;
    while let Some(byte) = data.get(*pos).copied() {
        if is_pdf_whitespace(byte) || is_delimiter(byte) {
            break;
        }
        *pos += 1;
    }
    if *pos == start {
        return Err(OxideError::ParseError(
            "expected token while reading PDF bytes".to_string(),
        ));
    }
    Ok(data[start..*pos].to_vec())
}

fn skip_ws_and_comments(data: &[u8], mut pos: usize) -> usize {
    loop {
        while matches!(data.get(pos), Some(byte) if is_pdf_whitespace(*byte)) {
            pos += 1;
        }
        if data.get(pos).copied() == Some(b'%') {
            while let Some(byte) = data.get(pos).copied() {
                pos += 1;
                if byte == b'\r' || byte == b'\n' {
                    break;
                }
            }
        } else {
            break;
        }
    }
    pos
}

fn bytes_at(data: &[u8], pos: usize, bytes: &[u8]) -> bool {
    data.get(pos..pos + bytes.len())
        .is_some_and(|slice| slice == bytes)
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, 0x00 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;

    fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn tiny_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n\n");
        let obj1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\n");
        let obj2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Count 0 /Kids [] >>\nendobj\n\n");
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n0 3\n0000000000 65535 f\n{obj1:010} 00000 n\n{obj2:010} 00000 n\ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    fn remove_startxref(mut pdf: Vec<u8>) -> Vec<u8> {
        let marker = pdf
            .windows(b"startxref".len())
            .rposition(|window| window == b"startxref")
            .expect("test PDF has startxref");
        pdf.truncate(marker);
        pdf.extend_from_slice(b"%%EOF\n");
        pdf
    }

    fn wrong_startxref(pdf: Vec<u8>) -> Vec<u8> {
        let marker = pdf
            .windows(b"startxref".len())
            .rposition(|window| window == b"startxref")
            .expect("test PDF has startxref");
        let mut out = pdf[..marker + b"startxref".len()].to_vec();
        out.extend_from_slice(b"\n999999\n%%EOF\n");
        out
    }

    fn remove_xref_and_trailer(mut pdf: Vec<u8>) -> Vec<u8> {
        let marker = pdf
            .windows(b"xref".len())
            .position(|window| window == b"xref")
            .expect("test PDF has xref");
        pdf.truncate(marker);
        pdf.extend_from_slice(b"%%EOF\n");
        pdf
    }

    fn assert_recovered_catalog(pdf: Vec<u8>) {
        let reader = PdfReader::from_bytes(pdf).unwrap();
        assert_eq!(reader.root_reference(), Some((1, 0)));
        let root = reader.get_and_resolve(1, 0).unwrap();
        let root_dict = root.as_dict().unwrap();
        assert_eq!(root_dict.get_name("Type"), Some("Catalog"));
    }

    #[test]
    fn parses_xref_stream_entries_from_widths() {
        let dict = dict(&[
            (
                "W",
                PdfObject::Array(vec![
                    PdfObject::Integer(1),
                    PdfObject::Integer(2),
                    PdfObject::Integer(1),
                ]),
            ),
            (
                "Index",
                PdfObject::Array(vec![PdfObject::Integer(1), PdfObject::Integer(2)]),
            ),
        ]);
        let raw = [1, 0, 42, 0, 2, 0, 5, 3];
        let entries = parse_xref_stream_entries(&dict, &raw).unwrap();
        assert_eq!(
            entries,
            vec![
                (1, 0, XrefEntry::Uncompressed { offset: 42 }),
                (
                    2,
                    0,
                    XrefEntry::Compressed {
                        stream_obj: 5,
                        index: 3
                    }
                )
            ]
        );
    }

    #[test]
    fn classic_xref_tolerates_overlarge_free_generation() {
        let data = b"xref
0 2
0000000000 65536 f
0000000015 00000 n
trailer
<< /Size 2 /Root 1 0 R >>
";
        let mut xref = HashMap::new();

        read_classic_xref(data, 0, &mut xref).unwrap();

        assert!(matches!(xref.get(&(0, u16::MAX)), Some(XrefEntry::Free)));
        assert!(matches!(
            xref.get(&(1, 0)),
            Some(XrefEntry::Uncompressed { offset: 15 })
        ));
    }

    #[test]
    fn xref_section_repairs_forward_classic_xref_offset() {
        let mut data = vec![b' '; 192];
        let xref_offset = 64usize;
        let xref = b"xref
0 2
0000000000 65535 f
0000000015 00000 n
trailer
<< /Size 2 /Root 1 0 R >>
";
        data[xref_offset..xref_offset + xref.len()].copy_from_slice(xref);
        let mut xref_map = HashMap::new();

        read_xref_section(&data, xref_offset - 40, &mut xref_map).unwrap();

        assert!(matches!(
            xref_map.get(&(1, 0)),
            Some(XrefEntry::Uncompressed { offset: 15 })
        ));
    }

    #[test]
    fn reader_repairs_bad_uncompressed_xref_offsets() {
        let mut pdf = tiny_pdf();
        let obj2 = pdf
            .windows(b"2 0 obj".len())
            .position(|window| window == b"2 0 obj")
            .unwrap();
        let old = format!("{obj2:010}");
        let bad = format!("{:010}", obj2 - 1);
        let text = String::from_utf8(pdf).unwrap().replace(&old, &bad);
        pdf = text.into_bytes();

        let reader = PdfReader::from_bytes(pdf).unwrap();

        assert!(matches!(
            reader.get_object(2, 0).unwrap(),
            PdfObject::Dictionary(_)
        ));
    }

    #[test]
    fn reader_recovers_when_startxref_is_missing_but_trailer_exists() {
        assert_recovered_catalog(remove_startxref(tiny_pdf()));
    }

    #[test]
    fn reader_recovers_when_startxref_points_beyond_eof() {
        assert_recovered_catalog(wrong_startxref(tiny_pdf()));
    }

    #[test]
    fn reader_synthesizes_trailer_from_object_scan_when_trailer_is_missing() {
        assert_recovered_catalog(remove_xref_and_trailer(tiny_pdf()));
    }

    #[test]
    fn xref_stream_tolerates_overlarge_free_generation() {
        let dict = dict(&[
            (
                "W",
                PdfObject::Array(vec![
                    PdfObject::Integer(1),
                    PdfObject::Integer(1),
                    PdfObject::Integer(3),
                ]),
            ),
            (
                "Index",
                PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Integer(1)]),
            ),
        ]);
        let raw = [0, 0, 1, 0, 0];

        let entries = parse_xref_stream_entries(&dict, &raw).unwrap();

        assert_eq!(entries, vec![(0, u16::MAX, XrefEntry::Free)]);
    }

    #[test]
    fn xref_stream_rejects_overlapping_index_ranges() {
        let dict = dict(&[
            (
                "W",
                PdfObject::Array(vec![
                    PdfObject::Integer(1),
                    PdfObject::Integer(1),
                    PdfObject::Integer(1),
                ]),
            ),
            (
                "Index",
                PdfObject::Array(vec![
                    PdfObject::Integer(1),
                    PdfObject::Integer(2),
                    PdfObject::Integer(2),
                    PdfObject::Integer(1),
                ]),
            ),
        ]);
        let raw = [1, 10, 0, 1, 20, 0, 1, 30, 0];

        let err = parse_xref_stream_entries(&dict, &raw).unwrap_err();

        assert!(err
            .to_string()
            .contains("xref stream /Index contains duplicate object 2"));
    }

    #[test]
    fn parses_object_stream_data_by_object_number() {
        let decoded = b"10 0 11 5 true /Name";
        let objects = parse_object_stream_data(decoded, 2, 10, None).unwrap();
        assert_eq!(objects.get(&10).unwrap().1, PdfObject::Boolean(true));
        assert_eq!(
            objects.get(&11).unwrap().1,
            PdfObject::Name("Name".to_string())
        );
    }

    #[test]
    fn object_stream_rejects_duplicate_object_numbers() {
        let decoded = b"10 0 10 5 true /Name";

        let err = parse_object_stream_data(decoded, 2, 10, None).unwrap_err();

        assert!(err
            .to_string()
            .contains("object stream contains duplicate object 10"));
    }

    #[test]
    fn object_stream_huge_n_does_not_allocate_or_panic() {
        // A crafted object stream declaring a colossal /N count in a tiny
        // buffer must NOT preallocate gigabytes (OOM) — the capacity hint is
        // bounded by the header length — and must return a clean error once
        // the header runs out of tokens. Regression for the unbounded
        // `Vec::with_capacity(n)` allocation (fuzz finding: ObjStm /N OOM).
        let decoded = b"10 0 true";
        let result = parse_object_stream_data(decoded, usize::MAX, 5, None);
        assert!(
            result.is_err(),
            "huge /N over a short header must error, not allocate or panic"
        );
    }

    #[test]
    fn repair_scan_ignores_object_like_bytes_inside_streams() {
        let data = b"%PDF-1.7
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Count 0 /Kids [] >>
endobj
3 0 obj
<< /Length 42 >>
stream
9 0 obj
<< /Type /Catalog /Pages 99 0 R >>
endobj
endstream
endobj
%%EOF
";

        let scanned = scan_indirect_object_headers(data);

        assert!(scanned.contains_key(&(1, 0)));
        assert!(scanned.contains_key(&(2, 0)));
        assert!(scanned.contains_key(&(3, 0)));
        assert!(!scanned.contains_key(&(9, 0)));
    }
}
