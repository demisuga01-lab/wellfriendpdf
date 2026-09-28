//! sfnt/TTC directory parsing for explicit font-face preparation. Table offsets
//! are file-relative, including inside collections; shared tables stay shared
//! until one selected face is materialized. This is not a font sanitizer.
use crate::{Result, WellfriendError};
use std::collections::BTreeMap;
use std::ops::Range;

pub(super) const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_FACES: usize = 256;
const MAX_TABLES: usize = 256;

pub(super) struct Directory {
    pub version: [u8; 4],
    pub tables: BTreeMap<[u8; 4], Range<usize>>,
}
pub(super) struct Container {
    pub collection: bool,
    pub collection_signature: bool,
    pub faces: Vec<Directory>,
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font container: {message}"))
}
fn slice(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| fail("offset overflow"))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| fail("truncated or out-of-range data"))
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(slice(bytes, at, 2)?.try_into().unwrap()))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(slice(bytes, at, 4)?.try_into().unwrap()))
}
fn extent(bytes: &[u8], offset: u32, len: u32) -> Result<Range<usize>> {
    let offset = usize::try_from(offset).map_err(|_| fail("offset outside address space"))?;
    let len = usize::try_from(len).map_err(|_| fail("length outside address space"))?;
    slice(bytes, offset, len)?;
    Ok(offset..offset + len)
}
fn overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    !a.is_empty() && !b.is_empty() && a.start < b.end && b.start < a.end
}

impl Container {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        crate::cancel::check_current_cancel("font container directories")?;
        if bytes.len() > MAX_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "font input exceeds 256 MiB".into(),
            ));
        }
        let collection = bytes.starts_with(b"ttcf");
        let mut reserved = Vec::new();
        let mut signature = None;
        let offsets = if collection {
            let version = u32_at(bytes, 4)?;
            if !matches!(version, 0x00010000 | 0x00020000) {
                return Err(fail("unsupported TTC header version"));
            }
            let count = u32_at(bytes, 8)? as usize;
            if count == 0 || count > MAX_FACES {
                return Err(WellfriendError::ResourceLimit(
                    "font collection requires 1..=256 faces".into(),
                ));
            }
            let end = 12 + count * 4;
            let header_end = end + if version == 0x00020000 { 12 } else { 0 };
            slice(bytes, 0, header_end)?;
            reserved.push(0..header_end);
            if version == 0x00020000 {
                let tag = u32_at(bytes, end)?;
                let len = u32_at(bytes, end + 4)?;
                let offset = u32_at(bytes, end + 8)?;
                if (tag, len, offset) != (0, 0, 0) {
                    if tag != u32::from_be_bytes(*b"DSIG") || len == 0 || offset % 4 != 0 {
                        return Err(fail("invalid collection DSIG declaration"));
                    }
                    signature = Some(extent(bytes, offset, len)?);
                }
            }
            (0..count)
                .map(|i| u32_at(bytes, 12 + i * 4).map(|n| n as usize))
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![0]
        };
        let mut faces = Vec::with_capacity(offsets.len());
        for offset in offsets {
            crate::cancel::check_current_cancel("font face directory")?;
            if offset % 4 != 0 {
                return Err(fail("unaligned face directory"));
            }
            let header = slice(bytes, offset, 12)?;
            let version: [u8; 4] = header[..4].try_into().unwrap();
            if !matches!(&version, b"\0\x01\0\0" | b"true" | b"OTTO") {
                return Err(fail("expected sfnt TrueType or OpenType outlines"));
            }
            let count = usize::from(u16_at(bytes, offset + 4)?);
            if count == 0 || count > MAX_TABLES {
                return Err(WellfriendError::ResourceLimit(
                    "font face requires 1..=256 tables".into(),
                ));
            }
            let directory_len = 12 + count * 16;
            slice(bytes, offset, directory_len)?;
            reserved.push(offset..offset + directory_len);
            let mut tables = BTreeMap::new();
            for i in 0..count {
                let at = offset + 12 + i * 16;
                let tag: [u8; 4] = slice(bytes, at, 4)?.try_into().unwrap();
                let range = extent(bytes, u32_at(bytes, at + 8)?, u32_at(bytes, at + 12)?)?;
                if !range.is_empty() && range.start % 4 != 0 {
                    return Err(fail("unaligned font table"));
                }
                if tables.insert(tag, range).is_some() {
                    return Err(fail("duplicate table tag in one face"));
                }
            }
            faces.push(Directory { version, tables });
        }
        reserved.sort_by_key(|range| (range.start, range.end));
        // Two collection indices may alias the exact same face directory.
        // Partial overlaps remain ambiguous; exact aliasing needs no repair.
        reserved.dedup();
        if reserved.windows(2).any(|pair| overlap(&pair[0], &pair[1])) {
            return Err(fail("overlapping collection header or face directories"));
        }
        // Distinct table ranges can legally overlap in interleaved sfnt data.
        // Do not impose a disjoint-table rule; bound materialization separately.
        // A table must never reinterpret a directory/header as font program data.
        for face in &faces {
            crate::cancel::check_current_cancel("font directory ownership")?;
            for range in face.tables.values().filter(|r| !r.is_empty()) {
                let index = reserved.partition_point(|r| r.end <= range.start);
                if reserved.get(index).is_some_and(|r| overlap(r, range)) {
                    return Err(fail("table overlaps a font directory or collection header"));
                }
                if signature.as_ref().is_some_and(|r| overlap(r, range)) {
                    return Err(fail("collection DSIG overlaps a face table"));
                }
            }
        }
        if let Some(range) = &signature {
            if reserved.iter().any(|r| overlap(r, range)) {
                return Err(fail("collection DSIG overlaps a directory"));
            }
        }
        Ok(Self {
            collection,
            collection_signature: signature.is_some(),
            faces,
        })
    }

    pub fn extract(&self, bytes: &[u8], index: usize, output_limit: usize) -> Result<Vec<u8>> {
        let face = self
            .faces
            .get(index)
            .ok_or_else(|| fail("face index outside collection"))?;
        let mut size = 12 + face.tables.len() * 16;
        for (tag, range) in &face.tables {
            if tag == b"DSIG" {
                continue;
            }
            size = size
                .checked_add(range.len() + 3)
                .ok_or_else(|| fail("expanded size overflow"))?;
            if size > output_limit.min(MAX_BYTES) {
                return Err(WellfriendError::ResourceLimit(
                    "selected font exceeds the preparation output budget".into(),
                ));
            }
        }
        let mut tables = BTreeMap::new();
        for (tag, range) in &face.tables {
            if tag == b"DSIG" {
                continue;
            }
            let mut table = Vec::with_capacity(range.len());
            for chunk in bytes[range.clone()].chunks(64 * 1024) {
                crate::cancel::check_current_cancel("font face table materialization")?;
                table.extend_from_slice(chunk);
            }
            tables.insert(*tag, table);
        }
        let output = super::sfnt_subset::build_sfnt(face.version, tables)
            .map_err(|error| fail(&format!("cannot rebuild selected face: {error}")))?;
        crate::cancel::check_current_cancel("font face checksum publication")?;
        Ok(output)
    }
}
