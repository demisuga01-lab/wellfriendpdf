//! Stream recovery is private to the explicit repair loader. Never locate the
//! end of binary content by searching for a textual `endstream` marker.

use super::{dictionary, eol, is_delimiter, is_whitespace, space, space0};
use crate::{Dictionary, Error, Object, Reader, Result, Stream};
use flate2::{Decompress, FlushDecompress, Status};
use nom::Parser;
use nom::bytes::complete::tag;
use nom::combinator::opt;
use nom::sequence::terminated;
use std::collections::HashSet;

fn failure() -> Error {
    Error::InvalidStream("Recovery needs a complete, unambiguous stream boundary".into())
}

fn boundary(input: &[u8]) -> bool {
    input
        .first()
        .is_none_or(|byte| is_whitespace(*byte) || is_delimiter(*byte))
}

// Return bytes following endstream only when the enclosing endobj is complete.
// The caller still consumes endobj using the ordinary object grammar.
fn terminated_stream(data: &[u8], length: usize) -> Option<&[u8]> {
    let tail = data.get(length..)?;
    let (tail, _) = opt(eol).parse(tail).ok()?;
    let tail = tail.strip_prefix(b"endstream")?;
    if !boundary(tail) {
        return None;
    }
    let (after_space, _) = space(tail).ok()?;
    let after_object = after_space.strip_prefix(b"endobj")?;
    boundary(after_object).then_some(tail)
}

fn first_filter(dict: &Dictionary) -> Option<&[u8]> {
    match dict.get(b"Filter").ok()? {
        Object::Name(name) => Some(name),
        Object::Array(filters) => filters.first()?.as_name().ok(),
        _ => None,
    }
}

fn flate_endpoint(data: &[u8], maximum_decoded: usize, budget: &mut usize) -> Result<usize> {
    let mut decoder = Decompress::new(true);
    let mut scratch = [0u8; 8192];
    let mut consumed = 0usize;
    let mut expanded = 0usize;
    loop {
        let end = consumed.saturating_add(8192).min(data.len());
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(&data[consumed..end], &mut scratch, FlushDecompress::None)
            .map_err(|_| failure())?;
        let read = usize::try_from(decoder.total_in() - before_in).map_err(|_| failure())?;
        let written = usize::try_from(decoder.total_out() - before_out).map_err(|_| failure())?;
        consumed = consumed.checked_add(read).ok_or_else(failure)?;
        expanded = expanded.checked_add(written).ok_or_else(failure)?;
        if expanded > maximum_decoded || written > *budget {
            return Err(failure());
        }
        *budget -= written;
        if status == Status::StreamEnd {
            // StreamEnd validates the zlib trailer/checksum. Trailing bytes are
            // handled solely by terminated_stream, never fed into another member.
            return Ok(consumed);
        }
        if read == 0 && written == 0 {
            return Err(failure());
        }
    }
}

pub(super) fn parse<'a>(input: &'a [u8], reader: &Reader, budget: &mut usize) -> Result<Option<(&'a [u8], Object)>> {
    let Ok((data, dict)) = terminated(dictionary, (space, tag(&b"stream"[..]), space0, eol)).parse(input) else {
        return Ok(None);
    };
    let length = dict.get(b"Length").and_then(|value| {
        if let Ok(id) = value.as_reference() {
            reader
                .get_object(id, &mut HashSet::new())
                .and_then(|value| value.as_i64())
        } else {
            value.as_i64()
        }
    });
    if let Ok(length) = length
        && let Ok(length) = usize::try_from(length)
        && let Some(remaining) = terminated_stream(data, length)
    {
        return Ok(Some((
            remaining,
            Object::Stream(Stream::new(dict, data[..length].to_vec())),
        )));
    }
    // Only an outer Flate filter has a decoder-proven endpoint here. External
    // file streams and other encodings retain the explicit ambiguous refusal.
    if dict.has(b"F") || !matches!(first_filter(&dict), Some(b"FlateDecode" | b"Fl")) {
        return Err(failure());
    }
    let limit = reader.max_decompressed_size.ok_or_else(failure)?;
    let length = flate_endpoint(data, limit, budget)?;
    let remaining = terminated_stream(data, length).ok_or_else(failure)?;
    Ok(Some((
        remaining,
        Object::Stream(Stream::new(dict, data[..length].to_vec())),
    )))
}
