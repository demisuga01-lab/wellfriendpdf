//! Explicit TrueType point/component model. Implied curve points are never
//! inserted: source point identities also belong to hint and layout programs.
use super::{
    tuple_variations::{Budget, TABLE_LIMIT},
    variation_store::{bytes, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::{ops::Range, sync::Arc};

pub(super) fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("glyf instance: {message}"))
}
pub(super) const MAX_DEPTH: usize = 64;
pub(super) const MAX_POINTS: usize = 2_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arguments {
    Offset([i16; 2]),
    Points([u16; 2]),
}
#[derive(Clone, Debug)]
pub(super) struct Component {
    pub flags: u16,
    pub glyph: u16,
    pub arguments: Arguments,
    /// xscale, scale01, scale10, yscale, in original F2DOT14 units.
    pub matrix: [i16; 4],
}
pub(super) enum Kind {
    Empty,
    Simple {
        points: Vec<[i32; 2]>,
        flags: Vec<u8>,
        ends: Vec<u16>,
    },
    Composite(Vec<Component>),
}
pub(super) struct Glyph {
    pub kind: Kind,
    pub bounds: Option<ttf_parser::Rect>,
    pub instructions: Range<usize>,
}
pub(super) struct Program {
    pub source: Arc<[u8]>,
    pub glyphs: Vec<Glyph>,
}
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = bytes(self.data, self.at, n)?;
        self.at += n;
        Ok(out)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn signed(&mut self) -> Result<i16> {
        Ok(self.word()? as i16)
    }
    fn instructions(&mut self, base: usize) -> Result<Range<usize>> {
        let length = usize::from(self.word()?);
        let start = self.at;
        self.take(length)?;
        Ok(base + start..base + self.at)
    }
}
impl Program {
    pub fn parse(
        source: Arc<[u8]>,
        loca: &[u8],
        head: &[u8],
        maxp: &[u8],
        budget: &mut Budget,
    ) -> Result<Self> {
        budget.charge(1)?;
        if source.len() > TABLE_LIMIT {
            return Err(WellfriendError::ResourceLimit("glyf exceeds 64 MiB".into()));
        }
        bytes(head, 0, 54)?;
        bytes(maxp, 0, 32)?;
        if u32_at(head, 0)? != 0x10000 || u16_at(head, 52)? != 0 || u32_at(maxp, 0)? != 0x10000 {
            return Err(fail(
                "invalid TrueType header/profile version or glyph format",
            ));
        }
        let count = usize::from(u16_at(maxp, 4)?);
        if count == 0 {
            return Err(fail("font has no glyph identities"));
        }
        let long = match u16_at(head, 50)? {
            0 => false,
            1 => true,
            _ => return Err(fail("invalid loca format")),
        };
        bytes(loca, 0, (count + 1) * if long { 4 } else { 2 })?;
        let offset = |i| -> Result<usize> {
            if long {
                Ok(u32_at(loca, i * 4)? as usize)
            } else {
                Ok(usize::from(u16_at(loca, i * 2)?) * 2)
            }
        };
        budget.charge(count)?;
        let mut glyphs = Vec::with_capacity(count);
        let mut point_total = 0usize;
        let mut start = offset(0)?;
        if start > source.len() {
            return Err(fail("initial loca offset outside glyf"));
        }
        for id in 0..count {
            budget.charge(1)?;
            let end = offset(id + 1)?;
            let length = end
                .checked_sub(start)
                .ok_or_else(|| fail("descending loca offsets"))?;
            let data = bytes(&source, start, length)?;
            if data.is_empty() {
                glyphs.push(Glyph {
                    kind: Kind::Empty,
                    bounds: None,
                    instructions: start..start,
                });
                start = end;
                continue;
            }
            let mut cursor = Cursor { data, at: 0 };
            let contours = cursor.signed()?;
            let bounds = ttf_parser::Rect {
                x_min: cursor.signed()?,
                y_min: cursor.signed()?,
                x_max: cursor.signed()?,
                y_max: cursor.signed()?,
            };
            if bounds.x_min > bounds.x_max || bounds.y_min > bounds.y_max {
                return Err(fail("unordered source glyph bounds"));
            }
            let (kind, instructions) = if contours >= 0 {
                let mut ends = Vec::with_capacity(contours as usize);
                for index in 0..contours {
                    if index % 256 == 0 {
                        crate::cancel::check_current_cancel("glyf contour endpoints")?;
                    }
                    let end = cursor.word()?;
                    if ends.last().is_some_and(|old| *old >= end) {
                        return Err(fail("non-increasing contour endpoints"));
                    }
                    ends.push(end);
                }
                let n = ends.last().map_or(0, |end| usize::from(*end) + 1);
                if n > usize::from(u16::MAX) {
                    return Err(fail("explicit point count exceeds maxp representation"));
                }
                point_total += n;
                if point_total > MAX_POINTS {
                    return Err(WellfriendError::ResourceLimit(
                        "glyf explicit point budget exceeded".into(),
                    ));
                }
                budget.charge(n * 3 + ends.len())?;
                let instructions = if contours == 0 && cursor.at == data.len() {
                    start + cursor.at..start + cursor.at
                } else {
                    cursor.instructions(start)?
                };
                let mut flags = Vec::with_capacity(n);
                while flags.len() < n {
                    budget.charge(1)?;
                    let flag = cursor.byte()?;
                    if flag & 0x80 != 0 {
                        return Err(fail("reserved simple glyph flag"));
                    }
                    let repeat = if flag & 8 != 0 {
                        usize::from(cursor.byte()?) + 1
                    } else {
                        1
                    };
                    if repeat > n - flags.len() {
                        return Err(fail("flag repeat exceeds explicit points"));
                    }
                    flags.resize(flags.len() + repeat, flag & !8);
                }
                let mut points = vec![[0; 2]; n];
                for axis in 0..2 {
                    let mut value = 0i32;
                    for (i, flag) in flags.iter().enumerate() {
                        if i % 256 == 0 {
                            crate::cancel::check_current_cancel("glyf coordinate decoding")?;
                        }
                        let short = 2 << axis;
                        let same = 0x10 << axis;
                        let delta = if flag & short != 0 {
                            let d = i32::from(cursor.byte()?);
                            if flag & same != 0 {
                                d
                            } else {
                                -d
                            }
                        } else if flag & same != 0 {
                            0
                        } else {
                            i32::from(cursor.signed()?)
                        };
                        value = value
                            .checked_add(delta)
                            .ok_or_else(|| fail("source coordinate overflow"))?;
                        if i16::try_from(value).is_err() {
                            return Err(fail("source point outside glyph design range"));
                        }
                        points[i][axis] = value;
                    }
                }
                (
                    Kind::Simple {
                        points,
                        flags,
                        ends,
                    },
                    instructions,
                )
            } else {
                let mut components = Vec::new();
                let mut has_instructions = false;
                loop {
                    budget.charge(1)?;
                    if components.len() == usize::from(u16::MAX) {
                        return Err(fail("too many components"));
                    }
                    let flags = cursor.word()?;
                    let glyph = cursor.word()?;
                    if flags & 0xe010 != 0 || (flags & 0xc8).count_ones() > 1 {
                        return Err(fail("reserved or competing component transform flags"));
                    }
                    if usize::from(glyph) >= count {
                        return Err(fail("component GID outside maxp"));
                    }
                    let xy = flags & 2 != 0;
                    if !xy && components.is_empty() {
                        return Err(fail("first component must use an offset"));
                    }
                    let arguments = match (flags & 1 != 0, xy) {
                        (true, true) => Arguments::Offset([cursor.signed()?, cursor.signed()?]),
                        (false, true) => Arguments::Offset([
                            i16::from(cursor.byte()? as i8),
                            i16::from(cursor.byte()? as i8),
                        ]),
                        (true, false) => Arguments::Points([cursor.word()?, cursor.word()?]),
                        (false, false) => Arguments::Points([
                            u16::from(cursor.byte()?),
                            u16::from(cursor.byte()?),
                        ]),
                    };
                    let mut matrix = [16384, 0, 0, 16384];
                    if flags & 8 != 0 {
                        let scale = cursor.signed()?;
                        matrix = [scale, 0, 0, scale];
                    } else if flags & 0x40 != 0 {
                        matrix[0] = cursor.signed()?;
                        matrix[3] = cursor.signed()?;
                    } else if flags & 0x80 != 0 {
                        for value in &mut matrix {
                            *value = cursor.signed()?;
                        }
                    }
                    has_instructions |= flags & 0x100 != 0;
                    components.push(Component {
                        flags,
                        glyph,
                        arguments,
                        matrix,
                    });
                    if flags & 0x20 == 0 {
                        break;
                    }
                }
                let instructions = if has_instructions {
                    cursor.instructions(start)?
                } else {
                    start + cursor.at..start + cursor.at
                };
                (Kind::Composite(components), instructions)
            };
            glyphs.push(Glyph {
                kind,
                bounds: Some(bounds),
                instructions,
            });
            start = end;
        }
        budget.charge(0)?;
        Ok(Self { source, glyphs })
    }
    pub fn order(&self, budget: &mut Budget) -> Result<Vec<usize>> {
        fn visit(
            id: usize,
            depth: usize,
            glyphs: &[Glyph],
            states: &mut [u8],
            out: &mut Vec<usize>,
            budget: &mut Budget,
        ) -> Result<()> {
            budget.charge(1)?;
            if states[id] == 2 {
                return Ok(());
            }
            if states[id] == 1 {
                return Err(fail("cyclic component graph"));
            }
            if depth > MAX_DEPTH {
                return Err(WellfriendError::ResourceLimit(
                    "component depth exceeds 64".into(),
                ));
            }
            states[id] = 1;
            if let Kind::Composite(components) = &glyphs[id].kind {
                for c in components {
                    visit(usize::from(c.glyph), depth + 1, glyphs, states, out, budget)?;
                }
            }
            states[id] = 2;
            out.push(id);
            Ok(())
        }
        let mut states = vec![0; self.glyphs.len()];
        let mut out = Vec::with_capacity(states.len());
        for id in 0..states.len() {
            visit(id, 0, &self.glyphs, &mut states, &mut out, budget)?;
        }
        Ok(out)
    }
}

pub(super) fn rectangle(points: &[[f64; 2]]) -> Result<Option<ttf_parser::Rect>> {
    let Some(first) = points.first() else {
        return Ok(None);
    };
    let mut lo = *first;
    let mut hi = *first;
    for (i, p) in points.iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("glyf control bounds")?;
        }
        for a in 0..2 {
            if !p[a].is_finite() {
                return Err(fail("non-finite transformed point"));
            }
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let bound = |n: f64| -> Result<i16> {
        if !(-32768. ..=32767.).contains(&n) {
            Err(fail("glyph bounds outside FWORD range"))
        } else {
            Ok(n as i16)
        }
    };
    Ok(Some(ttf_parser::Rect {
        x_min: bound(lo[0].floor())?,
        y_min: bound(lo[1].floor())?,
        x_max: bound(hi[0].ceil())?,
        y_max: bound(hi[1].ceil())?,
    }))
}
pub(super) fn header(out: &mut Vec<u8>, contours: i16, bounds: Option<ttf_parser::Rect>) {
    let b = bounds.unwrap_or(ttf_parser::Rect {
        x_min: 0,
        y_min: 0,
        x_max: 0,
        y_max: 0,
    });
    for value in [contours, b.x_min, b.y_min, b.x_max, b.y_max] {
        out.extend(value.to_be_bytes());
    }
}
pub(super) fn encode_simple(
    points: &[[i32; 2]],
    flags: &[u8],
    ends: &[u16],
    instructions: &[u8],
    bounds: Option<ttf_parser::Rect>,
    variable: bool,
    budget: &mut Budget,
) -> Result<Vec<u8>> {
    budget.charge(points.len() * 3 + instructions.len())?;
    let mut out = Vec::new();
    header(&mut out, ends.len() as i16, bounds);
    for end in ends {
        out.extend(end.to_be_bytes());
    }
    out.extend((instructions.len() as u16).to_be_bytes());
    out.extend(instructions);
    let mut encoded = Vec::with_capacity(points.len());
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut previous = [0i32; 2];
    let overlap = variable || flags.iter().any(|f| f & 0x40 != 0);
    for (i, point) in points.iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("glyf point emission")?;
        }
        let mut flag = flags[i] & 1;
        if i == 0 && overlap {
            flag |= 0x40;
        }
        for (axis, dest) in [&mut x, &mut y].into_iter().enumerate() {
            let delta = point[axis] - previous[axis];
            previous[axis] = point[axis];
            if delta == 0 {
                flag |= 0x10 << axis;
            } else if (-255..=255).contains(&delta) {
                flag |= 2 << axis;
                if delta > 0 {
                    flag |= 0x10 << axis;
                }
                dest.push(delta.unsigned_abs() as u8);
            } else {
                dest.extend(i16::try_from(delta).map_err(|_| fail("varied point displacement cannot retain 16-bit source point identities"))?.to_be_bytes());
            }
        }
        encoded.push(flag);
    }
    let mut i = 0;
    while i < encoded.len() {
        budget.charge(1)?;
        let mut n = 1;
        while n < 256 && i + n < encoded.len() && encoded[i + n] == encoded[i] {
            n += 1;
        }
        if n == 1 {
            out.push(encoded[i]);
        } else {
            out.extend([encoded[i] | 8, (n - 1) as u8]);
        }
        i += n;
    }
    out.extend(x);
    out.extend(y);
    Ok(out)
}
pub(super) fn encode_composite(
    components: &[Component],
    instructions: &[u8],
    bounds: Option<ttf_parser::Rect>,
    variable: bool,
    budget: &mut Budget,
) -> Result<Vec<u8>> {
    budget.charge(components.len() + instructions.len())?;
    let mut out = Vec::new();
    header(&mut out, -1, bounds);
    for (i, c) in components.iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("glyf component emission")?;
        }
        let mut flags = c.flags & !(1 | 0x20 | 0x100 | 0x400);
        // Word arguments preserve signed offsets and unsigned point identities.
        flags |= 1;
        if i + 1 < components.len() {
            flags |= 0x20;
        }
        if i + 1 == components.len() && !instructions.is_empty() {
            flags |= 0x100;
        }
        if i == 0 && (variable || components.iter().any(|part| part.flags & 0x400 != 0)) {
            flags |= 0x400;
        }
        out.extend(flags.to_be_bytes());
        out.extend(c.glyph.to_be_bytes());
        match c.arguments {
            Arguments::Offset(v) => {
                for n in v {
                    out.extend(n.to_be_bytes());
                }
            }
            Arguments::Points(v) => {
                for n in v {
                    out.extend(n.to_be_bytes());
                }
            }
        }
        if flags & 8 != 0 {
            out.extend(c.matrix[0].to_be_bytes());
        } else if flags & 0x40 != 0 {
            out.extend(c.matrix[0].to_be_bytes());
            out.extend(c.matrix[3].to_be_bytes());
        } else if flags & 0x80 != 0 {
            for n in c.matrix {
                out.extend(n.to_be_bytes());
            }
        }
    }
    if !instructions.is_empty() {
        out.extend((instructions.len() as u16).to_be_bytes());
        out.extend(instructions);
    }
    Ok(out)
}
