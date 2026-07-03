use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SfntSubset {
    pub bytes: Vec<u8>,
    pub metrics: SfntSubsetMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SfntSubsetMetrics {
    pub original_bytes: usize,
    pub subset_bytes: usize,
    pub glyphs_requested: usize,
    pub glyphs_embedded: usize,
    pub strategy: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SfntSubsetError {
    Unsupported(&'static str),
    Malformed(String),
    ResourceLimit(&'static str),
}

impl SfntSubsetError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "font.subset.fallback.unsupported_format",
            Self::Malformed(_) => "font.subset.fallback.malformed_glyf",
            Self::ResourceLimit(_) => "font.subset.fallback.resource_limit",
        }
    }
}

impl fmt::Display for SfntSubsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => f.write_str(reason),
            Self::Malformed(reason) => f.write_str(reason),
            Self::ResourceLimit(reason) => f.write_str(reason),
        }
    }
}

#[derive(Debug, Clone)]
struct TableRecord {
    tag: [u8; 4],
    offset: usize,
    length: usize,
}

#[derive(Debug)]
struct SfntTables<'a> {
    version: [u8; 4],
    tables: BTreeMap<[u8; 4], &'a [u8]>,
}

const TAG_HEAD: &[u8; 4] = b"head";
const TAG_MAXP: &[u8; 4] = b"maxp";
const TAG_LOCA: &[u8; 4] = b"loca";
const TAG_GLYF: &[u8; 4] = b"glyf";
const TAG_DSIG: &[u8; 4] = b"DSIG";
const CHECKSUM_MAGIC: u32 = 0xB1B0_AFBA;
const MAX_GLYPHS: usize = 65_535;
const MAX_COMPOSITE_DEPTH: usize = 32;
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;

pub(crate) fn subset_glyf_preserving_gids(
    font_bytes: &[u8],
    requested_glyphs: &BTreeSet<u16>,
) -> Result<SfntSubset, SfntSubsetError> {
    let sfnt = parse_sfnt(font_bytes)?;
    let head = required_table(&sfnt, TAG_HEAD)?;
    let maxp = required_table(&sfnt, TAG_MAXP)?;
    let loca = required_table(&sfnt, TAG_LOCA)?;
    let glyf = required_table(&sfnt, TAG_GLYF)?;

    if head.len() < 54 {
        return Err(SfntSubsetError::Malformed(
            "head table is too short".to_string(),
        ));
    }
    if maxp.len() < 6 {
        return Err(SfntSubsetError::Malformed(
            "maxp table is too short".to_string(),
        ));
    }

    let glyph_count = usize::from(read_u16(maxp, 4)?);
    if glyph_count == 0 || glyph_count > MAX_GLYPHS {
        return Err(SfntSubsetError::ResourceLimit("invalid glyph count"));
    }

    let index_to_loca_format = read_i16(head, 50)?;
    let original_offsets = parse_loca(loca, glyph_count, index_to_loca_format, glyf.len())?;
    let mut included = BTreeSet::new();
    included.insert(0);
    for gid in requested_glyphs {
        let gid_usize = usize::from(*gid);
        if gid_usize >= glyph_count {
            return Err(SfntSubsetError::Malformed(format!(
                "requested glyph id {gid} exceeds maxp glyph count {glyph_count}"
            )));
        }
        collect_glyph_dependencies(*gid, &original_offsets, glyf, &mut included, 0)?;
    }

    let (new_glyf, new_offsets) = rebuild_glyf(glyf, &original_offsets, &included);
    let (new_loca, new_loca_format) = build_loca(&new_offsets, index_to_loca_format)?;

    let mut rebuilt = BTreeMap::new();
    for (tag, data) in &sfnt.tables {
        if tag == TAG_DSIG {
            continue;
        }
        let value = match tag {
            TAG_HEAD => patch_head(data, new_loca_format)?,
            TAG_LOCA => new_loca.clone(),
            TAG_GLYF => new_glyf.clone(),
            _ => data.to_vec(),
        };
        rebuilt.insert(*tag, value);
    }

    let bytes = build_sfnt(sfnt.version, rebuilt)?;
    Ok(SfntSubset {
        metrics: SfntSubsetMetrics {
            original_bytes: font_bytes.len(),
            subset_bytes: bytes.len(),
            glyphs_requested: requested_glyphs.len(),
            glyphs_embedded: included.len(),
            strategy: "preserve-gids-prune-glyf",
        },
        bytes,
    })
}

fn parse_sfnt(bytes: &[u8]) -> Result<SfntTables<'_>, SfntSubsetError> {
    if bytes.len() < 12 {
        return Err(SfntSubsetError::Malformed(
            "sfnt header is too short".to_string(),
        ));
    }
    let version = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if version == *b"ttcf" {
        return Err(SfntSubsetError::Unsupported(
            "TrueType collections are not subset by this writer",
        ));
    }
    if version == *b"OTTO" {
        return Err(SfntSubsetError::Unsupported(
            "CFF/OpenType fonts require a CFF subset writer",
        ));
    }
    if version != [0x00, 0x01, 0x00, 0x00] && version != *b"true" {
        return Err(SfntSubsetError::Unsupported("unsupported sfnt scaler type"));
    }

    let num_tables = usize::from(read_u16(bytes, 4)?);
    if num_tables == 0 || num_tables > 256 {
        return Err(SfntSubsetError::ResourceLimit("invalid sfnt table count"));
    }
    let directory_len = 12usize
        .checked_add(
            num_tables
                .checked_mul(16)
                .ok_or(SfntSubsetError::ResourceLimit(
                    "sfnt table directory overflow",
                ))?,
        )
        .ok_or(SfntSubsetError::ResourceLimit(
            "sfnt table directory overflow",
        ))?;
    if bytes.len() < directory_len {
        return Err(SfntSubsetError::Malformed(
            "sfnt table directory is truncated".to_string(),
        ));
    }

    let mut records = Vec::with_capacity(num_tables);
    for idx in 0..num_tables {
        let pos = 12 + idx * 16;
        let tag = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
        let offset = usize::try_from(read_u32(bytes, pos + 8)?)
            .map_err(|_| SfntSubsetError::ResourceLimit("table offset overflow"))?;
        let length = usize::try_from(read_u32(bytes, pos + 12)?)
            .map_err(|_| SfntSubsetError::ResourceLimit("table length overflow"))?;
        let end = offset
            .checked_add(length)
            .ok_or(SfntSubsetError::ResourceLimit("table end overflow"))?;
        if offset > bytes.len() || end > bytes.len() {
            return Err(SfntSubsetError::Malformed(format!(
                "table {} points outside the font",
                tag_name(&tag)
            )));
        }
        records.push(TableRecord {
            tag,
            offset,
            length,
        });
    }

    let mut tables = BTreeMap::new();
    for record in records {
        tables.insert(
            record.tag,
            &bytes[record.offset..record.offset + record.length],
        );
    }
    Ok(SfntTables { version, tables })
}

fn required_table<'a>(
    sfnt: &'a SfntTables<'a>,
    tag: &[u8; 4],
) -> Result<&'a [u8], SfntSubsetError> {
    sfnt.tables.get(tag).copied().ok_or({
        SfntSubsetError::Unsupported(match tag {
            TAG_HEAD => "required head table is missing",
            TAG_MAXP => "required maxp table is missing",
            TAG_LOCA => "required loca table is missing",
            TAG_GLYF => "required glyf table is missing",
            _ => "required table is missing",
        })
    })
}

fn parse_loca(
    loca: &[u8],
    glyph_count: usize,
    format: i16,
    glyf_len: usize,
) -> Result<Vec<usize>, SfntSubsetError> {
    let count = glyph_count
        .checked_add(1)
        .ok_or(SfntSubsetError::ResourceLimit("loca count overflow"))?;
    let mut offsets = Vec::with_capacity(count);
    match format {
        0 => {
            let required = count
                .checked_mul(2)
                .ok_or(SfntSubsetError::ResourceLimit("loca size overflow"))?;
            if loca.len() < required {
                return Err(SfntSubsetError::Malformed(
                    "short loca table is truncated".to_string(),
                ));
            }
            for idx in 0..count {
                offsets.push(usize::from(read_u16(loca, idx * 2)?) * 2);
            }
        }
        1 => {
            let required = count
                .checked_mul(4)
                .ok_or(SfntSubsetError::ResourceLimit("loca size overflow"))?;
            if loca.len() < required {
                return Err(SfntSubsetError::Malformed(
                    "long loca table is truncated".to_string(),
                ));
            }
            for idx in 0..count {
                offsets.push(
                    usize::try_from(read_u32(loca, idx * 4)?)
                        .map_err(|_| SfntSubsetError::ResourceLimit("loca offset overflow"))?,
                );
            }
        }
        _ => {
            return Err(SfntSubsetError::Malformed(format!(
                "unsupported indexToLocFormat {format}"
            )))
        }
    }

    let mut previous = 0;
    for offset in &offsets {
        if *offset < previous || *offset > glyf_len {
            return Err(SfntSubsetError::Malformed(
                "loca offsets are out of range or not monotonic".to_string(),
            ));
        }
        previous = *offset;
    }
    Ok(offsets)
}

fn collect_glyph_dependencies(
    gid: u16,
    offsets: &[usize],
    glyf: &[u8],
    included: &mut BTreeSet<u16>,
    depth: usize,
) -> Result<(), SfntSubsetError> {
    if depth > MAX_COMPOSITE_DEPTH {
        return Err(SfntSubsetError::Malformed(
            "composite glyph dependency depth exceeded".to_string(),
        ));
    }
    if !included.insert(gid) && depth > 0 {
        return Ok(());
    }
    let glyph = glyph_slice(glyf, offsets, gid)?;
    if glyph.is_empty() {
        return Ok(());
    }
    if glyph.len() < 10 {
        return Err(SfntSubsetError::Malformed(
            "glyf record is too short".to_string(),
        ));
    }
    let contours = read_i16(glyph, 0)?;
    if contours >= 0 {
        return Ok(());
    }

    let mut cursor = 10usize;
    loop {
        if cursor.checked_add(4).is_none_or(|end| end > glyph.len()) {
            return Err(SfntSubsetError::Malformed(
                "composite glyph component is truncated".to_string(),
            ));
        }
        let flags = read_u16(glyph, cursor)?;
        let component_gid = read_u16(glyph, cursor + 2)?;
        cursor += 4;
        collect_glyph_dependencies(component_gid, offsets, glyf, included, depth + 1)?;

        cursor += if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            4
        } else {
            2
        };
        if flags & WE_HAVE_A_SCALE != 0 {
            cursor += 2;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            cursor += 4;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            cursor += 8;
        }
        if cursor > glyph.len() {
            return Err(SfntSubsetError::Malformed(
                "composite glyph transform is truncated".to_string(),
            ));
        }
        if flags & MORE_COMPONENTS == 0 {
            if flags & WE_HAVE_INSTRUCTIONS != 0 {
                if cursor.checked_add(2).is_none_or(|end| end > glyph.len()) {
                    return Err(SfntSubsetError::Malformed(
                        "composite glyph instruction length is truncated".to_string(),
                    ));
                }
                let instruction_len = usize::from(read_u16(glyph, cursor)?);
                cursor += 2;
                if cursor
                    .checked_add(instruction_len)
                    .is_none_or(|end| end > glyph.len())
                {
                    return Err(SfntSubsetError::Malformed(
                        "composite glyph instructions are truncated".to_string(),
                    ));
                }
            }
            break;
        }
    }
    Ok(())
}

fn glyph_slice<'a>(
    glyf: &'a [u8],
    offsets: &[usize],
    gid: u16,
) -> Result<&'a [u8], SfntSubsetError> {
    let idx = usize::from(gid);
    let Some((&start, &end)) = offsets.get(idx).zip(offsets.get(idx + 1)) else {
        return Err(SfntSubsetError::Malformed(format!(
            "glyph id {gid} is outside loca range"
        )));
    };
    if start > end || end > glyf.len() {
        return Err(SfntSubsetError::Malformed(
            "glyph offset is outside glyf table".to_string(),
        ));
    }
    Ok(&glyf[start..end])
}

fn rebuild_glyf(glyf: &[u8], offsets: &[usize], included: &BTreeSet<u16>) -> (Vec<u8>, Vec<usize>) {
    let glyph_count = offsets.len() - 1;
    let mut new_glyf = Vec::new();
    let mut new_offsets = Vec::with_capacity(offsets.len());
    for gid in 0..glyph_count {
        align4(&mut new_glyf);
        new_offsets.push(new_glyf.len());
        if included.contains(&(gid as u16)) {
            let start = offsets[gid];
            let end = offsets[gid + 1];
            new_glyf.extend_from_slice(&glyf[start..end]);
        }
    }
    align4(&mut new_glyf);
    new_offsets.push(new_glyf.len());
    (new_glyf, new_offsets)
}

fn build_loca(offsets: &[usize], original_format: i16) -> Result<(Vec<u8>, i16), SfntSubsetError> {
    let can_short = offsets
        .iter()
        .all(|offset| offset % 2 == 0 && *offset <= usize::from(u16::MAX) * 2);
    let format = if original_format == 0 && can_short {
        0
    } else {
        1
    };
    let mut loca = Vec::new();
    if format == 0 {
        loca.reserve(offsets.len() * 2);
        for offset in offsets {
            let value = u16::try_from(offset / 2)
                .map_err(|_| SfntSubsetError::ResourceLimit("short loca offset overflow"))?;
            write_u16(&mut loca, value);
        }
    } else {
        loca.reserve(offsets.len() * 4);
        for offset in offsets {
            let value = u32::try_from(*offset)
                .map_err(|_| SfntSubsetError::ResourceLimit("long loca offset overflow"))?;
            write_u32(&mut loca, value);
        }
    }
    Ok((loca, format))
}

fn patch_head(head: &[u8], loca_format: i16) -> Result<Vec<u8>, SfntSubsetError> {
    if head.len() < 54 {
        return Err(SfntSubsetError::Malformed(
            "head table is too short".to_string(),
        ));
    }
    let mut out = head.to_vec();
    out[8..12].fill(0);
    out[50..52].copy_from_slice(&loca_format.to_be_bytes());
    Ok(out)
}

fn build_sfnt(
    version: [u8; 4],
    tables: BTreeMap<[u8; 4], Vec<u8>>,
) -> Result<Vec<u8>, SfntSubsetError> {
    let num_tables = u16::try_from(tables.len())
        .map_err(|_| SfntSubsetError::ResourceLimit("too many sfnt tables"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&version);
    write_u16(&mut out, num_tables);
    let (search_range, entry_selector, range_shift) = search_params(num_tables);
    write_u16(&mut out, search_range);
    write_u16(&mut out, entry_selector);
    write_u16(&mut out, range_shift);

    let directory_start = out.len();
    out.resize(out.len() + usize::from(num_tables) * 16, 0);

    let mut records = Vec::new();
    for (tag, data) in tables {
        align4(&mut out);
        let offset = u32::try_from(out.len())
            .map_err(|_| SfntSubsetError::ResourceLimit("sfnt offset overflow"))?;
        let length = u32::try_from(data.len())
            .map_err(|_| SfntSubsetError::ResourceLimit("sfnt table length overflow"))?;
        out.extend_from_slice(&data);
        let checksum = table_checksum(&data);
        records.push((tag, checksum, offset, length));
    }
    align4(&mut out);

    let mut head_table_offset = None;
    for (idx, (tag, checksum, offset, length)) in records.iter().enumerate() {
        let pos = directory_start + idx * 16;
        out[pos..pos + 4].copy_from_slice(tag);
        out[pos + 4..pos + 8].copy_from_slice(&checksum.to_be_bytes());
        out[pos + 8..pos + 12].copy_from_slice(&offset.to_be_bytes());
        out[pos + 12..pos + 16].copy_from_slice(&length.to_be_bytes());
        if tag == TAG_HEAD {
            head_table_offset = Some(
                usize::try_from(*offset)
                    .map_err(|_| SfntSubsetError::ResourceLimit("head table offset overflow"))?,
            );
        }
    }

    let Some(head_offset) = head_table_offset else {
        return Err(SfntSubsetError::Malformed(
            "rebuilt sfnt is missing head table".to_string(),
        ));
    };
    if head_offset
        .checked_add(12)
        .is_none_or(|end| end > out.len())
    {
        return Err(SfntSubsetError::Malformed(
            "rebuilt head offset is invalid".to_string(),
        ));
    }
    out[head_offset + 8..head_offset + 12].fill(0);
    let adjustment = CHECKSUM_MAGIC.wrapping_sub(table_checksum(&out));
    out[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    Ok(out)
}

fn search_params(num_tables: u16) -> (u16, u16, u16) {
    let mut max_power = 1u16;
    let mut selector = 0u16;
    while max_power.saturating_mul(2) <= num_tables {
        max_power *= 2;
        selector += 1;
    }
    let search_range = max_power * 16;
    let range_shift = num_tables * 16 - search_range;
    (search_range, selector, range_shift)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, SfntSubsetError> {
    if offset.checked_add(2).is_none_or(|end| end > data.len()) {
        return Err(SfntSubsetError::Malformed(
            "u16 read is out of bounds".to_string(),
        ));
    }
    Ok(u16::from_be_bytes([data[offset], data[offset + 1]]))
}

fn read_i16(data: &[u8], offset: usize) -> Result<i16, SfntSubsetError> {
    Ok(read_u16(data, offset)? as i16)
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, SfntSubsetError> {
    if offset.checked_add(4).is_none_or(|end| end > data.len()) {
        return Err(SfntSubsetError::Malformed(
            "u32 read is out of bounds".to_string(),
        ));
    }
    Ok(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn align4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn tag_name(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct NoopOutlineBuilder;

    impl ttf_parser::OutlineBuilder for NoopOutlineBuilder {
        fn move_to(&mut self, _x: f32, _y: f32) {}

        fn line_to(&mut self, _x: f32, _y: f32) {}

        fn quad_to(&mut self, _x1: f32, _y1: f32, _x: f32, _y: f32) {}

        fn curve_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _x: f32, _y: f32) {}

        fn close(&mut self) {}
    }

    fn test_font() -> &'static [u8] {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf")).as_slice()
    }

    fn glyph_id(ch: char) -> u16 {
        ttf_parser::Face::parse(test_font(), 0)
            .unwrap()
            .glyph_index(ch)
            .unwrap()
            .0
    }

    #[test]
    fn subsets_simple_glyf_font_and_preserves_requested_glyph() {
        let gid_h = glyph_id('H');
        let gid_e = glyph_id('e');
        let mut requested = BTreeSet::new();
        requested.insert(gid_h);
        requested.insert(gid_e);

        let subset = subset_glyf_preserving_gids(test_font(), &requested).unwrap();
        assert!(subset.bytes.len() < test_font().len());
        assert_eq!(subset.metrics.glyphs_requested, 2);
        assert!(subset.metrics.glyphs_embedded >= 3); // includes .notdef

        let face = ttf_parser::Face::parse(&subset.bytes, 0).unwrap();
        let mapped = face.glyph_index('H').unwrap();
        assert_eq!(mapped.0, gid_h);
        let mut builder = NoopOutlineBuilder;
        assert!(face.outline_glyph(mapped, &mut builder).is_some());
    }

    #[test]
    fn output_is_deterministic() {
        let mut requested = BTreeSet::new();
        requested.insert(glyph_id('O'));
        requested.insert(glyph_id('x'));
        let a = subset_glyf_preserving_gids(test_font(), &requested).unwrap();
        let b = subset_glyf_preserving_gids(test_font(), &requested).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.metrics, b.metrics);
    }

    #[test]
    fn composite_dependencies_are_included() {
        let sfnt = parse_sfnt(test_font()).unwrap();
        let head = required_table(&sfnt, TAG_HEAD).unwrap();
        let maxp = required_table(&sfnt, TAG_MAXP).unwrap();
        let loca = required_table(&sfnt, TAG_LOCA).unwrap();
        let glyf = required_table(&sfnt, TAG_GLYF).unwrap();
        let glyph_count = usize::from(read_u16(maxp, 4).unwrap());
        let offsets =
            parse_loca(loca, glyph_count, read_i16(head, 50).unwrap(), glyf.len()).unwrap();
        let composite_gid = (1..glyph_count as u16)
            .find(|gid| {
                glyph_slice(glyf, &offsets, *gid)
                    .ok()
                    .filter(|glyph| glyph.len() >= 10)
                    .and_then(|glyph| read_i16(glyph, 0).ok())
                    .is_some_and(|contours| contours < 0)
            })
            .expect("test font should contain at least one composite glyph");

        let mut requested = BTreeSet::new();
        requested.insert(composite_gid);
        let subset = subset_glyf_preserving_gids(test_font(), &requested).unwrap();
        assert!(
            subset.metrics.glyphs_embedded > subset.metrics.glyphs_requested + 1,
            "composite dependencies should add component glyphs"
        );
    }

    #[test]
    fn rejects_non_sfnt_bytes_cleanly() {
        let err = subset_glyf_preserving_gids(b"not a font", &BTreeSet::new()).unwrap_err();
        assert!(matches!(err, SfntSubsetError::Malformed(_)));
    }
}
