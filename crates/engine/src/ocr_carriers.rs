//! Native relocation of explicitly selected invisible text operands. Original
//! codes, fonts, matrices and spacing are replayed; no OCR or font substitution
//! is performed here. All offsets belong to one immutable source revision.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrCarrierSelection {
    /// Exact IDs from analyze_multi_run_text_range on the image's source page.
    pub span_ids: Vec<String>,
    /// Concatenated source-decoded Unicode in stream order, not a search query.
    /// ActualText scopes are preserved separately and may carry different text.
    pub expected_text: String,
    /// Required when the selected invisible carrier is owned by a nested Form
    /// occurrence. Its path is revision-bound and uses Form-local span IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_target: Option<super::form_text::FormTextTarget>,
}
pub(crate) type Patches = BTreeMap<usize, Vec<(usize, usize, Vec<u8>)>>;
pub(crate) struct OcrCapture {
    pub program: Vec<u8>,
    pub source_edits: Patches,
    pub text: String,
    pub spans: usize,
    /// Source-font-decoded scalar intervals, using the same page-logical model
    /// as initial story frame selections. Offsets belong to the input revision.
    pub logical_ranges: Vec<[usize; 2]>,
    /// Form-local ranges must never be rebased as page-logical story offsets.
    pub form_local: bool,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

pub(crate) fn apply_patches(
    data: &[u8],
    mut patches: Vec<(usize, usize, Vec<u8>)>,
) -> Result<Vec<u8>> {
    patches.sort_by_key(|p| (p.0, p.1));
    let mut size = data.len();
    let mut cursor = 0;
    for (start, end, replacement) in &patches {
        if *start < cursor || start > end || *end > data.len() {
            return Err(fail("OCR carrier patches overlap or leave source bounds"));
        }
        cursor = *end;
        size = size
            .checked_sub(end - start)
            .and_then(|n| n.checked_add(replacement.len()))
            .ok_or_else(|| fail("OCR carrier patch size overflow"))?;
    }
    if size > 64 * 1024 * 1024 {
        return Err(fail("OCR carrier stream budget exceeded"));
    }
    let mut out = Vec::with_capacity(size);
    cursor = 0;
    for (start, end, replacement) in patches {
        crate::cancel::check_current_cancel("OCR carrier source rewrite")?;
        out.extend_from_slice(&data[cursor..start]);
        out.extend(replacement);
        cursor = end;
    }
    out.extend_from_slice(&data[cursor..]);
    Ok(out)
}

pub(crate) fn capture(
    input: &[u8],
    engine: &ContentEngine,
    page: &crate::document::PdfPage,
    buffers: &[Vec<u8>],
    selection: &OcrCarrierSelection,
    approved_figure_owner: bool,
) -> Result<OcrCapture> {
    if selection.form_target.is_some() {
        return Err(fail(
            "Form-local OCR selection cannot be applied to a page content scope",
        ));
    }
    capture_with_state(
        Some(input),
        engine,
        page,
        buffers,
        selection,
        approved_figure_owner,
        ScannedTextTokenState::default(),
        None,
        false,
    )
}

pub(crate) fn capture_form(
    engine: &ContentEngine,
    selection: &OcrCarrierSelection,
    revision: &str,
    approved_figure_owner: bool,
) -> Result<OcrCapture> {
    let target = selection
        .form_target
        .as_ref()
        .ok_or_else(|| fail("nested Form OCR selection is missing its exact Form target"))?;
    let scope = super::form_text::ocr_capture_scope(engine, target, revision)?;
    let reader = engine.document().reader();
    let owner = scope.source_page.contents[0];
    let decoded = decode_stream_lossless_with_limits(
        &reader.get_object(owner.0, owner.1)?,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: (64 * 1024 * 1024) as u64,
            ..Default::default()
        },
    )?;
    if decoded.status != StreamDecodeStatus::Complete {
        return Err(fail("nested Form OCR source is not losslessly decodable"));
    }
    capture_with_state(
        None,
        engine,
        &scope.source_page,
        &[decoded.data],
        selection,
        approved_figure_owner,
        scope.initial,
        Some(&scope.span_prefix),
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn capture_with_state(
    input: Option<&[u8]>,
    engine: &ContentEngine,
    page: &crate::document::PdfPage,
    buffers: &[Vec<u8>],
    selection: &OcrCarrierSelection,
    approved_figure_owner: bool,
    mut state: ScannedTextTokenState,
    span_prefix: Option<&str>,
    form_local: bool,
) -> Result<OcrCapture> {
    if selection.span_ids.is_empty()
        || selection.span_ids.len() > 4096
        || selection.expected_text.len() > 4_000_000
        || buffers.len() != page.contents.len()
    {
        return Err(fail("OCR carrier selection/count budget invalid"));
    }
    let selected = selection
        .span_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let identity_limit = if form_local { 512 } else { 128 };
    if selected.len() != selection.span_ids.len()
        || selected.iter().any(|id| id.len() > identity_limit)
    {
        return Err(fail("duplicate or invalid OCR carrier span identity"));
    }
    // OCR provenance has two deliberately different Unicode views. The
    // selected source bytes are validated against source-font decoding, while
    // story frame offsets live in the page-logical model (and therefore may be
    // owned by /ActualText). Bind both views through the immutable span ID;
    // never use source-decoded scalar counts to rebase a logical story range.
    let page_logical_spans = if form_local {
        None
    } else {
        let input = input.ok_or_else(|| fail("page OCR capture is missing its source revision"))?;
        Some(
            analyze_multi_run_text_range(input, page.page_number)?
                .source_spans
                .into_iter()
                .map(|span| (span.span_id, (span.logical_range, span.source_text)))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut metrics = inline_text::Metrics::new(&resources, reader);
    let mut found = BTreeSet::new();
    let mut text = String::new();
    let mut source_edits = Patches::new();
    let mut carrier_edits = Patches::new();
    let mut scopes = BTreeMap::<(u32, u16, usize, usize), (ActualTextSource, bool, bool)>::new();
    // A destructive Form-local OCR move must retire the old tagged-content
    // identity as well as its glyph operands. Otherwise the cloned source Form
    // still advertises an MCID for content whose structure owner has moved to
    // the destination page. Bind each text-showing operation to its innermost
    // direct MCID value and clear it only when the complete nonempty text scope
    // is selected.
    let mut mcid_scopes = BTreeMap::<(usize, usize, usize), (bool, bool, bool)>::new();
    let mut mcid_nontext_paint = BTreeSet::<(usize, usize, usize)>::new();
    let mut active_mcids = Vec::<Option<(usize, usize, usize)>>::new();
    let mut count = 0usize;
    let mut logical_cursor = 0usize;
    let mut logical_ranges = Vec::new();
    for (index, data) in buffers.iter().enumerate() {
        crate::cancel::check_current_cancel("OCR carrier provenance scan")?;
        let owner = page.contents[index];
        let mut operation_mcids = BTreeMap::<usize, (usize, usize, usize)>::new();
        crate::image_fragments::operations(data, |start, end, operation, _| {
            match operation.operator.as_str() {
                "BMC" => active_mcids.push(None),
                "BDC" => {
                    let lexical = lex_content(&data[start..end])?;
                    let operands = lexical
                        .get(..lexical.len().saturating_sub(1))
                        .ok_or_else(|| fail("invalid OCR marked-content scope"))?;
                    let mcid = marked_property(operands, "MCID")?
                        .map(|value| (index, start + value.start, start + value.end));
                    active_mcids.push(mcid);
                }
                "EMC" => {
                    active_mcids
                        .pop()
                        .ok_or_else(|| fail("OCR marked-content scope underflow"))?;
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    if let Some(scope) = active_mcids.iter().rev().flatten().next().copied() {
                        operation_mcids.insert(start, scope);
                    }
                }
                "Do" | "sh" | "BI" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    if let Some(scope) = active_mcids.iter().rev().flatten().next().copied() {
                        mcid_nontext_paint.insert(scope);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        for token in
            scan_text_string_tokens_with_metrics(data, &mut state, Some(owner), Some(&mut metrics))?
        {
            count += 1;
            if count > 4096 {
                return Err(fail("OCR carrier page operand budget exceeded"));
            }
            let local_id = format!("p{}:s{index}:o{}", page.page_number, token.token_start);
            let id = span_prefix.map_or(local_id.clone(), |prefix| format!("{prefix}:{local_id}"));
            let chosen = selected.contains(id.as_str());
            if let Some(&scope_key) = operation_mcids.get(&token.operation_start) {
                let scope = mcid_scopes.entry(scope_key).or_default();
                scope.0 |= chosen;
                scope.1 |= !chosen && !token.decoded.is_empty();
                scope.2 |= mcid_nontext_paint.contains(&scope_key);
            }
            if chosen
                && (token.text_render_mode != 3
                    || (!token.flow_relocatable && !approved_figure_owner)
                    || token.unresolved_actual_text
                    || token_has_named_actual_text(&token, &resources, reader))
            {
                return Err(fail("OCR carrier must be invisible, page-owned and free of unresolved/shared logical ownership"));
            }
            if matches!(token.text_render_mode, 4..=7) {
                return Err(fail(
                    "OCR native capture cannot suppress text-derived clipping",
                ));
            }
            let font=resources.fonts.get(&token.font_name).ok_or_else(||fail("OCR native capture needs resolvable source font metrics, including preceding text"))?;
            let resolver = FontResolver::new(font, reader);
            let decoded = resolver.decode_string(&token.decoded);
            let end = logical_cursor
                .checked_add(decoded.chars().count())
                .ok_or_else(|| fail("OCR logical range overflow"))?;
            if chosen {
                found.insert(id.clone());
                if text.len().saturating_add(decoded.len()) > 4_000_000 {
                    return Err(fail("OCR carrier Unicode budget exceeded"));
                }
                text.push_str(&decoded);
                if let Some(spans) = &page_logical_spans {
                    let (range, source_text) = spans.get(&id).ok_or_else(|| {
                        fail("OCR carrier disappeared from page-logical analysis")
                    })?;
                    if source_text != &decoded {
                        return Err(fail(
                            "OCR carrier source decoding disagrees with page-logical provenance",
                        ));
                    }
                    // A later operand inside the same /ActualText owner has an
                    // empty logical range: deleting it changes source paint but
                    // does not displace subsequent logical selections.
                    if range[0] != range[1] {
                        logical_ranges.push(*range);
                    }
                } else {
                    logical_ranges.push([logical_cursor, end]);
                }
            }
            logical_cursor = end;
            for source in &token.actual_text_sources {
                let key = (
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end,
                );
                let scope = scopes
                    .entry(key)
                    .or_insert_with(|| (source.clone(), false, false));
                scope.1 |= chosen;
                scope.2 |= !chosen && !token.decoded.is_empty();
            }
            let edit = rewrite_source_text_destructively(
                &token,
                &resolver,
                &[],
                &token.decoded,
                &[],
                false,
                None,
            )?;
            if chosen {
                source_edits.entry(index).or_default().push(edit);
            } else {
                carrier_edits.entry(index).or_default().push(edit);
            }
        }
    }
    if found.len() != selected.len() || text != selection.expected_text || text.is_empty() {
        return Err(fail(
            "OCR carrier selection is stale, empty or disagrees with expected source Unicode",
        ));
    }
    if !active_mcids.is_empty() {
        return Err(fail("unterminated OCR marked-content scope"));
    }
    for ((index, start, end), (chosen, other, nontext_paint)) in mcid_scopes {
        if !chosen {
            continue;
        }
        if other {
            return Err(fail(
                "OCR carrier selection splits an MCID owner; select all its nonempty text operands",
            ));
        }
        if nontext_paint {
            // The marked-content owner still owns visible paint (typically the
            // Figure image itself). Its semantic migration path will retire or
            // rebind the complete wrapper; clearing the MCID here would hide it
            // from that atomic owner transaction.
            continue;
        }
        source_edits
            .entry(index)
            .or_default()
            .push((start, end, b"null".to_vec()));
    }
    let mut retained_scopes = BTreeSet::new();
    for ((number, generation, start, end), (_, chosen, other)) in scopes {
        if !chosen {
            continue;
        }
        if other {
            return Err(fail(
                "OCR carrier selection splits an ActualText owner; select all its glyph operands",
            ));
        }
        let owners = page
            .contents
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == (number, generation))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if owners.len() != 1 {
            return Err(fail("shared/repeated ActualText property stream requires occurrence isolation before OCR capture"));
        }
        source_edits
            .entry(owners[0])
            .or_default()
            .push((start, end, b"null".to_vec()));
        retained_scopes.insert((owners[0], start, end));
    }
    let mut program = Vec::new();
    for (index, data) in buffers.iter().enumerate() {
        let edits = carrier_edits.entry(index).or_default();
        crate::image_fragments::operations(data, |start, end, op, _| {
            let replacement = match op.operator.as_str() {
                "Do" | "sh" | "BI" | "MP" | "DP" => Some(b"\n".to_vec()),
                "S" | "f" | "F" | "f*" | "B" | "B*" => Some(b"n\n".to_vec()),
                "s" | "b" | "b*" => Some(b"h n\n".to_vec()),
                "BMC" | "BDC" => {
                    if op
                        .operands
                        .first()
                        .and_then(crate::content::operation::Operand::as_name)
                        == Some("OC")
                    {
                        return Err(fail(
                            "OCR carrier optional-content state needs explicit group migration",
                        ));
                    }
                    if retained_scopes
                        .range((index, start, 0)..=(index, end, usize::MAX))
                        .any(|(_, _, b)| *b <= end)
                    {
                        // The outer page-level Figure MCID is regenerated by
                        // the tagged story transaction. A nested search Form
                        // must not retain that old page's MCID namespace.
                        let lexical = lex_content(&data[start..end])?;
                        let operands = lexical
                            .get(..lexical.len().saturating_sub(1))
                            .ok_or_else(|| fail("invalid retained OCR property dictionary"))?;
                        if let Some(value) = marked_property(operands, "MCID")? {
                            Some(apply_patches(
                                &data[start..end],
                                vec![(value.start, value.end, b"null".to_vec())],
                            )?)
                        } else {
                            None
                        }
                    } else {
                        Some(b"/Span BMC\n".to_vec())
                    }
                }
                "gs" => {
                    let name = op
                        .operands
                        .first()
                        .and_then(crate::content::operation::Operand::as_name)
                        .ok_or_else(|| fail("invalid OCR graphics state"))?;
                    metrics
                        .ext_font_named(name)
                        .map_err(|_| fail("unresolved OCR ExtGState font selection"))?;
                    // Keep the original gs command and source resource graph.
                    // The shared scanner already resolved its font and size;
                    // no synthesized Tf is needed inside the search capsule.
                    None
                }
                _ => None,
            };
            if let Some(bytes) = replacement {
                edits.push((start, end, bytes));
            }
            Ok(())
        })?;
        let patched = apply_patches(data, carrier_edits.remove(&index).unwrap_or_default())?;
        if program
            .len()
            .saturating_add(patched.len())
            .saturating_add(1)
            > 64 * 1024 * 1024
        {
            return Err(fail("OCR carrier program budget exceeded"));
        }
        program.extend(patched);
        program.push(b'\n');
    }
    crate::image_fragments::validate_carrier_program(&program)?;
    Ok(OcrCapture {
        program,
        source_edits,
        text,
        spans: found.len(),
        logical_ranges,
        form_local,
    })
}
