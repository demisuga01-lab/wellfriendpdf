//! Compose source codes -> CIDs -> Unicode without confusing any of the keys.
use super::{decode_unicode_encoding, resources};
use crate::fonts::{
    character_code::CharacterCode as Code,
    cid_encoding::CidEncoding,
    cmap::ToUnicodeCMap,
    cmap_program::{Kind, Program, Result},
    pdf_embedding::CidSystem,
};
use crate::{PdfDictionary, PdfReader};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

fn font_system(
    descendant: Option<&PdfDictionary>,
    reader: Option<&PdfReader>,
) -> Result<Option<CidSystem>> {
    descendant
        .and_then(|dict| dict.get("CIDSystemInfo"))
        .map(|system| crate::fonts::cmap_stream::read_system(system, reader))
        .transpose()
}
pub(crate) fn validate_font_system(
    encoding: &CidEncoding,
    descendant: Option<&PdfDictionary>,
    reader: Option<&PdfReader>,
) -> Result<()> {
    if let (Some(font), Some(map)) = (font_system(descendant, reader)?, &encoding.program.system) {
        // Identity-H/V are collection-neutral. Supplement differences do not
        // redefine existing CIDs; unavailable newer glyphs remain .notdef.
        if map.ordering.as_slice() != b"Identity"
            && (font.registry != map.registry || font.ordering != map.ordering)
        {
            return Err("Encoding CMap and CIDFont character collections disagree".into());
        }
    }
    Ok(())
}

type Cached = ((&'static str, String), ToUnicodeCMap, usize);
static CACHE: OnceLock<Mutex<VecDeque<Cached>>> = OnceLock::new();

pub(crate) fn font_unicode(
    encoding: &CidEncoding,
    descendant: Option<&PdfDictionary>,
    reader: Option<&PdfReader>,
) -> Result<Option<ToUnicodeCMap>> {
    crate::cancel::check_current_cancel("predefined source Unicode mapping")
        .map_err(|e| e.to_string())?;
    validate_font_system(encoding, descendant, reader)?;
    let system = font_system(descendant, reader)?.or_else(|| encoding.program.system.clone());
    let collection = system.as_ref().map(|system| {
        format!(
            "{}-{}",
            String::from_utf8_lossy(&system.registry),
            String::from_utf8_lossy(&system.ordering)
        )
    });
    let collection_name = collection.as_deref().map(|name| format!("{name}-UCS2"));
    let collection_asset = collection_name
        .as_deref()
        .and_then(|name| resources::asset(name, Kind::Unicode));
    let source_asset = encoding
        .predefined_name
        .and_then(|name| resources::asset(name, Kind::Cid));
    let direct_unicode = source_asset.is_some_and(|asset| asset.unicode_encoding != 0);
    let source_unicode_encoding = source_asset.map_or(0, |asset| asset.unicode_encoding);
    if collection_asset.is_none() && !direct_unicode {
        if system
            .as_ref()
            .is_some_and(|system| system.ordering.as_slice() != b"Identity")
        {
            return Err(
                "CIDFont character collection has no bundled Unicode resource; supply ToUnicode"
                    .into(),
            );
        }
        return Ok(None);
    }
    let key = encoding
        .predefined_name
        .map(|name| (name, collection.clone().unwrap_or_default()));
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Some(key) = &key {
        let mut entries = cache
            .lock()
            .map_err(|_| "predefined Unicode cache poisoned")?;
        if let Some(index) = entries.iter().position(|entry| &entry.0 == key) {
            let entry = entries
                .remove(index)
                .ok_or("predefined Unicode cache index")?;
            let result = entry.1.clone();
            entries.push_back(entry);
            return Ok(Some(result));
        }
    }
    let collection_map = collection_asset
        .map(|asset| resources::load(asset.name, Kind::Unicode))
        .transpose()?;
    let mut unicode = BTreeMap::new();
    let mut insert = |code: Code, cid: u16| {
        let text = decode_unicode_encoding(source_unicode_encoding, code).or_else(|| {
            collection_map.as_ref().and_then(|map| {
                Code::new(u32::from(cid), 2)
                    .ok()
                    .and_then(|cid| map.unicode.get(&cid).cloned())
            })
        });
        if let Some(text) = text {
            unicode.insert(code, text);
        }
    };
    if encoding.program.identity {
        if let Some(map) = &collection_map {
            for (index, cid) in map.unicode.keys().enumerate() {
                if index % 256 == 0 {
                    crate::cancel::check_current_cancel("Identity collection Unicode mapping")
                        .map_err(|e| e.to_string())?;
                }
                if let Ok(value) = u16::try_from(cid.value()) {
                    insert(Code::new(u32::from(value), 2)?, value);
                }
            }
        }
    }
    for (index, (code, cid)) in encoding.program.cids.iter().enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("CID collection Unicode mapping")
                .map_err(|e| e.to_string())?;
        }
        insert(*code, *cid);
    }
    let weight = unicode.len().saturating_mul(192)
        + unicode.values().map(|text| text.len() * 2).sum::<usize>();
    let map = ToUnicodeCMap::from_program(Program {
        space: Arc::clone(&encoding.program.space),
        cids: BTreeMap::new(),
        notdef: BTreeMap::new(),
        unicode,
        identity: false,
        wmode: None,
        wmode_declared: false,
        name: None,
        system: None,
        system_declared: false,
    })?;
    if let Some(key) = key {
        let mut entries = cache
            .lock()
            .map_err(|_| "predefined Unicode cache poisoned")?;
        while !entries.is_empty()
            && (entries.len() >= 16
                || entries.iter().map(|entry| entry.2).sum::<usize>() + weight > 32 * 1024 * 1024)
        {
            entries.pop_front();
        }
        if weight <= 32 * 1024 * 1024 {
            entries.push_back((key, map.clone(), weight));
        }
    }
    Ok(Some(map))
}
