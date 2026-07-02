use std::collections::VecDeque;
use std::io::{self, Cursor, Read};

use flate2::read::{DeflateDecoder, ZlibDecoder};

use crate::error::{OxideError, Result};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

/// Absolute backstop on how large a single FlateDecode stream may expand to.
///
/// This guards against decompression bombs: a tiny compressed stream that
/// inflates to gigabytes and OOMs the process. 512 MiB comfortably exceeds any
/// legitimate single PDF stream (the largest real streams are uncompressed
/// images, themselves bounded by page/image limits) while stopping absurd
/// expansion ratios. The server layers tighter, configurable per-request caps
/// on top of this; this is the engine's own hard floor so any caller — CLI,
/// tests, embedders — is protected even without the server.
pub const MAX_FLATE_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILTER_OUTPUT_BYTES: u64 = MAX_FLATE_DECOMPRESSED_BYTES;
const MAX_FILTER_CHAIN_DEPTH: usize = 16;
const MAX_PREDICTOR_ROW_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamDecodeStatus {
    Complete,
    StoppedAtImageFilter(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedStream {
    pub data: Vec<u8>,
    pub status: StreamDecodeStatus,
}

pub(crate) struct DecodedStreamReader<'a> {
    pub reader: Box<dyn Read + 'a>,
    pub status: StreamDecodeStatus,
}

/// Fully decodes a stream through all implemented filters.
///
/// Image-only filters (`DCTDecode`, `JPXDecode`, `CCITTFaxDecode`, and
/// `JBIG2Decode`) are intentionally not decoded in this layer. Use
/// [`decode_stream_lossless`] when callers want bytes decoded through preceding
/// lossless filters and an explicit status naming the remaining image filter.
pub fn decode_stream(stream: &PdfObject, reader: &PdfReader) -> Result<Vec<u8>> {
    let decoded = decode_stream_lossless(stream, reader)?;
    match decoded.status {
        StreamDecodeStatus::Complete => Ok(decoded.data),
        StreamDecodeStatus::StoppedAtImageFilter(filter) => Err(OxideError::UnsupportedFeature(
            format!("image filter remains: {filter}"),
        )),
    }
}

/// Compress `data` with zlib/DEFLATE (the PDF `FlateDecode` filter format) at
/// the given level (0..=9; 9 = best). The inverse of [`flate_decode`]. Used by
/// the `optimize` op to recompress uncompressed content streams.
pub fn flate_encode(data: &[u8], level: u32) -> Vec<u8> {
    use std::io::Write;
    let mut enc =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level.min(9)));
    // Writing to a Vec is infallible; finish() returns the compressed buffer.
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

/// Decodes implemented lossless filters in order and stops before an image
/// codec filter, returning the current bytes plus a status.
pub fn decode_stream_lossless(stream: &PdfObject, reader: &PdfReader) -> Result<DecodedStream> {
    let (dict, raw) = stream.as_stream().ok_or_else(|| {
        OxideError::MalformedPdf("decode_stream requires a stream object".to_string())
    })?;
    decode_stream_parts(dict, raw, Some(reader))
}

pub(crate) fn decode_stream_from_dict(dict: &PdfDictionary, raw: &[u8]) -> Result<Vec<u8>> {
    let decoded = decode_stream_parts(dict, raw, None)?;
    match decoded.status {
        StreamDecodeStatus::Complete => Ok(decoded.data),
        StreamDecodeStatus::StoppedAtImageFilter(filter) => Err(OxideError::UnsupportedFeature(
            format!("image filter remains: {filter}"),
        )),
    }
}

pub(crate) fn decode_stream_lossless_reader<'a, R: Read + 'a>(
    dict: &PdfDictionary,
    raw: R,
    reader: Option<&PdfReader>,
) -> Result<DecodedStreamReader<'a>> {
    decode_stream_reader_with_cap(dict, raw, reader, MAX_FLATE_DECOMPRESSED_BYTES)
}

pub(crate) fn apply_filter_bytes(
    filter_name: &str,
    input: &[u8],
    decode_parms: Option<&PdfDictionary>,
) -> Result<Vec<u8>> {
    match filter_name {
        "FlateDecode" | "Fl" => {
            let data = flate_decode(input)?;
            apply_predictor(data, decode_parms)
        }
        "LZWDecode" | "LZW" => {
            let early_change = int_param(decode_parms, "EarlyChange", 1)?;
            if !(0..=1).contains(&early_change) {
                return Err(OxideError::MalformedPdf(format!(
                    "invalid LZW EarlyChange value {early_change}"
                )));
            }
            let data = lzw_decode(input, early_change as u8)?;
            apply_predictor(data, decode_parms)
        }
        "ASCIIHexDecode" | "AHx" => ascii_hex_decode(input),
        "ASCII85Decode" | "A85" => ascii85_decode(input),
        "RunLengthDecode" | "RL" => run_length_decode(input),
        other => Err(OxideError::UnsupportedFeature(format!(
            "unsupported inline image filter {other}"
        ))),
    }
}

/// Fuzz-only entry point: drive a single stream decoder by a leading selector
/// byte, with the remaining input fed to that decoder as raw filter bytes.
///
/// Exposed only under the `fuzzing` feature so libFuzzer (which can reach
/// `pub` items only) can exercise the otherwise-private decoders directly,
/// without constructing a `PdfReader`. Not part of the normal public API.
#[cfg(feature = "fuzzing")]
pub fn fuzz_decode_filter(input: &[u8]) -> Result<Vec<u8>> {
    let Some((selector, rest)) = input.split_first() else {
        return Ok(Vec::new());
    };
    let filter = match selector % 6 {
        0 => "FlateDecode",
        1 => "LZWDecode",
        2 => "ASCIIHexDecode",
        3 => "ASCII85Decode",
        4 => "RunLengthDecode",
        // Exercise the predictor path on top of Flate with a small fixed
        // DecodeParms so the predictor code is reachable from the fuzzer.
        _ => "FlateDecode",
    };
    apply_filter_bytes(filter, rest, None)
}

/// Fuzz-only entry point for the PNG/TIFF predictor stage in isolation: the
/// first three bytes select Predictor, Colors, and Columns, the rest is the
/// data buffer.
#[cfg(feature = "fuzzing")]
pub fn fuzz_apply_predictor(input: &[u8]) -> Result<Vec<u8>> {
    let predictor = i64::from(input.first().copied().unwrap_or(0));
    let colors = i64::from(input.get(1).copied().unwrap_or(1)).max(1);
    let columns = i64::from(input.get(2).copied().unwrap_or(1)).max(1);
    let body = input.get(3..).unwrap_or(&[]).to_vec();

    let mut params = PdfDictionary::empty();
    params.insert("Predictor", PdfObject::Integer(predictor));
    params.insert("Colors", PdfObject::Integer(colors));
    params.insert("Columns", PdfObject::Integer(columns));
    params.insert("BitsPerComponent", PdfObject::Integer(8));

    apply_predictor(body, Some(&params))
}

fn decode_stream_parts(
    dict: &PdfDictionary,
    raw: &[u8],
    reader: Option<&PdfReader>,
) -> Result<DecodedStream> {
    let filters = filter_names(dict, reader)?;
    enforce_filter_chain_depth(filters.len())?;
    let params = decode_params(dict, reader, filters.len())?;
    let mut data = raw.to_vec();

    for (idx, filter) in filters.iter().enumerate() {
        let param = params.get(idx).and_then(Option::as_ref);
        match filter.as_str() {
            "FlateDecode" | "Fl" => {
                data = flate_decode(&data)?;
                data = apply_predictor(data, param)?;
            }
            "LZWDecode" | "LZW" => {
                let early_change = int_param(param, "EarlyChange", 1)?;
                if !(0..=1).contains(&early_change) {
                    return Err(OxideError::MalformedPdf(format!(
                        "invalid LZW EarlyChange value {early_change}"
                    )));
                }
                data = lzw_decode(&data, early_change as u8)?;
                data = apply_predictor(data, param)?;
            }
            "ASCIIHexDecode" | "AHx" => data = ascii_hex_decode(&data)?,
            "ASCII85Decode" | "A85" => data = ascii85_decode(&data)?,
            "RunLengthDecode" | "RL" => data = run_length_decode(&data)?,
            // The reader applies PDF crypt filters while fetching stream
            // objects, because decryption needs object/generation numbers and
            // the document encryption dictionary. At the decode-filter layer,
            // `/Crypt` is therefore just the marker for that already-applied
            // step. If there is no active encryption context, only the
            // explicit `/Identity` crypt filter is a no-op; any other crypt
            // filter means the caller needs to reopen with the right password.
            "Crypt" => {
                if reader.and_then(PdfReader::encryption).is_none()
                    && !crypt_filter_is_identity(param)
                {
                    return Err(OxideError::EncryptedPdf(
                        "stream uses /Crypt filter; provide the correct password".to_string(),
                    ));
                }
            }
            "DCTDecode" | "DCT" | "JPXDecode" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode" => {
                return Ok(DecodedStream {
                    data,
                    status: StreamDecodeStatus::StoppedAtImageFilter(filter.clone()),
                });
            }
            other => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "unsupported stream filter {other}"
                )));
            }
        }
    }

    Ok(DecodedStream {
        data,
        status: StreamDecodeStatus::Complete,
    })
}

fn decode_stream_reader_with_cap<'a, R: Read + 'a>(
    dict: &PdfDictionary,
    raw: R,
    reader: Option<&PdfReader>,
    cap: u64,
) -> Result<DecodedStreamReader<'a>> {
    let filters = filter_names(dict, reader)?;
    enforce_filter_chain_depth(filters.len())?;
    let params = decode_params(dict, reader, filters.len())?;
    let mut current: Box<dyn Read + 'a> = Box::new(raw);

    for (idx, filter) in filters.iter().enumerate() {
        let param = params.get(idx).and_then(Option::as_ref);
        match filter.as_str() {
            "FlateDecode" | "Fl" => {
                let flate = StreamingFlateReader::new(current).map_err(OxideError::Io)?;
                current = Box::new(CappedReader::new(
                    flate,
                    cap,
                    "FlateDecode output exceeds decompression cap",
                ));
                current = streaming_predictor_reader(current, param)?;
            }
            "LZWDecode" | "LZW" => {
                let early_change = int_param(param, "EarlyChange", 1)?;
                if !(0..=1).contains(&early_change) {
                    return Err(OxideError::MalformedPdf(format!(
                        "invalid LZW EarlyChange value {early_change}"
                    )));
                }
                current = Box::new(CappedReader::new(
                    LzwReader::new(current, early_change as u8),
                    cap,
                    "LZWDecode output exceeds decompression cap",
                ));
                current = streaming_predictor_reader(current, param)?;
            }
            "ASCIIHexDecode" | "AHx" => {
                current = Box::new(CappedReader::new(
                    AsciiHexReader::new(current),
                    cap,
                    "ASCIIHexDecode output exceeds decompression cap",
                ));
            }
            "ASCII85Decode" | "A85" => {
                current = Box::new(CappedReader::new(
                    Ascii85Reader::new(current),
                    cap,
                    "ASCII85Decode output exceeds decompression cap",
                ));
            }
            "RunLengthDecode" | "RL" => {
                current = Box::new(CappedReader::new(
                    RunLengthReader::new(current),
                    cap,
                    "RunLengthDecode output exceeds decompression cap",
                ));
            }
            "Crypt" => {
                if reader.and_then(PdfReader::encryption).is_none()
                    && !crypt_filter_is_identity(param)
                {
                    return Err(OxideError::EncryptedPdf(
                        "stream uses /Crypt filter; provide the correct password".to_string(),
                    ));
                }
            }
            "DCTDecode" | "DCT" | "JPXDecode" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode" => {
                return Ok(DecodedStreamReader {
                    reader: current,
                    status: StreamDecodeStatus::StoppedAtImageFilter(filter.clone()),
                });
            }
            other => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "unsupported stream filter {other}"
                )));
            }
        }
    }

    Ok(DecodedStreamReader {
        reader: current,
        status: StreamDecodeStatus::Complete,
    })
}

fn streaming_predictor_reader<'a>(
    current: Box<dyn Read + 'a>,
    params: Option<&PdfDictionary>,
) -> Result<Box<dyn Read + 'a>> {
    let predictor = int_param(params, "Predictor", 1)?;
    if predictor == 1 {
        return Ok(current);
    }
    Ok(Box::new(PredictorReader::new(current, params)?))
}

fn reader_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn result_to_io<T>(result: Result<T>) -> io::Result<T> {
    result.map_err(|err| reader_error(err.to_string()))
}

fn read_one<R: Read>(inner: &mut R) -> io::Result<Option<u8>> {
    let mut byte = [0u8; 1];
    match inner.read(&mut byte)? {
        0 => Ok(None),
        _ => Ok(Some(byte[0])),
    }
}

struct PrefixReader<R> {
    prefix: Cursor<Vec<u8>>,
    inner: R,
}

impl<R> PrefixReader<R> {
    fn new(prefix: Vec<u8>, inner: R) -> Self {
        Self {
            prefix: Cursor::new(prefix),
            inner,
        }
    }
}

impl<R: Read> Read for PrefixReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let prefix_read = self.prefix.read(buf)?;
        if prefix_read > 0 {
            return Ok(prefix_read);
        }
        self.inner.read(buf)
    }
}

enum StreamingFlateReader<R: Read> {
    Zlib(ZlibDecoder<PrefixReader<R>>),
    Raw(DeflateDecoder<PrefixReader<R>>),
}

impl<R: Read> StreamingFlateReader<R> {
    fn new(mut inner: R) -> io::Result<Self> {
        let mut prefix = Vec::with_capacity(2);
        while prefix.len() < 2 {
            match read_one(&mut inner)? {
                Some(byte) => prefix.push(byte),
                None => break,
            }
        }
        let reader = PrefixReader::new(prefix.clone(), inner);
        if looks_like_zlib_header(&prefix) {
            Ok(Self::Zlib(ZlibDecoder::new(reader)))
        } else {
            Ok(Self::Raw(DeflateDecoder::new(reader)))
        }
    }
}

impl<R: Read> Read for StreamingFlateReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Zlib(reader) => reader.read(buf),
            Self::Raw(reader) => reader.read(buf),
        }
    }
}

fn looks_like_zlib_header(prefix: &[u8]) -> bool {
    let [cmf, flg, ..] = prefix else {
        return true;
    };
    (cmf & 0x0F) == 8 && (cmf >> 4) <= 7 && ((u16::from(*cmf) << 8) | u16::from(*flg)) % 31 == 0
}

struct CappedReader<R> {
    inner: R,
    emitted: u64,
    cap: u64,
    message: &'static str,
}

impl<R> CappedReader<R> {
    fn new(inner: R, cap: u64, message: &'static str) -> Self {
        Self {
            inner,
            emitted: 0,
            cap,
            message,
        }
    }
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.emitted > self.cap {
            return Err(reader_error(self.message));
        }
        let remaining = self.cap.saturating_sub(self.emitted).saturating_add(1);
        let max_len = buf
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max_len])?;
        self.emitted = self
            .emitted
            .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        if self.emitted > self.cap {
            return Err(reader_error(self.message));
        }
        Ok(n)
    }
}

struct AsciiHexReader<R> {
    inner: R,
    high: Option<u8>,
    pending: VecDeque<u8>,
    done: bool,
}

impl<R> AsciiHexReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            high: None,
            pending: VecDeque::new(),
            done: false,
        }
    }
}

impl<R: Read> Read for AsciiHexReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < buf.len() {
            if let Some(byte) = self.pending.pop_front() {
                buf[written] = byte;
                written += 1;
                continue;
            }
            if self.done {
                break;
            }
            match read_one(&mut self.inner)? {
                Some(b'>') => {
                    if let Some(high) = self.high.take() {
                        self.pending.push_back(high << 4);
                    }
                    self.done = true;
                }
                Some(byte) if is_pdf_whitespace(byte) => {}
                Some(byte) => {
                    let value = hex_value(byte).ok_or_else(|| {
                        reader_error(format!("invalid ASCIIHex digit 0x{byte:02X}"))
                    })?;
                    match self.high.take() {
                        Some(high) => self.pending.push_back((high << 4) | value),
                        None => self.high = Some(value),
                    }
                }
                None => {
                    if let Some(high) = self.high.take() {
                        self.pending.push_back(high << 4);
                    }
                    self.done = true;
                }
            }
        }
        Ok(written)
    }
}

struct Ascii85Reader<R> {
    inner: R,
    group: Vec<u8>,
    pending: VecDeque<u8>,
    done: bool,
}

impl<R: Read> Ascii85Reader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            group: Vec::with_capacity(5),
            pending: VecDeque::new(),
            done: false,
        }
    }

    fn push_group(&mut self, output_len: usize) -> io::Result<()> {
        let mut out = Vec::new();
        result_to_io(push_ascii85_group(&self.group, output_len, &mut out))?;
        self.pending.extend(out);
        self.group.clear();
        Ok(())
    }

    fn finish_partial(&mut self) -> io::Result<()> {
        if self.group.is_empty() {
            return Ok(());
        }
        if self.group.len() == 1 {
            return Err(reader_error("ASCII85 final group cannot contain one digit"));
        }
        let output_len = self.group.len() - 1;
        while self.group.len() < 5 {
            self.group.push(84);
        }
        self.push_group(output_len)
    }

    fn fill_pending(&mut self) -> io::Result<()> {
        while self.pending.is_empty() && !self.done {
            let Some(byte) = read_one(&mut self.inner)? else {
                self.finish_partial()?;
                self.done = true;
                break;
            };
            if is_pdf_whitespace(byte) {
                continue;
            }
            if byte == b'~' {
                let mut saw_end = false;
                while let Some(next) = read_one(&mut self.inner)? {
                    if is_pdf_whitespace(next) {
                        continue;
                    }
                    if next == b'>' {
                        saw_end = true;
                        break;
                    }
                    return Err(reader_error("ASCII85 '~' must be followed by '>'"));
                }
                if !saw_end {
                    return Err(reader_error("unterminated ASCII85 EOD marker"));
                }
                self.finish_partial()?;
                self.done = true;
                break;
            }
            if byte == b'z' {
                if !self.group.is_empty() {
                    return Err(reader_error("ASCII85 'z' cannot appear inside a group"));
                }
                self.pending.extend([0, 0, 0, 0]);
                break;
            }
            if !(b'!'..=b'u').contains(&byte) {
                return Err(reader_error(format!("invalid ASCII85 byte 0x{byte:02X}")));
            }
            self.group.push(byte - b'!');
            if self.group.len() == 5 {
                self.push_group(4)?;
                break;
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for Ascii85Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < buf.len() {
            if self.pending.is_empty() {
                self.fill_pending()?;
            }
            let Some(byte) = self.pending.pop_front() else {
                break;
            };
            buf[written] = byte;
            written += 1;
        }
        Ok(written)
    }
}

struct RunLengthReader<R> {
    inner: R,
    pending: VecDeque<u8>,
    done: bool,
}

impl<R: Read> RunLengthReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            pending: VecDeque::new(),
            done: false,
        }
    }

    fn fill_pending(&mut self) -> io::Result<()> {
        while self.pending.is_empty() && !self.done {
            let Some(len) = read_one(&mut self.inner)? else {
                self.done = true;
                break;
            };
            match len {
                0..=127 => {
                    let count = usize::from(len) + 1;
                    for _ in 0..count {
                        let byte = read_one(&mut self.inner)?
                            .ok_or_else(|| reader_error("truncated RunLength literal run"))?;
                        self.pending.push_back(byte);
                    }
                }
                128 => self.done = true,
                129..=255 => {
                    let byte = read_one(&mut self.inner)?
                        .ok_or_else(|| reader_error("truncated RunLength repeat run"))?;
                    let count = usize::from(257u16 - u16::from(len));
                    self.pending.extend(std::iter::repeat_n(byte, count));
                }
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for RunLengthReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < buf.len() {
            if self.pending.is_empty() {
                self.fill_pending()?;
            }
            let Some(byte) = self.pending.pop_front() else {
                break;
            };
            buf[written] = byte;
            written += 1;
        }
        Ok(written)
    }
}

struct StreamingMsbBitReader<R> {
    inner: R,
    bit_buffer: u32,
    bit_count: usize,
}

impl<R> StreamingMsbBitReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            bit_buffer: 0,
            bit_count: 0,
        }
    }
}

impl<R: Read> StreamingMsbBitReader<R> {
    fn read_bits(&mut self, count: usize) -> io::Result<Option<u16>> {
        if count == 0 {
            return Ok(None);
        }
        while self.bit_count < count {
            let Some(byte) = read_one(&mut self.inner)? else {
                return Ok(None);
            };
            self.bit_buffer = (self.bit_buffer << 8) | u32::from(byte);
            self.bit_count += 8;
        }
        let shift = self.bit_count - count;
        let mask = (1u32 << count) - 1;
        let value = (self.bit_buffer >> shift) & mask;
        self.bit_buffer = if shift == 0 {
            0
        } else {
            self.bit_buffer & ((1u32 << shift) - 1)
        };
        self.bit_count = shift;
        Ok(Some(value as u16))
    }
}

struct LzwReader<R: Read> {
    bits: StreamingMsbBitReader<R>,
    table: Vec<Option<Vec<u8>>>,
    code_width: usize,
    next_code: usize,
    previous: Option<Vec<u8>>,
    pending: VecDeque<u8>,
    done: bool,
    early_change: u8,
}

impl<R: Read> LzwReader<R> {
    fn new(inner: R, early_change: u8) -> Self {
        Self {
            bits: StreamingMsbBitReader::new(inner),
            table: initial_lzw_table(),
            code_width: 9,
            next_code: 258,
            previous: None,
            pending: VecDeque::new(),
            done: false,
            early_change,
        }
    }

    fn reset_table(&mut self) {
        self.table = initial_lzw_table();
        self.code_width = 9;
        self.next_code = 258;
        self.previous = None;
    }

    fn fill_pending(&mut self) -> io::Result<()> {
        while self.pending.is_empty() && !self.done {
            let Some(code) = self.bits.read_bits(self.code_width)? else {
                self.done = true;
                break;
            };
            match code {
                256 => self.reset_table(),
                257 => self.done = true,
                code => {
                    let code_usize = usize::from(code);
                    let entry = if code_usize < self.table.len() {
                        self.table[code_usize].clone().ok_or_else(|| {
                            reader_error(format!("invalid empty LZW code {code_usize}"))
                        })?
                    } else if code_usize == self.next_code {
                        let prev = self
                            .previous
                            .as_ref()
                            .ok_or_else(|| reader_error("LZW KwKwK code without previous entry"))?;
                        let mut synthesized = prev.clone();
                        let first = *prev
                            .first()
                            .ok_or_else(|| reader_error("empty LZW previous entry"))?;
                        synthesized.push(first);
                        synthesized
                    } else {
                        return Err(reader_error(format!("invalid LZW code {code_usize}")));
                    };

                    self.pending.extend(entry.iter().copied());

                    if let Some(prev) = self.previous.as_ref() {
                        if self.next_code < 4096 {
                            let mut new_entry = prev.clone();
                            let first = *entry
                                .first()
                                .ok_or_else(|| reader_error("empty LZW entry"))?;
                            new_entry.push(first);
                            if self.table.len() <= self.next_code {
                                self.table.resize(self.next_code + 1, None);
                            }
                            self.table[self.next_code] = Some(new_entry);
                            self.next_code += 1;
                            if self.code_width < 12
                                && self.next_code + usize::from(self.early_change)
                                    >= (1usize << self.code_width)
                            {
                                self.code_width += 1;
                            }
                        }
                    }

                    self.previous = Some(entry);
                }
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for LzwReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < buf.len() {
            if self.pending.is_empty() {
                self.fill_pending()?;
            }
            let Some(byte) = self.pending.pop_front() else {
                break;
            };
            buf[written] = byte;
            written += 1;
        }
        Ok(written)
    }
}

struct PredictorReader<R> {
    inner: R,
    mode: PredictorMode,
    pending: VecDeque<u8>,
    done: bool,
}

enum PredictorMode {
    Tiff {
        row_len: usize,
        colors: usize,
        bits_per_component: usize,
    },
    Png {
        row_len: usize,
        bytes_per_pixel: usize,
        prev_row: Vec<u8>,
    },
}

impl<R: Read> PredictorReader<R> {
    fn new(inner: R, params: Option<&PdfDictionary>) -> Result<Self> {
        let predictor = int_param(params, "Predictor", 1)?;
        let columns = positive_usize_param(params, "Columns", 1)?;
        let colors = positive_usize_param(params, "Colors", 1)?;
        let bits_per_component = positive_usize_param(params, "BitsPerComponent", 8)?;
        let row_bits = columns
            .checked_mul(colors)
            .and_then(|v| v.checked_mul(bits_per_component))
            .ok_or_else(|| {
                OxideError::MalformedPdf(
                    "predictor row dimensions overflow (Columns x Colors x BitsPerComponent)"
                        .to_string(),
                )
            })?;
        let row_len = ceil_div(row_bits, 8);
        validate_predictor_row_len(row_len)?;
        let mode = match predictor {
            2 => PredictorMode::Tiff {
                row_len,
                colors,
                bits_per_component,
            },
            10..=15 => PredictorMode::Png {
                row_len,
                bytes_per_pixel: predictor_bytes_per_pixel(colors, bits_per_component)?,
                prev_row: vec![0; row_len],
            },
            other => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "unsupported predictor {other}"
                )));
            }
        };
        Ok(Self {
            inner,
            mode,
            pending: VecDeque::new(),
            done: false,
        })
    }

    fn read_exact_or_eof(&mut self, len: usize, context: &str) -> io::Result<Option<Vec<u8>>> {
        let mut row = vec![0u8; len];
        let mut filled = 0usize;
        while filled < len {
            let n = self.inner.read(&mut row[filled..])?;
            if n == 0 {
                if filled == 0 {
                    return Ok(None);
                }
                return Err(reader_error(format!(
                    "{context} data length is not row-aligned"
                )));
            }
            filled += n;
        }
        Ok(Some(row))
    }

    fn fill_pending(&mut self) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        enum ModeSnapshot {
            Tiff {
                row_len: usize,
                colors: usize,
                bits_per_component: usize,
            },
            Png {
                row_len: usize,
                bytes_per_pixel: usize,
                prev_row: Vec<u8>,
            },
        }
        let snapshot = match &self.mode {
            PredictorMode::Tiff {
                row_len,
                colors,
                bits_per_component,
            } => ModeSnapshot::Tiff {
                row_len: *row_len,
                colors: *colors,
                bits_per_component: *bits_per_component,
            },
            PredictorMode::Png {
                row_len,
                bytes_per_pixel,
                prev_row,
            } => ModeSnapshot::Png {
                row_len: *row_len,
                bytes_per_pixel: *bytes_per_pixel,
                prev_row: prev_row.clone(),
            },
        };
        match snapshot {
            ModeSnapshot::Tiff {
                row_len,
                colors,
                bits_per_component,
            } => {
                let Some(mut row) = self.read_exact_or_eof(row_len, "TIFF predictor")? else {
                    self.done = true;
                    return Ok(());
                };
                result_to_io(decode_tiff_predictor_row(
                    &mut row,
                    row_len,
                    colors,
                    bits_per_component,
                ))?;
                self.pending.extend(row);
            }
            ModeSnapshot::Png {
                row_len,
                bytes_per_pixel,
                prev_row,
            } => {
                let row_with_filter = row_len
                    .checked_add(1)
                    .ok_or_else(|| reader_error("PNG predictor row dimensions overflow"))?;
                let Some(encoded_row) = self.read_exact_or_eof(row_with_filter, "PNG predictor")?
                else {
                    self.done = true;
                    return Ok(());
                };
                let filter = encoded_row[0];
                let mut row = encoded_row[1..].to_vec();
                result_to_io(decode_png_predictor_row(
                    &mut row,
                    filter,
                    &prev_row,
                    bytes_per_pixel,
                ))?;
                if let PredictorMode::Png { prev_row, .. } = &mut self.mode {
                    *prev_row = row.clone();
                }
                self.pending.extend(row);
            }
        }
        Ok(())
    }
}

impl<R: Read> Read for PredictorReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut written = 0usize;
        while written < buf.len() {
            if self.pending.is_empty() {
                self.fill_pending()?;
            }
            let Some(byte) = self.pending.pop_front() else {
                break;
            };
            buf[written] = byte;
            written += 1;
        }
        Ok(written)
    }
}

fn resolved_object(obj: &PdfObject, reader: Option<&PdfReader>) -> Result<PdfObject> {
    match reader {
        Some(reader) => reader.resolve(obj.clone()),
        None => Ok(obj.clone()),
    }
}

fn crypt_filter_is_identity(param: Option<&PdfDictionary>) -> bool {
    matches!(
        param.and_then(|dict| dict.get_name("Name")),
        Some("Identity")
    )
}

fn enforce_filter_chain_depth(filter_count: usize) -> Result<()> {
    if filter_count > MAX_FILTER_CHAIN_DEPTH {
        return Err(OxideError::MalformedPdf(format!(
            "filter chain depth {filter_count} exceeds limit of {MAX_FILTER_CHAIN_DEPTH}"
        )));
    }
    Ok(())
}

fn filter_names(dict: &PdfDictionary, reader: Option<&PdfReader>) -> Result<Vec<String>> {
    let Some(filter_obj) = dict.get("Filter").or_else(|| dict.get("F")) else {
        return Ok(Vec::new());
    };
    let filter_obj = resolved_object(filter_obj, reader)?;
    match filter_obj {
        PdfObject::Name(name) => Ok(vec![name]),
        PdfObject::Array(items) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                match resolved_object(&item, reader)? {
                    PdfObject::Name(name) => names.push(name),
                    other => {
                        return Err(OxideError::MalformedPdf(format!(
                            "filter array contains {}",
                            other.variant_name()
                        )));
                    }
                }
            }
            Ok(names)
        }
        PdfObject::Null => Ok(Vec::new()),
        other => Err(OxideError::MalformedPdf(format!(
            "Filter must be a name or array, got {}",
            other.variant_name()
        ))),
    }
}

fn decode_params(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    filter_count: usize,
) -> Result<Vec<Option<PdfDictionary>>> {
    let Some(params_obj) = dict.get("DecodeParms").or_else(|| dict.get("DP")) else {
        return Ok(vec![None; filter_count]);
    };
    let params_obj = resolved_object(params_obj, reader)?;
    match params_obj {
        PdfObject::Null => Ok(vec![None; filter_count]),
        PdfObject::Dictionary(params) => {
            let mut out = vec![None; filter_count];
            if !out.is_empty() {
                out[0] = Some(params);
            }
            Ok(out)
        }
        PdfObject::Array(items) => {
            let mut out = Vec::with_capacity(filter_count);
            for item in items.into_iter().take(filter_count) {
                match resolved_object(&item, reader)? {
                    PdfObject::Null => out.push(None),
                    PdfObject::Dictionary(params) => out.push(Some(params)),
                    other => {
                        return Err(OxideError::MalformedPdf(format!(
                            "DecodeParms array contains {}",
                            other.variant_name()
                        )));
                    }
                }
            }
            while out.len() < filter_count {
                out.push(None);
            }
            Ok(out)
        }
        other => Err(OxideError::MalformedPdf(format!(
            "DecodeParms must be a dictionary or array, got {}",
            other.variant_name()
        ))),
    }
}

fn flate_decode(data: &[u8]) -> Result<Vec<u8>> {
    flate_decode_capped(data, MAX_FLATE_DECOMPRESSED_BYTES)
}

/// FlateDecode with an explicit decompressed-size cap (parameterized so tests
/// can exercise the bomb guard without allocating the full production cap).
fn flate_decode_capped(data: &[u8], cap: u64) -> Result<Vec<u8>> {
    // Cap reads at one byte over the limit so we can distinguish "exactly at the
    // limit" (fine) from "exceeded it" (bomb). `take` makes the decoder stop
    // reading past the cap instead of inflating unbounded into memory.
    let read_cap = cap + 1;
    let mut out = Vec::new();
    let mut zlib = ZlibDecoder::new(data).take(read_cap);
    match zlib.read_to_end(&mut out) {
        Ok(_) => check_decompressed_size(out, cap),
        Err(zlib_error) => {
            let mut raw_out = Vec::new();
            let mut deflate = DeflateDecoder::new(data).take(read_cap);
            match deflate.read_to_end(&mut raw_out) {
                Ok(_) => check_decompressed_size(raw_out, cap),
                Err(_) => Err(OxideError::ParseError(format!(
                    "FlateDecode failed: {zlib_error}"
                ))),
            }
        }
    }
}

/// Reject output that hit the decompression-bomb backstop.
fn check_decompressed_size(out: Vec<u8>, cap: u64) -> Result<Vec<u8>> {
    if out.len() as u64 > cap {
        return Err(OxideError::MalformedPdf(format!(
            "FlateDecode output exceeds {} byte limit (possible decompression bomb)",
            cap
        )));
    }
    Ok(out)
}

pub(crate) fn apply_predictor(data: Vec<u8>, params: Option<&PdfDictionary>) -> Result<Vec<u8>> {
    let predictor = int_param(params, "Predictor", 1)?;
    if predictor == 1 {
        return Ok(data);
    }

    let columns = positive_usize_param(params, "Columns", 1)?;
    let colors = positive_usize_param(params, "Colors", 1)?;
    let bits_per_component = positive_usize_param(params, "BitsPerComponent", 8)?;
    // Columns/Colors/BitsPerComponent are attacker-controlled. Computing the
    // row length by plain multiplication can overflow `usize` on crafted input
    // (e.g. a huge /Columns), which panics under overflow checks and silently
    // wraps to a bogus length otherwise. Use checked arithmetic and reject
    // overflow as malformed rather than crashing or misparsing.
    let row_bits = columns
        .checked_mul(colors)
        .and_then(|v| v.checked_mul(bits_per_component))
        .ok_or_else(|| {
            OxideError::MalformedPdf(
                "predictor row dimensions overflow (Columns × Colors × BitsPerComponent)"
                    .to_string(),
            )
        })?;
    let row_len = ceil_div(row_bits, 8);
    if row_len == 0 {
        return Ok(data);
    }
    validate_predictor_row_len(row_len)?;

    match predictor {
        2 => tiff_predictor(data, row_len, colors, bits_per_component),
        10..=15 => png_predictor(data, row_len, colors, bits_per_component),
        other => Err(OxideError::UnsupportedFeature(format!(
            "unsupported predictor {other}"
        ))),
    }
}

fn int_param(params: Option<&PdfDictionary>, key: &str, default: i64) -> Result<i64> {
    match params.and_then(|dict| dict.get(key)) {
        Some(PdfObject::Integer(value)) => Ok(*value),
        Some(other) => Err(OxideError::MalformedPdf(format!(
            "DecodeParms /{key} must be an integer, got {}",
            other.variant_name()
        ))),
        None => Ok(default),
    }
}

fn positive_usize_param(
    params: Option<&PdfDictionary>,
    key: &str,
    default: usize,
) -> Result<usize> {
    let value = int_param(params, key, default as i64)?;
    if value <= 0 {
        return Err(OxideError::MalformedPdf(format!(
            "DecodeParms /{key} must be positive"
        )));
    }
    usize::try_from(value).map_err(|_| {
        OxideError::MalformedPdf(format!("DecodeParms /{key} is too large for this platform"))
    })
}

fn tiff_predictor(
    mut data: Vec<u8>,
    row_len: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    if !data.len().is_multiple_of(row_len) {
        return Err(OxideError::MalformedPdf(
            "TIFF predictor data length is not row-aligned".to_string(),
        ));
    }

    for row in data.chunks_mut(row_len) {
        decode_tiff_predictor_row(row, row_len, colors, bits_per_component)?;
    }

    Ok(data)
}

fn decode_tiff_predictor_row(
    row: &mut [u8],
    row_len: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<()> {
    match bits_per_component {
        8 => {
            for idx in colors..row.len() {
                row[idx] = row[idx].wrapping_add(row[idx - colors]);
            }
        }
        16 => {
            let stride = colors * 2;
            if !row_len.is_multiple_of(2) {
                return Err(OxideError::MalformedPdf(
                    "16-bit TIFF predictor row has odd byte length".to_string(),
                ));
            }
            let mut idx = stride;
            while idx + 1 < row.len() {
                let current = u16::from_be_bytes([row[idx], row[idx + 1]]);
                let prior = u16::from_be_bytes([row[idx - stride], row[idx + 1 - stride]]);
                let decoded = current.wrapping_add(prior).to_be_bytes();
                row[idx] = decoded[0];
                row[idx + 1] = decoded[1];
                idx += 2;
            }
        }
        other => {
            return Err(OxideError::UnsupportedFeature(format!(
                "TIFF predictor with {other} bits per component"
            )));
        }
    }
    Ok(())
}

fn png_predictor(
    data: Vec<u8>,
    row_len: usize,
    colors: usize,
    bits_per_component: usize,
) -> Result<Vec<u8>> {
    let row_with_filter = row_len.checked_add(1).ok_or_else(|| {
        OxideError::MalformedPdf("PNG predictor row dimensions overflow".to_string())
    })?;
    if !data.len().is_multiple_of(row_with_filter) {
        return Err(OxideError::MalformedPdf(
            "PNG predictor data length is not row-aligned".to_string(),
        ));
    }

    let bytes_per_pixel = predictor_bytes_per_pixel(colors, bits_per_component)?;
    let mut out = Vec::with_capacity((data.len() / row_with_filter) * row_len);
    let mut prev_row = vec![0u8; row_len];

    for encoded_row in data.chunks(row_with_filter) {
        let filter = encoded_row[0];
        let encoded = &encoded_row[1..];
        let mut row = encoded.to_vec();
        decode_png_predictor_row(&mut row, filter, &prev_row, bytes_per_pixel)?;
        out.extend_from_slice(&row);
        prev_row = row;
    }

    Ok(out)
}

fn decode_png_predictor_row(
    row: &mut [u8],
    filter: u8,
    prev_row: &[u8],
    bytes_per_pixel: usize,
) -> Result<()> {
    match filter {
        0 => {}
        1 => {
            for idx in 0..row.len() {
                let left = idx
                    .checked_sub(bytes_per_pixel)
                    .and_then(|left_idx| row.get(left_idx).copied())
                    .unwrap_or(0);
                row[idx] = row[idx].wrapping_add(left);
            }
        }
        2 => {
            for idx in 0..row.len() {
                row[idx] = row[idx].wrapping_add(prev_row[idx]);
            }
        }
        3 => {
            for idx in 0..row.len() {
                let left = idx
                    .checked_sub(bytes_per_pixel)
                    .and_then(|left_idx| row.get(left_idx).copied())
                    .unwrap_or(0);
                let up = prev_row[idx];
                row[idx] = row[idx].wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8);
            }
        }
        4 => {
            for idx in 0..row.len() {
                let left = idx
                    .checked_sub(bytes_per_pixel)
                    .and_then(|left_idx| row.get(left_idx).copied())
                    .unwrap_or(0);
                let up = prev_row[idx];
                let up_left = idx
                    .checked_sub(bytes_per_pixel)
                    .and_then(|left_idx| prev_row.get(left_idx).copied())
                    .unwrap_or(0);
                row[idx] = row[idx].wrapping_add(paeth(left, up, up_left));
            }
        }
        other => {
            return Err(OxideError::MalformedPdf(format!(
                "invalid PNG predictor row filter {other}"
            )));
        }
    }
    Ok(())
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let left_i = i32::from(left);
    let up_i = i32::from(up);
    let up_left_i = i32::from(up_left);
    let estimate = left_i + up_i - up_left_i;
    let pa = (estimate - left_i).abs();
    let pb = (estimate - up_i).abs();
    let pc = (estimate - up_left_i).abs();
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        up_left
    }
}

fn ceil_div(value: usize, divisor: usize) -> usize {
    if value == 0 {
        0
    } else {
        1 + ((value - 1) / divisor)
    }
}

fn validate_predictor_row_len(row_len: usize) -> Result<()> {
    if row_len > MAX_PREDICTOR_ROW_BYTES {
        return Err(OxideError::MalformedPdf(format!(
            "predictor row length {row_len} exceeds limit of {MAX_PREDICTOR_ROW_BYTES} bytes"
        )));
    }
    Ok(())
}

fn predictor_bytes_per_pixel(colors: usize, bits_per_component: usize) -> Result<usize> {
    let bits = colors.checked_mul(bits_per_component).ok_or_else(|| {
        OxideError::MalformedPdf(
            "predictor bytes-per-pixel dimensions overflow (Colors x BitsPerComponent)".to_string(),
        )
    })?;
    Ok(ceil_div(bits, 8).max(1))
}

fn ascii_hex_decode(data: &[u8]) -> Result<Vec<u8>> {
    ascii_hex_decode_capped(data, MAX_FILTER_OUTPUT_BYTES)
}

fn ascii_hex_decode_capped(data: &[u8], cap: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut high: Option<u8> = None;

    for &byte in data {
        if byte == b'>' {
            break;
        }
        if is_pdf_whitespace(byte) {
            continue;
        }
        let value = hex_value(byte).ok_or_else(|| {
            OxideError::ParseError(format!("invalid ASCIIHex digit 0x{byte:02X}"))
        })?;
        match high.take() {
            Some(high_nibble) => {
                push_capped(&mut out, (high_nibble << 4) | value, cap, "ASCIIHexDecode")?
            }
            None => high = Some(value),
        }
    }

    if let Some(high_nibble) = high {
        push_capped(&mut out, high_nibble << 4, cap, "ASCIIHexDecode")?;
    }

    Ok(out)
}

fn ascii85_decode(data: &[u8]) -> Result<Vec<u8>> {
    ascii85_decode_capped(data, MAX_FILTER_OUTPUT_BYTES)
}

fn ascii85_decode_capped(data: &[u8], cap: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut group: Vec<u8> = Vec::with_capacity(5);
    let mut idx = 0;

    while idx < data.len() {
        let byte = data[idx];
        idx += 1;

        if is_pdf_whitespace(byte) {
            continue;
        }

        if byte == b'~' {
            let mut saw_end = false;
            while idx < data.len() {
                let next = data[idx];
                idx += 1;
                if is_pdf_whitespace(next) {
                    continue;
                }
                if next == b'>' {
                    saw_end = true;
                    break;
                }
                return Err(OxideError::ParseError(
                    "ASCII85 '~' must be followed by '>'".to_string(),
                ));
            }
            if !saw_end {
                return Err(OxideError::ParseError(
                    "unterminated ASCII85 EOD marker".to_string(),
                ));
            }
            break;
        }

        if byte == b'z' {
            if !group.is_empty() {
                return Err(OxideError::ParseError(
                    "ASCII85 'z' cannot appear inside a group".to_string(),
                ));
            }
            extend_capped(&mut out, &[0, 0, 0, 0], cap, "ASCII85Decode")?;
            continue;
        }

        if !(b'!'..=b'u').contains(&byte) {
            return Err(OxideError::ParseError(format!(
                "invalid ASCII85 byte 0x{byte:02X}"
            )));
        }

        group.push(byte - b'!');
        if group.len() == 5 {
            push_ascii85_group_capped(&group, 4, &mut out, cap)?;
            group.clear();
        }
    }

    if !group.is_empty() {
        if group.len() == 1 {
            return Err(OxideError::ParseError(
                "ASCII85 final group cannot contain one digit".to_string(),
            ));
        }
        let output_len = group.len() - 1;
        while group.len() < 5 {
            group.push(84);
        }
        push_ascii85_group_capped(&group, output_len, &mut out, cap)?;
    }

    Ok(out)
}

fn push_ascii85_group_capped(
    group: &[u8],
    output_len: usize,
    out: &mut Vec<u8>,
    cap: u64,
) -> Result<()> {
    let before = out.len();
    push_ascii85_group(group, output_len, out)?;
    if out.len() as u64 > cap {
        out.truncate(before);
        return Err(OxideError::MalformedPdf(format!(
            "ASCII85Decode output exceeds {cap} byte limit"
        )));
    }
    Ok(())
}

fn push_ascii85_group(group: &[u8], output_len: usize, out: &mut Vec<u8>) -> Result<()> {
    if group.len() != 5 || output_len > 4 {
        return Err(OxideError::ParseError(
            "invalid ASCII85 group length".to_string(),
        ));
    }
    let mut value = 0u32;
    for &digit in group {
        value = value
            .checked_mul(85)
            .and_then(|v| v.checked_add(u32::from(digit)))
            .ok_or_else(|| OxideError::ParseError("ASCII85 group overflows".to_string()))?;
    }
    let bytes = value.to_be_bytes();
    out.extend_from_slice(&bytes[..output_len]);
    Ok(())
}

fn run_length_decode(data: &[u8]) -> Result<Vec<u8>> {
    run_length_decode_capped(data, MAX_FILTER_OUTPUT_BYTES)
}

fn run_length_decode_capped(data: &[u8], cap: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut idx = 0;

    while idx < data.len() {
        // Bound the expansion: RunLength can grow ~128x, so cap the cumulative
        // output the same way Flate is capped (a multi-hundred-MB input must not
        // be allowed to expand into an OOM).
        if out.len() as u64 > cap {
            return Err(OxideError::ParseError(
                "RunLengthDecode output exceeds decompression cap".to_string(),
            ));
        }
        let len = data[idx];
        idx += 1;
        match len {
            0..=127 => {
                let count = usize::from(len) + 1;
                let end = idx.checked_add(count).ok_or_else(|| {
                    OxideError::ParseError("RunLength literal count overflow".to_string())
                })?;
                if end > data.len() {
                    return Err(OxideError::ParseError(
                        "truncated RunLength literal run".to_string(),
                    ));
                }
                extend_capped(&mut out, &data[idx..end], cap, "RunLengthDecode")?;
                idx = end;
            }
            128 => break,
            129..=255 => {
                if idx >= data.len() {
                    return Err(OxideError::ParseError(
                        "truncated RunLength repeat run".to_string(),
                    ));
                }
                let count = usize::from(257u16 - u16::from(len));
                extend_repeat_capped(&mut out, data[idx], count, cap, "RunLengthDecode")?;
                idx += 1;
            }
        }
    }

    Ok(out)
}

fn lzw_decode(data: &[u8], early_change: u8) -> Result<Vec<u8>> {
    lzw_decode_capped(data, early_change, MAX_FILTER_OUTPUT_BYTES)
}

fn lzw_decode_capped(data: &[u8], early_change: u8, cap: u64) -> Result<Vec<u8>> {
    let mut reader = MsbBitReader::new(data);
    let mut table = initial_lzw_table();
    let mut code_width = 9usize;
    let mut next_code = 258usize;
    let mut out = Vec::new();
    let mut previous: Option<Vec<u8>> = None;

    while let Some(code) = reader.read_bits(code_width) {
        // Cap cumulative LZW output (the dictionary is bounded to 4096 entries
        // but the output stream is not) so a crafted stream cannot OOM.
        if out.len() as u64 > cap {
            return Err(OxideError::ParseError(
                "LZWDecode output exceeds decompression cap".to_string(),
            ));
        }
        match code {
            256 => {
                table = initial_lzw_table();
                code_width = 9;
                next_code = 258;
                previous = None;
            }
            257 => break,
            code => {
                let code_usize = usize::from(code);
                let entry = if code_usize < table.len() {
                    table[code_usize].clone().ok_or_else(|| {
                        OxideError::ParseError(format!("invalid empty LZW code {code_usize}"))
                    })?
                } else if code_usize == next_code {
                    let prev = previous.as_ref().ok_or_else(|| {
                        OxideError::ParseError("LZW KwKwK code without previous entry".to_string())
                    })?;
                    let mut synthesized = prev.clone();
                    let first = *prev.first().ok_or_else(|| {
                        OxideError::ParseError("empty LZW previous entry".to_string())
                    })?;
                    synthesized.push(first);
                    synthesized
                } else {
                    return Err(OxideError::ParseError(format!(
                        "invalid LZW code {code_usize}"
                    )));
                };

                extend_capped(&mut out, &entry, cap, "LZWDecode")?;

                if let Some(prev) = previous.as_ref() {
                    if next_code < 4096 {
                        let mut new_entry = prev.clone();
                        let first = *entry
                            .first()
                            .ok_or_else(|| OxideError::ParseError("empty LZW entry".to_string()))?;
                        new_entry.push(first);
                        if table.len() <= next_code {
                            table.resize(next_code + 1, None);
                        }
                        table[next_code] = Some(new_entry);
                        next_code += 1;
                        if code_width < 12
                            && next_code + usize::from(early_change) >= (1usize << code_width)
                        {
                            code_width += 1;
                        }
                    }
                }

                previous = Some(entry);
            }
        }
    }

    Ok(out)
}

fn push_capped(out: &mut Vec<u8>, byte: u8, cap: u64, filter: &str) -> Result<()> {
    if out.len() as u64 >= cap {
        return Err(OxideError::MalformedPdf(format!(
            "{filter} output exceeds {cap} byte limit"
        )));
    }
    out.push(byte);
    Ok(())
}

fn extend_capped(out: &mut Vec<u8>, bytes: &[u8], cap: u64, filter: &str) -> Result<()> {
    let new_len = (out.len() as u64)
        .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        .ok_or_else(|| OxideError::MalformedPdf(format!("{filter} output length overflows u64")))?;
    if new_len > cap {
        return Err(OxideError::MalformedPdf(format!(
            "{filter} output exceeds {cap} byte limit"
        )));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn extend_repeat_capped(
    out: &mut Vec<u8>,
    byte: u8,
    count: usize,
    cap: u64,
    filter: &str,
) -> Result<()> {
    let new_len = (out.len() as u64)
        .checked_add(u64::try_from(count).unwrap_or(u64::MAX))
        .ok_or_else(|| OxideError::MalformedPdf(format!("{filter} output length overflows u64")))?;
    if new_len > cap {
        return Err(OxideError::MalformedPdf(format!(
            "{filter} output exceeds {cap} byte limit"
        )));
    }
    out.extend(std::iter::repeat_n(byte, count));
    Ok(())
}

fn initial_lzw_table() -> Vec<Option<Vec<u8>>> {
    let mut table = Vec::with_capacity(4096);
    for byte in 0u16..=255 {
        table.push(Some(vec![byte as u8]));
    }
    table.push(None);
    table.push(None);
    table
}

struct MsbBitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> MsbBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bits(&mut self, count: usize) -> Option<u16> {
        if count == 0 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }
        let mut value = 0u16;
        for _ in 0..count {
            let byte = self.data[self.bit_pos / 8];
            let bit_offset = 7 - (self.bit_pos % 8);
            let bit = (byte >> bit_offset) & 1;
            value = (value << 1) | u16::from(bit);
            self.bit_pos += 1;
        }
        Some(value)
    }
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, 0x00 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn ascii_hex_decodes_odd_nibble() {
        assert_eq!(ascii_hex_decode(b"61 62 3>").unwrap(), b"ab0");
    }

    #[test]
    fn ascii85_decodes_known_vector() {
        assert_eq!(
            ascii85_decode(b"87cURD]i,\"Ebo7~>").unwrap(),
            b"Hello World"
        );
        assert_eq!(ascii85_decode(b"z~>").unwrap(), [0, 0, 0, 0]);
    }

    #[test]
    fn run_length_decodes_packbits() {
        let encoded = [2, b'a', b'b', b'c', 253, b'x', 128];
        assert_eq!(run_length_decode(&encoded).unwrap(), b"abcxxxx");
    }

    #[test]
    fn lzw_decodes_literal_codes() {
        let encoded = pack_lzw_codes(&[65, 66, 67, 257], 9);
        assert_eq!(lzw_decode(&encoded, 1).unwrap(), b"ABC");
    }

    #[test]
    fn filter_chain_depth_is_capped() {
        let filters = PdfObject::Array(
            (0..=MAX_FILTER_CHAIN_DEPTH)
                .map(|_| PdfObject::Name("RunLengthDecode".to_string()))
                .collect(),
        );
        let err = decode_stream_parts(&dict(&[("Filter", filters)]), &[128], None).unwrap_err();
        assert!(
            matches!(err, OxideError::MalformedPdf(ref message) if message.contains("filter chain depth")),
            "expected chain-depth diagnostic, got {err:?}"
        );
    }

    #[test]
    fn predictor_row_length_is_capped() {
        let params = dict(&[
            ("Predictor", PdfObject::Integer(12)),
            (
                "Columns",
                PdfObject::Integer(i64::try_from(MAX_PREDICTOR_ROW_BYTES + 1).unwrap()),
            ),
            ("Colors", PdfObject::Integer(1)),
            ("BitsPerComponent", PdfObject::Integer(8)),
        ]);
        let err = apply_predictor(vec![0u8; 8], Some(&params)).unwrap_err();
        assert!(
            matches!(err, OxideError::MalformedPdf(ref message) if message.contains("predictor row length")),
            "expected row-length diagnostic, got {err:?}"
        );
    }

    #[test]
    fn buffered_ascii_filters_enforce_output_cap() {
        let hex_err = ascii_hex_decode_capped(b"6162>", 1).unwrap_err();
        assert!(
            matches!(hex_err, OxideError::MalformedPdf(ref message) if message.contains("ASCIIHexDecode output")),
            "expected ASCIIHex cap diagnostic, got {hex_err:?}"
        );

        let ascii85_err = ascii85_decode_capped(b"zz~>", 4).unwrap_err();
        assert!(
            matches!(ascii85_err, OxideError::MalformedPdf(ref message) if message.contains("ASCII85Decode output")),
            "expected ASCII85 cap diagnostic, got {ascii85_err:?}"
        );
    }

    #[test]
    fn buffered_run_length_and_lzw_enforce_output_cap() {
        let run_err =
            run_length_decode_capped(&[4, b'a', b'b', b'c', b'd', b'e', 128], 3).unwrap_err();
        assert!(
            matches!(run_err, OxideError::MalformedPdf(ref message) if message.contains("RunLengthDecode output")),
            "expected RunLength cap diagnostic, got {run_err:?}"
        );

        let encoded = pack_lzw_codes(&[65, 66, 67, 257], 9);
        let lzw_err = lzw_decode_capped(&encoded, 1, 2).unwrap_err();
        assert!(
            matches!(lzw_err, OxideError::MalformedPdf(ref message) if message.contains("LZWDecode output")),
            "expected LZW cap diagnostic, got {lzw_err:?}"
        );
    }

    #[test]
    fn flate_decode_accepts_normal_stream() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write as _;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"hello flate").unwrap();
        let compressed = enc.finish().unwrap();
        assert_eq!(flate_decode(&compressed).unwrap(), b"hello flate");
    }

    #[test]
    fn flate_decode_rejects_decompression_bomb() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write as _;
        // A 4 MiB run of zeros compresses to a tiny input but inflates well past
        // a small test cap, simulating the bomb without allocating the 512 MiB
        // production limit.
        let raw = vec![0u8; 4 * 1024 * 1024];
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();
        assert!(
            compressed.len() < 64 * 1024,
            "bomb input should be tiny, was {}",
            compressed.len()
        );
        // Cap of 1 MiB < 4 MiB decompressed => rejected.
        let err = flate_decode_capped(&compressed, 1024 * 1024).unwrap_err();
        assert!(
            matches!(err, OxideError::MalformedPdf(ref m) if m.contains("decompression bomb")),
            "expected decompression-bomb rejection, got {:?}",
            err
        );
        // The same data under a generous cap decodes fine.
        let ok = flate_decode_capped(&compressed, 8 * 1024 * 1024).unwrap();
        assert_eq!(ok.len(), raw.len());
    }

    #[test]
    fn png_predictor_up_decodes_rows() {
        let params = dict(&[
            ("Predictor", PdfObject::Integer(12)),
            ("Columns", PdfObject::Integer(3)),
        ]);
        let encoded = vec![0, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(
            apply_predictor(encoded, Some(&params)).unwrap(),
            vec![1, 2, 3, 2, 3, 4]
        );
    }

    #[test]
    fn predictor_row_dimensions_overflow_returns_err_not_panic() {
        // Attacker-controlled Columns/Colors/BitsPerComponent whose product
        // overflows usize must yield a clean MalformedPdf error rather than
        // panicking under overflow checks (or wrapping to a bogus row length).
        // Regression for the unchecked `columns * colors * bits_per_component`
        // multiplication (fuzz finding: predictor size-field overflow).
        let huge = i64::MAX;
        let params = dict(&[
            ("Predictor", PdfObject::Integer(12)),
            ("Columns", PdfObject::Integer(huge)),
            ("Colors", PdfObject::Integer(huge)),
            ("BitsPerComponent", PdfObject::Integer(16)),
        ]);
        let err = apply_predictor(vec![0u8; 32], Some(&params)).unwrap_err();
        assert!(
            matches!(err, OxideError::MalformedPdf(_)),
            "expected MalformedPdf on dimension overflow, got {err:?}"
        );
    }

    #[test]
    fn streaming_reader_matches_buffered_filters() {
        let flate = zlib_bytes(b"BT /F1 12 Tf (hello) Tj ET");
        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("FlateDecode".to_string()))]),
            &flate,
        );

        let raw_deflate = raw_deflate_bytes(b"raw deflate bytes");
        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("FlateDecode".to_string()))]),
            &raw_deflate,
        );

        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("ASCIIHexDecode".to_string()))]),
            b"61 62 3>",
        );
        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("ASCII85Decode".to_string()))]),
            b"87cURD]i,\"Ebo7~>",
        );
        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("RunLengthDecode".to_string()))]),
            &[2, b'a', b'b', b'c', 253, b'x', 128],
        );
        let lzw = pack_lzw_codes(&[65, 66, 67, 257], 9);
        assert_streaming_matches(
            &dict(&[("Filter", PdfObject::Name("LZWDecode".to_string()))]),
            &lzw,
        );
    }

    #[test]
    fn streaming_reader_matches_buffered_filter_chain() {
        let compressed = zlib_bytes(b"BT (chain) Tj ET");
        let encoded = ascii_hex_bytes(&compressed);
        let filters = PdfObject::Array(vec![
            PdfObject::Name("ASCIIHexDecode".to_string()),
            PdfObject::Name("FlateDecode".to_string()),
        ]);
        assert_streaming_matches(&dict(&[("Filter", filters)]), &encoded);
    }

    #[test]
    fn streaming_reader_matches_buffered_predictors() {
        let png_params = dict(&[
            ("Predictor", PdfObject::Integer(12)),
            ("Columns", PdfObject::Integer(3)),
        ]);
        let png_encoded = zlib_bytes(&[0, 1, 2, 3, 2, 1, 1, 1]);
        assert_streaming_matches(
            &dict(&[
                ("Filter", PdfObject::Name("FlateDecode".to_string())),
                ("DecodeParms", PdfObject::Dictionary(png_params)),
            ]),
            &png_encoded,
        );

        let tiff_params = dict(&[
            ("Predictor", PdfObject::Integer(2)),
            ("Columns", PdfObject::Integer(4)),
        ]);
        let tiff_encoded = zlib_bytes(&[1, 1, 1, 1]);
        assert_streaming_matches(
            &dict(&[
                ("Filter", PdfObject::Name("FlateDecode".to_string())),
                ("DecodeParms", PdfObject::Dictionary(tiff_params)),
            ]),
            &tiff_encoded,
        );
    }

    #[test]
    fn streaming_reader_enforces_decompression_cap() {
        use std::io::Read as _;

        let raw = vec![0u8; 4 * 1024 * 1024];
        let compressed = zlib_bytes(&raw);
        let dict = dict(&[("Filter", PdfObject::Name("FlateDecode".to_string()))]);
        let mut decoded = decode_stream_reader_with_cap(
            &dict,
            std::io::Cursor::new(compressed),
            None,
            1024 * 1024,
        )
        .unwrap();
        let mut out = Vec::new();
        let err = decoded.reader.read_to_end(&mut out).unwrap_err();
        assert!(
            err.to_string().contains("decompression cap"),
            "expected streaming cap error, got {err}"
        );
    }

    #[test]
    fn streaming_ascii_filters_enforce_decompression_cap() {
        use std::io::Read as _;

        let dict = dict(&[("Filter", PdfObject::Name("ASCIIHexDecode".to_string()))]);
        let mut decoded =
            decode_stream_reader_with_cap(&dict, std::io::Cursor::new(b"6162>".to_vec()), None, 1)
                .unwrap();
        let mut out = Vec::new();
        let err = decoded.reader.read_to_end(&mut out).unwrap_err();
        assert!(
            err.to_string().contains("ASCIIHexDecode output"),
            "expected streaming ASCIIHex cap error, got {err}"
        );
    }

    fn assert_streaming_matches(dict: &PdfDictionary, raw: &[u8]) {
        use std::io::Read as _;

        let buffered = decode_stream_parts(dict, raw, None).unwrap();
        let mut streaming =
            decode_stream_lossless_reader(dict, std::io::Cursor::new(raw.to_vec()), None).unwrap();
        let mut out = Vec::new();
        streaming.reader.read_to_end(&mut out).unwrap();
        assert_eq!(streaming.status, buffered.status);
        assert_eq!(out, buffered.data);
    }

    fn zlib_bytes(data: &[u8]) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write as _;

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn raw_deflate_bytes(data: &[u8]) -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write as _;

        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn ascii_hex_bytes(data: &[u8]) -> Vec<u8> {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = Vec::with_capacity(data.len() * 2 + 1);
        for &byte in data {
            out.push(HEX[usize::from(byte >> 4)]);
            out.push(HEX[usize::from(byte & 0x0F)]);
        }
        out.push(b'>');
        out
    }

    fn pack_lzw_codes(codes: &[u16], width: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut current = 0u8;
        let mut used = 0usize;
        for &code in codes {
            for bit_idx in (0..width).rev() {
                let bit = ((code >> bit_idx) & 1) as u8;
                current = (current << 1) | bit;
                used += 1;
                if used == 8 {
                    out.push(current);
                    current = 0;
                    used = 0;
                }
            }
        }
        if used > 0 {
            current <<= 8 - used;
            out.push(current);
        }
        out
    }
}
