//! Undo and redo by object-level deltas.
//!
//! Every operation maps a PDF to a PDF. The delta of one applied operation
//! lists the objects whose definition differs between the two object graphs
//! (read with lopdf), each with its definition before and after (`None`:
//! absent), plus the trailer entries that name the document (`/Root`,
//! `/Info`, `/ID`, `/Encrypt`) and the highest object number on each side.
//!
//! Setting the "before" side of every listed object on the output gives the
//! input's object graph; setting the "after" side on the input gives the
//! output's. That is exact for every operation kind without a semantic
//! inverse per operation (an inverse `replaceText` would have to restore
//! subset glyphs, an inverse redaction the removed bytes, an inverse page
//! insertion the page tree), and its size is what the operation changed:
//! a text edit stores its content stream and font subset, not the document.
//!
//! Object and cross-reference streams and linearization dictionaries are
//! containers of the file layout, not objects of the graph, and are left
//! out (lopdf reads their members as objects and never writes them back).
//! The file layout that decides how the next operation numbers new objects
//! is restored too: lopdf numbers a new object after the highest entry of
//! the cross-reference data it read, and writes a cross-reference stream
//! (an entry of its own, one above every object) when the file it read had
//! one. Each side records that highest number and the cross-reference kind;
//! the restored document is written so that reading it back gives the same
//! two (a `null` object holds the number when no object reaches it), so an
//! operation applied after an undo numbers its objects exactly as the
//! server's replay of the shorter list does. A `null` object and an absent
//! one are the same to PDF readers and to the graph comparison.

use lopdf::xref::XrefType;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::error::EditorError;
use crate::overlay;

const MAGIC: &[u8; 5] = b"WFOD\x02";
/// Trailer entries that belong to the document, not to the file layout.
const TRAILER_KEYS: [&[u8]; 4] = [b"Root", b"Info", b"ID", b"Encrypt"];

fn invalid(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_DELTA_INVALID", message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Restore the input (undo).
    Backward,
    /// Restore the output (redo).
    Forward,
}

#[derive(Debug, Clone, Default)]
pub struct ObjectDelta {
    pub entries: Vec<(ObjectId, Option<Object>, Option<Object>)>,
    pub trailer_before: Dictionary,
    pub trailer_after: Dictionary,
    pub max_before: u32,
    pub max_after: u32,
    /// The side was read from a file with a cross-reference stream.
    pub stream_before: bool,
    pub stream_after: bool,
}

/// lopdf `Stream` equality includes the read position; graphs compare the
/// dictionary and the (encoded) bytes only, and numbers by value.
pub fn same(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Stream(x), Object::Stream(y)) => same_dict(&x.dict, &y.dict) && x.content == y.content,
        (Object::Dictionary(x), Object::Dictionary(y)) => same_dict(x, y),
        (Object::Array(x), Object::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        (Object::Real(x), Object::Real(y)) => x.to_bits() == y.to_bits() || x == y,
        // lopdf writes a real with an integral value as an integer, so a
        // number keeps its value, not its lexical kind, through a save.
        (Object::Real(x), Object::Integer(y)) | (Object::Integer(y), Object::Real(x)) => f64::from(*x) == *y as f64,
        _ => a == b,
    }
}

fn same_dict(a: &Dictionary, b: &Dictionary) -> bool {
    a.len() == b.len() && a.iter().all(|(key, value)| b.get(key).is_ok_and(|other| same(value, other)))
}

/// Containers of the file layout (`/ObjStm`, `/XRef`, `/Linearized`), the
/// same test lopdf's writer uses to leave them out.
fn layout_container(object: &Object) -> bool {
    let dict = match object {
        Object::Stream(stream) => &stream.dict,
        Object::Dictionary(dict) => dict,
        _ => return false,
    };
    dict.get(b"Type")
        .and_then(Object::as_name)
        .is_ok_and(|name| [b"ObjStm".as_slice(), b"XRef".as_slice(), b"Linearized".as_slice()].contains(&name))
        || dict.has(b"Linearized")
}

fn trailer_of(document: &Document) -> Dictionary {
    let mut out = Dictionary::new();
    for key in TRAILER_KEYS {
        if let Ok(value) = document.trailer.get(key) {
            out.set(key.to_vec(), value.clone());
        }
    }
    out
}

fn streamed(document: &Document) -> bool {
    matches!(document.reference_table.cross_reference_type, XrefType::CrossReferenceStream)
}

/// The delta between two object graphs.
pub fn between(before: &Document, after: &Document) -> ObjectDelta {
    let mut entries = Vec::new();
    fn graph<'a>(document: &'a Document, id: &ObjectId) -> Option<&'a Object> {
        document.objects.get(id).filter(|o| !layout_container(o) && !matches!(o, Object::Null))
    }
    let mut ids: Vec<ObjectId> = before.objects.keys().chain(after.objects.keys()).copied().collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        match (graph(before, &id), graph(after, &id)) {
            (None, None) => {}
            (Some(a), Some(b)) if same(a, b) => {}
            (a, b) => entries.push((id, a.cloned(), b.cloned())),
        }
    }
    ObjectDelta {
        entries,
        trailer_before: trailer_of(before),
        trailer_after: trailer_of(after),
        max_before: before.max_id,
        max_after: after.max_id,
        stream_before: streamed(before),
        stream_after: streamed(after),
    }
}

/// The delta of an operation that turned `before` into `after`.
pub fn compute(before: &[u8], after: &[u8]) -> Result<ObjectDelta, EditorError> {
    let a = overlay::load(before)?;
    let b = overlay::load(after)?;
    Ok(between(&a, &b))
}

impl ObjectDelta {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && same_dict(&self.trailer_before, &self.trailer_after)
            && self.max_before == self.max_after
            && self.stream_before == self.stream_after
    }

    /// Set one side of every listed object on `document`.
    pub fn restore(&self, document: &mut Document, direction: Direction) {
        for (id, before, after) in &self.entries {
            let side = match direction {
                Direction::Backward => before,
                Direction::Forward => after,
            };
            match side {
                Some(object) => {
                    document.objects.insert(*id, object.clone());
                }
                None => {
                    document.objects.remove(id);
                }
            }
        }
        let (trailer, max, stream) = match direction {
            Direction::Backward => (&self.trailer_before, self.max_before, self.stream_before),
            Direction::Forward => (&self.trailer_after, self.max_after, self.stream_after),
        };
        for key in TRAILER_KEYS {
            match trailer.get(key) {
                Ok(value) => document.trailer.set(key.to_vec(), value.clone()),
                Err(_) => {
                    document.trailer.remove(key);
                }
            }
        }
        // Layout containers lopdf still holds are dropped by its writer.
        document.objects.retain(|_, object| !layout_container(object));
        let highest = document.objects.keys().map(|id| id.0).max().unwrap_or(0);
        if stream && highest < max {
            // The writer puts its cross-reference stream at `max_id + 1`.
            document.reference_table.cross_reference_type = XrefType::CrossReferenceStream;
            document.max_id = max - 1;
        } else {
            // A table lists the objects only: hold `max` with a null object.
            document.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
            if highest < max {
                document.objects.insert((max, 0), Object::Null);
            }
            document.max_id = max.max(highest);
        }
    }

    /// Apply the delta to the bytes of the other side.
    pub fn apply(&self, bytes: &[u8], direction: Direction) -> Result<Vec<u8>, EditorError> {
        let mut document = overlay::load(bytes)?;
        self.restore(&mut document, direction);
        overlay::save(&mut document)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&self.max_before.to_le_bytes());
        out.extend_from_slice(&self.max_after.to_le_bytes());
        out.push(u8::from(self.stream_before) | (u8::from(self.stream_after) << 1));
        write_dict(&self.trailer_before, &mut out);
        write_dict(&self.trailer_after, &mut out);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (id, before, after) in &self.entries {
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&id.1.to_le_bytes());
            for side in [before, after] {
                match side {
                    Some(object) => {
                        out.push(1);
                        write_object(object, &mut out);
                    }
                    None => out.push(0),
                }
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EditorError> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.take(MAGIC.len())? != MAGIC {
            return Err(invalid("not an object delta"));
        }
        let max_before = reader.u32()?;
        let max_after = reader.u32()?;
        let streams = reader.u8()?;
        let trailer_before = reader.dict(0)?;
        let trailer_after = reader.dict(0)?;
        let count = reader.u32()? as usize;
        let mut entries = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let id = (reader.u32()?, reader.u16()?);
            let mut side = || -> Result<Option<Object>, EditorError> {
                match reader.u8()? {
                    0 => Ok(None),
                    1 => Ok(Some(reader.object(0)?)),
                    _ => Err(invalid("malformed delta entry")),
                }
            };
            let before = side()?;
            let after = side()?;
            entries.push((id, before, after));
        }
        if reader.at != bytes.len() {
            return Err(invalid("trailing bytes after the delta"));
        }
        Ok(Self {
            entries,
            trailer_before,
            trailer_after,
            max_before,
            max_after,
            stream_before: streams & 1 != 0,
            stream_after: streams & 2 != 0,
        })
    }
}

// ---- an exact binary encoding of lopdf objects ----

fn write_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn write_dict(dict: &Dictionary, out: &mut Vec<u8>) {
    out.extend_from_slice(&(dict.len() as u32).to_le_bytes());
    for (key, value) in dict.iter() {
        write_bytes(key, out);
        write_object(value, out);
    }
}

fn write_object(object: &Object, out: &mut Vec<u8>) {
    match object {
        Object::Null => out.push(0),
        Object::Boolean(value) => out.extend_from_slice(&[1, u8::from(*value)]),
        Object::Integer(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Object::Real(value) => {
            out.push(3);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Object::Name(name) => {
            out.push(4);
            write_bytes(name, out);
        }
        Object::String(text, format) => {
            out.push(5);
            out.push(u8::from(*format == StringFormat::Hexadecimal));
            write_bytes(text, out);
        }
        Object::Array(items) => {
            out.push(6);
            out.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for item in items {
                write_object(item, out);
            }
        }
        Object::Dictionary(dict) => {
            out.push(7);
            write_dict(dict, out);
        }
        Object::Stream(stream) => {
            out.push(8);
            out.push(u8::from(stream.allows_compression));
            write_dict(&stream.dict, out);
            write_bytes(&stream.content, out);
        }
        Object::Reference(id) => {
            out.push(9);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&id.1.to_le_bytes());
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], EditorError> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.bytes.len()).ok_or_else(|| invalid("truncated delta"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, EditorError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, EditorError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }
    fn u32(&mut self) -> Result<u32, EditorError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }
    fn u64(&mut self) -> Result<u64, EditorError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, EditorError> {
        let n = usize::try_from(self.u64()?).map_err(|_| invalid("delta length overflow"))?;
        Ok(self.take(n)?.to_vec())
    }
    fn dict(&mut self, depth: usize) -> Result<Dictionary, EditorError> {
        let count = self.u32()?;
        let mut dict = Dictionary::new();
        for _ in 0..count {
            let key = self.bytes()?;
            let value = self.object(depth + 1)?;
            dict.set(key, value);
        }
        Ok(dict)
    }
    fn object(&mut self, depth: usize) -> Result<Object, EditorError> {
        if depth > 256 {
            return Err(invalid("delta nesting too deep"));
        }
        Ok(match self.u8()? {
            0 => Object::Null,
            1 => Object::Boolean(self.u8()? != 0),
            2 => Object::Integer(i64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes"))),
            3 => Object::Real(f32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"))),
            4 => Object::Name(self.bytes()?),
            5 => {
                let format = if self.u8()? == 1 { StringFormat::Hexadecimal } else { StringFormat::Literal };
                Object::String(self.bytes()?, format)
            }
            6 => {
                let count = self.u32()? as usize;
                let mut items = Vec::with_capacity(count.min(1 << 16));
                for _ in 0..count {
                    items.push(self.object(depth + 1)?);
                }
                Object::Array(items)
            }
            7 => Object::Dictionary(self.dict(depth + 1)?),
            8 => {
                let allows_compression = self.u8()? != 0;
                let dict = self.dict(depth + 1)?;
                let content = self.bytes()?;
                // Built field by field: `Stream::new` would rewrite /Length.
                Object::Stream(Stream {
                    dict,
                    content,
                    allows_compression,
                    start_position: None,
                })
            }
            9 => Object::Reference((self.u32()?, self.u16()?)),
            _ => return Err(invalid("unknown object tag in delta")),
        })
    }
}

/// Whether two documents have the same object graph (layout containers
/// aside), the same document trailer entries and the same highest number.
pub fn same_graph(a: &Document, b: &Document) -> bool {
    between(a, b).is_empty()
}
