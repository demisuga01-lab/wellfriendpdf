//! Generated regression fonts, not downloaded/vendor font files. Unexecuted.
use std::collections::BTreeMap;

fn index(entries: &[&[u8]]) -> Vec<u8> {
    let mut out = (entries.len() as u16).to_be_bytes().to_vec();
    if entries.is_empty() {
        return out;
    }
    out.push(4);
    let mut offset = 1u32;
    out.extend_from_slice(&offset.to_be_bytes());
    for entry in entries {
        offset += entry.len() as u32;
        out.extend_from_slice(&offset.to_be_bytes());
    }
    for entry in entries {
        out.extend_from_slice(entry);
    }
    out
}
fn number(out: &mut Vec<u8>, n: u32) {
    out.push(29);
    out.extend_from_slice(&n.to_be_bytes());
}

pub(crate) fn cff(keyed: bool) -> Vec<u8> {
    let name = index(&[b"WFCffFixture"]);
    let strings = if keyed {
        index(&[b"Wellfriend", b"FixtureCids"])
    } else {
        index(&[])
    };
    let global = index(&[]);
    let charset = if keyed {
        vec![0, 0, 42, 0, 7, 3, 232]
    } else {
        vec![0, 0, 34, 0, 35, 0, 1]
    };
    let a = [139, 139, 21, 239, 139, 139, 239, 39, 139, 139, 39, 5, 14];
    let b = [139, 139, 21, 189, 139, 139, 239, 89, 139, 139, 39, 5, 14];
    let charstrings = index(&[&[14], &a, &b, &[14]]);
    let fdarray = index(&[&[]]);
    let fdselect = [0, 0, 0, 0, 0];
    let top = |charset: u32, chars: u32, fd: u32, select: u32| {
        let mut dict = Vec::new();
        if keyed {
            number(&mut dict, 391);
            number(&mut dict, 392);
            number(&mut dict, 0);
            dict.extend_from_slice(&[12, 30]);
        }
        number(&mut dict, charset);
        dict.push(15);
        number(&mut dict, chars);
        dict.push(17);
        if keyed {
            number(&mut dict, fd);
            dict.extend_from_slice(&[12, 36]);
            number(&mut dict, select);
            dict.extend_from_slice(&[12, 37]);
        }
        index(&[&dict])
    };
    let base = 4 + name.len() + top(0, 0, 0, 0).len() + strings.len() + global.len();
    let chars = base + charset.len();
    let fd = chars + charstrings.len();
    let select = fd + fdarray.len();
    let mut out = vec![1, 0, 4, 4];
    out.extend(name);
    out.extend(top(base as u32, chars as u32, fd as u32, select as u32));
    out.extend(strings);
    out.extend(global);
    out.extend(charset);
    out.extend(charstrings);
    if keyed {
        out.extend(fdarray);
        out.extend(fdselect);
    }
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes
        .chunks(4)
        .map(|chunk| {
            let mut value = [0; 4];
            value[..chunk.len()].copy_from_slice(chunk);
            u32::from_be_bytes(value)
        })
        .fold(0u32, u32::wrapping_add)
}

/// Synthetic rights variant with directory/table checksums repaired.
pub(crate) fn with_rights(mut bytes: Vec<u8>, rights: u16) -> Vec<u8> {
    let count = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let mut head = None;
    for index in 0..count {
        let entry = 12 + index * 16;
        let offset = u32::from_be_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(bytes[entry + 12..entry + 16].try_into().unwrap()) as usize;
        if &bytes[entry..entry + 4] == b"OS/2" {
            bytes[offset + 8..offset + 10].copy_from_slice(&rights.to_be_bytes());
            let sum = checksum(&bytes[offset..offset + length]);
            bytes[entry + 4..entry + 8].copy_from_slice(&sum.to_be_bytes());
        } else if &bytes[entry..entry + 4] == b"head" {
            head = Some(offset);
        }
    }
    let offset = head.unwrap();
    bytes[offset + 8..offset + 12].fill(0);
    let sum = 0xB1B0AFBAu32.wrapping_sub(checksum(&bytes));
    bytes[offset + 8..offset + 12].copy_from_slice(&sum.to_be_bytes());
    bytes
}

pub(crate) fn font(keyed: bool, rights: u16) -> Vec<u8> {
    let template = crate::render::get_fallback_font("Symbol").unwrap();
    let face = ttf_parser::Face::parse(template, 0).unwrap();
    let mut tables = BTreeMap::new();
    let mut head = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"head"))
        .unwrap()
        .to_vec();
    head[8..12].fill(0);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[36..44].copy_from_slice(&[0, 0, 0, 0, 0, 100, 0, 100]);
    tables.insert(*b"head", head);
    let mut os2 = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"OS/2"))
        .unwrap()
        .to_vec();
    os2[8..10].copy_from_slice(&rights.to_be_bytes());
    tables.insert(*b"OS/2", os2);
    let mut hhea = vec![0; 36];
    hhea[..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[10..12].copy_from_slice(&600u16.to_be_bytes());
    hhea[18..20].copy_from_slice(&1u16.to_be_bytes());
    hhea[34..36].copy_from_slice(&4u16.to_be_bytes());
    tables.insert(*b"hhea", hhea);
    let hmtx = [600u16, 600, 600, 300]
        .into_iter()
        .flat_map(|w| [w.to_be_bytes(), [0, 0]].concat())
        .collect();
    tables.insert(*b"hmtx", hmtx);
    tables.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 4]);
    let mut post = vec![0; 32];
    post[..4].copy_from_slice(&0x00030000u32.to_be_bytes());
    tables.insert(*b"post", post);
    let ps = "WFCffFixture"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect::<Vec<_>>();
    let mut name = vec![0, 0, 0, 1, 0, 18, 0, 3, 0, 1, 4, 9, 0, 6];
    name.extend_from_slice(&(ps.len() as u16).to_be_bytes());
    name.extend_from_slice(&[0, 0]);
    name.extend(ps);
    tables.insert(*b"name", name);
    let mut cmap = vec![0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12, 0, 12, 0, 0];
    cmap.extend_from_slice(&40u32.to_be_bytes());
    cmap.extend_from_slice(&0u32.to_be_bytes());
    cmap.extend_from_slice(&2u32.to_be_bytes());
    for group in [[32u32, 32, 3], [65, 66, 1]] {
        for value in group {
            cmap.extend_from_slice(&value.to_be_bytes());
        }
    }
    tables.insert(*b"cmap", cmap);
    tables.insert(*b"CFF ", cff(keyed));
    let n = tables.len();
    let power = 1usize << (usize::BITS - 1 - n.leading_zeros());
    let mut out = b"OTTO".to_vec();
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&((power * 16) as u16).to_be_bytes());
    out.extend_from_slice(&(power.trailing_zeros() as u16).to_be_bytes());
    out.extend_from_slice(&((n * 16 - power * 16) as u16).to_be_bytes());
    let mut body = Vec::new();
    let mut head_offset = 0;
    for (tag, bytes) in tables {
        let offset = 12 + 16 * n + body.len();
        if tag == *b"head" {
            head_offset = offset;
        }
        out.extend_from_slice(&tag);
        out.extend_from_slice(&checksum(&bytes).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        body.extend(bytes);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.extend(body);
    let adjustment = 0xB1B0AFBAu32.wrapping_sub(checksum(&out));
    out[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    out
}
