//! Shared CFF2 control/variation expansion. Outline projection omits hints;
//! explicit static publication preserves them and rejects unknown semantics.
use super::{data, fail, number, Program};
use crate::{Result, WellfriendError};

struct State<'a> {
    program: &'a Program,
    coords: &'a [ttf_parser::NormalizedCoordinate],
    fd: usize,
    stack: Vec<f64>,
    scalars: Option<Vec<f64>>,
    vsindex: usize,
    seen_vsindex: bool,
    blended: bool,
    stems: usize,
    work: usize,
    output: Vec<u8>,
    persist: bool,
    width: Option<f64>,
    path_started: bool,
    mask_started: bool,
    hints_closed: bool,
    vertical_stems: bool,
    plain_stems: bool,
    masks: usize,
    blends: usize,
    flexes: usize,
}
pub(super) fn freeze(
    program: &Program,
    gid: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
) -> Result<Vec<u8>> {
    Ok(freeze_mode(program, gid, coords, None)?.bytes)
}
pub(super) struct FrozenGlyph {
    pub bytes: Vec<u8>,
    pub work: usize,
    pub stems: usize,
    pub masks: usize,
    pub blends: usize,
    pub flexes: usize,
}
/// Persistence is separate from outline projection: no hint or unknown owner
/// is silently discarded, and widths come from the frozen hmtx authority.
pub(super) fn freeze_static(
    program: &Program,
    gid: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
    width_delta: f64,
) -> Result<FrozenGlyph> {
    freeze_mode(program, gid, coords, Some(width_delta))
}
fn freeze_mode(
    program: &Program,
    gid: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
    width: Option<f64>,
) -> Result<FrozenGlyph> {
    let range = program
        .glyphs
        .get(gid)
        .ok_or_else(|| fail("glyph outside CharStrings"))?;
    let fd = program.selection[gid];
    if let Some(store) = &program.store {
        if store.axis_count() != coords.len() {
            return Err(fail("CFF2/fvar axis count mismatch"));
        }
    }
    let mut state = State {
        program,
        coords,
        fd,
        stack: Vec::new(),
        scalars: None,
        vsindex: program.dictionaries[fd].vsindex,
        seen_vsindex: false,
        blended: false,
        stems: 0,
        work: 0,
        output: Vec::new(),
        persist: width.is_some(),
        width,
        path_started: false,
        mask_started: false,
        hints_closed: false,
        vertical_stems: false,
        plain_stems: false,
        masks: 0,
        blends: 0,
        flexes: 0,
    };
    state.visit(&program.source[range.clone()], 0)?;
    if !state.stack.is_empty() {
        return Err(fail("trailing CharString operands"));
    }
    state.emit(&[], &[14])?; // CFF1 endchar; CFF2 uses end-of-buffer.
    state.step()?;
    Ok(FrozenGlyph {
        bytes: state.output,
        work: state.work,
        stems: state.stems,
        masks: state.masks,
        blends: state.blends,
        flexes: state.flexes,
    })
}
impl State<'_> {
    fn step(&mut self) -> Result<()> {
        self.work += 1;
        if self.work > 1_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "CFF2 glyph instruction budget".into(),
            ));
        }
        if self.work % 256 == 1 {
            crate::cancel::check_current_cancel("CFF2 glyph expansion")?;
        }
        if self.output.len() > if self.persist { 65535 } else { 4 * 1024 * 1024 } {
            return Err(WellfriendError::ResourceLimit(
                "CFF2 frozen glyph exceeds publication/projection byte budget".into(),
            ));
        }
        Ok(())
    }
    fn visit(&mut self, bytes: &[u8], depth: usize) -> Result<()> {
        if depth > 10 {
            return Err(fail("CharString subroutine nesting exceeds 10"));
        }
        let mut at = 0;
        while at < bytes.len() {
            self.step()?;
            let op = bytes[at];
            at += 1;
            if let Some(n) = number(bytes, &mut at, op, false)? {
                self.stack.push(n);
                if self.stack.len() > 513 {
                    return Err(fail("CharString operand stack exceeds 513"));
                }
                continue;
            }
            match op {
                10 | 29 => {
                    let n = self
                        .stack
                        .pop()
                        .ok_or_else(|| fail("missing subroutine index"))?;
                    if n.fract() != 0.0 {
                        return Err(fail("fractional subroutine index"));
                    }
                    let index = if op == 10 {
                        &self.program.dictionaries[self.fd].local
                    } else {
                        &self.program.globals
                    };
                    let bias = if index.len() < 1240 {
                        107
                    } else if index.len() < 33900 {
                        1131
                    } else {
                        32768
                    };
                    let n = n + f64::from(bias);
                    if n < 0.0 || n >= index.len() as f64 {
                        return Err(fail("subroutine index out of range"));
                    }
                    let range = index[n as usize].clone();
                    // The immutable program borrow is independent of expansion state.
                    let program = self.program;
                    let source = &program.source[range];
                    self.visit(source, depth + 1)?;
                }
                15 => {
                    if self.seen_vsindex || self.blended || self.stack.len() != 1 {
                        return Err(fail("vsindex must precede blend and occur once"));
                    }
                    self.vsindex = super::integer(self.stack.pop().unwrap())?;
                    let store = self
                        .program
                        .store
                        .as_ref()
                        .ok_or_else(|| fail("vsindex without VariationStore"))?;
                    self.scalars = Some(store.scalars(self.vsindex, self.coords)?);
                    self.seen_vsindex = true;
                }
                16 => self.blend()?,
                1 | 3 | 18 | 23 => {
                    self.hints()?;
                    let operands = std::mem::take(&mut self.stack);
                    if self.persist {
                        self.stem_output(op, &operands)?;
                    }
                }
                19 | 20 => {
                    self.hints()?;
                    let operands = std::mem::take(&mut self.stack);
                    let count = self.stems.div_ceil(8);
                    let mask = data(bytes, at, count)?;
                    if self.persist {
                        if !operands.is_empty() {
                            self.stem_output(23, &operands)?;
                        }
                        if count == 0
                            || (op == 19 && self.plain_stems)
                            || (op == 20 && (self.mask_started || self.path_started))
                        {
                            return Err(fail("invalid static hint/counter mask sequence"));
                        }
                        if !self.stems.is_multiple_of(8)
                            && mask[count - 1] & ((1u8 << (8 - self.stems % 8)) - 1) != 0
                        {
                            return Err(fail("nonzero unused hint-mask bits"));
                        }
                        self.emit(&[], &[op])?;
                        self.output.extend_from_slice(mask);
                        self.masks += 1;
                        self.hints_closed = true;
                        if op == 19 {
                            self.mask_started = true;
                        }
                    }
                    at += count;
                }
                12 => {
                    let escaped = *bytes
                        .get(at)
                        .ok_or_else(|| fail("truncated CharString escape"))?;
                    at += 1;
                    let arity = match escaped {
                        34 => 7,
                        35 => 13,
                        36 => 9,
                        37 => 11,
                        _ => {
                            if self.persist {
                                return Err(fail(
                                    "unknown CFF2 escaped operator cannot be persisted",
                                ));
                            }
                            self.stack.clear();
                            continue;
                        }
                    };
                    if self.stack.len() != arity {
                        return Err(fail("flex operand count"));
                    }
                    if self.persist && !self.path_started {
                        return Err(fail("flex before moveto"));
                    }
                    let operands = std::mem::take(&mut self.stack);
                    self.emit(&operands, &[12, escaped])?;
                    self.flexes += 1;
                }
                4 | 5 | 6 | 7 | 8 | 21 | 22 | 24 | 25 | 26 | 27 | 30 | 31 => {
                    let operands = std::mem::take(&mut self.stack);
                    self.path(op, &operands)?;
                }
                // CFF2 forward compatibility: unknown operators consume the
                // operand stack; they are not CFF1 return/endchar instructions.
                _ if self.persist => return Err(fail("unknown CFF2 operator cannot be persisted")),
                _ => self.stack.clear(),
            }
        }
        Ok(())
    }
    fn hints(&mut self) -> Result<()> {
        if !self.stack.len().is_multiple_of(2) {
            return Err(fail("CFF2 has no width operand before stems"));
        }
        self.stems += self.stack.len() / 2;
        if self.stems > 96 {
            return Err(fail("CFF2 hint count exceeds 96"));
        }
        Ok(())
    }
    fn stem_output(&mut self, op: u8, values: &[f64]) -> Result<()> {
        let vertical = op == 3 || op == 23;
        if values.is_empty()
            || self.path_started
            || self.hints_closed
            || (!vertical && self.vertical_stems)
        {
            return Err(fail("invalid persisted stem sequence"));
        }
        self.vertical_stems |= vertical;
        self.plain_stems |= op == 1 || op == 3;
        let mut offset = 0.;
        // Each emitted stem operator starts its delta origin at zero. Rebase
        // split groups; retaining the next raw delta would move later hints.
        for chunk in values.chunks(46) {
            let mut operands = chunk.to_vec();
            operands[0] += offset;
            self.emit(&operands, &[op])?;
            // Rebase against already emitted 16.16 deltas, not unrounded blend
            // values: many half-unit deltas otherwise accumulate a different
            // origin at the split boundary.
            offset += chunk
                .iter()
                .map(|n| (n * 65536.).round() / 65536.)
                .sum::<f64>();
        }
        Ok(())
    }
    fn blend(&mut self) -> Result<()> {
        let count = super::integer(self.stack.pop().ok_or_else(|| fail("blend count"))?)?;
        if count == 0 || count > 513 {
            return Err(fail("invalid blend count"));
        }
        if self.scalars.is_none() {
            self.scalars = Some(
                self.program
                    .store
                    .as_ref()
                    .ok_or_else(|| fail("blend without VariationStore"))?
                    .scalars(self.vsindex, self.coords)?,
            );
        }
        let scalars = self.scalars.as_ref().unwrap();
        let needed = count
            .checked_mul(scalars.len() + 1)
            .ok_or_else(|| fail("blend size overflow"))?;
        let start = self
            .stack
            .len()
            .checked_sub(needed)
            .ok_or_else(|| fail("blend operand count"))?;
        for i in 0..count {
            let mut value = self.stack[start + i];
            // Deltas are grouped by result, then by region, not region-major.
            for (region, scalar) in scalars.iter().enumerate() {
                value += self.stack[start + count + i * scalars.len() + region] * scalar;
            }
            if !value.is_finite() {
                return Err(fail("non-finite blend result"));
            }
            self.stack[start + i] = value;
        }
        self.stack.truncate(start + count);
        self.blended = true;
        self.blends += 1;
        Ok(())
    }
    fn emit(&mut self, values: &[f64], op: &[u8]) -> Result<()> {
        self.step()?;
        if values.len() + usize::from(self.width.is_some()) > 48 {
            return Err(fail("normalized CFF1 operand count"));
        }
        if let Some(width) = self.width.take() {
            encode_number(&mut self.output, width)?;
        }
        for value in values {
            encode_number(&mut self.output, *value)?;
        }
        self.output.extend_from_slice(op);
        Ok(())
    }
    fn groups(&mut self, values: &[f64], size: usize, op: u8) -> Result<()> {
        if values.is_empty() || !values.len().is_multiple_of(size) {
            return Err(fail("path operand count"));
        }
        for chunk in values.chunks(48 / size * size) {
            self.emit(chunk, &[op])?;
        }
        Ok(())
    }
    fn path(&mut self, op: u8, a: &[f64]) -> Result<()> {
        if self.persist {
            if matches!(op, 4 | 21 | 22) {
                self.path_started = true;
            } else if !self.path_started {
                return Err(fail("path before moveto"));
            }
        }
        match op {
            4 | 22 => {
                if a.len() != 1 {
                    return Err(fail("move operand count"));
                }
                self.emit(a, &[op])
            }
            21 => {
                if a.len() != 2 {
                    return Err(fail("move operand count"));
                }
                self.emit(a, &[op])
            }
            5 => self.groups(a, 2, 5),
            8 => self.groups(a, 6, 8),
            6 | 7 => {
                if a.is_empty() {
                    return Err(fail("line operand count"));
                }
                for (i, value) in a.iter().enumerate() {
                    let horizontal = (op == 6) ^ (i % 2 == 1);
                    self.emit(
                        &if horizontal {
                            [*value, 0.0]
                        } else {
                            [0.0, *value]
                        },
                        &[5],
                    )?;
                }
                Ok(())
            }
            24 => {
                if a.len() < 8 {
                    return Err(fail("rcurveline operands"));
                }
                self.groups(&a[..a.len() - 2], 6, 8)?;
                self.emit(&a[a.len() - 2..], &[5])
            }
            25 => {
                if a.len() < 8 {
                    return Err(fail("rlinecurve operands"));
                }
                self.groups(&a[..a.len() - 6], 2, 5)?;
                self.emit(&a[a.len() - 6..], &[8])
            }
            26 | 27 => {
                if a.len() < 4 || a.len() % 4 > 1 {
                    return Err(fail("vv/hh curve operands"));
                }
                let extra = a.len() % 4;
                let mut initial = if extra == 1 { a[0] } else { 0.0 };
                for chunk in a[extra..].chunks_exact(4) {
                    let curve = if op == 26 {
                        [initial, chunk[0], chunk[1], chunk[2], 0.0, chunk[3]]
                    } else {
                        [chunk[0], initial, chunk[1], chunk[2], chunk[3], 0.0]
                    };
                    self.emit(&curve, &[8])?;
                    initial = 0.0;
                }
                Ok(())
            }
            30 | 31 => {
                if a.len() < 4 || a.len() % 4 > 1 {
                    return Err(fail("vh/hv curve operands"));
                }
                let count = a.len() / 4;
                let extra = if a.len() % 4 == 1 {
                    a[a.len() - 1]
                } else {
                    0.0
                };
                for (i, c) in a[..count * 4].chunks_exact(4).enumerate() {
                    let last = if i + 1 == count { extra } else { 0.0 };
                    let horizontal = (op == 31) ^ (i % 2 == 1);
                    let curve = if horizontal {
                        [c[0], 0.0, c[1], c[2], last, c[3]]
                    } else {
                        [0.0, c[0], c[1], c[2], c[3], last]
                    };
                    self.emit(&curve, &[8])?;
                }
                Ok(())
            }
            _ => Err(fail("unknown path operator")),
        }
    }
}
pub(super) fn encode_number(out: &mut Vec<u8>, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(fail("non-finite outline operand"));
    }
    if value.fract() == 0.0 && (-32768.0..=32767.0).contains(&value) {
        let n = value as i32;
        if (-107..=107).contains(&n) {
            out.push((n + 139) as u8);
        } else {
            out.push(28);
            out.extend_from_slice(&(n as i16).to_be_bytes());
        }
    } else {
        let fixed = (value * 65536.0).round();
        if fixed < f64::from(i32::MIN) || fixed > f64::from(i32::MAX) {
            return Err(fail("outline operand outside Type 2 16.16 range"));
        }
        out.push(255);
        out.extend_from_slice(&(fixed as i32).to_be_bytes());
    }
    Ok(())
}
fn cff_index(entries: &[&[u8]]) -> Vec<u8> {
    let mut out = (entries.len() as u16).to_be_bytes().to_vec();
    if entries.is_empty() {
        return out;
    }
    out.push(4);
    let mut at = 1u32;
    out.extend_from_slice(&at.to_be_bytes());
    for value in entries {
        at += value.len() as u32;
        out.extend_from_slice(&at.to_be_bytes());
    }
    for value in entries {
        out.extend_from_slice(value);
    }
    out
}
/// A transient two-glyph CFF1 program. It is never inserted into a PDF resource;
/// source shaping/metrics and GIDs remain authoritative outside this decoder.
pub(super) fn glyph_font(code: &[u8]) -> Result<Vec<u8>> {
    if code.len() > 4 * 1024 * 1024 {
        return Err(fail("frozen outline budget"));
    }
    let name = cff_index(&[b"WFCff2Outline"]);
    let string = cff_index(&[]);
    let global = cff_index(&[]);
    let top = |offset: u32| {
        let mut dict = vec![29];
        dict.extend_from_slice(&offset.to_be_bytes());
        dict.push(17);
        cff_index(&[&dict])
    };
    let offset = 4 + name.len() + top(0).len() + string.len() + global.len();
    let mut out = vec![1, 0, 4, 4];
    out.extend(name);
    out.extend(top(offset as u32));
    out.extend(string);
    out.extend(global);
    out.extend(cff_index(&[&[14], code]));
    Ok(out)
}
