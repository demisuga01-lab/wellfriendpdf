//! Read the original PDF tail without rewriting signed bytes or offsets.
//!
//! Comments are PDF lexical whitespace, including between the startxref
//! integer and EOF. A comment containing either marker is still a comment.
//! All walks are linear in the caller's already bounded input, with no copied
//! buffer, recursion, regex backtracking, or search through earlier revisions
//! after a malformed final tail.

fn whitespace(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}

fn trivia(bytes: &[u8], mut cursor: usize, before_eof: bool) -> Option<usize> {
    while let Some(&byte) = bytes.get(cursor) {
        if whitespace(byte) {
            cursor += 1;
        } else if byte == b'%' {
            if before_eof && bytes[cursor..].starts_with(b"%%EOF") {
                let remainder = &bytes[cursor + 5..];
                let line_end = remainder
                    .iter()
                    .position(|byte| matches!(byte, b'\r' | b'\n'))
                    .unwrap_or(remainder.len());
                if remainder[..line_end].iter().all(|byte| whitespace(*byte)) {
                    break;
                }
            }
            // An unterminated comment cannot supply the following tail token.
            cursor += 1;
            while bytes.get(cursor).is_some_and(|b| !matches!(b, b'\r' | b'\n')) {
                cursor += 1;
            }
            bytes.get(cursor)?;
        } else {
            break;
        }
    }
    Some(cursor)
}

fn integer(bytes: &[u8], mut cursor: usize) -> Option<(usize, i64)> {
    if bytes.get(cursor) == Some(&b'+') {
        cursor += 1;
    }
    let first = cursor;
    let mut value = 0i64;
    while let Some(byte) = bytes.get(cursor).filter(|b| b.is_ascii_digit()) {
        value = value.checked_mul(10)?.checked_add(i64::from(*byte - b'0'))?;
        cursor += 1;
    }
    (cursor > first).then_some((cursor, value))
}

pub(super) fn parse(bytes: &[u8]) -> Option<i64> {
    if !bytes.starts_with(b"startxref") {
        return None;
    }
    let start = b"startxref".len();
    let number = trivia(bytes, start, false)?;
    // Preserve the original line-separated tail grammar while accepting PDF
    // whitespace/comments. A keyword prefix and a same-line integer are not
    // silently accepted as the original startxref token.
    if number == start || !bytes[start..number].iter().any(|b| matches!(b, b'\r' | b'\n')) {
        return None;
    }
    let (end, value) = integer(bytes, number)?;
    let eof = trivia(bytes, end, true)?;
    if eof == end || !bytes[end..eof].iter().any(|b| matches!(b, b'\r' | b'\n')) {
        return None;
    }
    if !bytes.get(eof..)?.starts_with(b"%%EOF") {
        return None;
    }
    if bytes.get(eof + 5).is_some_and(|byte| !whitespace(*byte)) {
        return None;
    }
    Some(value)
}

fn trim_start(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|byte| whitespace(**byte)).count()
}

fn offset_line(bytes: &[u8]) -> bool {
    let Some((end, _)) = integer(bytes, 0) else {
        return false;
    };
    let tail = &bytes[end..];
    let significant = &tail[trim_start(tail)..];
    significant.is_empty() || significant[0] == b'%'
}

/// Resolve the final EOF selected by Reader using lexical lines, rather than
/// looking only 25 bytes back or selecting marker-shaped text inside comments.
pub(crate) fn before_eof(bytes: &[u8], eof: usize) -> Option<usize> {
    let eof_end = eof.checked_add(5)?;
    if bytes.get(eof..eof_end)? != b"%%EOF" || bytes.get(eof_end).is_some_and(|byte| !whitespace(*byte)) {
        return None;
    }
    let eof_line = bytes[..eof]
        .iter()
        .rposition(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(0, |position| position + 1);
    if !bytes[eof_line..eof].iter().all(|byte| whitespace(*byte)) {
        return None;
    }
    let mut end = eof_line;
    let mut saw_offset = false;
    while end > 0 {
        while end > 0 && matches!(bytes[end - 1], b'\r' | b'\n') {
            end -= 1;
        }
        let start = bytes[..end]
            .iter()
            .rposition(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(0, |position| position + 1);
        let line = &bytes[start..end];
        let significant_start = trim_start(line);
        let significant = &line[significant_start..];
        if significant.is_empty() || significant[0] == b'%' {
            end = start;
            continue;
        }
        if significant.starts_with(b"startxref") {
            if !saw_offset {
                return None;
            }
            let keyword = start.checked_add(significant_start)?;
            let offset = usize::try_from(super::xref_start(&bytes[keyword..eof_end])?).ok()?;
            // The reader still validates the actual xref table/stream. The
            // tail lexer only rejects a negative/overflow/out-of-source offset.
            return (offset < keyword).then_some(offset);
        }
        if saw_offset || !offset_line(significant) {
            return None;
        }
        saw_offset = true;
        end = start;
    }
    None
}
