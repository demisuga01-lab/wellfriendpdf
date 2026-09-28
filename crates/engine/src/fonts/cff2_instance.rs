//! Complete CFF2-to-CFF1 table staging. Keeps GIDs, selected hint metadata and
//! mask ownership; this is not the outline-only projection used by rendering.
use super::{charstring, data, dict, fail, integer, number, Program};
use crate::{Result, WellfriendError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[path = "cff2_contours.rs"]
mod contours;
#[cfg(test)]
pub(crate) use contours::tests::compatible_source_font as compatible_normalization_test_font;
#[cfg(test)]
pub(crate) use contours::tests::source_font as normalized_test_font;
pub use contours::{ContourNormalization, ContourReport};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CffReport {
    pub source_font_dicts: usize,
    pub output_font_dicts: usize,
    pub glyphs: usize,
    pub stem_hints: usize,
    pub masks: usize,
    pub glyph_blends: usize,
    pub private_blends: usize,
    pub reordered_stem_snap_arrays: usize,
    pub expanded_instructions: usize,
    /// Both preserving and normalization publication check compatibility.
    /// This is exact for linear classification and numerical for curves, not
    /// independent pixel proof. Older unchecked receipts can still read false.
    pub contour_overlaps_checked: bool,
    pub contour_overlaps_removed: bool,
    pub contour_normalization: Option<ContourReport>,
    /// Read-only source classification when no contour rewrite was requested.
    #[serde(default)]
    pub preserved_contour_check: Option<ContourReport>,
}
pub(crate) struct CffStage {
    pub bytes: Vec<u8>,
    pub report: CffReport,
    pub normalized_bounds: Option<Vec<Option<ttf_parser::Rect>>>,
}
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
const MAX_WORK: usize = 16_000_000;

#[derive(Default)]
struct PrivatePool {
    values: Vec<Arc<[u8]>>,
    by_value: BTreeMap<Arc<[u8]>, usize>,
    bytes: usize,
}
impl PrivatePool {
    fn intern(&mut self, value: Vec<u8>) -> Result<usize> {
        if let Some(index) = self.by_value.get(value.as_slice()) {
            return Ok(*index);
        }
        self.bytes = self
            .bytes
            .checked_add(value.len())
            .ok_or_else(|| fail("frozen private byte overflow"))?;
        if self.bytes > MAX_OUTPUT {
            return Err(WellfriendError::ResourceLimit(
                "frozen CFF private dictionary budget".into(),
            ));
        }
        let index = self.values.len();
        let value: Arc<[u8]> = value.into();
        self.values.push(Arc::clone(&value));
        self.by_value.insert(value, index);
        Ok(index)
    }
}

fn encode_dict(out: &mut Vec<u8>, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(fail("non-finite static DICT value"));
    }
    if value.fract() == 0. && (i32::MIN as f64..=i32::MAX as f64).contains(&value) {
        let n = value as i32;
        if (-107..=107).contains(&n) {
            out.push((n + 139) as u8);
        } else {
            out.push(29);
            out.extend(n.to_be_bytes());
        }
        return Ok(());
    }
    // Scientific notation is bounded even for tiny finite values. Remove an
    // e0 suffix since DICT exponent syntax excludes a zero-leading exponent.
    let mut text = format!("{value:e}");
    if text.ends_with("e0") {
        text.truncate(text.len() - 2);
    }
    let mut nibbles = Vec::new();
    let mut chars = text.bytes().peekable();
    while let Some(byte) = chars.next() {
        nibbles.push(match byte {
            b'0'..=b'9' => byte - b'0',
            b'.' => 10,
            b'-' => 14,
            b'e' => {
                if chars.peek() == Some(&b'-') {
                    chars.next();
                    12
                } else {
                    11
                }
            }
            _ => return Err(fail("cannot encode static DICT real")),
        });
    }
    nibbles.push(15);
    if nibbles.len() % 2 != 0 {
        nibbles.push(15);
    }
    out.push(30);
    out.extend(nibbles.chunks_exact(2).map(|p| p[0] << 4 | p[1]));
    Ok(())
}
fn operator(out: &mut Vec<u8>, op: u16) {
    if op > 255 {
        out.push(12);
    }
    out.push(op as u8);
}
fn entry(out: &mut Vec<u8>, op: u16, values: &[f64]) -> Result<()> {
    if values.len() > 48 {
        return Err(fail("static DICT exceeds CFF1 operand capacity"));
    }
    for value in values {
        encode_dict(out, *value)?;
    }
    operator(out, op);
    Ok(())
}
fn offset(out: &mut Vec<u8>, value: usize) -> Result<()> {
    let value = i32::try_from(value).map_err(|_| fail("CFF1 structural offset overflow"))?;
    out.push(29);
    out.extend(value.to_be_bytes());
    Ok(())
}
fn cff_index(entries: &[Vec<u8>], limit: usize) -> Result<Vec<u8>> {
    let count = u16::try_from(entries.len()).map_err(|_| fail("CFF1 INDEX count overflow"))?;
    let size =
        2 + if entries.is_empty() {
            0
        } else {
            1 + (entries.len() + 1) * 4
        } + entries.iter().map(Vec::len).sum::<usize>();
    if size > limit.min(MAX_OUTPUT) {
        return Err(WellfriendError::ResourceLimit(
            "CFF1 INDEX byte budget".into(),
        ));
    }
    let mut out = Vec::with_capacity(size);
    out.extend(count.to_be_bytes());
    if entries.is_empty() {
        return Ok(out);
    }
    out.push(4);
    let mut at = 1u32;
    out.extend(at.to_be_bytes());
    for value in entries {
        at = at
            .checked_add(value.len() as u32)
            .ok_or_else(|| fail("CFF1 INDEX offset overflow"))?;
        out.extend(at.to_be_bytes());
    }
    for value in entries {
        for chunk in value.chunks(65536) {
            crate::cancel::check_current_cancel("CFF1 INDEX serialization")?;
            out.extend(chunk);
        }
    }
    Ok(out)
}
fn blended(stack: &mut Vec<f64>, scalars: &[f64]) -> Result<()> {
    let n = integer(stack.pop().ok_or_else(|| fail("Private blend count"))?)?;
    if n == 0 || n > 513 {
        return Err(fail("Private blend result count"));
    }
    let needed = n
        .checked_mul(scalars.len() + 1)
        .ok_or_else(|| fail("Private blend overflow"))?;
    let start = stack
        .len()
        .checked_sub(needed)
        .ok_or_else(|| fail("Private blend operands"))?;
    for i in 0..n {
        let mut value = stack[start + i];
        for (j, scalar) in scalars.iter().enumerate() {
            value += stack[start + n + i * scalars.len() + j] * scalar;
        }
        if !value.is_finite() {
            return Err(fail("Private blend is non-finite"));
        }
        stack[start + i] = value;
    }
    stack.truncate(start + n);
    Ok(())
}
fn hint_arity(op: u16, values: &[f64]) -> Result<()> {
    let n = values.len();
    let valid = match op {
        6 | 8 => n > 0 && n <= 14 && n.is_multiple_of(2),
        7 | 9 => n > 0 && n <= 10 && n.is_multiple_of(2),
        0x0c0c | 0x0c0d => n > 0 && n <= 12,
        10 | 11 | 0x0c09 | 0x0c0a | 0x0c0b | 0x0c11 | 0x0c12 => n == 1,
        _ => false,
    };
    if !valid {
        return Err(fail("unknown Private hint owner or invalid arity"));
    }
    if matches!(op, 6..=9) {
        let mut absolute = 0.;
        for pair in values.chunks_exact(2) {
            absolute += pair[0];
            let upper = absolute + pair[1];
            if !absolute.is_finite() || !upper.is_finite() || upper < absolute {
                return Err(fail("inverted Private alignment zone"));
            }
            absolute = upper;
        }
    }
    if (matches!(op, 10 | 11) && values[0] < 0.) || (op == 0x0c09 && values[0] <= 0.) {
        return Err(fail("invalid Private hint metric"));
    }
    if matches!(op, 0x0c0a | 0x0c0b | 0x0c12) && values[0] < 0. {
        return Err(fail("negative Private hint parameter"));
    }
    if op == 0x0c11 && values[0].fract() != 0. {
        return Err(fail("fractional language group"));
    }
    Ok(())
}
fn normalize_snap_widths(values: &mut Vec<f64>) -> Result<bool> {
    // Variation can cross two snap widths. Their set, not their order, defines
    // the hints. Decode the delta array, sort the selected absolute widths,
    // coalesce duplicates, then re-encode for CFF1 consumers expecting order.
    let mut absolute = Vec::with_capacity(values.len());
    let mut width = 0.;
    for delta in values.iter() {
        width += delta;
        if !width.is_finite() || width < 0. {
            return Err(fail("invalid selected stem snap width"));
        }
        absolute.push(width);
    }
    if absolute.windows(2).all(|p| p[0] < p[1]) {
        return Ok(false);
    }
    absolute.sort_by(f64::total_cmp);
    absolute.dedup();
    values.clear();
    let mut previous = 0.;
    for value in absolute {
        values.push(value - previous);
        previous = value;
    }
    Ok(true)
}
fn freeze_private(
    program: &Program,
    fd: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
) -> Result<(Vec<u8>, usize, usize, usize)> {
    let source = &program.source[program.dictionaries[fd].private.clone()];
    let mut at = 0;
    let mut stack = Vec::new();
    let mut seen = BTreeSet::new();
    let mut scalars = None;
    let mut blend_count = 0;
    let mut pending_blend = false;
    let mut out = Vec::new();
    let mut steps = 0;
    let mut reordered = 0;
    while at < source.len() {
        steps += 1;
        if steps % 256 == 1 {
            crate::cancel::check_current_cancel("CFF2 Private freezing")?;
        }
        let byte = source[at];
        at += 1;
        if let Some(n) = number(source, &mut at, byte, true)? {
            stack.push(n);
            if stack.len() > 513 {
                return Err(fail("Private stack exceeds 513"));
            }
            continue;
        }
        let op = if byte == 12 {
            let second = *source.get(at).ok_or_else(|| fail("Private escape"))?;
            at += 1;
            0x0c00 | u16::from(second)
        } else {
            u16::from(byte)
        };
        if op == 23 {
            if scalars.is_none() {
                scalars = Some(
                    program
                        .store
                        .as_ref()
                        .ok_or_else(|| fail("Private blend without store"))?
                        .scalars(0, coords)?,
                );
            }
            blended(&mut stack, scalars.as_ref().unwrap())?;
            blend_count += 1;
            pending_blend = true;
            continue;
        }
        if !seen.insert(op) {
            return Err(fail("duplicate Private owner"));
        }
        if op == 22 {
            if stack.len() != 1 || blend_count != 0 {
                return Err(fail("Private vsindex ordering/arity"));
            }
            let vs = integer(stack[0])?;
            scalars = Some(
                program
                    .store
                    .as_ref()
                    .ok_or_else(|| fail("Private vsindex without store"))?
                    .scalars(vs, coords)?,
            );
        } else if op == 19 {
            if stack.len() != 1 || pending_blend {
                return Err(fail("Private Subrs arity or blended offset"));
            }
            integer(stack[0])?; // Local calls were bound by Program and are expanded.
        } else {
            if pending_blend && !matches!(op, 6 | 7 | 10 | 11 | 0x0c0c | 0x0c0d) {
                return Err(fail("non-blendable Private owner"));
            }
            hint_arity(op, &stack)?;
            if matches!(op, 0x0c0c | 0x0c0d) && normalize_snap_widths(&mut stack)? {
                reordered += 1;
            }
            if (op == 7 && !seen.contains(&6)) || (op == 9 && !seen.contains(&8)) {
                return Err(fail("Private alignment-zone ordering"));
            }
            entry(&mut out, op, &stack)?;
        }
        stack.clear();
        pending_blend = false;
    }
    if !stack.is_empty() {
        return Err(fail("trailing Private operands"));
    }
    // Every glyph carries a signed width delta. The midpoint covers the full
    // unsigned hmtx domain without overflowing Type 2's signed number format.
    entry(&mut out, 20, &[0.])?;
    entry(&mut out, 21, &[32768.])?;
    Ok((out, blend_count, steps, reordered))
}

#[cfg(test)]
pub(crate) fn freeze(
    program: &Program,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    advances: &[u16],
    bounds: [i16; 4],
    postscript_name: &str,
    limit: usize,
) -> Result<CffStage> {
    freeze_with_contours(
        program,
        coordinates,
        advances,
        bounds,
        postscript_name,
        limit,
        None,
    )
}
pub(crate) fn freeze_with_contours(
    program: &Program,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    advances: &[u16],
    mut bounds: [i16; 4],
    postscript_name: &str,
    limit: usize,
    normalization: Option<&ContourNormalization>,
) -> Result<CffStage> {
    crate::cancel::check_current_cancel("CFF2 static table")?;
    if let Some(options) = normalization {
        options.validate()?;
    }
    if advances.len() != program.glyph_count() || advances.is_empty() {
        return Err(fail("static glyph/metric identity mismatch"));
    }
    if postscript_name.is_empty()
        || postscript_name.len() > 63
        || postscript_name
            .bytes()
            .any(|c| !(33..=126).contains(&c) || b"[](){}<>/%".contains(&c))
    {
        return Err(fail("invalid static CFF PostScript name"));
    }
    if let Some(store) = &program.store {
        if store.axis_count() != coordinates.len() {
            return Err(fail("static CFF2 coordinate domain"));
        }
    }
    let header = usize::from(program.source[2]);
    let top_len = usize::from(super::u16_at(&program.source, 3)?);
    for (op, _) in dict(data(&program.source, header, top_len)?)? {
        if ![17, 24, 0x0c24, 0x0c25, 0x0c07].contains(&op) {
            return Err(fail("unknown CFF2 Top owner cannot be persisted"));
        }
    }
    for range in &program.font_dicts {
        for (op, _) in dict(&program.source[range.clone()])? {
            if op != 18 {
                return Err(fail("unknown CFF2 FontDICT owner cannot be persisted"));
            }
        }
    }
    let mut report = CffReport {
        source_font_dicts: program.dictionaries.len(),
        output_font_dicts: 0,
        glyphs: advances.len(),
        stem_hints: 0,
        masks: 0,
        glyph_blends: 0,
        private_blends: 0,
        reordered_stem_snap_arrays: 0,
        expanded_instructions: 0,
        contour_overlaps_checked: false,
        contour_overlaps_removed: false,
        contour_normalization: normalization.map(ContourReport::new),
        preserved_contour_check: normalization
            .is_none()
            .then(ContourReport::preserving_check),
    };
    let used = program.selection.iter().copied().collect::<BTreeSet<_>>();
    let mut frozen_privates = PrivatePool::default();
    let mut owners = BTreeMap::new();
    let mut private_cache = BTreeMap::new();
    for fd in used {
        crate::cancel::check_current_cancel("CFF2 private instance ownership")?;
        let range = &program.dictionaries[fd].private;
        let key = (range.start, range.end);
        if let Some(owner) = private_cache.get(&key) {
            owners.insert(fd, *owner);
            continue;
        }
        let (value, blends, work, reordered) = freeze_private(program, fd, coordinates)?;
        report.expanded_instructions += work;
        if report.expanded_instructions > MAX_WORK {
            return Err(WellfriendError::ResourceLimit(
                "CFF2 aggregate private expansion budget".into(),
            ));
        }
        report.private_blends += blends;
        report.reordered_stem_snap_arrays += reordered;
        let next = frozen_privates.intern(value)?;
        if normalization.is_none() && frozen_privates.values.len() > 256 {
            return Err(WellfriendError::UnsupportedFeature(
                "more than 256 distinct frozen CFF1 private dictionaries".into(),
            ));
        }
        owners.insert(fd, next);
        private_cache.insert(key, next);
    }
    let mut unhinted = Vec::new();
    entry(&mut unhinted, 20, &[0.])?;
    entry(&mut unhinted, 21, &[32768.])?;
    let unhinted = frozen_privates.intern(unhinted)?;
    let mut glyph_private_ids = Vec::with_capacity(advances.len());
    let mut private_bypassed = BTreeSet::new();
    let mut private_retained = BTreeSet::new();
    let mut glyphs = Vec::with_capacity(advances.len());
    let mut glyph_bytes = 0usize;
    let mut normalized_bounds = normalization.map(|_| Vec::with_capacity(advances.len()));
    for (gid, width) in advances.iter().enumerate() {
        crate::cancel::check_current_cancel("CFF2 glyph publication")?;
        let mut glyph =
            charstring::freeze_static(program, gid, coordinates, f64::from(*width) - 32768.)?;
        let source_private = owners[&program.selection[gid]];
        let private_has_hints = source_private != unhinted;
        let has_hint_owners =
            private_has_hints || glyph.stems != 0 || glyph.masks != 0 || glyph.flexes != 0;
        let mut output_private = source_private;
        report.expanded_instructions = report
            .expanded_instructions
            .checked_add(glyph.work)
            .ok_or_else(|| fail("CFF2 expansion work overflow"))?;
        if report.expanded_instructions > MAX_WORK {
            return Err(WellfriendError::ResourceLimit(
                "CFF2 aggregate expansion budget".into(),
            ));
        }
        if let Some(options) = normalization {
            let receipt = report.contour_normalization.as_mut().unwrap();
            let outcome = contours::normalize_if_needed(
                &glyph.bytes,
                f64::from(*width) - 32768.,
                gid as u16,
                options,
                receipt,
                has_hint_owners,
            )?;
            glyph.bytes = outcome.bytes;
            if outcome.rewritten {
                output_private = unhinted;
                glyph.stems = 0;
                glyph.masks = 0;
                if private_has_hints {
                    private_bypassed.insert(source_private);
                }
            } else if private_has_hints {
                private_retained.insert(source_private);
            }
            normalized_bounds
                .as_mut()
                .unwrap()
                .push(glyph_bounds(&glyph.bytes)?);
        } else {
            contours::verify_preserved(
                &glyph.bytes,
                gid as u16,
                report.preserved_contour_check.as_mut().unwrap(),
                has_hint_owners,
            )?;
            if private_has_hints {
                private_retained.insert(source_private);
            }
        }
        glyph_bytes += glyph.bytes.len();
        if glyph_bytes > MAX_OUTPUT.min(limit) {
            return Err(WellfriendError::ResourceLimit(
                "CFF1 table byte budget".into(),
            ));
        }
        report.stem_hints += glyph.stems;
        report.masks += glyph.masks;
        report.glyph_blends += glyph.blends;
        glyphs.push(glyph.bytes);
        glyph_private_ids.push(output_private);
    }
    // Assign output FDs after per-glyph decisions. A source private owner may
    // remain on compatible glyphs while rewritten ones use the unhinted owner.
    let used_privates = glyph_private_ids.iter().copied().collect::<BTreeSet<_>>();
    if used_privates.len() > 256 {
        return Err(WellfriendError::UnsupportedFeature(
            "more than 256 distinct output CFF1 private dictionaries after contour decisions"
                .into(),
        ));
    }
    let private_mapping = used_privates
        .iter()
        .enumerate()
        .map(|(i, source)| (*source, i as u8))
        .collect::<BTreeMap<_, _>>();
    let privates = used_privates
        .iter()
        .map(|i| Arc::clone(&frozen_privates.values[*i]))
        .collect::<Vec<_>>();
    report.output_font_dicts = privates.len();
    for receipt in report
        .contour_normalization
        .iter_mut()
        .chain(report.preserved_contour_check.iter_mut())
    {
        receipt.private_hint_dictionaries_removed =
            private_bypassed.difference(&private_retained).count();
        receipt.private_hint_dictionaries_bypassed = private_bypassed.len();
        receipt.private_hint_dictionaries_retained = private_retained.len();
    }
    report.contour_overlaps_checked = true;
    if let Some(glyphs) = &normalized_bounds {
        let mut bbox: Option<[i16; 4]> = None;
        for r in glyphs.iter().flatten() {
            let next = [r.x_min, r.y_min, r.x_max, r.y_max];
            bbox = Some(bbox.map_or(next, |b| {
                [
                    b[0].min(next[0]),
                    b[1].min(next[1]),
                    b[2].max(next[2]),
                    b[3].max(next[3]),
                ]
            }));
        }
        bounds = bbox.unwrap_or([0; 4]);
        report.contour_overlaps_checked = true;
        report.contour_overlaps_removed = true;
    }
    let names = cff_index(&[postscript_name.as_bytes().to_vec()], limit)?;
    let mut strings = vec![b"Adobe".to_vec(), b"Identity".to_vec()];
    // FD names are independent short internal identifiers, not aliases for the
    // user-facing identity. The top Name INDEX carries the approved PS name.
    for fd in 0..privates.len() {
        strings.push(format!("WFInstanceFD{fd}").into_bytes());
    }
    let strings = cff_index(&strings, limit)?;
    let global = [0, 0];
    let chars = cff_index(&glyphs, limit)?;
    let mut charset = vec![0];
    for gid in 1..advances.len() {
        charset.extend((gid as u16).to_be_bytes());
    }
    let mut selection = vec![0];
    selection.extend(glyph_private_ids.iter().map(|owner| private_mapping[owner]));
    let fd_array = |locations: &[usize]| -> Result<Vec<u8>> {
        let mut entries = Vec::new();
        for (i, private) in privates.iter().enumerate() {
            let mut value = Vec::new();
            entry(&mut value, 0x0c26, &[(393 + i) as f64])?;
            offset(&mut value, private.len())?;
            offset(&mut value, locations[i])?;
            operator(&mut value, 18);
            entries.push(value);
        }
        cff_index(&entries, limit)
    };
    let top = |charset: usize, chars: usize, fd: usize, select: usize| -> Result<Vec<u8>> {
        let mut value = Vec::new();
        entry(&mut value, 0x0c1e, &[391., 392., 0.])?;
        entry(&mut value, 0x0c22, &[advances.len() as f64])?;
        entry(
            &mut value,
            0x0c07,
            &[program.matrix_scale, 0., 0., program.matrix_scale, 0., 0.],
        )?;
        entry(&mut value, 5, &bounds.map(f64::from))?;
        for (op, at) in [(15, charset), (17, chars), (0x0c24, fd), (0x0c25, select)] {
            offset(&mut value, at)?;
            operator(&mut value, op);
        }
        cff_index(&[value], limit)
    };
    let charset_at = 4 + names.len() + top(0, 0, 0, 0)?.len() + strings.len() + global.len();
    let chars_at = charset_at + charset.len();
    let fds_at = chars_at + chars.len();
    let select_at = fds_at + fd_array(&vec![0; privates.len()])?.len();
    let mut end = select_at + selection.len();
    let mut locations = Vec::new();
    for value in &privates {
        locations.push(end);
        end += value.len();
    }
    if end > MAX_OUTPUT.min(limit) {
        return Err(WellfriendError::ResourceLimit(
            "complete CFF1 table exceeds output budget".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(end);
    bytes.extend([1, 0, 4, 4]);
    bytes.extend(names);
    bytes.extend(top(charset_at, chars_at, fds_at, select_at)?);
    bytes.extend(strings);
    bytes.extend(global);
    bytes.extend(charset);
    bytes.extend(chars);
    bytes.extend(fd_array(&locations)?);
    bytes.extend(selection);
    for value in privates {
        bytes.extend_from_slice(&value);
    }
    if bytes.len() != end {
        return Err(fail("CFF1 relocation changed structural lengths"));
    }
    let saved = ttf_parser::cff::Table::parse(&bytes)
        .ok_or_else(|| fail("static CFF1 table cannot reopen"))?;
    if usize::from(saved.number_of_glyphs()) != advances.len() {
        return Err(fail("saved CFF1 glyph domain"));
    }
    for gid in 0..advances.len() {
        if gid % 256 == 0 {
            crate::cancel::check_current_cancel("CFF1 charset postconditions")?;
        }
        if saved.glyph_cid(ttf_parser::GlyphId(gid as u16)) != Some(gid as u16) {
            return Err(fail("saved CFF1 changed GID/CID identity"));
        }
    }
    Ok(CffStage {
        bytes,
        report,
        normalized_bounds,
    })
}

fn glyph_bounds(code: &[u8]) -> Result<Option<ttf_parser::Rect>> {
    struct Sink;
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let bytes = charstring::glyph_font(code)?;
    let table = ttf_parser::cff::Table::parse(&bytes)
        .ok_or_else(|| fail("normalized glyph cannot reopen"))?;
    match table.outline(ttf_parser::GlyphId(1), &mut Sink) {
        Ok(rect) => Ok(Some(rect)),
        Err(ttf_parser::CFFError::ZeroBBox) => Ok(None),
        Err(_) => Err(fail("normalized glyph outline is invalid")),
    }
}

#[cfg(test)]
#[path = "cff2_instance_tests.rs"]
mod tests;
