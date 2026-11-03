use std::io::Read;

use crate::error::{Result, WellfriendError};

#[derive(Debug, Clone, PartialEq)]
pub enum ContentToken {
    Null,
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Name(String),
    LiteralString(Vec<u8>),
    HexString(Vec<u8>),
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
    Operator(String),
    InlineImageData(Vec<u8>),
}

/// A decoded content token together with its exact half-open byte range in the
/// decoded content stream. Inline-image data and its synthetic `EI` token keep
/// independent, non-overlapping ranges so mutation code can replace one
/// occurrence without scanning binary image bytes as PDF operators.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedContentToken {
    pub start: usize,
    pub end: usize,
    pub token: ContentToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InlineImageState {
    Normal,
    Params,
    Data,
    PendingEnd,
}

pub struct ContentTokenizer<'a> {
    data: &'a [u8],
    pos: usize,
    inline_image_state: InlineImageState,
    last_span: Option<[usize; 2]>,
    pending_inline_end_span: Option<[usize; 2]>,
}

/// Bytes requested from the reader per refill, as the original fixed buffer.
const STREAMING_READ_CHUNK: usize = 64 * 1024;

/// Tokenizes a content program read from `R`.
///
/// The bytes not yet consumed sit in one contiguous window
/// (`window[pos..]`), so a peek is an indexed load. The reader is asked for
/// more bytes only when a token needs a byte past everything read so far,
/// [`STREAMING_READ_CHUNK`] at a time, so reads (and their errors) happen at
/// the same stream offsets as a byte-at-a-time lookahead would see them. A
/// program already in memory ([`StreamingContentTokenizer::from_bytes`]) is
/// the window itself and is never copied.
pub struct StreamingContentTokenizer<R: Read> {
    reader: R,
    window: Vec<u8>,
    pos: usize,
    eof: bool,
    /// The window holds the whole program: tokens are scanned in place.
    complete: bool,
    inline_image_state: InlineImageState,
}

impl<'a> ContentTokenizer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            inline_image_state: InlineImageState::Normal,
            last_span: None,
            pending_inline_end_span: None,
        }
    }

    /// Return the next token with its exact decoded-stream source range.
    pub fn next_spanned(&mut self) -> Result<Option<SpannedContentToken>> {
        let Some(token) = self.next_token()? else {
            return Ok(None);
        };
        let [start, end] = self.last_span.ok_or_else(|| {
            WellfriendError::ParseError("content tokenizer lost token provenance".to_string())
        })?;
        Ok(Some(SpannedContentToken { start, end, token }))
    }

    fn next_token(&mut self) -> Result<Option<ContentToken>> {
        if self.inline_image_state == InlineImageState::PendingEnd {
            self.inline_image_state = InlineImageState::Normal;
            self.last_span = self.pending_inline_end_span.take();
            return Ok(Some(ContentToken::Operator("EI".to_string())));
        }
        if self.inline_image_state == InlineImageState::Data {
            let start = self.pos;
            let token = self.read_inline_image_data()?;
            let end = self
                .pending_inline_end_span
                .map(|span| span[0])
                .unwrap_or(self.pos);
            self.last_span = Some([start, end]);
            return Ok(Some(token));
        }

        self.skip_ws_and_comments();
        if self.pos >= self.data.len() {
            return Ok(None);
        }

        let token_start = self.pos;
        let byte = self.data[self.pos];
        let token = match byte {
            b'[' => {
                self.pos += 1;
                ContentToken::ArrayStart
            }
            b']' => {
                self.pos += 1;
                ContentToken::ArrayEnd
            }
            b'<' if self.starts_with(b"<<") => {
                self.pos += 2;
                ContentToken::DictStart
            }
            b'<' => self.read_hex_string()?,
            b'>' if self.starts_with(b">>") => {
                self.pos += 2;
                ContentToken::DictEnd
            }
            b'>' => {
                self.pos += 1;
                ContentToken::Operator("?".to_string())
            }
            b'/' => self.read_name(),
            b'(' => self.read_literal_string()?,
            byte if is_number_start(self.data, self.pos) => self.read_number(byte),
            _ => self.read_operator(),
        };

        match &token {
            ContentToken::Operator(op) if op == "BI" => {
                self.inline_image_state = InlineImageState::Params;
            }
            ContentToken::Operator(op)
                if op == "ID" && self.inline_image_state == InlineImageState::Params =>
            {
                self.consume_inline_image_data_separator();
                self.inline_image_state = InlineImageState::Data;
            }
            _ => {}
        }

        self.last_span = Some([token_start, self.pos]);

        Ok(Some(token))
    }

    fn read_number(&mut self, _first: u8) -> ContentToken {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let mut saw_dot = false;
        while let Some(byte) = self.peek() {
            match byte {
                b'0'..=b'9' => self.pos += 1,
                b'.' if !saw_dot => {
                    saw_dot = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.data[start..self.pos]).unwrap_or("");
        if saw_dot {
            text.parse::<f64>()
                .map(ContentToken::Real)
                .unwrap_or_else(|_| ContentToken::Operator("?".to_string()))
        } else {
            text.parse::<i64>()
                .map(ContentToken::Integer)
                .unwrap_or_else(|_| ContentToken::Operator("?".to_string()))
        }
    }

    fn read_name(&mut self) -> ContentToken {
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(byte) = self.peek() {
            if is_pdf_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.pos += 1;
            if byte == b'#' {
                let high = self.peek().and_then(hex_value);
                let low = self.data.get(self.pos + 1).copied().and_then(hex_value);
                if let (Some(high), Some(low)) = (high, low) {
                    self.pos += 2;
                    out.push((high << 4) | low);
                } else {
                    out.push(byte);
                }
            } else {
                out.push(byte);
            }
        }
        ContentToken::Name(String::from_utf8_lossy(&out).into_owned())
    }

    fn read_literal_string(&mut self) -> Result<ContentToken> {
        self.pos += 1;
        let mut depth = 1usize;
        let mut out = Vec::new();

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
                        return Ok(ContentToken::LiteralString(out));
                    }
                    out.push(byte);
                }
                b'\\' => self.read_literal_escape(&mut out),
                _ => out.push(byte),
            }
        }

        Err(WellfriendError::ParseError(
            "unterminated content literal string".to_string(),
        ))
    }

    fn read_literal_escape(&mut self, out: &mut Vec<u8>) {
        let Some(byte) = self.next_byte() else {
            return;
        };
        match byte {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0C),
            b'(' | b')' | b'\\' => out.push(byte),
            b'\r' => {
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut value = u16::from(byte - b'0');
                for _ in 0..2 {
                    match self.peek() {
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
    }

    fn read_hex_string(&mut self) -> Result<ContentToken> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut high: Option<u8> = None;

        while let Some(byte) = self.next_byte() {
            if byte == b'>' {
                if let Some(high_nibble) = high {
                    out.push(high_nibble << 4);
                }
                return Ok(ContentToken::HexString(out));
            }
            if is_pdf_whitespace(byte) {
                continue;
            }
            let Some(value) = hex_value(byte) else {
                continue;
            };
            match high.take() {
                Some(high_nibble) => out.push((high_nibble << 4) | value),
                None => high = Some(value),
            }
        }

        Err(WellfriendError::ParseError(
            "unterminated content hex string".to_string(),
        ))
    }

    fn read_operator(&mut self) -> ContentToken {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            if is_pdf_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            self.pos += 1;
            return ContentToken::Operator("?".to_string());
        }
        let op = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        match op.as_str() {
            "null" => ContentToken::Null,
            "true" => ContentToken::Boolean(true),
            "false" => ContentToken::Boolean(false),
            _ => ContentToken::Operator(op),
        }
    }

    fn read_inline_image_data(&mut self) -> Result<ContentToken> {
        let start = self.pos;
        let mut cursor = self.pos;
        while cursor + 2 < self.data.len() {
            if is_pdf_whitespace(self.data[cursor])
                && self.data[cursor + 1] == b'E'
                && self.data[cursor + 2] == b'I'
                && self
                    .data
                    .get(cursor + 3)
                    .copied()
                    .is_none_or(is_inline_image_end_follow)
            {
                let data = self.data[start..cursor].to_vec();
                self.pos = cursor + 3;
                self.pending_inline_end_span = Some([cursor + 1, cursor + 3]);
                self.inline_image_state = InlineImageState::PendingEnd;
                return Ok(ContentToken::InlineImageData(data));
            }
            cursor += 1;
        }

        // No `EI` terminator before end of data. Treat the remaining bytes as
        // the (unterminated) inline-image payload, consume to EOF, and LEAVE the
        // inline-image state so iteration terminates at EOF. Returning an error
        // here while staying in `Data` at the same position would loop forever
        // in a caller that recovers from token errors (regression: tokenizer
        // inline-image hang).
        let data = self.data[start..].to_vec();
        self.pos = self.data.len();
        self.inline_image_state = InlineImageState::Normal;
        self.pending_inline_end_span = None;
        Ok(ContentToken::InlineImageData(data))
    }

    fn consume_inline_image_data_separator(&mut self) {
        match self.peek() {
            Some(b'\r') => {
                self.pos += 1;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            Some(byte) if is_pdf_whitespace(byte) => self.pos += 1,
            _ => {}
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(byte) if is_pdf_whitespace(byte)) {
                self.pos += 1;
            }
            if self.peek() == Some(b'%') {
                while let Some(byte) = self.next_byte() {
                    if byte == b'\r' || byte == b'\n' {
                        if byte == b'\r' && self.peek() == Some(b'\n') {
                            self.pos += 1;
                        }
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn starts_with(&self, bytes: &[u8]) -> bool {
        self.data
            .get(self.pos..self.pos + bytes.len())
            .is_some_and(|slice| slice == bytes)
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }
}

impl Iterator for ContentTokenizer<'_> {
    type Item = Result<ContentToken>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token().transpose()
    }
}

impl StreamingContentTokenizer<std::io::Empty> {
    /// Tokenize a content program that is already in memory. Produces exactly
    /// the tokens `StreamingContentTokenizer::new(Cursor::new(data))` does.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            reader: std::io::empty(),
            window: data,
            pos: 0,
            eof: false,
            complete: true,
            inline_image_state: InlineImageState::Normal,
        }
    }
}

impl<R: Read> StreamingContentTokenizer<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            window: Vec::new(),
            pos: 0,
            eof: false,
            complete: false,
            inline_image_state: InlineImageState::Normal,
        }
    }

    /// With the whole program in the window, the length of the regular
    /// (non-whitespace, non-delimiter) run at the cursor.
    fn regular_run(&self) -> Option<usize> {
        if !self.complete {
            return None;
        }
        let rest = &self.window[self.pos..];
        Some(
            rest.iter()
                .position(|&byte| is_pdf_whitespace(byte) || is_delimiter(byte))
                .unwrap_or(rest.len()),
        )
    }

    /// The `index`-th unconsumed byte, after `ensure_lookahead(index + 1)`.
    fn lookahead_get(&self, index: usize) -> Option<u8> {
        self.window.get(self.pos + index).copied()
    }

    fn next_token(&mut self) -> Result<Option<ContentToken>> {
        if self.inline_image_state == InlineImageState::PendingEnd {
            self.inline_image_state = InlineImageState::Normal;
            return Ok(Some(ContentToken::Operator("EI".to_string())));
        }
        if self.inline_image_state == InlineImageState::Data {
            return self.read_inline_image_data().map(Some);
        }

        self.skip_ws_and_comments()?;
        if self.peek()?.is_none() {
            return Ok(None);
        }

        let byte = self.peek()?.expect("peek checked above");
        let token = match byte {
            b'[' => {
                self.consume(1)?;
                ContentToken::ArrayStart
            }
            b']' => {
                self.consume(1)?;
                ContentToken::ArrayEnd
            }
            b'<' if self.starts_with(b"<<")? => {
                self.consume(2)?;
                ContentToken::DictStart
            }
            b'<' => self.read_hex_string()?,
            b'>' if self.starts_with(b">>")? => {
                self.consume(2)?;
                ContentToken::DictEnd
            }
            b'>' => {
                self.consume(1)?;
                ContentToken::Operator("?".to_string())
            }
            b'/' => self.read_name()?,
            b'(' => self.read_literal_string()?,
            byte if self.is_number_start()? => self.read_number(byte)?,
            _ => self.read_operator()?,
        };

        match &token {
            ContentToken::Operator(op) if op == "BI" => {
                self.inline_image_state = InlineImageState::Params;
            }
            ContentToken::Operator(op)
                if op == "ID" && self.inline_image_state == InlineImageState::Params =>
            {
                self.consume_inline_image_data_separator()?;
                self.inline_image_state = InlineImageState::Data;
            }
            _ => {}
        }

        Ok(Some(token))
    }

    fn read_number(&mut self, _first: u8) -> Result<ContentToken> {
        if self.complete {
            // Scan in place: the same sign/digits/single-dot grammar.
            let rest = &self.window[self.pos..];
            let mut len = usize::from(matches!(rest.first(), Some(b'+' | b'-')));
            let mut saw_dot = false;
            while let Some(&byte) = rest.get(len) {
                match byte {
                    b'0'..=b'9' => len += 1,
                    b'.' if !saw_dot => {
                        saw_dot = true;
                        len += 1;
                    }
                    _ => break,
                }
            }
            let text = &rest[..len];
            let token = if saw_dot {
                parse_content_real(text)
                    .map(ContentToken::Real)
                    .ok_or_else(|| WellfriendError::ParseError("invalid content real".to_string()))
            } else {
                parse_content_integer(text)
                    .map(ContentToken::Integer)
                    .ok_or_else(|| {
                        WellfriendError::ParseError("invalid content integer".to_string())
                    })
            };
            self.pos += len;
            return token;
        }
        let mut out = NumberText::default();
        if matches!(self.peek()?, Some(b'+' | b'-')) {
            out.push(self.next_byte()?.expect("peek checked"));
        }
        let mut saw_dot = false;
        while let Some(byte) = self.peek()? {
            match byte {
                b'0'..=b'9' => out.push(self.next_byte()?.expect("peek checked")),
                b'.' if !saw_dot => {
                    saw_dot = true;
                    out.push(self.next_byte()?.expect("peek checked"));
                }
                _ => break,
            }
        }
        if saw_dot {
            out.real()
                .map(ContentToken::Real)
                .ok_or_else(|| WellfriendError::ParseError("invalid content real".to_string()))
        } else {
            out.integer()
                .map(ContentToken::Integer)
                .ok_or_else(|| WellfriendError::ParseError("invalid content integer".to_string()))
        }
    }

    fn read_name(&mut self) -> Result<ContentToken> {
        self.consume(1)?;
        if let Some(len) = self.regular_run() {
            let name = &self.window[self.pos..self.pos + len];
            if !name.contains(&b'#') {
                let name = String::from_utf8_lossy(name).into_owned();
                self.pos += len;
                return Ok(ContentToken::Name(name));
            }
        }
        let mut out = Vec::new();
        while let Some(byte) = self.peek()? {
            if is_pdf_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            let byte = self.next_byte()?.expect("peek checked");
            if byte == b'#' {
                let high = self.peek()?.and_then(hex_value);
                self.ensure_lookahead(2)?;
                let low = self.lookahead_get(1).and_then(hex_value);
                if let (Some(high), Some(low)) = (high, low) {
                    self.consume(2)?;
                    out.push((high << 4) | low);
                } else {
                    out.push(byte);
                }
            } else {
                out.push(byte);
            }
        }
        Ok(ContentToken::Name(
            String::from_utf8_lossy(&out).into_owned(),
        ))
    }

    fn read_literal_string(&mut self) -> Result<ContentToken> {
        self.consume(1)?;
        let mut depth = 1usize;
        let mut out = Vec::new();

        while let Some(byte) = self.next_byte()? {
            match byte {
                b'(' => {
                    depth += 1;
                    out.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(ContentToken::LiteralString(out));
                    }
                    out.push(byte);
                }
                b'\\' => self.read_literal_escape(&mut out)?,
                _ => out.push(byte),
            }
        }

        Err(WellfriendError::ParseError(
            "unterminated content literal string".to_string(),
        ))
    }

    fn read_literal_escape(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let Some(byte) = self.next_byte()? else {
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
                if self.peek()? == Some(b'\n') {
                    self.consume(1)?;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut value = u16::from(byte - b'0');
                for _ in 0..2 {
                    match self.peek()? {
                        Some(next @ b'0'..=b'7') => {
                            self.consume(1)?;
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

    fn read_hex_string(&mut self) -> Result<ContentToken> {
        self.consume(1)?;
        let mut out = Vec::new();
        let mut high: Option<u8> = None;

        while let Some(byte) = self.next_byte()? {
            if byte == b'>' {
                if let Some(high_nibble) = high {
                    out.push(high_nibble << 4);
                }
                return Ok(ContentToken::HexString(out));
            }
            if is_pdf_whitespace(byte) {
                continue;
            }
            let Some(value) = hex_value(byte) else {
                continue;
            };
            match high.take() {
                Some(high_nibble) => out.push((high_nibble << 4) | value),
                None => high = Some(value),
            }
        }

        Err(WellfriendError::ParseError(
            "unterminated content hex string".to_string(),
        ))
    }

    fn read_operator(&mut self) -> Result<ContentToken> {
        if let Some(len) = self.regular_run() {
            if len == 0 {
                self.consume(1)?;
                return Ok(ContentToken::Operator("?".to_string()));
            }
            let token = match &self.window[self.pos..self.pos + len] {
                b"null" => ContentToken::Null,
                b"true" => ContentToken::Boolean(true),
                b"false" => ContentToken::Boolean(false),
                op => ContentToken::Operator(String::from_utf8_lossy(op).into_owned()),
            };
            self.pos += len;
            return Ok(token);
        }
        let mut out = Vec::new();
        while let Some(byte) = self.peek()? {
            if is_pdf_whitespace(byte) || is_delimiter(byte) {
                break;
            }
            out.push(self.next_byte()?.expect("peek checked"));
        }
        if out.is_empty() {
            self.consume(1)?;
            return Ok(ContentToken::Operator("?".to_string()));
        }
        Ok(match out.as_slice() {
            b"null" => ContentToken::Null,
            b"true" => ContentToken::Boolean(true),
            b"false" => ContentToken::Boolean(false),
            _ => ContentToken::Operator(utf8_lossy_owned(out)),
        })
    }

    fn read_inline_image_data(&mut self) -> Result<ContentToken> {
        let mut data = Vec::new();
        loop {
            let Some(byte) = self.next_byte()? else {
                self.inline_image_state = InlineImageState::Normal;
                return Ok(ContentToken::InlineImageData(data));
            };
            if is_pdf_whitespace(byte) && self.starts_with(b"EI")? {
                self.ensure_lookahead(3)?;
                let follows = self.lookahead_get(2).is_none_or(is_inline_image_end_follow);
                if follows {
                    self.consume(2)?;
                    self.inline_image_state = InlineImageState::PendingEnd;
                    return Ok(ContentToken::InlineImageData(data));
                }
            }
            data.push(byte);
        }
    }

    fn consume_inline_image_data_separator(&mut self) -> Result<()> {
        match self.peek()? {
            Some(b'\r') => {
                self.consume(1)?;
                if self.peek()? == Some(b'\n') {
                    self.consume(1)?;
                }
            }
            Some(byte) if is_pdf_whitespace(byte) => self.consume(1)?,
            _ => {}
        }
        Ok(())
    }

    fn skip_ws_and_comments(&mut self) -> Result<()> {
        loop {
            while matches!(self.peek()?, Some(byte) if is_pdf_whitespace(byte)) {
                self.consume(1)?;
            }
            if self.peek()? == Some(b'%') {
                while let Some(byte) = self.next_byte()? {
                    if byte == b'\r' || byte == b'\n' {
                        if byte == b'\r' && self.peek()? == Some(b'\n') {
                            self.consume(1)?;
                        }
                        break;
                    }
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn is_number_start(&mut self) -> Result<bool> {
        let Some(first) = self.peek()? else {
            return Ok(false);
        };
        Ok(match first {
            b'0'..=b'9' => true,
            b'.' => {
                self.ensure_lookahead(2)?;
                matches!(self.lookahead_get(1), Some(b'0'..=b'9'))
            }
            b'+' | b'-' => {
                self.ensure_lookahead(2)?;
                matches!(self.lookahead_get(1), Some(b'0'..=b'9' | b'.'))
            }
            _ => false,
        })
    }

    fn starts_with(&mut self, bytes: &[u8]) -> Result<bool> {
        self.ensure_lookahead(bytes.len())?;
        Ok(self
            .window
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(bytes)))
    }

    #[inline]
    fn peek(&mut self) -> Result<Option<u8>> {
        if let Some(&byte) = self.window.get(self.pos) {
            return Ok(Some(byte));
        }
        self.ensure_lookahead(1)?;
        Ok(self.window.get(self.pos).copied())
    }

    #[inline]
    fn next_byte(&mut self) -> Result<Option<u8>> {
        let byte = self.peek()?;
        if byte.is_some() {
            self.pos += 1;
        }
        Ok(byte)
    }

    fn consume(&mut self, n: usize) -> Result<()> {
        self.ensure_lookahead(n)?;
        self.pos += n.min(self.window.len() - self.pos);
        Ok(())
    }

    /// Make at least `n` unconsumed bytes available, unless the reader ends
    /// first. The reader is called only when a byte past everything read so
    /// far is needed.
    fn ensure_lookahead(&mut self, n: usize) -> Result<()> {
        if self.window.len() - self.pos >= n || self.eof {
            return Ok(());
        }
        // Drop the consumed prefix before growing the window.
        if self.pos > 0 {
            self.window.drain(..self.pos);
            self.pos = 0;
        }
        while self.window.len() < n && !self.eof {
            let filled = self.window.len();
            self.window.resize(filled + STREAMING_READ_CHUNK, 0);
            match self.reader.read(&mut self.window[filled..]) {
                Ok(read) => {
                    self.window.truncate(filled + read);
                    if read == 0 {
                        self.eof = true;
                    }
                }
                Err(err) => {
                    self.window.truncate(filled);
                    return Err(WellfriendError::Io(err));
                }
            }
        }
        Ok(())
    }
}

/// `String::from_utf8_lossy(&bytes).into_owned()` without copying valid UTF-8.
fn utf8_lossy_owned(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    }
}

/// The text of a content-stream number, collected byte by byte when the
/// tokenizer reads from a stream.
#[derive(Default)]
struct NumberText {
    bytes: Vec<u8>,
}

impl NumberText {
    fn push(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    fn integer(&self) -> Option<i64> {
        parse_content_integer(&self.bytes)
    }

    fn real(&self) -> Option<f64> {
        parse_content_real(&self.bytes)
    }
}

/// The sign and the unsigned remainder of a number's text.
fn split_sign(bytes: &[u8]) -> (bool, &[u8]) {
    match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        Some(b'+') => (false, &bytes[1..]),
        _ => (false, bytes),
    }
}

/// `str::parse::<i64>` of a number's text, without the round trip through
/// the standard parser for ordinary values.
fn parse_content_integer(bytes: &[u8]) -> Option<i64> {
    let (negative, digits) = split_sign(bytes);
    // Up to 18 digits cannot overflow an i64; longer values (and malformed
    // ones) take the standard parser for its exact verdict.
    if !digits.is_empty() && digits.len() <= 18 && digits.iter().all(u8::is_ascii_digit) {
        let magnitude = digits
            .iter()
            .fold(0i64, |value, digit| value * 10 + i64::from(digit - b'0'));
        return Some(if negative { -magnitude } else { magnitude });
    }
    std::str::from_utf8(bytes).ok()?.parse::<i64>().ok()
}

/// `str::parse::<f64>` of a number's text, bit for bit.
fn parse_content_real(bytes: &[u8]) -> Option<f64> {
    let slow = || std::str::from_utf8(bytes).ok()?.parse::<f64>().ok();
    let (negative, digits) = split_sign(bytes);
    let mut mantissa = 0u64;
    let mut significant = 0usize;
    let mut fraction_digits = 0usize;
    let mut seen_dot = false;
    for &byte in digits {
        match byte {
            b'.' if !seen_dot => seen_dot = true,
            b'0'..=b'9' => {
                if significant == 15 {
                    return slow();
                }
                mantissa = mantissa * 10 + u64::from(byte - b'0');
                significant += 1;
                if seen_dot {
                    fraction_digits += 1;
                }
            }
            _ => return slow(),
        }
    }
    if significant == 0 {
        return slow();
    }
    // A mantissa below 10^15 and a power of ten up to 10^15 are both exact
    // doubles, so one correctly rounded division yields the correctly rounded
    // value of the decimal, which is what `str::parse` returns.
    let magnitude = mantissa as f64 / POWERS_OF_TEN[fraction_digits];
    Some(if negative { -magnitude } else { magnitude })
}

/// 10^0 ..= 10^15, all exact doubles.
const POWERS_OF_TEN: [f64; 16] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
];

impl<R: Read> Iterator for StreamingContentTokenizer<R> {
    type Item = Result<ContentToken>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token().transpose()
    }
}

pub fn tokenize_all(data: &[u8]) -> Result<Vec<ContentToken>> {
    ContentTokenizer::new(data).collect()
}

fn is_number_start(data: &[u8], pos: usize) -> bool {
    match data.get(pos).copied() {
        Some(b'0'..=b'9') => true,
        Some(b'.') => matches!(data.get(pos + 1), Some(b'0'..=b'9')),
        Some(b'+' | b'-') => matches!(data.get(pos + 1), Some(b'0'..=b'9' | b'.')),
        _ => false,
    }
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

fn is_inline_image_end_follow(byte: u8) -> bool {
    is_pdf_whitespace(byte) || is_delimiter(byte)
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
    fn tokenizes_basic_text_stream() {
        let tokens = tokenize_all(b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET").unwrap();
        assert_eq!(
            tokens,
            vec![
                ContentToken::Operator("BT".to_string()),
                ContentToken::Name("F1".to_string()),
                ContentToken::Integer(12),
                ContentToken::Operator("Tf".to_string()),
                ContentToken::Integer(100),
                ContentToken::Integer(700),
                ContentToken::Operator("Td".to_string()),
                ContentToken::LiteralString(b"Hello World".to_vec()),
                ContentToken::Operator("Tj".to_string()),
                ContentToken::Operator("ET".to_string()),
            ]
        );
    }

    #[test]
    fn streaming_tokenizer_matches_slice_tokenizer() {
        let data = b"BT /F1 12 Tf 100 700 Td [(Hello) 50 <576f726c64>] TJ ET\n% comment\nq Q";
        let slice_tokens = tokenize_all(data).unwrap();
        let streaming_tokens: Vec<_> = StreamingContentTokenizer::new(std::io::Cursor::new(data))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(streaming_tokens, slice_tokens);
    }

    #[test]
    fn tokenizes_numbers_and_reals() {
        let tokens = tokenize_all(b"1 2.5 -3 -.75 +0").unwrap();
        assert_eq!(
            tokens,
            vec![
                ContentToken::Integer(1),
                ContentToken::Real(2.5),
                ContentToken::Integer(-3),
                ContentToken::Real(-0.75),
                ContentToken::Integer(0),
            ]
        );
    }

    #[test]
    fn tokenizes_null_in_both_slice_and_streaming_modes() {
        let data = b"[null << /DecodeParms null >>]";
        let expected = vec![
            ContentToken::ArrayStart,
            ContentToken::Null,
            ContentToken::DictStart,
            ContentToken::Name("DecodeParms".to_string()),
            ContentToken::Null,
            ContentToken::DictEnd,
            ContentToken::ArrayEnd,
        ];
        assert_eq!(tokenize_all(data).unwrap(), expected);
        let streaming = StreamingContentTokenizer::new(std::io::Cursor::new(data))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(streaming, expected);
    }

    #[test]
    fn tokenizes_hex_string_and_dict_markers() {
        let tokens = tokenize_all(b"<</Key <48656C6C6F>>>").unwrap();
        assert_eq!(
            tokens,
            vec![
                ContentToken::DictStart,
                ContentToken::Name("Key".to_string()),
                ContentToken::HexString(b"Hello".to_vec()),
                ContentToken::DictEnd,
            ]
        );
    }

    #[test]
    fn tokenizes_literal_string_escapes() {
        assert_eq!(
            tokenize_all(b"(Hello\\nWorld)").unwrap(),
            vec![ContentToken::LiteralString(b"Hello\nWorld".to_vec())]
        );
    }

    #[test]
    fn tokenizes_balanced_nested_parentheses() {
        assert_eq!(
            tokenize_all(b"(outer (inner) text)").unwrap(),
            vec![ContentToken::LiteralString(b"outer (inner) text".to_vec())]
        );
    }

    #[test]
    fn skips_comments() {
        assert_eq!(
            tokenize_all(b"1 % this is a comment\n2").unwrap(),
            vec![ContentToken::Integer(1), ContentToken::Integer(2)]
        );
    }

    #[test]
    fn tokenizes_name_with_hex_escape() {
        assert_eq!(
            tokenize_all(b"/F#231").unwrap(),
            vec![ContentToken::Name("F#1".to_string())]
        );
    }

    #[test]
    fn tokenizes_inline_image_data_without_parsing_binary_bytes() {
        let tokens = tokenize_all(b"BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\xFF\xFF\x00 EI").unwrap();
        assert_eq!(
            tokens,
            vec![
                ContentToken::Operator("BI".to_string()),
                ContentToken::Name("W".to_string()),
                ContentToken::Integer(2),
                ContentToken::Name("H".to_string()),
                ContentToken::Integer(2),
                ContentToken::Name("CS".to_string()),
                ContentToken::Name("G".to_string()),
                ContentToken::Name("BPC".to_string()),
                ContentToken::Integer(8),
                ContentToken::Operator("ID".to_string()),
                ContentToken::InlineImageData(vec![0x00, 0xFF, 0xFF, 0x00]),
                ContentToken::Operator("EI".to_string()),
            ]
        );
    }

    #[test]
    fn unterminated_inline_image_terminates_and_does_not_hang() {
        // Regression: an inline image (`BI`/`ID`) with NO `EI` terminator used
        // to return a token error while leaving the tokenizer in the Data state
        // at the same position — a caller that recovers from token errors
        // (`ContentParser::parse`) then looped forever. The tokenizer must now
        // consume the rest as inline-image data and terminate at EOF.
        //
        // This is the libFuzzer-minimized class of input from the
        // `content_tokenizer` target.
        let data = b"BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\xFF\xFF\x00 no terminator here";
        let tokens = tokenize_all(data).expect("tokenizes without error");
        // Must include the BI, ID, and an InlineImageData token, then stop.
        assert!(tokens
            .iter()
            .any(|t| matches!(t, ContentToken::InlineImageData(_))));
        assert!(tokens
            .iter()
            .any(|t| matches!(t, ContentToken::Operator(op) if op == "ID")));

        // And the higher-level parser must also terminate (this is what hung).
        let ops = crate::content::ContentParser::parse(data).unwrap();
        assert!(ops.iter().any(|o| o.operator == "ID"));
    }

    fn number_text(text: &str) -> NumberText {
        let mut out = NumberText::default();
        for &byte in text.as_bytes() {
            out.push(byte);
        }
        out
    }

    /// The number fast paths return exactly what `str::parse` returns,
    /// bit for bit, for every sign, digit count and fraction length.
    #[test]
    fn number_fast_paths_match_the_standard_parser() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut texts: Vec<String> = [
            "0",
            "-0",
            "+0",
            "0.0",
            "-0.0",
            ".5",
            "-.5",
            "+.5",
            "5.",
            "-5.",
            "007",
            "1.000",
            "999999999999999",
            "9999999999999999",
            "0.000000000000001",
            "123456.789012345",
            "1234567.890123456",
            "-9223372036854775808",
            "9223372036854775807",
            "9223372036854775808",
            "123456789012345678",
            "1234567890123456789",
            "-",
            "+",
            ".",
            "-.",
            "+.",
            "",
            "0.1",
            "0.2",
            "0.3",
            "2.675",
            "1.005",
            "33.33333333333333",
        ]
        .iter()
        .map(|text| text.to_string())
        .collect();
        for _ in 0..20_000 {
            let digits = 1 + (next() % 19) as usize;
            let mut text = String::new();
            match next() % 3 {
                0 => text.push('-'),
                1 => text.push('+'),
                _ => {}
            }
            let dot = (next() % (digits as u64 + 2)) as usize;
            for index in 0..digits {
                if index == dot {
                    text.push('.');
                }
                text.push(char::from(b'0' + (next() % 10) as u8));
            }
            texts.push(text);
        }
        for text in &texts {
            let number = number_text(text);
            if text.contains('.') {
                let expected = text.parse::<f64>().ok().map(f64::to_bits);
                assert_eq!(number.real().map(f64::to_bits), expected, "{text}");
            } else {
                assert_eq!(number.integer(), text.parse::<i64>().ok(), "{text}");
            }
        }
    }

    /// A reader that hands out a few bytes per call and can fail once: the
    /// windowed tokenizer must see the same tokens and the same error as a
    /// byte-at-a-time reader.
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        step: usize,
        fail_at: Option<usize>,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.fail_at.is_some_and(|at| self.pos >= at) {
                self.fail_at = None;
                return Err(std::io::Error::other("trickle failure"));
            }
            let n = self.step.min(buf.len()).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn windowed_streaming_tokenizer_matches_slice_tokens_across_reads() {
        let data: &[u8] = b"BT /F#201 12 Tf 1 0 0 1 72 700 Tm [(Hel\\)lo) -250 <48 65>] TJ ET\n\
            q 0.5 0 0 .5 -3 +4 cm BI /W 1 /H 1 /BPC 8 /CS /G ID \x7f EI Q % end\n\
            [null true false] << /A 1 >> 1.5 -2.25 d";
        let expected = tokenize_all(data).unwrap();
        for step in [1, 2, 3, 7, 64, 100_000] {
            let tokens: Vec<_> = StreamingContentTokenizer::new(Trickle {
                data: data.to_vec(),
                pos: 0,
                step,
                fail_at: None,
            })
            .collect::<Result<Vec<_>>>()
            .unwrap();
            assert_eq!(tokens, expected, "step {step}");
        }
        let buffered: Vec<_> = StreamingContentTokenizer::from_bytes(data.to_vec())
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(buffered, expected);
    }

    #[test]
    fn windowed_streaming_tokenizer_reports_reader_errors() {
        let tokens: Vec<_> = StreamingContentTokenizer::new(Trickle {
            data: b"1 2 3 4 5 6 7 8 9".to_vec(),
            pos: 0,
            step: 4,
            fail_at: Some(8),
        })
        .collect();
        assert!(tokens
            .iter()
            .any(|token| matches!(token, Err(WellfriendError::Io(_)))));
        // The reader recovers after its single failure; every number before
        // and after the failure is still read in order.
        let numbers: Vec<i64> = tokens
            .iter()
            .filter_map(|token| match token {
                Ok(ContentToken::Integer(value)) => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }
}
