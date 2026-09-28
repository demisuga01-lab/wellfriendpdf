//! Shared non-painting, zero-advance PDF text for glyphless logical lines.
//!
//! This is new logical whitespace, never retained/deleted source text. A
//! dedicated TrueType space outline supplies real text-showing operands and
//! exact ToUnicode mappings without depending on the user's selected fonts.
use super::*;
use crate::fonts::logical_carrier::{self, encode};

#[cfg(test)]
#[path = "advanced_story_carrier_tests.rs"]
mod tests;

const MARKER: &str = "WFLogicalLineCarrier";
pub(super) const OBJECT_COUNT: u32 = 12; // horizontal and vertical Type0 graphs

pub(crate) fn is_font(dict: &crate::PdfDictionary) -> bool {
    matches!(dict.get(MARKER), Some(PdfObject::Integer(1)))
        || matches!(
            dict.get("WFStoryLogicalCarrier"),
            Some(PdfObject::Integer(1))
        ) && matches!(dict.get("WFStoryOwner"), Some(PdfObject::String(owner))
            if owner.len() == 64 && owner.iter().all(u8::is_ascii_hexdigit))
}

pub(super) fn needs_text_carrier(text: &str) -> bool {
    logical_carrier::is_text(text)
}

pub(super) fn needs_carrier(line: &StoryPaintLine) -> bool {
    needs_text_carrier(&line.text)
}

/// Inline BT/ET writers cannot append another text object. Supply actual
/// zero-advance glyph operands using the private empty space program instead.
pub(super) fn inline_glyphs(
    text: &str,
    face: &ttf_parser::Face<'_>,
) -> Result<Vec<GeneratedGlyph>> {
    if text.len() > 4_000_000 {
        return Err(WellfriendError::ResourceLimit(
            "inline logical carrier text budget".into(),
        ));
    }
    let space = face.glyph_index(' ').ok_or_else(|| {
        WellfriendError::invalid_input("inline logical carrier lacks a space glyph")
    })?;
    if face.tables().glyf.is_none() || space.0 == 0 || face.glyph_bounding_box(space).is_some() {
        return Err(WellfriendError::invalid_input(
            "inline logical carrier requires an empty TrueType outline",
        ));
    }
    let mut glyphs = Vec::new();
    for (offset, ch) in text.char_indices() {
        if glyphs.len() % 1024 == 0 {
            crate::cancel::check_current_cancel("inline logical carrier glyphs")?;
        }
        if glyphs.len() >= MAX_ADVANCED_EDITING_GLYPHS {
            return Err(WellfriendError::ResourceLimit(
                "inline logical carrier glyph budget".into(),
            ));
        }
        let cid = logical_carrier::code(ch).ok_or_else(|| {
            WellfriendError::invalid_input("invalid inline logical carrier scalar")
        })?;
        glyphs.push(GeneratedGlyph {
            cid,
            gid: space.0,
            logical_byte_start: offset,
            visual_unicode: ch.to_string(),
            to_unicode: Some(ch.to_string()),
            advance: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            orientation: VerticalGlyphOrientation::Upright,
            cross_advance: 0.0,
            font_width: 0.0,
            bounds: None,
        });
    }
    Ok(glyphs)
}

#[derive(Default)]
pub(super) struct CarrierFonts {
    names: [Option<String>; 2],
}

impl CarrierFonts {
    pub(super) fn prepare(
        lines: &[StoryPaintLine],
        owner_key: &str,
        base: u32,
        resources: &mut crate::PdfDictionary,
        updates: &mut Vec<IncrementalObject>,
    ) -> Result<Self> {
        let mut modes = [false; 2];
        for line in lines {
            crate::cancel::check_current_cancel("story logical carrier planning")?;
            if needs_carrier(line) {
                modes[usize::from(line.writing_mode.is_vertical())] = true;
            }
        }
        let alphabet = logical_carrier::alphabet(
            lines
                .iter()
                .filter(|line| needs_carrier(line))
                .map(|line| line.text.as_str()),
        )?;
        Self::prepare_modes(modes, &alphabet, Some(owner_key), base, resources, updates)
    }

    fn prepare_modes(
        modes: [bool; 2],
        alphabet: &BTreeSet<char>,
        owner_key: Option<&str>,
        base: u32,
        resources: &mut crate::PdfDictionary,
        updates: &mut Vec<IncrementalObject>,
    ) -> Result<Self> {
        let mut result = Self::default();
        if !modes.iter().any(|needed| *needed) {
            return Ok(result);
        }
        base.checked_add(OBJECT_COUNT - 1)
            .ok_or_else(|| WellfriendError::ResourceLimit("logical carrier object range".into()))?;
        let font = get_fallback_font("Symbol").ok_or_else(|| {
            WellfriendError::UnsupportedFeature("bundled logical carrier font unavailable".into())
        })?;
        let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
            WellfriendError::MalformedPdf("bundled logical carrier font is not sfnt".into())
        })?;
        let space = face.glyph_index(' ').ok_or_else(|| {
            WellfriendError::MalformedPdf("bundled logical carrier font lacks a space glyph".into())
        })?;
        // Do not rely on Tr=3: ordinary whitespace must remain available when
        // callers exclude OCR/invisible layers. Only an empty glyf outline is
        // permitted, with zero PDF advance in both writing modes.
        if face.tables().glyf.is_none() || space.0 == 0 || face.glyph_bounding_box(space).is_some()
        {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier requires a non-notdef, empty TrueType space outline".into(),
            ));
        }
        let glyphs = alphabet
            .iter()
            .map(|ch| GeneratedGlyph {
                cid: logical_carrier::code(*ch).expect("validated carrier alphabet"),
                gid: space.0,
                logical_byte_start: 0,
                visual_unicode: ch.to_string(),
                to_unicode: Some(ch.to_string()),
                advance: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                orientation: VerticalGlyphOrientation::Upright,
                cross_advance: 0.0,
                font_width: 0.0,
                bounds: None,
            })
            .collect::<Vec<_>>();
        for (mode, needed) in modes.into_iter().enumerate() {
            if !needed {
                continue;
            }
            crate::cancel::check_current_cancel("story logical carrier font")?;
            let number = base.checked_add(mode as u32 * 6).ok_or_else(|| {
                WellfriendError::ResourceLimit("logical carrier object range".into())
            })?;
            let name = format!("WFStoryLogical{number}");
            if resources.contains_key(&name) {
                return Err(WellfriendError::MalformedPdf(
                    "logical carrier font resource collision".into(),
                ));
            }
            let mut objects = build_type0_font_objects(
                font,
                &glyphs,
                mode == 1,
                number,
                number + 1,
                number + 2,
                number + 3,
                number + 4,
                number + 5,
            )?;
            let dict = objects
                .iter_mut()
                .find(|o| o.number == number + 5)
                .and_then(|o| o.object.as_dict_mut())
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf("logical carrier Type0 object missing".into())
                })?;
            dict.insert(MARKER, PdfObject::Integer(1));
            if let Some(owner_key) = owner_key {
                dict.insert(
                    "WFStoryOwner",
                    PdfObject::String(owner_key.as_bytes().to_vec()),
                );
            }
            updates.extend(objects);
            resources.insert(
                name.clone(),
                PdfObject::Reference {
                    number: number + 5,
                    generation: 0,
                },
            );
            result.names[mode] = Some(name);
        }
        Ok(result)
    }

    /// Called inside the line's existing ActualText/paragraph/artifact scope.
    pub(super) fn append_line(&self, output: &mut String, line: &StoryPaintLine) -> Result<()> {
        self.append_text(
            output,
            &line.text,
            [line.x, line.baseline],
            line.font_size,
            line.writing_mode.is_vertical(),
        )
    }

    fn append_text(
        &self,
        output: &mut String,
        text: &str,
        position: [f64; 2],
        size: f64,
        vertical: bool,
    ) -> Result<()> {
        let codes = encode(text)?;
        if codes.is_empty() {
            return Ok(());
        }
        if ![position[0], position[1], size]
            .iter()
            .all(|v| v.is_finite())
            || size <= 0.0
        {
            return Err(WellfriendError::invalid_input(
                "invalid logical carrier position or font size",
            ));
        }
        let name = self.names[usize::from(vertical)].as_ref().ok_or_else(|| {
            WellfriendError::MalformedPdf("logical carrier font was not prepared".into())
        })?;
        output.push_str(&format!(
            "BT\n0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n/{} {} Tf\n1 0 0 1 {} {} Tm\n<{}> Tj\nET\n",
            serialized_name_body(name),
            fmt_num(size),
            fmt_num(position[0]),
            fmt_num(position[1]),
            codes,
        ));
        Ok(())
    }
}

/// Legacy generated writers already own their source paint slot. Add logical
/// whitespace only when the complete replacement has no visible characters.
/// The caller reserves OBJECT_COUNT objects starting at `base`, installs the
/// returned resources, and supplies the same source-order ownership decision it
/// uses for ordinary generated glyphs. Insertion artifacts must not acquire a
/// nested nonempty ActualText scope, which could duplicate the source carrier.
pub(super) fn attach_generated(
    mut content: String,
    text: &str,
    vertical: bool,
    options: &AdvancedTextEditOptions,
    base: u32,
    updates: &mut Vec<IncrementalObject>,
    owns_logical_text: bool,
) -> Result<(String, crate::PdfDictionary)> {
    let mut resources = crate::PdfDictionary::empty();
    if !needs_text_carrier(text) {
        return Ok((content, resources));
    }
    let mut modes = [false; 2];
    modes[usize::from(vertical)] = true;
    let alphabet = logical_carrier::alphabet([text])?;
    let fonts = CarrierFonts::prepare_modes(modes, &alphabet, None, base, &mut resources, updates)?;
    content.push_str("\nq\n");
    fonts.append_text(
        &mut content,
        text,
        [options.region[0], options.region[3] - options.font_size],
        options.font_size,
        vertical,
    )?;
    content.push_str("Q\n");
    if owns_logical_text {
        content = wrap_generated_visual_with_actual_text(content, text);
    }
    Ok((content, resources))
}

pub(super) fn install(
    resources: &crate::PdfDictionary,
    fonts: &mut crate::PdfDictionary,
) -> Result<()> {
    for (name, value) in resources.iter() {
        if fonts.contains_key(name) {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier font resource collision".into(),
            ));
        }
        fonts.insert(name.clone(), value.clone());
    }
    Ok(())
}

/// Reopened source-code verification for non-story writers. Resource names are
/// newly allocated by this transaction, and traversal is restricted to its
/// rebound page/Form/appearance program, not a whole-document text search.
pub(super) fn verify_generated(
    engine: &ContentEngine,
    page: &crate::document::PdfPage,
    generated_fonts: &crate::PdfDictionary,
    text: &str,
    vertical: bool,
    owns_logical_text: bool,
) -> Result<()> {
    if generated_fonts.is_empty() {
        return Ok(());
    }
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut resolvers = BTreeMap::new();
    let alphabet = logical_carrier::alphabet([text])?;
    for (name, _) in generated_fonts.iter() {
        crate::cancel::check_current_cancel("generated carrier mappings")?;
        let dict = resources.fonts.get(name).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "generated logical carrier font missing after save".into(),
            )
        })?;
        if !is_font(dict) {
            return Err(WellfriendError::MalformedPdf(
                "generated logical carrier identity changed".into(),
            ));
        }
        let resolver = checked_resolver(dict, reader, &alphabet)?;
        if resolver.is_vertical() != vertical {
            return Err(WellfriendError::MalformedPdf(
                "generated logical carrier writing mode changed".into(),
            ));
        }
        resolvers.insert(name.clone(), resolver);
    }
    let mut count = 0usize;
    let mut state = ScannedTextTokenState::default();
    for &(number, generation) in &page.contents {
        crate::cancel::check_current_cancel("generated logical carrier verification")?;
        let object = reader.get_object(number, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::MalformedPdf(
                "incomplete generated logical carrier stream".into(),
            ));
        }
        for token in scan_text_string_tokens_with_metrics(
            &decoded.data,
            &mut state,
            Some((number, generation)),
            None,
        )? {
            let Some(resolver) = resolvers.get(&token.font_name) else {
                continue;
            };
            count += 1;
            let logical_owner = token
                .actual_text_sources
                .first()
                .map(|s| s.logical_text.as_ref());
            if resolver.decode_string(&token.decoded) != text
                || token.text_render_mode != 0
                || token.character_spacing != 0.0
                || token.word_spacing != 0.0
                || token.horizontal_scaling != 100.0
                || token.text_rise != 0.0
                || logical_owner != Some(if owns_logical_text { text } else { "" })
            {
                return Err(WellfriendError::MalformedPdf(
                    "generated logical carrier source or ownership changed".into(),
                ));
            }
        }
    }
    if count != 1 {
        return Err(WellfriendError::MalformedPdf(
            "generated logical carrier is missing or duplicated".into(),
        ));
    }
    Ok(())
}

fn checked_resolver(
    dict: &crate::PdfDictionary,
    reader: &crate::PdfReader,
    alphabet: &BTreeSet<char>,
) -> Result<FontResolver> {
    let resolver = FontResolver::new(dict, reader);
    for separator in alphabet {
        crate::cancel::check_current_cancel("logical carrier mapping verification")?;
        let code = logical_carrier::code(*separator)
            .ok_or_else(|| WellfriendError::invalid_input("unsupported logical carrier scalar"))?;
        let (decoded, provenance) = resolver.decode_char_with_source(code);
        if decoded != separator.to_string()
            || provenance != crate::fonts::resolver::FontDecodeSource::ToUnicode
            || resolver.glyph_width(code) != 0.0
            || resolver.is_vertical() && resolver.vertical_metrics(code) != (0.0, 0.0, 0.0)
        {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier font mapping or advance changed".into(),
            ));
        }
    }
    Ok(resolver)
}

/// Validate output content, not saved request metadata or a text match elsewhere
/// on the page. Owner-local collection retains the exact whitespace and WMode;
/// geometric plain-text formatting is deliberately not the oracle here.
pub(super) fn verify(
    engine: &ContentEngine,
    page_number: usize,
    owner_key: &str,
    lines: &[StoryPaintLine],
) -> Result<()> {
    let expected = lines
        .iter()
        .filter(|line| needs_carrier(line))
        .collect::<Vec<_>>();
    if expected.is_empty() {
        return Ok(());
    }
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let owner = locate_story_frame(reader, &page.contents, owner_key)?.ok_or_else(|| {
        WellfriendError::MalformedPdf("logical carrier frame owner missing after save".into())
    })?;
    let object = reader.get_object(owner.stream.0, owner.stream.1)?;
    let decoded = decode_stream_lossless_with_limits(
        &object,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::MalformedPdf(
            "incomplete logical carrier stream".into(),
        ));
    }
    let cancel = crate::cancel::current_cancel_token();
    let owner_bytes = decoded.data.get(owner.range).ok_or_else(|| {
        WellfriendError::MalformedPdf("logical carrier owner range outside stream".into())
    })?;
    let operations =
        crate::content::parser::ContentParser::parse_cancellable(owner_bytes, &cancel)?;
    let resources = PageResources::from_dict(&page.resources, reader);
    let carrier_names = resources
        .fonts
        .iter()
        .filter(|(_, dict)| {
            is_font(dict)
                && matches!(dict.get("WFStoryOwner"),
            Some(PdfObject::String(key)) if key == owner_key.as_bytes())
        })
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    // ActualText alone could hide a damaged ToUnicode map from an extractor,
    // while the source-selection model still decodes the wrong characters.
    let alphabet = logical_carrier::alphabet(expected.iter().map(|line| line.text.as_str()))?;
    let mut resolvers = BTreeMap::new();
    for name in &carrier_names {
        cancel.check("logical carrier mappings")?;
        resolvers.insert(
            name.clone(),
            checked_resolver(&resources.fonts[name], reader, &alphabet)?,
        );
    }
    // Correct maps alone are insufficient: ActualText can conceal a wrong code
    // sequence using another valid mapping. Bind the exact owner-local operands
    // and zero-advance state before considering the extractor's logical view.
    let tokens = scan_text_string_tokens_with_metrics(
        owner_bytes,
        &mut ScannedTextTokenState::default(),
        Some(owner.stream),
        None,
    )?;
    let mut source_carriers = tokens
        .iter()
        .filter(|token| carrier_names.contains(&token.font_name));
    for line in &expected {
        cancel.check("logical carrier source verification")?;
        let token = source_carriers.next().ok_or_else(|| {
            WellfriendError::MalformedPdf("logical carrier source operand missing".into())
        })?;
        let resolver = &resolvers[&token.font_name];
        if resolver.decode_string(&token.decoded) != line.text
            || resolver.is_vertical() != line.writing_mode.is_vertical()
            || token.text_render_mode != 0
            || token.character_spacing != 0.0
            || token.word_spacing != 0.0
            || token.horizontal_scaling != 100.0
            || token.text_rise != 0.0
            || token
                .actual_text_sources
                .first()
                .map(|source| source.logical_text.as_ref())
                != Some(line.text.as_str())
        {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier source/state does not match its line".into(),
            ));
        }
    }
    if source_carriers.next().is_some() {
        return Err(WellfriendError::MalformedPdf(
            "duplicate logical carrier source operand".into(),
        ));
    }
    let mut collector = crate::text::TextCollector::new(resources, reader);
    let chunks = collector.collect_scoped(
        &operations,
        &crate::text::TextTraversalLimits::default(),
        &cancel,
    )?;
    let mut carriers = chunks
        .iter()
        .filter(|c| carrier_names.contains(&c.chunk.font_name));
    for line in expected {
        cancel.check("logical carrier output validation")?;
        let Some(chunk) = carriers.next().map(|c| &c.chunk) else {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier omitted during save/reopen".into(),
            ));
        };
        if chunk.text != line.text
            || !chunk.is_actual_text
            || chunk.is_invisible
            || chunk.is_vertical != line.writing_mode.is_vertical()
            || chunk.width != 0.0
        {
            return Err(WellfriendError::MalformedPdf(
                "logical carrier output does not match its line".into(),
            ));
        }
    }
    if carriers.next().is_some() {
        return Err(WellfriendError::MalformedPdf(
            "unexpected duplicate logical carrier".into(),
        ));
    }
    Ok(())
}
