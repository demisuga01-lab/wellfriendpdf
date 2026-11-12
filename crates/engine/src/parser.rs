use std::collections::BTreeMap;

use crate::decode_scanner::find_marker_accelerated;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};

pub trait ParserResolver {
    fn resolve_for_parser(&self, object: &PdfObject) -> Result<PdfObject>;
}

/// Maximum nesting depth for syntactic structures (arrays/dictionaries within
/// arrays/dictionaries). A malformed or malicious PDF can otherwise drive the
/// recursive descent parser arbitrarily deep — e.g. thousands of unmatched
/// `[` or `<<` bytes — and overflow the call stack, aborting the whole
/// process. Real PDFs usually nest only a handful of levels, but wild files can
/// exceed 64 in benign catalog/action structures. 256 keeps recursion bounded
/// while allowing those files to be parsed instead of rejected.
const MAX_PARSE_DEPTH: usize = 256;

/// What [`PdfParser::skip_bare_keyword_at`] found where an object was
/// expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BareKeyword {
    /// `endobj`, left unconsumed.
    EndObj,
    /// Some other non-object token, now consumed.
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndirectObject {
    pub number: u32,
    pub generation: u16,
    pub object: PdfObject,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndirectStreamHeader {
    pub number: u32,
    pub generation: u16,
    pub dict: PdfDictionary,
    pub stream_start: usize,
    pub length: Option<i64>,
}

pub struct PdfParser<'a> {
    data: &'a [u8],
    pos: usize,
    resolver: Option<&'a dyn ParserResolver>,
    /// Current syntactic nesting depth, bounded by [`MAX_PARSE_DEPTH`].
    depth: usize,
}

impl<'a> PdfParser<'a> {
    pub fn new(data: &'a [u8], offset: usize) -> Result<Self> {
        Self::with_resolver(data, offset, None)
    }

    pub fn with_resolver(
        data: &'a [u8],
        offset: usize,
        resolver: Option<&'a dyn ParserResolver>,
    ) -> Result<Self> {
        if offset > data.len() {
            return Err(WellfriendError::ParseError(format!(
                "offset {offset} is beyond input length {}",
                data.len()
            )));
        }
        Ok(Self {
            data,
            pos: offset,
            resolver,
            depth: 0,
        })
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn parse_object(&mut self) -> Result<PdfObject> {
        self.skip_ws_and_comments();
        let byte = self.peek_byte().ok_or_else(|| {
            WellfriendError::ParseError("unexpected end of input while parsing object".to_string())
        })?;

        match byte {
            b'<' if self.starts_with(b"<<") => self.parse_dictionary_or_stream(),
            b'<' => self.parse_hex_string(),
            b'(' => self.parse_literal_string(),
            b'/' => self.parse_name().map(PdfObject::Name),
            b'[' => self.parse_array(),
            b't' if self.consume_keyword(b"true") => Ok(PdfObject::Boolean(true)),
            b'f' if self.consume_keyword(b"false") => Ok(PdfObject::Boolean(false)),
            b'n' if self.consume_keyword(b"null") => Ok(PdfObject::Null),
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number_or_reference(),
            other => Err(WellfriendError::ParseError(format!(
                "unexpected byte 0x{other:02X} while parsing object"
            ))),
        }
    }

    /// Enter a nested structure, enforcing the recursion bound. Returns the
    /// new depth on success so the caller can restore it on exit; returns an
    /// error (instead of recursing and overflowing the stack) once the limit
    /// is exceeded.
    fn enter_nesting(&mut self) -> Result<()> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(WellfriendError::ParseError(format!(
                "object nesting exceeded depth limit {MAX_PARSE_DEPTH}"
            )));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave_nesting(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn parse_indirect_object(&mut self) -> Result<IndirectObject> {
        self.skip_ws_and_comments();
        let number = self.parse_unsigned_integer_token()?;
        self.skip_ws_and_comments();
        let generation = self.parse_unsigned_integer_token()?;
        let number = u32::try_from(number).map_err(|_| {
            WellfriendError::ParseError(format!("object number {number} does not fit in u32"))
        })?;
        let generation = u16::try_from(generation).map_err(|_| {
            WellfriendError::ParseError(format!("generation {generation} does not fit in u16"))
        })?;
        self.skip_ws_and_comments();
        if !self.consume_keyword(b"obj") {
            return Err(WellfriendError::ParseError(
                "indirect object header is missing obj keyword".to_string(),
            ));
        }
        let object = self.parse_object()?;
        self.skip_ws_and_comments();
        if !self.consume_keyword(b"endobj") {
            // Poppler accepts an object whose `endobj` is missing (the next
            // object header or EOF follows); keep the parsed object.
            log::warn!("object {number} {generation} is missing endobj");
        }
        Ok(IndirectObject {
            number,
            generation,
            object,
        })
    }

    pub fn parse_indirect_stream_header(&mut self) -> Result<IndirectStreamHeader> {
        self.skip_ws_and_comments();
        let number = self.parse_unsigned_integer_token()?;
        self.skip_ws_and_comments();
        let generation = self.parse_unsigned_integer_token()?;
        let number = u32::try_from(number).map_err(|_| {
            WellfriendError::ParseError(format!("object number {number} does not fit in u32"))
        })?;
        let generation = u16::try_from(generation).map_err(|_| {
            WellfriendError::ParseError(format!("generation {generation} does not fit in u16"))
        })?;
        self.skip_ws_and_comments();
        if !self.consume_keyword(b"obj") {
            return Err(WellfriendError::ParseError(
                "indirect object header is missing obj keyword".to_string(),
            ));
        }
        self.skip_ws_and_comments();
        let dict = self.parse_dictionary()?;
        self.skip_ws_and_comments();
        if !self.consume_keyword(b"stream") {
            return Err(WellfriendError::ParseError(format!(
                "object {number} {generation} is not a stream"
            )));
        }
        match self.peek_byte() {
            Some(b'\r') => {
                self.pos += 1;
                if self.peek_byte() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            Some(b'\n') => self.pos += 1,
            _ => {}
        }
        let length = self.resolve_stream_length(&dict)?;
        Ok(IndirectStreamHeader {
            number,
            generation,
            dict,
            stream_start: self.pos,
            length,
        })
    }

    fn parse_dictionary_or_stream(&mut self) -> Result<PdfObject> {
        let dict = self.parse_dictionary()?;
        let after_dict = self.pos;
        self.skip_ws_and_comments();
        if self.consume_keyword(b"stream") {
            let raw = self.parse_stream_bytes(&dict)?;
            Ok(PdfObject::Stream { dict, raw })
        } else {
            self.pos = after_dict;
            Ok(PdfObject::Dictionary(dict))
        }
    }

    fn parse_dictionary(&mut self) -> Result<PdfDictionary> {
        self.expect_bytes(b"<<")?;
        self.enter_nesting()?;
        let mut entries = BTreeMap::new();
        loop {
            self.skip_ws_and_comments();
            if self.starts_with(b">>") {
                self.pos += 2;
                break;
            }
            if self.peek_byte().is_none() {
                self.leave_nesting();
                return Err(WellfriendError::ParseError(
                    "unterminated dictionary".to_string(),
                ));
            }
            if self.peek_byte() != Some(b'/') {
                if self.starts_with(b"endobj") {
                    // Missing `>>`: end the dictionary at `endobj`.
                    log::warn!("dictionary is missing >> before endobj");
                    break;
                }
                // Poppler skips a non-name key with a warning and goes on
                // with the next token, so `<< /A 1 2 /B 3 >>` keeps /A and /B.
                if let Err(err) = self.skip_non_name_dictionary_key() {
                    self.leave_nesting();
                    return Err(err);
                }
                continue;
            }
            let key = match self.parse_name() {
                Ok(key) => key,
                Err(err) => {
                    self.leave_nesting();
                    return Err(err);
                }
            };
            let value_start = self.pos;
            let value = match self.parse_object() {
                Ok(value) => value,
                Err(err) => {
                    // A bare keyword in value position (`/Type XObject`,
                    // `/K j`): PDFium drops the key and goes on; `endobj`
                    // ends the dictionary.
                    if self.starts_with_at(value_start, b">>") {
                        self.pos = value_start;
                        self.skip_ws_and_comments();
                        self.pos += 2;
                        log::warn!("dictionary value for /{key} is missing before >>");
                        break;
                    }
                    match self.skip_bare_keyword_at(value_start) {
                        Some(BareKeyword::EndObj) => {
                            log::warn!("dictionary value is missing before endobj");
                            break;
                        }
                        Some(BareKeyword::Skipped) => {
                            log::warn!(
                                "dictionary value for /{key} is a bare keyword; dropping it"
                            );
                            continue;
                        }
                        None => {
                            self.leave_nesting();
                            return Err(err);
                        }
                    }
                }
            };
            entries.insert(key, value);
        }
        self.leave_nesting();
        Ok(PdfDictionary::new(entries))
    }

    /// Skip one token that appears where a dictionary key was expected.
    fn skip_non_name_dictionary_key(&mut self) -> Result<()> {
        let start = self.pos;
        match self.parse_object() {
            Ok(object) => {
                log::warn!(
                    "dictionary key must be a name, got {}; skipping it",
                    object.variant_name()
                );
                Ok(())
            }
            // Never swallow the recursion guard.
            Err(err) if err.to_string().contains("depth limit") => Err(err),
            Err(_) => {
                log::warn!("skipping unexpected byte in dictionary key position");
                self.pos = start + 1;
                Ok(())
            }
        }
    }

    fn parse_array(&mut self) -> Result<PdfObject> {
        self.expect_byte(b'[')?;
        self.enter_nesting()?;
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match self.peek_byte() {
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => {
                    let item_start = self.pos;
                    match self.parse_object() {
                        Ok(item) => items.push(item),
                        Err(err) => {
                            // PDFium ends an array at the first token that is
                            // not an object (`[1 0 R p`), consuming it;
                            // `endobj` and `>>` are left for the enclosing
                            // object.
                            if self.starts_with_at(item_start, b">>") {
                                self.pos = item_start;
                                log::warn!("array is missing ] before >>");
                                break;
                            }
                            match self.skip_bare_keyword_at(item_start) {
                                Some(_) => {
                                    log::warn!("array ends at a bare keyword");
                                    break;
                                }
                                None => {
                                    self.leave_nesting();
                                    return Err(err);
                                }
                            }
                        }
                    }
                }
                None => {
                    self.leave_nesting();
                    return Err(WellfriendError::ParseError(
                        "unterminated array".to_string(),
                    ));
                }
            }
        }
        self.leave_nesting();
        Ok(PdfObject::Array(items))
    }

    fn parse_literal_string(&mut self) -> Result<PdfObject> {
        self.expect_byte(b'(')?;
        let mut out = Vec::new();
        let mut depth = 1usize;

        while self.pos < self.data.len() {
            let byte = self.data[self.pos];
            self.pos += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    out.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(PdfObject::String(out));
                    }
                    out.push(byte);
                }
                b'\\' => self.parse_literal_escape(&mut out)?,
                _ => out.push(byte),
            }
        }

        Err(WellfriendError::ParseError(
            "unterminated literal string".to_string(),
        ))
    }

    fn parse_literal_escape(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let Some(byte) = self.next_byte() else {
            return Ok(());
        };
        match byte {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0C),
            b'(' | b')' | b'\\' => out.push(byte),
            b'\r' => {
                if self.peek_byte() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut value = u16::from(byte - b'0');
                for _ in 0..2 {
                    match self.peek_byte() {
                        Some(next @ b'0'..=b'7') => {
                            self.pos += 1;
                            value = (value << 3) + u16::from(next - b'0');
                        }
                        _ => break,
                    }
                }
                out.push((value & 0xFF) as u8);
            }
            other => out.push(other),
        }
        Ok(())
    }

    fn parse_hex_string(&mut self) -> Result<PdfObject> {
        self.expect_byte(b'<')?;
        let mut out = Vec::new();
        let mut high: Option<u8> = None;

        loop {
            let byte = self.next_byte().ok_or_else(|| {
                WellfriendError::ParseError("unterminated hex string".to_string())
            })?;
            if byte == b'>' {
                break;
            }
            if is_pdf_whitespace(byte) {
                continue;
            }
            let value = hex_value(byte).ok_or_else(|| {
                WellfriendError::ParseError(format!("invalid hex string digit 0x{byte:02X}"))
            })?;
            match high.take() {
                Some(high_nibble) => out.push((high_nibble << 4) | value),
                None => high = Some(value),
            }
        }

        if let Some(high_nibble) = high {
            out.push(high_nibble << 4);
        }

        Ok(PdfObject::String(out))
    }

    fn parse_name(&mut self) -> Result<String> {
        self.expect_byte(b'/')?;
        let mut out = Vec::new();

        while let Some(byte) = self.peek_byte() {
            if is_pdf_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.pos += 1;
            if byte == b'#' {
                let maybe_high = self.peek_byte();
                let maybe_low = self.data.get(self.pos + 1).copied();
                match (
                    maybe_high.and_then(hex_value),
                    maybe_low.and_then(hex_value),
                ) {
                    (Some(high), Some(low)) => {
                        self.pos += 2;
                        out.push((high << 4) | low);
                    }
                    _ => out.push(byte),
                }
            } else {
                out.push(byte);
            }
        }

        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn parse_number_or_reference(&mut self) -> Result<PdfObject> {
        let token = self.parse_number_token()?;
        let is_real = token.contains(&b'.');
        if !is_real {
            let Ok(integer) = parse_i64_token(&token) else {
                // An integer too large for i64 is kept as a real number, as
                // Poppler does, instead of failing the whole object.
                return parse_real_token(&token);
            };
            if let Some(reference) = self.try_reference(integer)? {
                return Ok(reference);
            }
            return Ok(PdfObject::Integer(integer));
        }

        let text = std::str::from_utf8(&token)
            .map_err(|err| WellfriendError::ParseError(format!("invalid real token: {err}")))?;
        let value = text
            .parse::<f64>()
            .map_err(|err| WellfriendError::ParseError(format!("invalid real number: {err}")))?;
        Ok(PdfObject::Real(value))
    }

    fn try_reference(&mut self, first: i64) -> Result<Option<PdfObject>> {
        let saved = self.pos;
        if first < 0 || first > i64::from(u32::MAX) {
            return Ok(None);
        }
        self.skip_ws_and_comments();
        let second_start = self.pos;
        let Ok(second_token) = self.parse_number_token() else {
            self.pos = saved;
            return Ok(None);
        };
        if second_token.contains(&b'.') {
            self.pos = saved;
            return Ok(None);
        }
        let Ok(generation) = parse_i64_token(&second_token) else {
            self.pos = saved;
            return Ok(None);
        };
        if generation < 0 || generation > i64::from(u16::MAX) {
            self.pos = saved;
            return Ok(None);
        }
        self.skip_ws_and_comments();
        if self.consume_keyword(b"R") {
            return Ok(Some(PdfObject::Reference {
                number: first as u32,
                generation: generation as u16,
            }));
        }
        self.pos = saved;
        if self.pos < second_start {
            self.pos = saved;
        }
        Ok(None)
    }

    fn parse_stream_bytes(&mut self, dict: &PdfDictionary) -> Result<Vec<u8>> {
        match self.peek_byte() {
            Some(b'\r') => {
                self.pos += 1;
                if self.peek_byte() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            Some(b'\n') => self.pos += 1,
            _ => {}
        }

        let stream_start = self.pos;
        // Junk after the `stream` keyword on its line (`stream   "x"`) is
        // skipped by PDFium, which starts the data on the next line; take
        // that start when /Length confirms it.
        let after_eol = stream_start > 0 && matches!(self.data[stream_start - 1], b'\n' | b'\r');
        let next_line_start = if after_eol {
            None
        } else {
            self.data[stream_start..]
                .iter()
                .position(|byte| matches!(byte, b'\r' | b'\n'))
                .map(|rel| skip_eol(self.data, stream_start + rel))
        };
        if let Some(length) = self.resolve_stream_length(dict)? {
            let length = usize::try_from(length).map_err(|_| {
                WellfriendError::MalformedPdf("stream Length is too large".to_string())
            })?;
            for start in std::iter::once(stream_start).chain(next_line_start) {
                let stream_end = start.checked_add(length).ok_or_else(|| {
                    WellfriendError::MalformedPdf("stream Length overflows".to_string())
                })?;
                if stream_end <= self.data.len() {
                    let after_raw = skip_eol(self.data, stream_end);
                    if endstream_keyword_at(self.data, after_raw) {
                        let raw = self.data[start..stream_end].to_vec();
                        self.pos = after_raw + b"endstream".len();
                        return Ok(raw);
                    }
                }
            }
        }

        self.scan_stream_until_endstream(stream_start)
    }

    fn resolve_stream_length(&self, dict: &PdfDictionary) -> Result<Option<i64>> {
        let Some(length_obj) = dict.get("Length") else {
            return Ok(None);
        };
        match length_obj {
            PdfObject::Integer(value) => Ok(Some(*value)),
            PdfObject::Reference { .. } => {
                let Some(resolver) = self.resolver else {
                    return Ok(None);
                };
                // An unresolvable /Length (missing object, a reference back
                // to the stream itself, a non-integer) is not fatal: the body
                // is delimited by scanning for `endstream` instead.
                match resolver.resolve_for_parser(length_obj) {
                    Ok(PdfObject::Integer(value)) => Ok(Some(value)),
                    Ok(PdfObject::Real(value)) if value.is_finite() && value >= 0.0 => {
                        Ok(Some(value as i64))
                    }
                    Ok(other) => {
                        log::warn!(
                            "stream Length reference resolved to {}; scanning for endstream",
                            other.variant_name()
                        );
                        Ok(None)
                    }
                    Err(WellfriendError::Cancelled(message)) => {
                        Err(WellfriendError::Cancelled(message))
                    }
                    Err(err) => {
                        log::warn!(
                            "stream Length reference could not be resolved ({err}); scanning for endstream"
                        );
                        Ok(None)
                    }
                }
            }
            other => Err(WellfriendError::MalformedPdf(format!(
                "stream Length must be integer or reference, got {}",
                other.variant_name()
            ))),
        }
    }

    fn scan_stream_until_endstream(&mut self, stream_start: usize) -> Result<Vec<u8>> {
        if stream_start <= self.data.len() {
            // PDFium ends a stream whose /Length is unusable at the first
            // `endstream` or `endobj` that stands as a keyword: `eendstream`
            // and `(endstream is text)` inside the data do not end it.
            // `endstream.` and `endstream%` are not keywords either; with no
            // keyword at all, a bare `endstream` still ends the data.
            let keyword_endobj = find_keyword(self.data, stream_start, b"endobj");
            let end = match find_keyword(self.data, stream_start, b"endstream") {
                Some(end) if keyword_endobj.is_none_or(|endobj| end < endobj) => Some(end),
                Some(_) => None,
                None if keyword_endobj.is_none() => {
                    find_marker_accelerated(&self.data[stream_start..], b"endstream")
                        .map(|rel| stream_start + rel)
                }
                None => None,
            };
            if let Some(end) = end {
                let raw_end = trim_single_eol_before(self.data, stream_start, end);
                self.pos = end + b"endstream".len();
                return Ok(self.data[stream_start..raw_end].to_vec());
            }
            if let Some(endobj) = keyword_endobj {
                let raw_end = trim_single_eol_before(self.data, stream_start, endobj);
                log::warn!("stream is missing endstream; ending it at endobj");
                self.pos = endobj;
                return Ok(self.data[stream_start..raw_end].to_vec());
            }
            // No `endstream` at all: the data runs to the object's `endobj`,
            // which is left for the object parser.
            if let Some(rel) = find_marker_accelerated(&self.data[stream_start..], b"endobj") {
                let cursor = stream_start + rel;
                let raw_end = trim_single_eol_before(self.data, stream_start, cursor);
                log::warn!("stream is missing endstream; ending it at endobj");
                self.pos = cursor;
                return Ok(self.data[stream_start..raw_end].to_vec());
            }
        }
        Err(WellfriendError::ParseError(
            "stream is missing endstream".to_string(),
        ))
    }

    fn parse_unsigned_integer_token(&mut self) -> Result<u64> {
        let token = self.parse_number_token()?;
        if token.contains(&b'.') || token.starts_with(b"-") {
            return Err(WellfriendError::ParseError(
                "expected unsigned integer token".to_string(),
            ));
        }
        let text = std::str::from_utf8(&token)
            .map_err(|err| WellfriendError::ParseError(format!("invalid integer token: {err}")))?;
        text.parse::<u64>()
            .map_err(|err| WellfriendError::ParseError(format!("invalid unsigned integer: {err}")))
    }

    fn parse_number_token(&mut self) -> Result<Vec<u8>> {
        self.skip_ws_and_comments();
        let start = self.pos;
        let mut token = Vec::new();
        if let Some(sign @ (b'+' | b'-')) = self.peek_byte() {
            token.push(sign);
            self.pos += 1;
        }
        let mut saw_digit = false;
        let mut saw_dot = false;
        while let Some(byte) = self.peek_byte() {
            match byte {
                b'0'..=b'9' => {
                    saw_digit = true;
                    token.push(byte);
                    self.pos += 1;
                }
                b'.' if !saw_dot => {
                    saw_dot = true;
                    token.push(byte);
                    self.pos += 1;
                }
                // A minus sign in the fractional part ("1.5-2") is ignored,
                // matching Adobe and Poppler.
                b'-' if saw_dot
                    && self
                        .data
                        .get(self.pos + 1)
                        .is_some_and(|next| next.is_ascii_digit()) =>
                {
                    log::warn!("ignoring stray minus sign inside a number");
                    self.pos += 1;
                }
                _ => break,
            }
        }
        if !saw_digit {
            self.pos = start;
            return Err(WellfriendError::ParseError(
                "expected numeric token".to_string(),
            ));
        }
        Ok(token)
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<()> {
        if self.starts_with(expected) {
            self.pos += expected.len();
            Ok(())
        } else {
            Err(WellfriendError::ParseError(format!(
                "expected {}",
                String::from_utf8_lossy(expected)
            )))
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<()> {
        match self.next_byte() {
            Some(byte) if byte == expected => Ok(()),
            Some(byte) => Err(WellfriendError::ParseError(format!(
                "expected byte 0x{expected:02X}, got 0x{byte:02X}"
            ))),
            None => Err(WellfriendError::ParseError(format!(
                "expected byte 0x{expected:02X}, got EOF"
            ))),
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.peek_byte(), Some(byte) if is_pdf_whitespace(byte)) {
                self.pos += 1;
            }
            if self.peek_byte() == Some(b'%') {
                while let Some(byte) = self.next_byte() {
                    if byte == b'\r' || byte == b'\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn consume_keyword(&mut self, keyword: &[u8]) -> bool {
        if !self.starts_with(keyword) {
            return false;
        }
        let after = self.pos + keyword.len();
        if self
            .data
            .get(after)
            .copied()
            .is_some_and(|byte| !is_pdf_whitespace(byte) && !is_delimiter(byte))
        {
            return false;
        }
        self.pos = after;
        true
    }

    fn starts_with(&self, bytes: &[u8]) -> bool {
        bytes_at(self.data, self.pos, bytes)
    }

    fn starts_with_at(&self, pos: usize, bytes: &[u8]) -> bool {
        let mut probe = PdfParser {
            data: self.data,
            pos,
            resolver: None,
            depth: 0,
        };
        probe.skip_ws_and_comments();
        probe.starts_with(bytes)
    }

    /// Recover from an object that failed to parse at `start` when the token
    /// there cannot begin an object at all (a bare keyword such as `R`,
    /// `XObject` or `p`, or a stray delimiter). Returns `None` for anything
    /// else, so real errors (nesting depth, unterminated strings, broken
    /// numbers) still propagate. `endobj` and `>>` are left unconsumed for
    /// the enclosing object; any other token is skipped.
    fn skip_bare_keyword_at(&mut self, start: usize) -> Option<BareKeyword> {
        self.pos = start;
        self.skip_ws_and_comments();
        let byte = self.peek_byte()?;
        if matches!(
            byte,
            b'<' | b'(' | b'/' | b'[' | b'+' | b'-' | b'.' | b'0'..=b'9'
        ) {
            return None;
        }
        if self.starts_with(b"endobj") {
            return Some(BareKeyword::EndObj);
        }
        let run = self.data[self.pos..]
            .iter()
            .take_while(|byte| !is_pdf_whitespace(**byte) && !is_delimiter(**byte))
            .count();
        self.pos += run.max(1);
        Some(BareKeyword::Skipped)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.pos += 1;
        Some(byte)
    }
}

fn parse_real_token(token: &[u8]) -> Result<PdfObject> {
    let text = std::str::from_utf8(token)
        .map_err(|err| WellfriendError::ParseError(format!("invalid real token: {err}")))?;
    let value = text
        .parse::<f64>()
        .map_err(|err| WellfriendError::ParseError(format!("invalid real number: {err}")))?;
    Ok(PdfObject::Real(value))
}

fn parse_i64_token(token: &[u8]) -> Result<i64> {
    let text = std::str::from_utf8(token)
        .map_err(|err| WellfriendError::ParseError(format!("invalid integer token: {err}")))?;
    text.parse::<i64>()
        .map_err(|err| WellfriendError::ParseError(format!("invalid integer: {err}")))
}

/// Check the bytes that follow a stream body: optional EOL, `endstream`, then
/// whitespace/comments and `endobj`, exactly as `parse_indirect_object` would
/// consume them. Returns the offset just past `endobj` when they match.
pub(crate) fn stream_object_trailer_end(tail: &[u8]) -> Option<usize> {
    let after_raw = skip_eol(tail, 0);
    if !bytes_at(tail, after_raw, b"endstream") {
        return None;
    }
    let mut parser = PdfParser::new(tail, after_raw + b"endstream".len()).ok()?;
    parser.skip_ws_and_comments();
    parser.consume_keyword(b"endobj").then_some(parser.pos)
}

/// `endstream` at `pos`, ended by whitespace, a delimiter or the end of the
/// data (`endstream.` is a different word).
pub(crate) fn endstream_keyword_at(data: &[u8], pos: usize) -> bool {
    bytes_at(data, pos, b"endstream")
        && data
            .get(pos + b"endstream".len())
            .is_none_or(|byte| is_pdf_whitespace(*byte) || is_delimiter(*byte))
}

/// The first occurrence of `word` at or after `start` that stands as a
/// keyword the way PDFium's stream-end search requires: whitespace (or
/// `start`) before it and whitespace (or the end of the data) after it.
pub(crate) fn find_keyword(data: &[u8], start: usize, word: &[u8]) -> Option<usize> {
    let mut from = start;
    while from <= data.len() {
        let at = from + find_marker_accelerated(&data[from..], word)?;
        let left = at == start || is_pdf_whitespace(data[at - 1]);
        let right = data
            .get(at + word.len())
            .is_none_or(|byte| is_pdf_whitespace(*byte));
        if left && right {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

fn skip_eol(data: &[u8], pos: usize) -> usize {
    match data.get(pos).copied() {
        Some(b'\r') => {
            if data.get(pos + 1).copied() == Some(b'\n') {
                pos + 2
            } else {
                pos + 1
            }
        }
        Some(b'\n') => pos + 1,
        _ => pos,
    }
}

fn trim_single_eol_before(data: &[u8], start: usize, end: usize) -> usize {
    if end > start && data.get(end - 1).copied() == Some(b'\n') {
        if end >= start + 2 && data.get(end - 2).copied() == Some(b'\r') {
            end - 2
        } else {
            end - 1
        }
    } else if end > start && data.get(end - 1).copied() == Some(b'\r') {
        end - 1
    } else {
        end
    }
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
    use super::*;

    #[test]
    fn parses_literal_string_escapes_and_nested_parentheses() {
        let mut input = br"(a \(b\) (c) \053".to_vec();
        input.extend_from_slice(br"\\");
        input.push(b'\\');
        input.push(b'\n');
        input.extend_from_slice(b"continued)");
        let mut parser = PdfParser::new(&input, 0).unwrap();
        assert_eq!(
            parser.parse_object().unwrap(),
            PdfObject::String(b"a (b) (c) +\\continued".to_vec())
        );
    }

    #[test]
    fn parses_hex_string_with_odd_nibble() {
        let mut parser = PdfParser::new(br"<61 62 3>", 0).unwrap();
        assert_eq!(
            parser.parse_object().unwrap(),
            PdfObject::String(b"ab0".to_vec())
        );
    }

    #[test]
    fn parses_name_hex_escapes() {
        let mut parser = PdfParser::new(br"/A#20Name", 0).unwrap();
        assert_eq!(
            parser.parse_object().unwrap(),
            PdfObject::Name("A Name".to_string())
        );
    }

    #[test]
    fn parses_non_utf8_names_lossily_instead_of_failing() {
        let mut parser = PdfParser::new(b"/A\xffName", 0).unwrap();
        assert_eq!(
            parser.parse_object().unwrap(),
            PdfObject::Name("A\u{fffd}Name".to_string())
        );
    }

    #[test]
    fn dictionary_skips_non_name_keys() {
        let mut parser = PdfParser::new(b"<< /A 1 2 /B 3 (junk) /C /D >>", 0).unwrap();
        let object = parser.parse_object().unwrap();
        let dict = object.as_dict().unwrap();
        assert_eq!(dict.get("A"), Some(&PdfObject::Integer(1)));
        assert_eq!(dict.get("B"), Some(&PdfObject::Integer(3)));
        assert_eq!(dict.get("C"), Some(&PdfObject::Name("D".to_string())));
        assert_eq!(dict.len(), 3);
    }

    #[test]
    fn missing_endobj_is_tolerated() {
        let mut parser =
            PdfParser::new(b"4 0 obj << /A 1 >>\n5 0 obj << /B 2 >> endobj", 0).unwrap();
        let object = parser.parse_indirect_object().unwrap();
        assert_eq!(object.number, 4);
        assert_eq!(
            object.object.as_dict().unwrap().get("A"),
            Some(&PdfObject::Integer(1))
        );
        let next = parser.parse_indirect_object().unwrap();
        assert_eq!(next.number, 5);
    }

    #[test]
    fn dictionary_missing_close_before_endobj_is_tolerated() {
        let mut parser = PdfParser::new(b"6 0 obj << /A 1 endobj", 0).unwrap();
        let object = parser.parse_indirect_object().unwrap();
        assert_eq!(
            object.object.as_dict().unwrap().get("A"),
            Some(&PdfObject::Integer(1))
        );
    }

    #[test]
    fn bare_keyword_values_and_array_items_are_tolerated_like_pdfium() {
        // `/Type XObject` (a missing slash) drops the key; an array ends at
        // the first non-object token; `endobj` closes everything still open.
        let mut parser = PdfParser::new(
            b"6 0 obj << /Type XObject /Subtype /Form /Kids [1 0 R p /A] /N 2 /R << /S R endobj",
            0,
        )
        .unwrap();
        let object = parser.parse_indirect_object().unwrap();
        let dict = object.object.as_dict().unwrap();
        assert_eq!(dict.get("Type"), None);
        assert_eq!(dict.get_name("Subtype"), Some("Form"));
        assert_eq!(
            dict.get("Kids"),
            Some(&PdfObject::Array(vec![PdfObject::Reference {
                number: 1,
                generation: 0
            }]))
        );
        assert_eq!(dict.get("N"), Some(&PdfObject::Integer(2)));
        assert!(dict.get("R").unwrap().as_dict().unwrap().get("S").is_none());
    }

    #[test]
    fn stream_end_scan_needs_keywords_like_pdfium() {
        // A wrong /Length: `eendstream`, `endstream.` and `(endstream ...)`
        // do not end the data (bug_524043); junk after `stream` on its line
        // is skipped when /Length fits from the next line (bug_543018).
        for (data, expected) in [
            (
                &b"1 0 obj << /Length 2 >> stream\nA\neendstream\nB\nendstream\nendobj"[..],
                &b"A\neendstream\nB"[..],
            ),
            (
                b"1 0 obj << /Length 2 >> stream\nA\nendstream.\n(endstream x)\nendobj",
                b"A\nendstream.\n(endstream x)",
            ),
            (
                b"1 0 obj << /Length 4 >> stream  \"junk\"\nBODY\nendstream\nendobj",
                b"BODY",
            ),
        ] {
            let mut parser = PdfParser::new(data, 0).unwrap();
            let object = parser.parse_indirect_object().unwrap();
            let (_, raw) = object.object.as_stream().unwrap();
            assert_eq!(raw, expected);
        }
    }

    #[test]
    fn stream_without_endstream_ends_at_endobj() {
        let mut parser = PdfParser::new(b"6 0 obj <<>> stream\n/1 cs\nendobj 7 0 obj", 0).unwrap();
        let object = parser.parse_indirect_object().unwrap();
        let (_, raw) = object.object.as_stream().unwrap();
        assert_eq!(raw, b"/1 cs");
    }

    #[test]
    fn integer_overflow_becomes_real() {
        let mut parser =
            PdfParser::new(b"[99999999999999999999 -99999999999999999999 7]", 0).unwrap();
        let PdfObject::Array(items) = parser.parse_object().unwrap() else {
            panic!("expected array");
        };
        assert_eq!(items[0], PdfObject::Real(99999999999999999999.0));
        assert_eq!(items[1], PdfObject::Real(-99999999999999999999.0));
        assert_eq!(items[2], PdfObject::Integer(7));
    }

    #[test]
    fn stray_minus_inside_number_is_ignored() {
        let mut parser = PdfParser::new(b"[0.00-5 1.5-2 -3 4 -5]", 0).unwrap();
        let PdfObject::Array(items) = parser.parse_object().unwrap() else {
            panic!("expected array");
        };
        assert_eq!(
            items,
            vec![
                PdfObject::Real(0.005),
                PdfObject::Real(1.52),
                PdfObject::Integer(-3),
                PdfObject::Integer(4),
                PdfObject::Integer(-5),
            ]
        );
    }

    struct FailingResolver;

    impl ParserResolver for FailingResolver {
        fn resolve_for_parser(&self, _object: &PdfObject) -> Result<PdfObject> {
            Err(WellfriendError::ParseError(
                "object is being resolved".to_string(),
            ))
        }
    }

    #[test]
    fn unresolvable_stream_length_scans_for_endstream() {
        let data = b"7 0 obj << /Length 7 0 R >> stream\nq 1 0 0 1 0 0 cm Q\nendstream endobj";
        let resolver = FailingResolver;
        let mut parser = PdfParser::with_resolver(data, 0, Some(&resolver)).unwrap();
        let object = parser.parse_indirect_object().unwrap();
        let (_, raw) = object.object.as_stream().unwrap();
        assert_eq!(raw, b"q 1 0 0 1 0 0 cm Q");
    }

    #[test]
    fn parses_references() {
        let mut parser = PdfParser::new(br"12 0 R", 0).unwrap();
        assert_eq!(
            parser.parse_object().unwrap(),
            PdfObject::Reference {
                number: 12,
                generation: 0
            }
        );
    }

    #[test]
    fn deeply_nested_arrays_error_instead_of_overflowing_stack() {
        // 100_000 unmatched '[' would recurse 100_000 frames deep without the
        // depth guard and abort the process with a stack overflow. With the
        // guard it must return a clean ParseError.
        let input = vec![b'['; 100_000];
        let mut parser = PdfParser::new(&input, 0).unwrap();
        let err = parser.parse_object().unwrap_err();
        assert!(
            matches!(err, WellfriendError::ParseError(ref msg) if msg.contains("depth limit")),
            "expected depth-limit ParseError, got {err:?}"
        );
    }

    #[test]
    fn deeply_nested_dictionaries_error_instead_of_overflowing_stack() {
        let mut input = Vec::new();
        for _ in 0..100_000 {
            input.extend_from_slice(b"<</K ");
        }
        let mut parser = PdfParser::new(&input, 0).unwrap();
        let err = parser.parse_object().unwrap_err();
        assert!(
            matches!(err, WellfriendError::ParseError(ref msg) if msg.contains("depth limit")),
            "expected depth-limit ParseError, got {err:?}"
        );
    }

    #[test]
    fn nesting_within_limit_still_parses_and_depth_resets() {
        // A wild but bounded well-formed nesting (180 levels) must parse fine,
        // and the parser's depth counter must return to 0 so sibling structures
        // are not penalised by earlier nesting.
        let depth = 180;
        let mut input = vec![b'['; depth];
        input.push(b'1');
        input.extend(std::iter::repeat_n(b']', depth));
        // A second, equally-nested sibling right after the first.
        let first_len = input.len();
        input.extend_from_within(0..first_len);

        let mut parser = PdfParser::new(&input, 0).unwrap();
        let first = parser.parse_object().expect("first nested array parses");
        let second = parser.parse_object().expect("second nested array parses");
        assert_eq!(first, second, "depth must reset between sibling structures");
    }

    #[test]
    fn nesting_beyond_limit_still_errors_cleanly() {
        // A well-formed nesting above the hard limit still returns a clean
        // ParseError rather than recursing until the stack overflows.
        let depth = MAX_PARSE_DEPTH + 1;
        let mut input = vec![b'['; depth];
        input.push(b'1');
        input.extend(std::iter::repeat_n(b']', depth));

        let mut parser = PdfParser::new(&input, 0).unwrap();
        let err = parser.parse_object().unwrap_err();
        assert!(
            matches!(err, WellfriendError::ParseError(ref msg) if msg.contains("depth limit")),
            "expected depth-limit ParseError, got {err:?}"
        );
    }

    #[test]
    fn modest_nesting_still_parses_and_depth_resets() {
        // A small, well-formed nesting (10 levels) must parse fine, and the
        // parser's depth counter must return to 0 so sibling structures are
        // not penalised by earlier nesting.
        let mut input = vec![b'['; 10];
        input.push(b'1');
        input.extend(std::iter::repeat_n(b']', 10));
        // A second, equally-nested sibling right after the first.
        let first_len = input.len();
        input.extend_from_within(0..first_len);

        let mut parser = PdfParser::new(&input, 0).unwrap();
        let first = parser.parse_object().expect("first nested array parses");
        let second = parser.parse_object().expect("second nested array parses");
        assert_eq!(first, second, "depth must reset between sibling structures");
    }
}
