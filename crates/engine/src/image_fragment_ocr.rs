//! An owned native image/search group. Child Forms retain original resources;
//! all geometry is applied to the common parent, never independently to OCR.
use super::*;
const PROGRAM: &[u8] = b"q\n/Visual Do\n/Search Do\nQ\n";

pub(super) fn group(
    updates: &mut Updates,
    prepared: &Prepared,
    visual: Ref,
    mut source_dict: PdfDictionary,
    capture: &OcrCapture,
) -> Result<Ref> {
    source_dict.remove("WFImageCapsule");
    source_dict.remove("WFImageSize");
    let search = updates.stream(&capture.program, source_dict)?;
    group_forms(updates, prepared, visual, search, capture)
}

pub(super) fn group_prebuilt(
    updates: &mut Updates,
    prepared: &Prepared,
    visual: Ref,
    search: Ref,
    capture: &OcrCapture,
) -> Result<Ref> {
    group_forms(updates, prepared, visual, search, capture)
}

fn group_forms(
    updates: &mut Updates,
    prepared: &Prepared,
    visual: Ref,
    search: Ref,
    capture: &OcrCapture,
) -> Result<Ref> {
    let mut xobjects = PdfDictionary::empty();
    xobjects.insert("Visual", reference(visual));
    xobjects.insert("Search", reference(search));
    let mut resources = PdfDictionary::empty();
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("XObject".into()));
    d.insert("Subtype", PdfObject::Name("Form".into()));
    d.insert("FormType", PdfObject::Integer(1));
    let [x, y, _, _] = prepared.preview.source_rect;
    let c = prepared.source.crop_box;
    d.insert("BBox", array([c[0] - x, c[1] - y, c[2] - x, c[3] - y]));
    d.insert("Resources", PdfObject::Dictionary(resources));
    d.insert("WFImageCapsule", PdfObject::Boolean(true));
    d.insert(
        "WFImageSize",
        PdfObject::Array(prepared.size.into_iter().map(PdfObject::Real).collect()),
    );
    d.insert("WFOcrGroup", PdfObject::Boolean(true));
    d.insert("WFOcrSpans", PdfObject::Integer(capture.spans as i64));
    d.insert(
        "WFOcrText",
        PdfObject::String(capture.text.as_bytes().to_vec()),
    );
    d.insert(
        "WFOcrProgramHash",
        PdfObject::String(hash(&capture.program).into_bytes()),
    );
    updates.stream(PROGRAM, d)
}

fn validate_nested_search(
    reader: &PdfReader,
    form: Ref,
    seen: &mut BTreeSet<Ref>,
    depth: usize,
    total: &mut usize,
) -> Result<String> {
    if depth >= 64 || !seen.insert(form) {
        return Err(fail(
            "nested OCR search chain is cyclic or exceeds its depth budget",
        ));
    }
    let object = reader.get_object(form.0, form.1)?;
    let dictionary = object
        .as_stream()
        .ok_or_else(|| fail("nested OCR search node is not a stream"))?
        .0;
    if dictionary.get_name("Subtype") != Some("Form")
        || dictionary.get_bool("WFNestedOcrSearch") != Some(true)
        || dictionary.contains_key("StructParent")
        || dictionary.contains_key("StructParents")
    {
        return Err(fail(
            "nested OCR search node lost its private Form identity",
        ));
    }
    let program = decode(reader, form)?;
    *total = total
        .checked_add(program.len())
        .ok_or_else(|| fail("nested OCR search byte count overflow"))?;
    if *total > MAX_TOTAL {
        return Err(fail(
            "nested OCR search chain exceeds its decoded-byte budget",
        ));
    }
    let mut calls = Vec::<(usize, usize, String)>::new();
    operations(&program, |start, end, operation, inline| {
        if inline {
            return Err(fail("nested OCR search chain contains inline visual paint"));
        }
        if operation.operator == "Do" {
            let name = operation
                .operands
                .first()
                .and_then(Operand::as_name)
                .ok_or_else(|| fail("nested OCR search has an invalid child invocation"))?;
            calls.push((start, end, name.to_owned()));
        }
        Ok(())
    })?;
    if calls.is_empty() {
        validate_carrier_program(&program)?;
        return Ok(hash(&program));
    }
    if calls.len() != 1 {
        return Err(fail(
            "nested OCR search node must invoke exactly one private child",
        ));
    }
    let (start, end, name) = &calls[0];
    let without_child =
        ocr_carriers::apply_patches(&program, vec![(*start, *end, b"\n".to_vec())])?;
    validate_carrier_program(&without_child)?;
    let resources = dict(reader, dictionary.get("Resources"))?;
    let xobjects = dict(reader, resources.get("XObject"))?;
    let child = xobjects
        .get(name)
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| fail("nested OCR search child resource is missing or indirect through an unsupported container"))?;
    validate_nested_search(reader, child, seen, depth + 1, total)
}

fn visual_chain_bytes(
    reader: &PdfReader,
    form: Ref,
    seen: &mut BTreeSet<Ref>,
    depth: usize,
    total: &mut usize,
) -> Result<()> {
    if depth >= 64 || !seen.insert(form) {
        return Err(fail(
            "nested OCR visual chain is cyclic or exceeds its depth budget",
        ));
    }
    let object = reader.get_object(form.0, form.1)?;
    let dictionary = object
        .as_stream()
        .ok_or_else(|| fail("OCR visual chain node is not a stream"))?
        .0;
    if dictionary.get_name("Subtype") != Some("Form")
        || (depth == 0 && dictionary.get_bool("WFImageCapsule") != Some(true))
        || (depth > 0 && dictionary.get_bool("WFNestedImageCapsule") != Some(true))
    {
        return Err(fail("OCR visual chain lost its private Form identity"));
    }
    let program = decode(reader, form)?;
    *total = total
        .checked_add(program.len())
        .ok_or_else(|| fail("nested OCR visual byte count overflow"))?;
    if *total > MAX_TOTAL {
        return Err(fail(
            "nested OCR visual chain exceeds its decoded-byte budget",
        ));
    }
    let resources = dict(reader, dictionary.get("Resources"))?;
    let xobjects = dict(reader, resources.get("XObject"))?;
    let mut nested = BTreeSet::new();
    operations(&program, |_, _, operation, inline| {
        if !inline && operation.operator == "Do" {
            if let Some(child) = operation
                .operands
                .first()
                .and_then(Operand::as_name)
                .and_then(|name| xobjects.get(name))
                .and_then(PdfObject::as_reference)
            {
                let child_object = reader.get_object(child.0, child.1)?;
                if child_object.as_stream().is_some_and(|(dictionary, _)| {
                    dictionary.get_bool("WFNestedImageCapsule") == Some(true)
                }) {
                    nested.insert(child);
                }
            }
        }
        Ok(())
    })?;
    if nested.len() > 1 {
        return Err(fail(
            "nested OCR visual node invokes multiple private capsule children",
        ));
    }
    if let Some(child) = nested.into_iter().next() {
        visual_chain_bytes(reader, child, seen, depth + 1, total)?;
    }
    Ok(())
}

/// Metadata is not an extraction result. Check the native child program hash
/// and grouping before describing this object as a coordinated OCR group.
pub(super) fn info(reader: &PdfReader, form: Ref) -> Result<(usize, String)> {
    inspect(reader, form).map(|(count, text, _)| (count, text))
}

/// Decoded program bytes are charged on reused groups too; prior ownership
/// must not bypass aggregate story capture/decompression budgets.
pub(super) fn inspect(reader: &PdfReader, form: Ref) -> Result<(usize, String, usize)> {
    let object = reader.get_object(form.0, form.1)?;
    let d = object
        .as_stream()
        .ok_or_else(|| fail("OCR group is not a Form stream"))?
        .0;
    if d.get_bool("WFOcrGroup") != Some(true) {
        return Ok((0, String::new(), decode(reader, form)?.len()));
    }
    if d.get_name("Subtype") != Some("Form")
        || d.contains_key("Matrix")
        || d.contains_key("Group")
        || decode(reader, form)? != PROGRAM
    {
        return Err(fail("native OCR group structure changed"));
    }
    let count = d
        .get_integer("WFOcrSpans")
        .filter(|v| (1..=4096).contains(v))
        .ok_or_else(|| fail("invalid OCR group span count"))? as usize;
    let text = d
        .get("WFOcrText")
        .and_then(PdfObject::as_string)
        .filter(|v| !v.is_empty() && v.len() <= 4_000_000)
        .and_then(|v| std::str::from_utf8(v).ok())
        .ok_or_else(|| fail("invalid OCR group source text"))?
        .to_owned();
    let resources = dict(reader, d.get("Resources"))?;
    let children = dict(reader, resources.get("XObject"))?;
    let visual = children
        .get("Visual")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| fail("OCR group visual child missing"))?;
    let search = children
        .get("Search")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| fail("OCR group search child missing"))?;
    if search == visual || search == form || visual == form {
        return Err(fail("cyclic/aliased OCR group children"));
    }
    let search_object = reader.get_object(search.0, search.1)?;
    let sd = search_object
        .as_stream()
        .ok_or_else(|| fail("OCR search child is not a stream"))?
        .0;
    let visual_object = reader.get_object(visual.0, visual.1)?;
    let vd = visual_object
        .as_stream()
        .ok_or_else(|| fail("OCR visual child is not a stream"))?
        .0;
    if sd.get_name("Subtype") != Some("Form")
        || vd.get_name("Subtype") != Some("Form")
        || vd.get_bool("WFImageCapsule") != Some(true)
        || sd.get("Matrix") != vd.get("Matrix")
        || sd.get("BBox") != vd.get("BBox")
    {
        return Err(fail("OCR group child geometry changed"));
    }
    let program = decode(reader, search)?;
    let (search_bytes, program_hash) = if sd.get_bool("WFNestedOcrSearch") == Some(true) {
        let mut total = 0usize;
        let hash = validate_nested_search(reader, search, &mut BTreeSet::new(), 0, &mut total)?;
        (total, hash)
    } else {
        validate_carrier_program(&program)?;
        (program.len(), hash(&program))
    };
    if d.get("WFOcrProgramHash").and_then(PdfObject::as_string) != Some(program_hash.as_bytes()) {
        return Err(fail("OCR group search program changed"));
    }
    let mut visual_bytes = 0usize;
    visual_chain_bytes(reader, visual, &mut BTreeSet::new(), 0, &mut visual_bytes)?;
    let bytes = search_bytes
        .checked_add(visual_bytes)
        .ok_or_else(|| fail("OCR group source byte count overflow"))?;
    Ok((count, text, bytes))
}
