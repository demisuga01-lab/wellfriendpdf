//! Native text-show operands. Never replace positioning operators with a drawn overlay.
use crate::{MAX_EDITABLE_TEXT_RUN_BYTES, PdfEditError};
use lopdf::content::Operation;
use lopdf::{Document, Encoding, Object, StringFormat};

#[derive(Default)]
pub(crate) struct FontState {
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) size: Option<f64>,
    stack: Vec<(Option<Vec<u8>>, Option<f64>)>,
}

impl FontState {
    pub(crate) fn apply(&mut self, operation: &Operation) -> Result<(), PdfEditError> {
        match operation.operator.as_str() {
            "Tf" => {
                self.name = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .map(ToOwned::to_owned);
                self.size = operation.operands.get(1).and_then(crate::pdf_number);
            }
            "q" => {
                if self.stack.len() >= crate::MAX_PDF_OBJECT_NESTING {
                    return Err(PdfEditError::ContentStreamUnsupported);
                }
                self.stack.push((self.name.clone(), self.size));
            }
            "Q" => {
                let (name, size) = self
                    .stack
                    .pop()
                    .ok_or(PdfEditError::ContentStreamUnsupported)?;
                self.name = name;
                self.size = size;
            }
            _ => {}
        }
        Ok(())
    }
}

fn string_operand(operation: &Operation) -> Result<usize, PdfEditError> {
    match (operation.operator.as_str(), operation.operands.as_slice()) {
        ("Tj" | "'", [Object::String(..)]) => Ok(0),
        ("\"", [word, character, Object::String(..)]) if number(word) && number(character) => Ok(2),
        _ => Err(PdfEditError::TextRunUnsupported),
    }
}

fn number(value: &Object) -> bool {
    matches!(value, Object::Integer(_))
        || matches!(value, Object::Real(number) if number.is_finite())
}

pub(crate) fn bytes(operation: &Operation) -> Result<Vec<u8>, PdfEditError> {
    if operation.operator != "TJ" {
        let value = operation.operands[string_operand(operation)?]
            .as_str()
            .unwrap();
        if value.len() > MAX_EDITABLE_TEXT_RUN_BYTES {
            return Err(PdfEditError::TextRunUnsupported);
        }
        return Ok(value.to_vec());
    }
    let [Object::Array(items)] = operation.operands.as_slice() else {
        return Err(PdfEditError::TextRunUnsupported);
    };
    if items.len() > MAX_EDITABLE_TEXT_RUN_BYTES * 4 {
        return Err(PdfEditError::TextRunUnsupported);
    }
    let mut result = Vec::new();
    let mut strings = 0;
    for item in items {
        if let Object::String(value, _) = item {
            if result.len().saturating_add(value.len()) > MAX_EDITABLE_TEXT_RUN_BYTES {
                return Err(PdfEditError::TextRunUnsupported);
            }
            result.extend_from_slice(value);
            strings += 1;
        } else if !number(item) {
            return Err(PdfEditError::TextRunUnsupported);
        }
    }
    if strings == 0 {
        return Err(PdfEditError::TextRunUnsupported);
    }
    Ok(result)
}

fn encode_exact(encoding: &Encoding<'_>, value: &str) -> Result<Vec<u8>, PdfEditError> {
    let encoded = Document::encode_text(encoding, value);
    if Document::decode_text(encoding, &encoded).ok().as_deref() != Some(value) {
        return Err(PdfEditError::GlyphUnavailable);
    }
    Ok(encoded)
}

pub(crate) fn replace(
    operation: &mut Operation,
    encoding: &Encoding<'_>,
    replacement: &str,
) -> Result<(), PdfEditError> {
    let source_bytes = bytes(operation)?;
    let source = Document::decode_text(encoding, &source_bytes)
        .map_err(|_| PdfEditError::TextRunUnsupported)?;
    let encoded = encode_exact(encoding, replacement)?;
    if operation.operator != "TJ" {
        let index = string_operand(operation)?;
        let Object::String(_, format) = operation.operands[index] else {
            unreachable!()
        };
        operation.operands[index] = Object::String(encoded, format);
        return Ok(());
    }

    // Kerning belongs to the original adjacent glyphs, not arbitrary byte offsets.
    // Keep adjustments in unchanged prefix/suffix text; discard adjustments inside
    // the replaced range. Encode complete Unicode slices, never split a CID byte pair.
    let old: Vec<char> = source.chars().collect();
    let new: Vec<char> = replacement.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let Object::Array(items) = &operation.operands[0] else {
        unreachable!()
    };
    let mut adjustments = Vec::new();
    let mut position = 0;
    let mut decoded_parts = String::new();
    for item in items {
        if let Object::String(value, _) = item {
            let part = Document::decode_text(encoding, value)
                .map_err(|_| PdfEditError::TextRunUnsupported)?;
            position += part.chars().count();
            if position > old.len() {
                return Err(PdfEditError::TextRunUnsupported);
            }
            decoded_parts.push_str(&part);
        } else if position <= prefix {
            adjustments.push((position, item.clone()));
        } else if position >= old.len() - suffix {
            adjustments.push((new.len() - (old.len() - position), item.clone()));
        }
    }
    if decoded_parts != source {
        return Err(PdfEditError::TextRunUnsupported);
    }
    let mut output = Vec::new();
    let mut cursor = 0;
    for (end, adjustment) in adjustments {
        if end > cursor {
            let text: String = new[cursor..end].iter().collect();
            output.push(Object::String(
                encode_exact(encoding, &text)?,
                StringFormat::Hexadecimal,
            ));
        }
        output.push(adjustment);
        cursor = end;
    }
    if cursor < new.len() || !output.iter().any(|item| matches!(item, Object::String(..))) {
        let text: String = new[cursor..].iter().collect();
        output.push(Object::String(
            encode_exact(encoding, &text)?,
            StringFormat::Hexadecimal,
        ));
    }
    operation.operands[0] = Object::Array(output);
    Ok(())
}
