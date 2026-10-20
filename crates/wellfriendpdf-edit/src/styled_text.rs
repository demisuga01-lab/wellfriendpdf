//! Source text rewriting with explicitly selected, embedded fonts.
//!
//! CID/GID/ToUnicode construction adapted from Wellfriend PDF SDK
//! c9fc1eafeb0910340d2519c3500e5a023b4bde79, advanced_editing.rs,
//! build_type0_font_objects/build_to_unicode_cmap. See LICENSE-wellfriendpdf-sdk.txt.
//! This uses the existing canonical writer and source selectors, not an overlay.
use crate::*;
use lopdf::{Stream, dictionary};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextStyle {
    pub font: String,
    pub font_size: f64,
    pub color: String,
}

// Text and graphics state can be inherited across BT/ET and content streams.
// Until that state is fully modelled, do not certify geometry for these pages.
pub(crate) fn page_style_eligible(document: &Document, page: ObjectId) -> bool {
    document.get_page_contents(page).into_iter().all(|id| {
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else {
            return false;
        };
        let Ok(bytes) = stream.get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES) else {
            return false;
        };
        let Ok(content) = Content::decode(&bytes) else {
            return false;
        };
        content.operations.len() <= MAX_CONTENT_OPERATIONS_PER_STREAM
            && content.operations.iter().all(|op| {
                !matches!(
                    op.operator.as_str(),
                    "cm" | "Tc" | "Tw" | "Tz" | "Ts" | "Tr" | "\""
                )
            })
    })
}

/// The Liberation fonts bundled with the editor, by PostScript-style name.
pub(crate) const BUNDLED_FONTS: &[&str] = &[
    "LiberationSans-Regular",
    "LiberationSans-Bold",
    "LiberationSans-Italic",
    "LiberationSans-BoldItalic",
    "LiberationSerif-Regular",
    "LiberationSerif-Bold",
    "LiberationSerif-Italic",
    "LiberationSerif-BoldItalic",
    "LiberationMono-Regular",
    "LiberationMono-Bold",
    "LiberationMono-Italic",
    "LiberationMono-BoldItalic",
];

pub(crate) fn font_data(name: &str) -> Result<&'static [u8], PdfEditError> {
    macro_rules! font {
        ($file:literal) => {
            include_bytes!(concat!("../fonts/", $file, ".ttf"))
        };
    }
    Ok(match name {
        "LiberationSans-Regular" => font!("LiberationSans-Regular"),
        "LiberationSans-Bold" => font!("LiberationSans-Bold"),
        "LiberationSans-Italic" => font!("LiberationSans-Italic"),
        "LiberationSans-BoldItalic" => font!("LiberationSans-BoldItalic"),
        "LiberationSerif-Regular" => font!("LiberationSerif-Regular"),
        "LiberationSerif-Bold" => font!("LiberationSerif-Bold"),
        "LiberationSerif-Italic" => font!("LiberationSerif-Italic"),
        "LiberationSerif-BoldItalic" => font!("LiberationSerif-BoldItalic"),
        "LiberationMono-Regular" => font!("LiberationMono-Regular"),
        "LiberationMono-Bold" => font!("LiberationMono-Bold"),
        "LiberationMono-Italic" => font!("LiberationMono-Italic"),
        "LiberationMono-BoldItalic" => font!("LiberationMono-BoldItalic"),
        _ => return Err(PdfEditError::TextRunUnsupported),
    })
}

fn rgb(value: &str) -> Result<Vec<Object>, PdfEditError> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PdfEditError::TextRunUnsupported);
    }
    (1..7)
        .step_by(2)
        .map(|i| {
            let byte = u8::from_str_radix(&value[i..i + 2], 16)
                .map_err(|_| PdfEditError::TextRunUnsupported)?;
            Ok(Object::Real(f32::from(byte) / 255.0))
        })
        .collect()
}

fn add_stream(
    document: &mut Document,
    dictionary: Dictionary,
    bytes: Vec<u8>,
) -> Result<ObjectId, PdfEditError> {
    let mut stream = Stream::new(dictionary, bytes);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    Ok(document.add_object(stream))
}

// PDF serialization can turn integral Real values into Integer values. Only
// newly constructed font objects use numeric semantic comparison; existing
// non-target source objects retain the stricter existing preservation check.
fn generated_equal(a: &Object, b: &Object) -> bool {
    if let (Some(a), Some(b)) = (pdf_number(a), pdf_number(b)) {
        return nearly_equal(a, b);
    }
    match (a, b) {
        (Object::Array(a), Object::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| generated_equal(a, b))
        }
        (Object::Dictionary(a), Object::Dictionary(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_ok_and(|b| generated_equal(a, b)))
        }
        (Object::Stream(a), Object::Stream(b)) => {
            a.content == b.content
                && generated_equal(
                    &Object::Dictionary(a.dict.clone()),
                    &Object::Dictionary(b.dict.clone()),
                )
        }
        _ => a == b,
    }
}

/// Font choice is explicit; never download fonts or reuse a restricted customer font.
fn embed_font(
    document: &mut Document,
    style: &TextStyle,
    text: &str,
) -> Result<(ObjectId, Vec<u8>, f64), PdfEditError> {
    let bytes = font_data(&style.font)?;
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|_| PdfEditError::GlyphUnavailable)?;
    let upem = f64::from(face.units_per_em());
    let unit = |v: i16| Object::Real((f64::from(v) * 1000.0 / upem) as f32);
    let mut glyph_map = vec![0u8, 0];
    let mut encoded = Vec::new();
    let mut widths = Vec::new();
    let mut mappings = Vec::new();
    let mut advance = 0.0;
    for (index, character) in text.chars().enumerate() {
        // This initial font bridge is horizontal Latin/Greek/Cyrillic. Complex
        // shaping must use the SDK shaper before it can be advertised or saved.
        if character.is_control()
            || matches!(character as u32, 0x300..=0x36f | 0x590..=0x10ff | 0x200b..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069)
        {
            return Err(PdfEditError::GlyphUnavailable);
        }
        let glyph = face
            .glyph_index(character)
            .filter(|id| id.0 != 0)
            .ok_or(PdfEditError::GlyphUnavailable)?;
        let width = f64::from(
            face.glyph_hor_advance(glyph)
                .ok_or(PdfEditError::GlyphUnavailable)?,
        ) * 1000.0
            / upem;
        advance += width;
        widths.push(Object::Real(width as f32));
        glyph_map.extend_from_slice(&glyph.0.to_be_bytes());
        let cid = u16::try_from(index + 1).map_err(|_| PdfEditError::TextRunUnsupported)?;
        encoded.extend_from_slice(&cid.to_be_bytes());
        let unicode = character
            .encode_utf16(&mut [0; 2])
            .iter()
            .map(|v| format!("{v:04X}"))
            .collect::<String>();
        mappings.push(format!("<{cid:04X}> <{unicode}>\n"));
    }
    let bbox = face.global_bounding_box();
    let file = add_stream(
        document,
        dictionary! { "Length1" => bytes.len() as i64 },
        bytes.to_vec(),
    )?;
    let descriptor = document.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => style.font.as_str(),
        "Flags" => if style.font.contains("Mono") { 33 } else { 32 },
        "FontBBox" => vec![unit(bbox.x_min), unit(bbox.y_min), unit(bbox.x_max), unit(bbox.y_max)],
        "ItalicAngle" => face.italic_angle(), "Ascent" => unit(face.ascender()),
        "Descent" => unit(face.descender()), "CapHeight" => unit(face.capital_height().unwrap_or(face.ascender())),
        "StemV" => 80, "FontFile2" => file,
    });
    let map = add_stream(document, Dictionary::new(), glyph_map)?;
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /WellfriendStyledText def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!(
            "{} beginbfchar\n{}endbfchar\n",
            chunk.len(),
            chunk.concat()
        ));
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let unicode = add_stream(document, Dictionary::new(), cmap.into_bytes())?;
    let descendant = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => style.font.as_str(),
        "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
        "FontDescriptor" => descriptor, "DW" => 1000,
        "W" => vec![Object::Integer(1), Object::Array(widths)], "CIDToGIDMap" => map,
    });
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => style.font.as_str(),
        "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(descendant)], "ToUnicode" => unicode,
    });
    Ok((font, encoded, advance * style.font_size / 1000.0))
}

pub fn replace_styled_text(
    request: &ReplaceTextRequest,
    style: &TextStyle,
) -> Result<serde_json::Value, PdfEditError> {
    if !style.font_size.is_finite()
        || !(1.0..=512.0).contains(&style.font_size)
        || request.replacement_text.len() > MAX_EDITABLE_TEXT_RUN_BYTES
        || request.replacement_text.chars().count() > MAX_EDITABLE_TEXT_RUN_CHARS
    {
        return Err(PdfEditError::TextRunUnsupported);
    }
    let color = rgb(&style.color)?;
    validate_paths(&request.input_path, &request.output_path)?;
    let metadata = checked_metadata(&request.input_path)?;
    let input_hash = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let signatures = signature_summaries(&document);
    if !signatures.is_empty() && !request.allow_signature_invalidation {
        return Err(PdfEditError::SignatureConfirmationRequired);
    }
    let source_objects = document.objects.clone();
    let pages = document.get_pages();
    let page = page_id(&document, request.selector.page_number)?;
    if !page_style_eligible(&document, page) {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let target: ObjectId = request.selector.content_stream_object.into();
    if !document.get_page_contents(page).contains(&target) {
        return Err(PdfEditError::ContentStreamNotOnPage);
    }
    if pages
        .values()
        .filter(|id| document.get_page_contents(**id).contains(&target))
        .count()
        != 1
    {
        return Err(PdfEditError::TextRunUnsupported);
    }
    // Authenticate source identity with its original encoding before font substitution.
    let (_, old_bytes, _) = replacement_bytes_for_target(
        &document,
        page,
        target,
        &request.selector,
        &request.selector.expected_text,
    )?;
    let plain = document
        .get_object(target)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let mut content =
        Content::decode(&plain).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let index = request.selector.operation_index;
    if !isolated_reflow_operation(&content, index) {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let matrix = text_matrix(&content.operations[index - 1].operands)
        .filter(horizontal_unit_text_matrix)
        .ok_or(PdfEditError::ReflowUnsafe)?;
    let begin = content.operations[..index]
        .iter()
        .rposition(|op| op.operator == "BT")
        .ok_or(PdfEditError::ReflowUnsafe)?;
    let end = index
        + 1
        + content.operations[index + 1..]
            .iter()
            .position(|op| op.operator == "ET")
            .ok_or(PdfEditError::ReflowUnsafe)?;
    let before = content.operations.clone();
    // Original Tf/TL/colour operations affect subsequent content, even outside
    // this BT/ET. Replaying them after Q preserves that original inherited state.
    let restore_state: Vec<_> = before[begin + 1..index]
        .iter()
        .filter(|op| {
            matches!(
                op.operator.as_str(),
                "Tf" | "TL" | "g" | "rg" | "k" | "G" | "RG" | "K"
            )
        })
        .cloned()
        .collect();
    let (font, encoded, width) = embed_font(&mut document, style, &request.replacement_text)?;
    let geometry = inspect_page_geometries(&document)?
        .0
        .into_iter()
        .find(|p| p.selector.page_number == request.selector.page_number)
        .ok_or(PdfEditError::ReflowUnsafe)?;
    let face = ttf_parser::Face::parse(font_data(&style.font)?, 0)
        .map_err(|_| PdfEditError::GlyphUnavailable)?;
    let factor = style.font_size / f64::from(face.units_per_em());
    let bounds = PdfBox {
        left: matrix.e,
        right: matrix.e + width,
        bottom: matrix.f + f64::from(face.descender()) * factor,
        top: matrix.f + f64::from(face.ascender()) * factor,
    };
    if !pdf_box_contains(&geometry.crop_box, &bounds) {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let resource_name = format!("WFEdit{}", font.0);
    let mut resources = page_effective_resources_clone(&document, page)?;
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        Ok(Object::Reference(id)) => document
            .get_object(*id)
            .and_then(Object::as_dict)
            .map_err(|_| PdfEditError::TextRunUnsupported)?
            .clone(),
        _ => return Err(PdfEditError::TextRunUnsupported),
    };
    if fonts.has(resource_name.as_bytes()) {
        return Err(PdfEditError::ValidationFailed);
    }
    fonts.set(resource_name.as_str(), font);
    resources.set("Font", fonts);
    document
        .get_object_mut(page)
        .and_then(Object::as_dict_mut)
        .map_err(|_| PdfEditError::ValidationFailed)?
        .set("Resources", resources);
    let font_index = (begin + 1..index)
        .rev()
        .find(|i| content.operations[*i].operator == "Tf")
        .ok_or(PdfEditError::ReflowUnsafe)?;
    content.operations[font_index].operands = vec![
        Object::Name(resource_name.as_bytes().to_vec()),
        Object::Real(style.font_size as f32),
    ];
    content.operations[index].operands =
        vec![Object::String(encoded.clone(), StringFormat::Hexadecimal)];
    // Place colour at the selected show operation, after any original colour.
    content
        .operations
        .insert(index - 1, Operation::new("rg", color));
    content
        .operations
        .insert(end + 2, Operation::new("Q", vec![]));
    content
        .operations
        .insert(begin, Operation::new("q", vec![]));
    content
        .operations
        .splice(end + 4..end + 4, restore_state.iter().cloned());
    let output_index = index + 2;
    // No unrelated operators change. q/Q confines the selected text's font/colour.
    let mut preserved = content.operations.clone();
    preserved.drain(end + 4..end + 4 + restore_state.len());
    preserved.remove(begin);
    preserved.remove(end + 2);
    preserved.remove(index - 1);
    preserved[font_index] = before[font_index].clone();
    preserved[index] = before[index].clone();
    if preserved.len() != before.len()
        || !preserved
            .iter()
            .zip(&before)
            .all(|(a, b)| a.operator == b.operator && a.operands == b.operands)
    {
        eprintln!("PDF_STYLE_VALIDATION_FAILED operators");
        return Err(PdfEditError::ValidationFailed);
    }
    let stream = document
        .get_object_mut(target)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    stream.set_plain_content(
        content
            .encode()
            .map_err(|_| PdfEditError::ValidationFailed)?,
    );
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let changed = changed_object_ids(&source_objects, &document.objects);
    if changed.len() != 8 || !changed.contains(&page) || !changed.contains(&target) {
        eprintln!(
            "PDF_STYLE_VALIDATION_FAILED changed_objects={}",
            changed.len()
        );
        return Err(PdfEditError::ValidationFailed);
    }
    let expected_objects = document.objects.clone();
    save_new_pdf(&mut document, &request.output_path)?;
    let validated = (|| {
        let output = load_checked_pdf(&request.output_path)?;
        let mut selector = request.selector.clone();
        selector.operation_index = output_index;
        let operation = target_operation(&output, page, target, &selector)?;
        let text_matches =
            decoded_tj_text(&output, page, &operation, &resource_name)? == request.replacement_text;
        let graph_matches = expected_objects.iter().all(|(id, object)| {
            is_writer_housekeeping_object(object)
                || output.objects.get(id).is_some_and(|value| {
                    if source_objects.contains_key(id) {
                        pdf_objects_semantically_equal(object, value)
                    } else {
                        generated_equal(object, value)
                    }
                })
        });
        if !text_matches
            || !graph_matches
            || output.get_pages() != pages
            || semantic_object_count(&output.objects) != semantic_object_count(&source_objects) + 6
        {
            eprintln!(
                "PDF_STYLE_VALIDATION_FAILED text={text_matches} graph={graph_matches} pages={} objects={}",
                output.get_pages() == pages,
                semantic_object_count(&output.objects)
                    == semantic_object_count(&source_objects) + 6
            );
            return Err(PdfEditError::ValidationFailed);
        }
        Ok(serde_json::json!({
            "operation": "REPLACE_TEXT", "capability": "PDF_TEXT_REPLACE", "textStyle": style,
            "inputSha256": input_hash, "outputSha256": sha256_file(&request.output_path)?,
            "inputBytes": metadata.len(), "outputBytes": checked_metadata(&request.output_path)?.len(),
            "pageCount": pages.len(), "sourceObjectCount": source_objects.len(), "outputObjectCount": output.objects.len(),
            "changedObjects": changed.into_iter().map(PdfObjectId::from).collect::<Vec<_>>(),
            "fontResource": resource_name, "sourceText": request.selector.expected_text, "replacementText": request.replacement_text,
            "sourceEncodedBytes": old_bytes.len(), "replacementEncodedBytes": encoded.len(), "saveMode": "FULL_REWRITE",
            "signatureEffect": if signatures.is_empty() { "NONE" } else { "INVALIDATES_EXISTING_SIGNATURES" },
            "validation": { "reparsed": true, "pageCountPreserved": true, "sourceObjectsPreserved": true,
                "addedFontObjects": 6, "nonTargetOperatorsPreserved": true, "replacementExtractable": true,
                "fontEmbedded": true, "styleIsolated": true, "regionConfined": true, "noPageRasterization": true }
        }))
    })();
    if validated.is_err() {
        let _ = fs::remove_file(&request.output_path);
    }
    validated
}
