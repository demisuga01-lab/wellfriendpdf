//! Batch source-grid detachment. Path identities are bound before mutation;
//! shared page-stream occurrences have already been cloned by the story writer.
use super::*;
use crate::advanced_editing::{lex_content, list_vector_objects, LexicalKind};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::PdfObject;

struct Binding {
    page: usize,
    stream_index: usize,
    range: [usize; 2],
    digest: String,
}
fn decode(reader: &crate::PdfReader, id: (u32, u16)) -> Result<Vec<u8>> {
    let decoded = decode_stream_lossless_with_limits(
        &reader.get_object(id.0, id.1)?,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: 64 * 1024 * 1024,
            ..Default::default()
        },
    )?;
    if decoded.status != StreamDecodeStatus::Complete {
        return Err(fail("table grid source stream is opaque"));
    }
    Ok(decoded.data)
}
fn bindings(input: &[u8], request: &LinkedStoryRequest) -> Result<Vec<Binding>> {
    let Some(table) = &request.table_layout else {
        return Ok(Vec::new());
    };
    if table.source_paint.len() > 100_000 {
        return Err(fail("table source-paint decision budget exceeded"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut decisions = BTreeMap::<(usize, String), SourcePaintAction>::new();
    for d in &table.source_paint {
        if decisions
            .insert((d.page, d.stable_id.clone()), d.action)
            .is_some()
        {
            return Err(fail("duplicate table source-paint decision"));
        }
    }
    let pages = request
        .frames
        .iter()
        .filter(|f| f.owner.is_none())
        .map(|f| f.page)
        .collect::<BTreeSet<_>>();
    let mut buffers = BTreeMap::new();
    let mut decoded_budget = 0usize;
    let mut out = Vec::new();
    for page_number in pages {
        let page = engine.document().get_page(page_number)?;
        let frames = request
            .frames
            .iter()
            .filter(|f| f.page == page_number && f.owner.is_none())
            .collect::<Vec<_>>();
        for object in list_vector_objects(input, page_number)?.objects {
            crate::cancel::check_current_cancel("table source grid ownership")?;
            // Include zero-height/width ruled paths; ordinary rectangle overlap
            // tests would miss exactly the horizontal and vertical table lines.
            let touches = frames.iter().any(|f| {
                object.bbox[0] <= f.rect[2]
                    && f.rect[0] <= object.bbox[2]
                    && object.bbox[1] <= f.rect[3]
                    && f.rect[1] <= object.bbox[3]
            });
            let decision = decisions.remove(&(page_number, object.stable_id.clone()));
            if !touches {
                if decision.is_some() {
                    return Err(fail(
                        "table source-paint decision lies outside its approved frames",
                    ));
                }
                continue;
            }
            let decision = decision.ok_or_else(|| fail("table source frame intersects undecided vector artwork; explicitly keep or remove each occurrence"))?;
            if decision == SourcePaintAction::Keep {
                continue;
            }
            let source = &object.provenance;
            if object.clipping_path
                || object.clipping_context
                || source.marked_content_depth != 0
                || source.ocg_context.is_some()
                || !source.form_stack.is_empty()
                || source.form_invocation.is_some()
                || !source.form_invocation_path.is_empty()
                || page.contents.get(source.content_stream_index)
                    != Some(&(source.object_number, source.generation))
            {
                return Err(fail("table grid removal requires a page-owned path without clipping, optional content or semantic ownership"));
            }
            let id = (source.object_number, source.generation);
            if let std::collections::btree_map::Entry::Vacant(e) = buffers.entry(id) {
                let data = decode(reader, id)?;
                decoded_budget = decoded_budget.saturating_add(data.len());
                if decoded_budget > 256 * 1024 * 1024 {
                    return Err(fail("table grid decoded-stream budget exceeded"));
                }
                e.insert(data);
            }
            let range = [source.operation_byte_start, source.operation_byte_end];
            let data = buffers[&id]
                .get(range[0]..range[1])
                .ok_or_else(|| fail("table path range exceeds source stream"))?;
            let tokens = lex_content(data)?;
            let mut paints = 0;
            for token in tokens {
                match &token.kind {
                    LexicalKind::Number(_) if paints == 0 => {},
                    LexicalKind::Word(op) if paints == 0 && matches!(op.as_str(),"m"|"l"|"c"|"v"|"y"|"h"|"re") => {},
                    LexicalKind::Word(op) if paints == 0 && matches!(op.as_str(),"S"|"s"|"f"|"F"|"f*"|"B"|"B*"|"b"|"b*"|"n") => paints += 1,
                    _ => return Err(fail("table path contains interleaved state/text/content; use a source-aware graph rewrite")),
                }
            }
            if paints != 1 {
                return Err(fail("table source path has no complete paint operation"));
            }
            out.push(Binding {
                page: page_number,
                stream_index: source.content_stream_index,
                range,
                digest: hash(data),
            });
        }
    }
    if !decisions.is_empty() {
        return Err(fail(
            "table source-paint identity is stale, duplicated or not in an unowned frame",
        ));
    }
    Ok(out)
}
pub(crate) fn validate_source(input: &[u8], request: &LinkedStoryRequest) -> Result<()> {
    bindings(input, request).map(|_| ())
}

pub(crate) fn detach_source_paint(
    original: &[u8],
    isolated: &[u8],
    request: &LinkedStoryRequest,
) -> Result<Vec<u8>> {
    let bindings = bindings(original, request)?;
    if bindings.is_empty() {
        return Ok(isolated.to_vec());
    }
    let engine = ContentEngine::open_bytes(isolated.to_vec())?;
    let reader = engine.document().reader();
    let mut streams = BTreeMap::<(u32, u16), Vec<Binding>>::new();
    for binding in bindings {
        let page = engine.document().get_page(binding.page)?;
        let id = *page
            .contents
            .get(binding.stream_index)
            .ok_or_else(|| fail("table stream occurrence changed during isolation"))?;
        streams.entry(id).or_default().push(binding);
    }
    let mut updates = Vec::new();
    let mut decoded_budget = 0usize;
    for (id, mut patches) in streams {
        crate::cancel::check_current_cancel("table atomic path removal")?;
        let data = decode(reader, id)?;
        decoded_budget = decoded_budget.saturating_add(data.len());
        if decoded_budget > 256 * 1024 * 1024 {
            return Err(fail("table path rewrite budget exceeded"));
        }
        patches.sort_by_key(|p| p.range);
        let mut cursor = 0;
        let mut output = Vec::with_capacity(data.len());
        for patch in patches {
            if patch.range[0] < cursor
                || data
                    .get(patch.range[0]..patch.range[1])
                    .is_none_or(|b| hash(b) != patch.digest)
            {
                return Err(fail("table path source bytes changed or overlap"));
            }
            output.extend_from_slice(&data[cursor..patch.range[0]]);
            output.extend_from_slice(b" n ");
            cursor = patch.range[1];
        }
        output.extend_from_slice(&data[cursor..]);
        let object = reader.get_object(id.0, id.1)?;
        let mut dict = object
            .as_stream()
            .ok_or_else(|| fail("table path owner is not a stream"))?
            .0
            .clone();
        let raw = crate::filters::flate_encode_cancellable(&output, 6)?;
        dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
        dict.remove("DecodeParms");
        updates.push(IncrementalObject {
            number: id.0,
            generation: id.1,
            object: PdfObject::Stream { dict, raw },
        });
    }
    write_incremental_update(reader, updates)
}
