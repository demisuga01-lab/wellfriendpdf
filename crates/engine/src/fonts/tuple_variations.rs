//! Bounded shared decoder for cvar and per-glyph gvar tuple stores. Source point
//! identities and duplicate references are retained until their deltas accumulate.
use super::variation_store::{axis_scalar, bytes};
use crate::{Result, WellfriendError};
use std::sync::Arc;

pub(super) const TABLE_LIMIT: usize = 64 * 1024 * 1024;
pub(super) const ITEM_LIMIT: usize = 1_000_000;
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font tuple variations: {message}"))
}
#[derive(Default)]
pub(crate) struct Budget {
    pub work: usize,
}
impl Budget {
    pub fn charge(&mut self, amount: usize) -> Result<()> {
        self.work = self
            .work
            .checked_add(amount)
            .ok_or_else(|| fail("work overflow"))?;
        if self.work > 16_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "tuple variation work exceeds 16 million units".into(),
            ));
        }
        crate::cancel::check_current_cancel("font tuple evaluation")
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Domain<'a> {
    Cvt(usize),
    /// Original explicit contour points, excluding implicit on-curve points and
    /// the four phantom points. Contour endpoints retain source numbering.
    Simple {
        points: &'a [[i32; 2]],
        contour_ends: &'a [u16],
    },
    /// Components are independent positions; no contour interpolation applies.
    Components(usize),
}
impl Domain<'_> {
    pub(super) fn count(self) -> Result<usize> {
        let count = match self {
            Self::Cvt(n) => n,
            Self::Simple {
                points,
                contour_ends,
            } => {
                if points.len() > 65536 {
                    return Err(fail("too many explicit glyph points"));
                }
                let mut start = 0;
                for (index, end) in contour_ends.iter().enumerate() {
                    if index % 256 == 0 {
                        crate::cancel::check_current_cancel("font contour validation")?;
                    }
                    let end = usize::from(*end);
                    if end < start || end >= points.len() {
                        return Err(fail("invalid contour endpoints"));
                    }
                    start = end + 1;
                }
                if start != points.len() {
                    return Err(fail("contours do not cover explicit points"));
                }
                points.len() + 4
            }
            Self::Components(n) => n
                .checked_add(4)
                .ok_or_else(|| fail("component count overflow"))?,
        };
        if count > ITEM_LIMIT {
            return Err(WellfriendError::ResourceLimit(
                "tuple target count exceeds one million".into(),
            ));
        }
        Ok(count)
    }
}
pub(crate) struct Deltas {
    /// Cvt uses element zero only. Glyph domains include the four phantom slots.
    pub values: Vec<[f64; 2]>,
    pub tuples: usize,
    pub active_tuples: usize,
}
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let data = bytes(self.data, self.at, n)?;
        self.at += n;
        Ok(data)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn tuple(&mut self, axes: usize) -> Result<Vec<i16>> {
        let mut result = Vec::with_capacity(axes);
        for value in self.take(axes * 2)?.chunks_exact(2) {
            let value = i16::from_be_bytes(value.try_into().unwrap());
            if !(-16384..=16384).contains(&value) {
                return Err(fail("tuple coordinate outside normalized domain"));
            }
            result.push(value);
        }
        Ok(result)
    }
}
#[derive(Clone)]
enum Points {
    All,
    Listed(Arc<[usize]>),
}
impl Points {
    fn len(&self, total: usize) -> usize {
        match self {
            Self::All => total,
            Self::Listed(v) => v.len(),
        }
    }
    fn at(&self, index: usize) -> usize {
        match self {
            Self::All => index,
            Self::Listed(v) => v[index],
        }
    }
}
fn points(cursor: &mut Cursor<'_>, total: usize, budget: &mut Budget) -> Result<Points> {
    let first = cursor.byte()?;
    let count = if first & 128 != 0 {
        (usize::from(first & 127) << 8) | usize::from(cursor.byte()?)
    } else {
        usize::from(first)
    };
    if count == 0 {
        return Ok(Points::All);
    }
    budget.charge(count)?;
    let mut out = Vec::with_capacity(count);
    let mut previous = 0usize;
    while out.len() < count {
        budget.charge(1)?;
        let control = cursor.byte()?;
        let run = usize::from(control & 127) + 1;
        if run > count - out.len() {
            return Err(fail("point run exceeds declared count"));
        }
        for _ in 0..run {
            let delta = if control & 128 != 0 {
                usize::from(cursor.word()?)
            } else {
                usize::from(cursor.byte()?)
            };
            previous = previous
                .checked_add(delta)
                .ok_or_else(|| fail("point number overflow"))?;
            if previous >= total {
                return Err(fail("point outside target domain"));
            }
            // Zero differences intentionally remain duplicates, not deduplicated.
            out.push(previous);
        }
    }
    Ok(Points::Listed(out.into()))
}
fn deltas(cursor: &mut Cursor<'_>, count: usize, budget: &mut Budget) -> Result<Vec<i16>> {
    budget.charge(count)?;
    let mut result = Vec::with_capacity(count);
    while result.len() < count {
        budget.charge(1)?;
        let control = cursor.byte()?;
        let run = usize::from(control & 63) + 1;
        if run > count - result.len() {
            return Err(fail("delta run exceeds target count"));
        }
        if control & 128 != 0 {
            result.resize(result.len() + run, 0);
        } else if control & 64 != 0 {
            for _ in 0..run {
                result.push(cursor.word()? as i16);
            }
        } else {
            for _ in 0..run {
                result.push(i16::from(cursor.byte()? as i8));
            }
        }
    }
    Ok(result)
}
struct Header {
    length: usize,
    private_points: bool,
    scalar: f64,
}
fn header(
    cursor: &mut Cursor<'_>,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    shared: &[Vec<i16>],
    cvt: bool,
    budget: &mut Budget,
) -> Result<Header> {
    let length = usize::from(cursor.word()?);
    let flags = cursor.word()?;
    if flags & 0x1000 != 0 {
        return Err(fail("reserved tuple-index flag"));
    }
    if cvt && flags & 0x8000 == 0 {
        return Err(fail("cvar requires an embedded peak"));
    }
    budget.charge(1 + coordinates.len() * 3)?;
    let embedded;
    let peak = if flags & 0x8000 != 0 {
        embedded = cursor.tuple(coordinates.len())?;
        &embedded
    } else {
        shared
            .get(usize::from(flags & 0x0fff))
            .ok_or_else(|| fail("shared peak index outside gvar array"))?
    };
    if peak.len() != coordinates.len() || peak.iter().any(|n| !(-16384..=16384).contains(n)) {
        return Err(fail("invalid shared peak dimensions/coordinates"));
    }
    let intermediate = if flags & 0x4000 != 0 {
        Some((
            cursor.tuple(coordinates.len())?,
            cursor.tuple(coordinates.len())?,
        ))
    } else {
        None
    };
    let mut scalar = 1.;
    for (i, value) in coordinates.iter().enumerate() {
        let (start, end) = intermediate
            .as_ref()
            .map_or((peak[i].min(0), peak[i].max(0)), |(a, b)| (a[i], b[i]));
        scalar *= axis_scalar([start, peak[i], end], value.get());
    }
    Ok(Header {
        length,
        private_points: flags & 0x2000 != 0,
        scalar,
    })
}
fn between(value: i32, first: i32, last: i32, first_delta: f64, last_delta: f64) -> f64 {
    if first == last {
        return if first_delta == last_delta {
            first_delta
        } else {
            0.
        };
    }
    if value <= first.min(last) {
        return if first < last {
            first_delta
        } else {
            last_delta
        };
    }
    if value >= first.max(last) {
        return if first > last {
            first_delta
        } else {
            last_delta
        };
    }
    let fraction = (f64::from(value) - f64::from(first)) / (f64::from(last) - f64::from(first));
    first_delta + fraction * (last_delta - first_delta)
}
fn infer(
    points: &[[i32; 2]],
    ends: &[u16],
    touched: &[bool],
    values: &mut [[f64; 2]],
    budget: &mut Budget,
) -> Result<()> {
    budget.charge(points.len() * 2)?;
    let mut start = 0usize;
    for end in ends {
        crate::cancel::check_current_cancel("font contour delta inference")?;
        let end = usize::from(*end);
        let Some(first) = (start..=end).find(|i| touched[*i]) else {
            start = end + 1;
            continue;
        };
        let mut previous = first;
        // Walk the contour once, including its wrap-around segment. Each gap is
        // filled once; do not rescan backwards/forwards for every missing point.
        for following in ((first + 1)..=end)
            .filter(|i| touched[*i])
            .chain(std::iter::once(first))
        {
            let mut i = if previous == end { start } else { previous + 1 };
            while i != following {
                if i % 256 == 0 {
                    crate::cancel::check_current_cancel("font inferred contour points")?;
                }
                for axis in 0..2 {
                    values[i][axis] = between(
                        points[i][axis],
                        points[previous][axis],
                        points[following][axis],
                        values[previous][axis],
                        values[following][axis],
                    );
                }
                i = if i == end { start } else { i + 1 };
            }
            previous = following;
        }
        start = end + 1;
    }
    Ok(())
}
pub(crate) fn resolve(
    data: &[u8],
    coordinates: &[ttf_parser::NormalizedCoordinate],
    shared_peaks: &[Vec<i16>],
    domain: Domain<'_>,
    budget: &mut Budget,
) -> Result<Deltas> {
    budget.charge(1)?;
    if data.len() > TABLE_LIMIT {
        return Err(WellfriendError::ResourceLimit(
            "tuple store exceeds 64 MiB".into(),
        ));
    }
    if coordinates.len() > 64 {
        return Err(fail("more than 64 tuple axes"));
    }
    let count = domain.count()?;
    budget.charge(count)?;
    let cvt = matches!(domain, Domain::Cvt(_));
    let mut cursor = Cursor { data, at: 0 };
    if cvt && (cursor.word()? != 1 || cursor.word()? != 0) {
        return Err(fail("unsupported cvar version"));
    }
    let flags = cursor.word()?;
    if flags & 0x7000 != 0 {
        return Err(fail("reserved tuple-count flags"));
    }
    let data_offset = usize::from(cursor.word()?);
    let tuples = usize::from(flags & 0x0fff);
    let mut headers = Vec::with_capacity(tuples);
    for _ in 0..tuples {
        headers.push(header(&mut cursor, coordinates, shared_peaks, cvt, budget)?);
    }
    if data_offset < cursor.at || data_offset > data.len() {
        return Err(fail("tuple payload overlaps headers or exceeds source"));
    }
    cursor.at = data_offset;
    let shared_points = if flags & 0x8000 != 0 {
        points(&mut cursor, count, budget)?
    } else {
        Points::All
    };
    let mut result = vec![[0.; 2]; count];
    let mut active = 0usize;
    for header in headers {
        let payload = cursor.take(header.length)?;
        let mut tuple = Cursor {
            data: payload,
            at: 0,
        };
        let selected = if header.private_points {
            points(&mut tuple, count, budget)?
        } else {
            shared_points.clone()
        };
        let entries = selected.len(count);
        let x = deltas(&mut tuple, entries, budget)?;
        let y = if cvt {
            Vec::new()
        } else {
            deltas(&mut tuple, entries, budget)?
        };
        // The enclosing range is authoritative. Padding remains outside the
        // expected runs; it is never treated as the next tuple's payload.
        if header.scalar == 0. {
            continue;
        }
        active += 1;
        budget.charge(entries)?;
        if let (
            Domain::Simple {
                points,
                contour_ends,
            },
            Points::Listed(_),
        ) = (domain, &selected)
        {
            budget.charge(count * 2)?;
            let mut values = vec![[0.; 2]; count];
            let mut touched = vec![false; count];
            for i in 0..entries {
                if i % 256 == 0 {
                    crate::cancel::check_current_cancel("font explicit tuple deltas")?;
                }
                let at = selected.at(i);
                touched[at] = true;
                values[at][0] += f64::from(x[i]);
                values[at][1] += f64::from(y[i]);
            }
            infer(points, contour_ends, &touched, &mut values, budget)?;
            for (i, (target, delta)) in result.iter_mut().zip(values).enumerate() {
                if i % 256 == 0 {
                    crate::cancel::check_current_cancel("font tuple accumulation")?;
                }
                for axis in 0..2 {
                    target[axis] += header.scalar * delta[axis];
                }
            }
        } else {
            for i in 0..entries {
                if i % 256 == 0 {
                    crate::cancel::check_current_cancel("font tuple accumulation")?;
                }
                let target = &mut result[selected.at(i)];
                target[0] += header.scalar * f64::from(x[i]);
                if !cvt {
                    target[1] += header.scalar * f64::from(y[i]);
                }
            }
        }
    }
    budget.charge(0)?;
    Ok(Deltas {
        values: result,
        tuples,
        active_tuples: active,
    })
}

#[cfg(test)]
#[path = "tuple_variations_tests.rs"]
mod tests;
