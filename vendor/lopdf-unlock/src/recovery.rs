//! Explicit damaged-xref recovery, isolated from ordinary PDF loading.
use crate::{
    Dictionary, Document, Error, LoadOptions, Object, ObjectId, ObjectStream, Reader, Result, parser,
    xref::{Xref, XrefEntry, XrefType},
};
use std::collections::{BTreeMap, HashSet};

fn failure() -> Error {
    Error::InvalidStream("Native cross-reference recovery could not establish a complete object graph".into())
}
fn whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}
fn boundary(byte: u8) -> bool {
    whitespace(byte) || b"()<>[]{}/%".contains(&byte)
}
fn unsigned(bytes: &[u8], mut offset: usize, max: u32) -> Option<(u32, usize)> {
    let start = offset;
    let mut value = 0u32;
    while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
        value = value.checked_mul(10)?.checked_add(u32::from(bytes[offset] - b'0'))?;
        if value > max || offset - start > 9 {
            return None;
        }
        offset += 1;
    }
    (offset > start).then_some((value, offset))
}
fn header(bytes: &[u8], offset: usize, max_id: u32) -> Option<ObjectId> {
    if offset > 0 && !boundary(bytes[offset - 1]) {
        return None;
    }
    let (id, mut end) = unsigned(bytes, offset, max_id)?;
    if id == 0 || !bytes.get(end).is_some_and(|byte| whitespace(*byte)) {
        return None;
    }
    while bytes.get(end).is_some_and(|byte| whitespace(*byte)) {
        end += 1
    }
    let (generation, next) = unsigned(bytes, end, u32::from(u16::MAX))?;
    end = next;
    if !bytes.get(end).is_some_and(|byte| whitespace(*byte)) {
        return None;
    }
    while bytes.get(end).is_some_and(|byte| whitespace(*byte)) {
        end += 1
    }
    (bytes.get(end..end + 3) == Some(b"obj".as_slice()) && bytes.get(end + 3).is_some_and(|byte| boundary(*byte)))
        .then_some((id, generation as u16))
}
fn token(bytes: &[u8], offset: usize, word: &[u8]) -> bool {
    bytes.get(offset..offset + word.len()) == Some(word)
        && bytes.get(offset + word.len()).is_none_or(|byte| boundary(*byte))
}

fn complete_graph(document: &Document) -> bool {
    let mut seen = HashSet::new();
    for object in document
        .objects
        .values()
        .chain(document.trailer.iter().map(|(_, value)| value))
    {
        let mut work = vec![(object, 0usize)];
        while let Some((value, depth)) = work.pop() {
            if depth > 100 {
                return false;
            }
            match value {
                Object::Reference(id) => {
                    if seen.insert(*id) && !document.objects.contains_key(id) {
                        return false;
                    }
                }
                Object::Array(values) => work.extend(values.iter().map(|value| (value, depth + 1))),
                Object::Dictionary(values) => work.extend(values.iter().map(|(_, value)| (value, depth + 1))),
                Object::Stream(stream) => {
                    if stream.start_position.is_some() {
                        return false;
                    }
                    work.extend(stream.dict.iter().map(|(_, value)| (value, depth + 1)));
                }
                _ => {}
            }
        }
    }
    true
}

fn complete_active_objects(document: &Document) -> bool {
    document.reference_table.entries.iter().all(|(id, entry)| match entry {
        XrefEntry::Normal { generation, .. } => document.objects.contains_key(&(*id, *generation)),
        XrefEntry::Compressed { .. } => document.objects.contains_key(&(*id, 0)),
        XrefEntry::Free | XrefEntry::UnusableFree => true,
    })
}

impl Document {
    /// Ordinary bounded load first; explicit recovery on a parse failure or an
    /// incomplete active graph (ordinary loading may skip malformed objects).
    /// No password/authority is inferred. Complete object spans exclude string/
    /// stream contents, actual forward length bindings are rechecked, encrypted
    /// or ambiguous recovery is refused, and object streams use the same bound.
    pub fn load_mem_with_recovery(
        buffer: &[u8], options: LoadOptions, maximum_objects: usize, maximum_object_id: u32,
    ) -> Result<(Document, bool)> {
        if buffer.is_empty()
            || buffer.len() > 300 << 20
            || maximum_objects == 0
            || maximum_objects > 100_000
            || maximum_object_id == 0
            || maximum_object_id > 1_000_000
            || options.password.is_some()
            || options.filter.is_some()
            || options
                .max_decompressed_size
                .is_none_or(|limit| limit == 0 || limit > 64 << 20)
        {
            return Err(failure());
        }
        if let Ok(document) = Self::load_mem_with_options(buffer, options.clone()) {
            if document.objects.len() > maximum_objects || document.max_id > maximum_object_id {
                return Err(failure());
            }
            if complete_active_objects(&document) && complete_graph(&document) {
                return Ok((document, false));
            }
        }
        let version = parser::header(buffer, options.strict).ok_or_else(failure)?;
        let mut seeds = BTreeMap::new();
        let mut count = 0usize;
        for offset in 0..buffer.len() {
            if buffer[offset].is_ascii_digit()
                && let Some(id) = header(buffer, offset, maximum_object_id)
            {
                count += 1;
                if count > maximum_objects {
                    return Err(failure());
                }
                seeds.insert(id, (offset, Object::Null));
            }
        }
        if seeds.is_empty() {
            return Err(failure());
        }
        let mut reader = Reader {
            buffer,
            document: Document::new(),
            encryption_state: None,
            raw_objects: BTreeMap::new(),
            password: None,
            strict: options.strict,
            max_decompressed_size: options.max_decompressed_size,
        };
        reader.document.reference_table = Xref::new(maximum_object_id + 1, XrefType::CrossReferenceTable);
        for (&id, &(offset, _)) in &seeds {
            reader.document.reference_table.entries.insert(
                id.0,
                XrefEntry::Normal {
                    offset: offset as u32,
                    generation: id.1,
                },
            );
        }
        let mut accepted = BTreeMap::new();
        let mut spans = Vec::new();
        let mut trailer = Dictionary::new();
        let mut offset = 0usize;
        // Shared by both physical passes and object-stream expansion. Endpoint
        // discovery cannot hide repeated decompression behind many small objects.
        let mut recovery_budget = 512usize << 20;
        while offset < buffer.len() {
            if whitespace(buffer[offset]) {
                offset += 1;
                continue;
            }
            if buffer[offset] == b'%' {
                while offset < buffer.len() && !matches!(buffer[offset], b'\n' | b'\r') {
                    offset += 1
                }
                continue;
            }
            if let Some(id) = header(buffer, offset, maximum_object_id) {
                let (value, end) = parser::recovery_indirect_object(buffer, offset, id, &reader, &mut recovery_budget)?;
                if end <= offset || accepted.len() >= maximum_objects {
                    return Err(failure());
                }
                if let Object::Stream(stream) = &value
                    && stream
                        .dict
                        .get(b"Type")
                        .and_then(Object::as_name)
                        .is_ok_and(|name| name == b"XRef")
                {
                    for (key, value) in stream.dict.iter() {
                        trailer.set(key.clone(), value.clone());
                    }
                }
                spans.push((id, offset, end, value.clone()));
                accepted.insert(id, (offset, value));
                offset = end;
                continue;
            }
            if token(buffer, offset, b"trailer") {
                let (value, end) = parser::recovery_direct_object(buffer, offset + 7)?;
                let Object::Dictionary(dictionary) = value else {
                    return Err(failure());
                };
                for (key, value) in dictionary.iter() {
                    trailer.set(key.clone(), value.clone());
                }
                offset = end;
                continue;
            }
            if matches!(buffer[offset], b'(' | b'<' | b'[') {
                let (_, end) = parser::recovery_direct_object(buffer, offset)?;
                if end <= offset {
                    return Err(failure());
                }
                offset = end;
                continue;
            }
            if token(buffer, offset, b"endstream") || token(buffer, offset, b"endobj") {
                return Err(failure());
            }
            // Consume an ordinary lexical token as a whole. Names/keywords do
            // not expose apparent object headers or trailer words inside them.
            offset += 1;
            while offset < buffer.len() && !boundary(buffer[offset]) {
                offset += 1
            }
        }
        if accepted.is_empty() || trailer.has(b"Encrypt") {
            return Err(failure());
        }
        // Replace speculative raw candidates with ONLY accepted physical spans,
        // then repeat native parsing under the actual forward-length bindings.
        reader.document.reference_table = Xref::new(maximum_object_id + 1, XrefType::CrossReferenceTable);
        for (&id, (offset, _)) in &accepted {
            reader.document.reference_table.entries.insert(
                id.0,
                XrefEntry::Normal {
                    offset: *offset as u32,
                    generation: id.1,
                },
            );
        }
        for (id, start, end, original) in &spans {
            let (actual, actual_end) =
                parser::recovery_indirect_object(buffer, *start, *id, &reader, &mut recovery_budget)?;
            if actual_end != *end || actual != *original {
                return Err(failure());
            }
        }
        let mut document = Document::new();
        document.version = version;
        let mut positions = BTreeMap::new();
        for (id, (position, value)) in &accepted {
            document.objects.insert(*id, value.clone());
            positions.insert(*id, *position);
        }
        // Decode recovered object streams with bounded native parsing. Later
        // physical definitions win; content that resembles an object header was
        // already excluded from accepted spans by the enclosing stream parser.
        for (position, value) in accepted.values() {
            let Object::Stream(stream) = value else { continue };
            if !stream
                .dict
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"ObjStm")
            {
                continue;
            }
            let mut owned = stream.clone();
            let limit = options.max_decompressed_size.unwrap().min(recovery_budget);
            let declared = stream.dict.get(b"N").and_then(Object::as_i64)?;
            if declared < 0 || declared as u64 > maximum_objects as u64 {
                return Err(failure());
            }
            let objects = ObjectStream::new_with_limit(&mut owned, Some(limit))?;
            if objects.objects.len() != declared as usize {
                return Err(failure());
            }
            // Ordinary loading is permissive about malformed object-stream
            // indices. Recovery requires every declared object and its entire
            // indexed direct-object span, rather than silently dropping data.
            let first = usize::try_from(owned.dict.get(b"First").and_then(Object::as_i64)?).map_err(|_| failure())?;
            let index = std::str::from_utf8(owned.content.get(..first).ok_or_else(failure)?).map_err(|_| failure())?;
            let mut numbers = Vec::new();
            for word in index.split_whitespace() {
                if numbers.len() >= (declared as usize).saturating_mul(2) {
                    return Err(failure());
                }
                numbers.push(word.parse::<u32>().map_err(|_| failure())?);
            }
            if numbers.len() != (declared as usize).saturating_mul(2) {
                return Err(failure());
            }
            let mut locations = BTreeMap::new();
            for pair in numbers.chunks_exact(2) {
                let start = first.checked_add(pair[1] as usize).ok_or_else(failure)?;
                if start >= owned.content.len() || locations.insert(start, pair[0]).is_some() {
                    return Err(failure());
                }
            }
            let locations: Vec<_> = locations.into_iter().collect();
            if locations.is_empty() && first != owned.content.len() {
                return Err(failure());
            }
            for (number, (start, id)) in locations.iter().enumerate() {
                let end = locations
                    .get(number + 1)
                    .map_or(owned.content.len(), |(offset, _)| *offset);
                let (actual, consumed) = parser::recovery_direct_object(&owned.content[..end], *start)?;
                if consumed != end || objects.objects.get(&(*id, 0)) != Some(&actual) {
                    return Err(failure());
                }
            }
            recovery_budget = recovery_budget.checked_sub(owned.content.len()).ok_or_else(failure)?;
            for (id, value) in objects.objects {
                if id.0 == 0 || id.0 > maximum_object_id || id.1 != 0 {
                    return Err(failure());
                }
                if positions.get(&id).is_none_or(|previous| previous < position) {
                    document.objects.insert(id, value);
                    positions.insert(id, *position);
                    if document.objects.len() > maximum_objects {
                        return Err(failure());
                    }
                }
            }
        }
        for key in [
            b"Prev".as_slice(),
            b"XRefStm",
            b"Type",
            b"W",
            b"Index",
            b"Length",
            b"Filter",
            b"DecodeParms",
        ] {
            trailer.remove(key);
        }
        if !trailer.has(b"Root") {
            let roots: Vec<_> = document
                .objects
                .iter()
                .filter_map(|(id, value)| {
                    value
                        .as_dict()
                        .ok()
                        .filter(|dict| {
                            dict.get(b"Type")
                                .and_then(Object::as_name)
                                .is_ok_and(|name| name == b"Catalog")
                        })
                        .map(|_| *id)
                })
                .collect();
            if roots.len() != 1 {
                return Err(failure());
            }
            trailer.set("Root", Object::Reference(roots[0]));
        }
        document.max_id = document.objects.keys().map(|id| id.0).max().ok_or_else(failure)?;
        trailer.set("Size", i64::from(document.max_id) + 1);
        document.trailer = trailer;
        document.reference_table = reader.document.reference_table;
        if document.catalog().is_err() || document.get_pages().is_empty() {
            return Err(failure());
        }
        // No secrets, filesystem writes, external engines or grant in recovery.
        if !complete_graph(&document) {
            return Err(failure());
        }
        Ok((document, true))
    }
}
