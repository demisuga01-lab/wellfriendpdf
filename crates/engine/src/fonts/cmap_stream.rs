//! Shared CMap stream/filter/inheritance loading. Stream bodies are parsed by
//! the canonical declarative grammar; referenced programs are bounded by depth.
use super::cmap_program::{Kind, Program, Result};
use crate::{PdfObject, PdfReader};
pub(crate) fn resolve(object: &PdfObject, reader: Option<&PdfReader>) -> Result<PdfObject> {
    reader.map_or_else(
        || Ok(object.clone()),
        |reader| reader.resolve(object.clone()).map_err(|e| e.to_string()),
    )
}
pub(crate) fn read_system(
    object: &PdfObject,
    reader: Option<&PdfReader>,
) -> Result<super::pdf_embedding::CidSystem> {
    let object = resolve(object, reader)?;
    let dict = object
        .as_dict()
        .ok_or("CIDSystemInfo must be a dictionary")?;
    let field = |key| -> Result<Vec<u8>> {
        let object = resolve(
            dict.get(key)
                .ok_or("missing CIDSystemInfo collection string")?,
            reader,
        )?;
        let bytes = object
            .as_string()
            .ok_or("CIDSystemInfo collection must be a string")?;
        if bytes.len() > 256 {
            return Err("CIDSystemInfo collection string limit".into());
        }
        Ok(bytes.to_vec())
    };
    let supplement = resolve(
        dict.get("Supplement")
            .ok_or("missing CIDSystemInfo supplement")?,
        reader,
    )?;
    Ok(super::pdf_embedding::CidSystem {
        registry: field("Registry")?,
        ordering: field("Ordering")?,
        supplement: supplement
            .as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("invalid CIDSystemInfo supplement")?,
    })
}
pub(crate) fn read(
    object: &PdfObject,
    reader: Option<&PdfReader>,
    kind: Kind,
    depth: usize,
) -> Result<Program> {
    crate::cancel::check_current_cancel("CMap inheritance").map_err(|e| e.to_string())?;
    if depth > 8 {
        return Err("CMap inheritance depth limit".into());
    }
    let object = resolve(object, reader)?;
    if let PdfObject::Name(name) = &object {
        return super::predefined_cmap::load_program(name, kind).map(|program| (*program).clone());
    }
    let PdfObject::Stream { dict, raw } = &object else {
        return Err("CMap must be a stream or supported resource name".into());
    };
    let inherited = dict
        .get("UseCMap")
        .map(|object| read(object, reader, kind, depth + 1))
        .transpose()?;
    let bytes = match reader {
        Some(reader) => {
            crate::filters::decode_stream_lossless(&object, reader)
                .map_err(|e| e.to_string())?
                .data
        }
        None => crate::filters::decode_stream_from_dict(dict, raw).map_err(|e| e.to_string())?,
    };
    let mut program = Program::parse(&bytes, kind, inherited, kind == Kind::Unicode)?;
    if let Some(system) = dict.get("CIDSystemInfo") {
        let system = read_system(system, reader)?;
        if program.system.as_ref().is_some_and(|body| {
            (program.system_declared && *body != system)
                || (body.ordering.as_slice() != b"Identity"
                    && (body.registry != system.registry || body.ordering != system.ordering))
        }) {
            return Err("CMap stream/dictionary character collection mismatch".into());
        }
        program.system = Some(system);
        program.system_declared = true;
    }
    if kind == Kind::Cid {
        if let Some(mode) = dict.get("WMode") {
            let mode = resolve(mode, reader)?
                .as_integer()
                .and_then(|value| u8::try_from(value).ok())
                .filter(|v| *v <= 1)
                .ok_or("invalid CMap dictionary WMode")?;
            if program.wmode_declared && program.wmode.is_some_and(|old| old != mode) {
                return Err("CMap stream/dictionary WMode mismatch".into());
            }
            program.wmode = Some(mode);
            program.wmode_declared = true;
        }
    }
    if let Some(name) = dict.get("CMapName") {
        let name = resolve(name, reader)?;
        let name = name.as_name().ok_or("invalid CMap dictionary name")?;
        if program.name.as_ref().is_some_and(|old| old != name) {
            return Err("CMap stream/dictionary name mismatch".into());
        }
        program.name = Some(name.into());
    }
    Ok(program)
}
