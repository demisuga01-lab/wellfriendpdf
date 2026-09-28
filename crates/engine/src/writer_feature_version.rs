//! Minimum versions for writer-managed features. This is not a complete PDF
//! conformance validator. Raw incremental/signing bodies remain caller-owned.
use super::*;
#[cfg(test)]
#[path = "writer_feature_version_tests.rs"]
mod tests;

fn version(value: &str) -> Result<(u16, u16)> {
    let Some((major, minor)) = value.split_once('.') else {
        return Err(WellfriendError::invalid_input("invalid PDF version"));
    };
    let major = major
        .parse()
        .map_err(|_| WellfriendError::invalid_input("invalid PDF version"))?;
    let minor = minor
        .parse()
        .map_err(|_| WellfriendError::invalid_input("invalid PDF version"))?;
    Ok((major, minor))
}

fn has_opentype<'a>(objects: impl IntoIterator<Item = &'a PdfObject>) -> Result<bool> {
    // PDF streams are indirect objects. No decoding, parsing of compressed
    // payloads or speculative search through strings is needed here.
    for (index, object) in objects.into_iter().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("writer feature version")?;
        }
        if matches!(object, PdfObject::Stream { dict, .. } if dict.get_name("Subtype") == Some("OpenType"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn header(writer: &PdfWriter) -> Result<String> {
    let mut required = version(&writer.version)?;
    if writer.mode != WriterMode::ClassicXref {
        required = required.max((1, 5));
    }
    if let Some(encryption) = &writer.encryption {
        required = required.max(version(encryption.state.pdf_version())?);
    }
    if has_opentype(writer.objects.iter().map(|entry| &entry.object))? {
        required = required.max((1, 6));
    }
    Ok(format!("{}.{}", required.0, required.1))
}

pub(super) fn incremental(reader: &PdfReader, objects: &mut Vec<IncrementalObject>) -> Result<()> {
    if !has_opentype(objects.iter().map(|entry| &entry.object))?
        || version(reader.version())? >= (1, 6)
    {
        return Ok(());
    }
    let (number, generation) = reader.root_reference().ok_or_else(|| {
        WellfriendError::invalid_input("OpenType update requires a catalog reference")
    })?;
    let existing = objects.iter().position(|entry| entry.number == number);
    let mut catalog = match existing {
        Some(index) => {
            if objects[index].generation != generation {
                return Err(WellfriendError::invalid_input(
                    "updated catalog generation differs from trailer",
                ));
            }
            objects[index].object.clone()
        }
        None => reader.resolve(PdfObject::Reference { number, generation })?,
    };
    let PdfObject::Dictionary(dict) = &mut catalog else {
        return Err(WellfriendError::invalid_input(
            "OpenType update requires a catalog dictionary",
        ));
    };
    if let Some(declared) = dict.get("Version") {
        let Some(name) = declared.as_name() else {
            return Err(WellfriendError::invalid_input("invalid catalog Version"));
        };
        if version(name)? >= (1, 6) {
            return Ok(());
        }
    }
    dict.insert("Version", PdfObject::Name("1.6".into()));
    match existing {
        Some(index) => objects[index].object = catalog,
        None => objects.push(IncrementalObject {
            number,
            generation,
            object: catalog,
        }),
    }
    Ok(())
}
