//! Exact source text positions for inline edits. Uses the canonical lexical
//! scanner; never interprets binary image data or invents glyph widths.
use super::*;
use crate::content::state::{concat_matrix, translate_matrix, Matrix, IDENTITY_MATRIX};

#[cfg(test)]
#[path = "advanced_inline_text_tests.rs"]
mod tests;

pub(super) struct Metrics<'a> {
    resources: &'a PageResources,
    reader: &'a crate::PdfReader,
    fonts: BTreeMap<String, FontResolver>,
}
impl<'a> Metrics<'a> {
    pub(super) fn new(resources: &'a PageResources, reader: &'a crate::PdfReader) -> Self {
        Self {
            resources,
            reader,
            fonts: BTreeMap::new(),
        }
    }
    fn font(&mut self, name: &str) -> Option<&FontResolver> {
        if !self.fonts.contains_key(name) {
            let dictionary = self.resources.fonts.get(name)?;
            self.fonts
                .insert(name.to_owned(), FontResolver::new(dictionary, self.reader));
        }
        self.fonts
            .get(name)
            .filter(|font| font.validate_encoding().is_ok())
    }

    /// None means gs leaves the font alone; Err means its selection cannot be
    /// known. A valid gs Font can restore knowledge after an earlier unknown Tf.
    pub(super) fn ext_font(
        &self,
        operands: &[LexicalToken],
    ) -> std::result::Result<Option<(String, f64)>, ()> {
        let [LexicalToken {
            kind: LexicalKind::Name(name),
            ..
        }] = operands
        else {
            return Err(());
        };
        self.ext_font_named(name)
    }

    pub(super) fn ext_font_named(
        &self,
        name: &str,
    ) -> std::result::Result<Option<(String, f64)>, ()> {
        let dictionary = self.resources.ext_g_states.get(name).ok_or(())?;
        let Some(value) = dictionary.get("Font") else {
            return Ok(None);
        };
        let Some([font, size]) = value.as_array() else {
            return Err(());
        };
        let name = font.as_name().ok_or(())?;
        let size = size.as_number().filter(|v| v.is_finite()).ok_or(())?;
        if !self.resources.fonts.contains_key(name) {
            return Err(());
        }
        Ok(Some((name.to_owned(), size)))
    }
}

pub(super) struct Parameters<'a> {
    pub font_name: &'a str,
    pub font_size: f64,
    pub horizontal_scaling: f64,
}

#[derive(Debug, Clone)]
pub(super) enum SourceBasis {
    Absent,
    Verified(Matrix),
    Invalid,
}
#[derive(Debug, Clone)]
enum BasisDeclaration {
    Absent,
    Declared { basis: Matrix, painted: Matrix },
    Invalid,
}
fn basis_declaration(operands: &[LexicalToken]) -> Result<BasisDeclaration> {
    let Some(value) = marked_property(operands, "WFTextBasisV1")? else {
        return Ok(BasisDeclaration::Absent);
    };
    if matches!(&value.kind, LexicalKind::Null) {
        return Ok(BasisDeclaration::Absent);
    }
    let Some(index) = operands.iter().position(|t| t.start == value.start) else {
        return Ok(BasisDeclaration::Invalid);
    };
    if !matches!(&value.kind, LexicalKind::ArrayStart)
        || !matches!(
            operands.get(index + 13).map(|t| &t.kind),
            Some(LexicalKind::ArrayEnd)
        )
    {
        return Ok(BasisDeclaration::Invalid);
    }
    let Some(values) = operands.get(index + 1..index + 13).and_then(numbers::<12>) else {
        return Ok(BasisDeclaration::Invalid);
    };
    Ok(BasisDeclaration::Declared {
        basis: std::array::from_fn(|i| values[i]),
        painted: std::array::from_fn(|i| values[i + 6]),
    })
}
fn same_matrix(a: Matrix, b: Matrix) -> bool {
    a.into_iter()
        .zip(b)
        .all(|(a, b)| (a - b).abs() <= 1e-10 * a.abs().max(b.abs()).max(1.0))
}

/// Tm is Tlm translated by `offset` in text space. Keep that displacement
/// explicitly: recovering it by inverting Tlm loses singular/near-singular
/// source matrices and cannot reliably restore subsequent Td/TD/T* semantics.
#[derive(Debug, Clone)]
pub(super) struct PositionSnapshot {
    line: Matrix,
    offset: [f64; 2],
    horizontal_font: Option<String>,
    vertical_font: Option<String>,
}
impl PositionSnapshot {
    fn new(line: Matrix) -> Self {
        Self {
            line,
            offset: [0.0; 2],
            horizontal_font: None,
            vertical_font: None,
        }
    }
    pub(super) fn matrix(&self) -> Matrix {
        concat_matrix(
            &translate_matrix(self.offset[0], self.offset[1]),
            &self.line,
        )
    }
    fn advance(&mut self, displacement: [f64; 2]) -> bool {
        self.offset[0] += displacement[0];
        self.offset[1] += displacement[1];
        self.offset
            .iter()
            .chain(self.matrix().iter())
            .all(|v| v.is_finite())
    }
}

#[derive(Debug, Clone)]
struct SavedPosition {
    leading: Option<f64>,
    position: Option<PositionSnapshot>,
    in_text: bool,
    font_known: bool,
}

#[derive(Debug, Clone)]
pub(super) struct PositionState {
    leading: Option<f64>,
    position: Option<PositionSnapshot>,
    in_text: bool,
    font_known: bool,
    stack: Vec<SavedPosition>,
    basis_stack: Vec<BasisDeclaration>,
}
impl Default for PositionState {
    fn default() -> Self {
        Self {
            leading: Some(0.0),
            position: None,
            in_text: false,
            font_known: true,
            stack: Vec::new(),
            basis_stack: Vec::new(),
        }
    }
}
fn numbers<const N: usize>(operands: &[LexicalToken]) -> Option<[f64; N]> {
    if operands.len() != N {
        return None;
    }
    let mut result = [0.0; N];
    for (slot, operand) in result.iter_mut().zip(operands) {
        let LexicalKind::Number(value) = &operand.kind else {
            return None;
        };
        if !value.is_finite() {
            return None;
        }
        *slot = *value;
    }
    Some(result)
}
impl PositionState {
    /// A Form has its own content/text objects and implicit save/restore. Text
    /// parameters (including leading) are inherited, but local q/Q frames must
    /// never pop a caller frame. A Do inside BT needs a different scope model.
    pub(super) fn form_entry(&self) -> Result<Self> {
        if self.in_text {
            return Err(WellfriendError::UnsupportedFeature(
                "Form text selection from a Do inside an open text object requires text-object scope normalization".into(),
            ));
        }
        Ok(Self {
            leading: self.leading,
            position: None,
            in_text: false,
            font_known: self.font_known,
            stack: Vec::new(),
            basis_stack: self.basis_stack.clone(),
        })
    }

    fn move_line(&mut self, x: f64, y: f64) {
        let Some(position) = &mut self.position else {
            return;
        };
        position.line = concat_matrix(&translate_matrix(x, y), &position.line);
        position.offset = [0.0; 2];
        if !position.line.iter().all(|v| v.is_finite()) {
            self.position = None;
        }
    }
    fn next_line(&mut self) {
        match self.leading {
            Some(leading) => self.move_line(0.0, -leading),
            None => self.position = None,
        }
    }
    pub(super) fn observe(
        &mut self,
        operator: &str,
        operands: &[LexicalToken],
        tokens: &mut [ContentStringToken],
        parameters: Parameters<'_>,
        metrics: Option<&mut Metrics<'_>>,
    ) -> Result<()> {
        match operator {
            "BMC" | "BDC" => {
                if self.basis_stack.len() >= 4096 {
                    return Err(WellfriendError::ResourceLimit(
                        "inline layout-origin nesting exceeds 4096".into(),
                    ));
                }
                self.basis_stack.push(if operator == "BDC" {
                    basis_declaration(operands)?
                } else {
                    BasisDeclaration::Absent
                });
            }
            "EMC" => {
                self.basis_stack.pop();
            }
            "BT" => {
                self.position = (!self.in_text).then(|| PositionSnapshot::new(IDENTITY_MATRIX));
                self.in_text = true;
            }
            "ET" => {
                self.position = None;
                self.in_text = false;
            }
            "q" => {
                if self.stack.len() >= 4096 {
                    return Err(WellfriendError::ResourceLimit(
                        "inline text graphics stack exceeds 4096".into(),
                    ));
                }
                self.stack.push(SavedPosition {
                    leading: self.leading,
                    position: self.position.clone(),
                    in_text: self.in_text,
                    font_known: self.font_known,
                });
            }
            "Q" => {
                if let Some(saved) = self.stack.pop() {
                    self.leading = saved.leading;
                    self.font_known = saved.font_known;
                    // ISO 32000-2 erratum 368: q/Q inside a text object also
                    // save/restore Tm and Tlm. Crossing a BT/ET boundary has no
                    // portable position interpretation, so do not infer one.
                    self.position = if self.in_text && saved.in_text {
                        saved.position
                    } else {
                        None
                    };
                } else {
                    self.position = None;
                    self.leading = None;
                    self.font_known = false;
                }
            }
            "Tf" => {
                self.font_known = operands.len() == 2
                    && matches!(&operands[0].kind, LexicalKind::Name(_))
                    && matches!(&operands[1].kind, LexicalKind::Number(v) if v.is_finite());
            }
            "gs" => {
                match metrics
                    .as_ref()
                    .ok_or(())
                    .and_then(|m| m.ext_font(operands))
                {
                    Ok(Some(_)) => self.font_known = true,
                    Ok(None) => {}
                    Err(()) => self.font_known = false,
                }
            }
            "TL" => self.leading = numbers::<1>(operands).map(|v| v[0]),
            "Tm" => {
                self.position = if self.in_text {
                    numbers::<6>(operands).map(PositionSnapshot::new)
                } else {
                    None
                }
            }
            "Td" | "TD" => {
                if let Some([x, y]) = numbers::<2>(operands) {
                    if operator == "TD" {
                        self.leading = Some(-y);
                    }
                    self.move_line(x, y);
                } else {
                    self.position = None;
                    if operator == "TD" {
                        self.leading = None;
                    }
                }
            }
            "T*" => self.next_line(),
            "Tj" | "'" | "\"" | "TJ" => {
                if operator == "'" || operator == "\"" {
                    self.next_line();
                }
                let resolver = if self.font_known {
                    metrics.and_then(|m| m.font(parameters.font_name))
                } else {
                    None
                };
                let Some(resolver) = resolver else {
                    self.position = None;
                    return Ok(());
                };
                let mut strings = tokens.iter_mut();
                for (index, operand) in operands.iter().enumerate() {
                    if index % 256 == 0 {
                        crate::cancel::check_current_cancel("inline source positions")?;
                    }
                    if let Some(position) = &mut self.position {
                        if resolver.is_vertical() {
                            position.vertical_font = Some(parameters.font_name.to_owned());
                        } else {
                            position.horizontal_font = Some(parameters.font_name.to_owned());
                        }
                    }
                    let displacement = match &operand.kind {
                        LexicalKind::String(_, _) => {
                            let Some(token) = strings.next() else {
                                return Err(WellfriendError::MalformedPdf(
                                    "inline position/source-token mismatch".into(),
                                ));
                            };
                            token.source_position = self.position.clone();
                            token.generated_basis = match self
                                .basis_stack
                                .iter()
                                .rev()
                                .find(|value| !matches!(value, BasisDeclaration::Absent))
                            {
                                Some(BasisDeclaration::Declared { basis, painted })
                                    if self
                                        .position
                                        .as_ref()
                                        .is_some_and(|p| same_matrix(p.matrix(), *painted)) =>
                                {
                                    SourceBasis::Verified(*basis)
                                }
                                Some(_) => SourceBasis::Invalid,
                                None => SourceBasis::Absent,
                            };
                            // Unsupported metrics invalidate this cursor until
                            // an explicit Tm/BT resets it; never reject unrelated
                            // content merely because it was scanned.
                            match source_displacement(token, resolver, &token.decoded) {
                                Ok(value) => Some(value),
                                Err(WellfriendError::UnsupportedFeature(_)) => None,
                                Err(error) => return Err(error),
                            }
                        }
                        LexicalKind::Number(value) if operator == "TJ" => {
                            Some(crate::fonts::resolver::text_position_adjustment(
                                -value,
                                parameters.font_size,
                                parameters.horizontal_scaling,
                                resolver.is_vertical(),
                            ))
                        }
                        _ => continue,
                    };
                    if let (Some(position), Some(displacement)) = (&mut self.position, displacement)
                    {
                        if !position.advance(displacement) {
                            self.position = None;
                        }
                    } else {
                        self.position = None;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Shared by source deletion, position tracking and endpoint restoration.
/// No division by font size: even zero-size text can advance through Tc/Tw.
pub(super) fn source_displacement(
    token: &ContentStringToken,
    resolver: &FontResolver,
    bytes: &[u8],
) -> Result<[f64; 2]> {
    resolver
        .validate_encoding()
        .map_err(WellfriendError::UnsupportedFeature)?;
    if ![
        token.font_size,
        token.character_spacing,
        token.word_spacing,
        token.horizontal_scaling,
    ]
    .iter()
    .all(|v| v.is_finite())
    {
        return Err(WellfriendError::UnsupportedFeature(
            "inline source has incomplete character codes or non-finite text metrics".into(),
        ));
    }
    let mut advance = 0.0;
    for (index, decoded) in resolver.codes(bytes).enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("inline source glyph metrics")?;
        }
        let code = decoded.map_err(WellfriendError::UnsupportedFeature)?.code;
        let word = code.is_word_space();
        advance += if resolver.is_vertical() {
            crate::fonts::resolver::vertical_text_advance(
                resolver.vertical_metrics_for_code(code).0,
                token.font_size,
                token.character_spacing,
                token.word_spacing,
                word,
            )
        } else {
            (resolver.width_for_code(code) / 1000.0 * token.font_size
                + token.character_spacing
                + if word { token.word_spacing } else { 0.0 })
                * token.horizontal_scaling
                / 100.0
        };
    }
    if !advance.is_finite() {
        return Err(WellfriendError::UnsupportedFeature(
            "non-finite inline source displacement".into(),
        ));
    }
    Ok(if resolver.is_vertical() {
        [0.0, advance]
    } else {
        [advance, 0.0]
    })
}

/// PDF real numbers have no exponent syntax. Retain more than the generic
/// six-decimal display formatter when writing a source matrix/cursor back.
fn number(value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(WellfriendError::UnsupportedFeature(
            "non-finite inline position".into(),
        ));
    }
    let mut value = format!("{value:.15}");
    while value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if value == "-0" {
        value = "0".into();
    }
    Ok(value)
}
fn matrix(output: &mut Vec<u8>, value: Matrix) -> Result<()> {
    let values = value.into_iter().map(number).collect::<Result<Vec<_>>>()?;
    output.extend_from_slice(format!("{} Tm\n", values.join(" ")).as_bytes());
    Ok(())
}

/// A private, bounded layout-origin hint, bound to the actual glyph Tm.
/// Ordinary readers ignore it. Re-editing must not mistake a glyph rotation or
/// GPOS offset for its paragraph/column baseline. ActualText=null makes this a
/// logically transparent Span; no MCID, tag owner or extractable text is added.
pub(super) fn append_glyph_marker(
    output: &mut Vec<u8>,
    basis: Matrix,
    painted: Matrix,
    cid: u16,
) -> Result<()> {
    let values = basis
        .into_iter()
        .chain(painted)
        .map(number)
        .collect::<Result<Vec<_>>>()?;
    output.extend_from_slice(
        format!(
            "/Span << /ActualText null /WFTextBasisV1 [{}] >> BDC\n",
            values.join(" ")
        )
        .as_bytes(),
    );
    // Keep Tm and Tj on the same line for the existing source diagnostics.
    let values = painted
        .into_iter()
        .map(number)
        .collect::<Result<Vec<_>>>()?;
    output.extend_from_slice(format!("{} Tm <{cid:04X}> Tj\nEMC\n", values.join(" ")).as_bytes());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rewrite_positioned(
    token: &ContentStringToken,
    paint: &ContentStringToken,
    resolver: &FontResolver,
    prefix: &[u8],
    selected: &[u8],
    replacement_text: &str,
    glyphs: &[GeneratedGlyph],
    generated_font: &str,
    generated_vertical: bool,
    paint_vertical: bool,
    suffix: &[u8],
) -> Result<(usize, usize, Vec<u8>)> {
    let mut start = token.source_position.clone().ok_or_else(|| WellfriendError::UnsupportedFeature(
        "generated inline editing requires a resolved source text matrix, line matrix and font history".into()))?;
    if ![
        paint.font_size,
        paint.horizontal_scaling,
        paint.text_rise,
        paint.character_spacing,
        paint.word_spacing,
    ]
    .iter()
    .all(|v| v.is_finite())
        || paint.font_size <= 0.0
    {
        return Err(WellfriendError::UnsupportedFeature(
            "invalid generated inline paint metrics".into(),
        ));
    }
    if !start.advance(source_displacement(token, resolver, prefix)?) {
        return Err(WellfriendError::UnsupportedFeature(
            "non-finite generated inline prefix position".into(),
        ));
    }
    let mut endpoint = start.clone();
    if !endpoint.advance(source_displacement(token, resolver, selected)?) {
        return Err(WellfriendError::UnsupportedFeature(
            "non-finite generated inline endpoint".into(),
        ));
    }
    let origin = match &token.generated_basis {
        SourceBasis::Absent => start.matrix(),
        SourceBasis::Verified(basis) => {
            let prefix = source_displacement(token,resolver,prefix)?;
            concat_matrix(&translate_matrix(prefix[0],prefix[1]),basis)
        }
        SourceBasis::Invalid => return Err(WellfriendError::UnsupportedFeature(
            "generated inline layout-origin metadata is malformed or no longer matches its source glyph matrix".into())),
    };
    let mut body = Vec::new();
    if !prefix.is_empty() {
        append_serialized_text_show(&mut body, prefix, token.representation);
    }
    body.extend_from_slice(
        format!(
            "/Span << /ActualText <{}> >> BDC\n0 Tc\n0 Tw\n0 Ts\n/{} {} Tf\n{} Tz\n{} Tr\n{}\n{}\n",
            utf16be_hex_with_bom(replacement_text),
            serialized_name_body(generated_font),
            number(paint.font_size)?,
            number(paint.horizontal_scaling)?,
            paint.text_render_mode,
            paint.fill_color_command,
            paint.stroke_color_command
        )
        .as_bytes(),
    );
    let scale = paint.font_size / 1000.0;
    let hscale = paint.horizontal_scaling / 100.0;
    let logical_only = crate::fonts::logical_carrier::is_text(replacement_text);
    let mut down = 0.0;
    let mut cross = 0.0;
    for (index, glyph) in glyphs.iter().enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("positioned inline glyph emission")?;
        }
        let rotated = glyph.orientation == VerticalGlyphOrientation::RotateClockwise;
        let ox = glyph.offset_x * scale * hscale;
        let oy = glyph.offset_y * scale;
        // Ts is applied in the SOURCE text axes, not the rotated glyph axes.
        // W2 has zero origin; HarfBuzz's offsets are included exactly once.
        let local = if !generated_vertical {
            [1.0, 0.0, 0.0, 1.0, cross + ox, oy + paint.text_rise]
        } else if rotated {
            [
                0.0,
                -1.0,
                1.0,
                0.0,
                cross + oy,
                -down - ox + paint.text_rise,
            ]
        } else {
            [1.0, 0.0, 0.0, 1.0, cross + ox, -down + oy + paint.text_rise]
        };
        let baseline = concat_matrix(
            &translate_matrix(
                cross,
                if generated_vertical {
                    -down + paint.text_rise
                } else {
                    paint.text_rise
                },
            ),
            &origin,
        );
        append_glyph_marker(
            &mut body,
            baseline,
            concat_matrix(&local, &origin),
            glyph.cid,
        )?;
        let cluster_end = glyphs
            .get(index + 1)
            .is_none_or(|next| next.logical_byte_start != glyph.logical_byte_start);
        let spacing = if cluster_end && !logical_only {
            paint.character_spacing
                + if glyph.visual_unicode == " " {
                    paint.word_spacing
                } else {
                    0.0
                }
        } else {
            0.0
        };
        if generated_vertical {
            down += glyph.advance * scale * if rotated { hscale } else { 1.0 }
                + if paint_vertical { -spacing } else { spacing };
            cross += glyph.cross_advance * scale * hscale;
        } else {
            cross += (glyph.advance * scale + spacing) * hscale;
        }
    }
    // Restore Tlm, then move ONLY Tm with numeric TJ operands. The existing
    // horizontal font is needed only when preceding mixed-writing-mode text
    // contributed an x displacement. No glyphs, font asset or source codes are
    // added for restoration. No inverse matrix or q/Q workaround is required.
    matrix(&mut body, endpoint.line)?;
    body.extend_from_slice(b"0 Tc\n0 Tw\n0 Ts\n100 Tz\n");
    if endpoint.offset[0] != 0.0 {
        let horizontal_font = if !generated_vertical {
            Some(generated_font)
        } else {
            endpoint.horizontal_font.as_deref()
        }
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "inline source horizontal displacement has no resolved horizontal font".into(),
            )
        })?;
        body.extend_from_slice(
            format!(
                "/{} 1000 Tf\n[{}] TJ\n",
                serialized_name_body(horizontal_font),
                number(-endpoint.offset[0])?
            )
            .as_bytes(),
        );
    }
    if endpoint.offset[1] != 0.0 {
        let vertical_font = if generated_vertical {
            Some(generated_font)
        } else {
            endpoint.vertical_font.as_deref()
        }
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "inline source vertical displacement has no resolved vertical font".into(),
            )
        })?;
        body.extend_from_slice(
            format!(
                "/{} 1000 Tf\n[{}] TJ\n",
                serialized_name_body(vertical_font),
                number(-endpoint.offset[1])?
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "/{} {} Tf\n{} Tc\n{} Tw\n{} Tz\n{} Ts\n{} Tr\n{}\n{}\nEMC\n",
            serialized_name_body(&token.font_name),
            number(token.font_size)?,
            number(token.character_spacing)?,
            number(token.word_spacing)?,
            number(token.horizontal_scaling)?,
            number(token.text_rise)?,
            token.text_render_mode,
            token.fill_color_command,
            token.stroke_color_command
        )
        .as_bytes(),
    );
    if !suffix.is_empty() {
        append_serialized_text_show(&mut body, suffix, token.representation);
    }
    match token.operator.as_str() {
        "Tj" => Ok((token.operation_start, token.operation_end, body)),
        "'" | "\"" => {
            let mut rewritten = if token.operator == "\"" {
                format!(
                    "{} Tw\n{} Tc\nT*\n",
                    number(token.word_spacing)?,
                    number(token.character_spacing)?
                )
                .into_bytes()
            } else {
                b"T*\n".to_vec()
            };
            rewritten.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, rewritten))
        }
        "TJ" => {
            let mut rewritten = b"] TJ\n".to_vec();
            rewritten.extend_from_slice(&body);
            rewritten.extend_from_slice(b"[\n");
            Ok((token.token_start, token.token_end, rewritten))
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "positioned inline editing does not support {other}"
        ))),
    }
}
