//! Pinned, offline Adobe resources. Decompression, hashing and parsing are lazy;
//! successful immutable programs are shared and bounded in the process cache.
// This file is included as the private `resources` child of
// `fonts::predefined_cmap`, so one `super` only reaches that parent module.
// Resolve the shared CMap program model from the enclosing `fonts` module.
use super::super::cmap_program::{Kind, Program, Result};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

pub(super) struct Asset {
    pub name: &'static str,
    pub kind: Kind,
    pub collection: &'static str,
    pub registry: &'static str,
    pub ordering: &'static str,
    pub supplement: u32,
    pub vertical: bool,
    pub code_size: u8,
    pub unicode_encoding: u8,
    pub source_sha256: &'static str,
    pub decoded_len: usize,
    pub gzip_hex: &'static str,
}
#[path = "predefined_resource_data.rs"]
mod data;
pub(super) use data::{ASSETS, METADATA};

type Key = (&'static str, Kind);
struct Cached {
    key: Key,
    program: Arc<Program>,
    weight: usize,
}
static CACHE: OnceLock<Mutex<VecDeque<Cached>>> = OnceLock::new();
thread_local! { static LOADING: RefCell<Vec<Key>> = const { RefCell::new(Vec::new()) }; }
struct Loading;
impl Loading {
    fn enter(key: Key) -> Result<Self> {
        LOADING.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.len() >= 8 || stack.contains(&key) {
                return Err("predefined CMap inheritance cycle or depth limit".into());
            }
            stack.push(key);
            Ok(Self)
        })
    }
}
impl Drop for Loading {
    fn drop(&mut self) {
        LOADING.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

pub(super) fn asset(name: &str, kind: Kind) -> Option<&'static Asset> {
    ASSETS
        .iter()
        .find(|entry| entry.name == name && entry.kind == kind)
}

pub(super) fn decoded(asset: &Asset) -> Result<Vec<u8>> {
    crate::cancel::check_current_cancel("predefined CMap resource decoding")
        .map_err(|e| e.to_string())?;
    let mut compressed = Vec::new();
    let mut high = None;
    for (index, byte) in asset.gzip_hex.bytes().enumerate() {
        if index % 4096 == 0 {
            crate::cancel::check_current_cancel("predefined CMap hex decoding")
                .map_err(|e| e.to_string())?;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return Err("invalid bundled CMap hexadecimal data".into()),
        };
        if let Some(upper) = high.take() {
            compressed.push(upper * 16 + digit);
        } else {
            high = Some(digit);
        }
        if compressed.len() > 1024 * 1024 {
            return Err("bundled CMap compressed-size limit".into());
        }
    }
    if high.is_some() || asset.decoded_len > 1024 * 1024 {
        return Err("invalid bundled CMap size".into());
    }
    let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
    let mut output = Vec::with_capacity(asset.decoded_len);
    let mut chunk = [0u8; 8192];
    loop {
        crate::cancel::check_current_cancel("predefined CMap inflation")
            .map_err(|e| e.to_string())?;
        let count = decoder.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if output.len() + count > asset.decoded_len {
            return Err("bundled CMap decoded-size mismatch".into());
        }
        output.extend_from_slice(&chunk[..count]);
    }
    if output.len() != asset.decoded_len
        || format!("{:x}", Sha256::digest(&output)) != asset.source_sha256
    {
        return Err("bundled CMap source hash mismatch".into());
    }
    Ok(output)
}

pub(super) fn load(name: &str, kind: Kind) -> Result<Arc<Program>> {
    crate::cancel::check_current_cancel("predefined CMap lookup").map_err(|e| e.to_string())?;
    let asset =
        asset(name, kind).ok_or_else(|| format!("unregistered {kind:?} CMap resource: {name}"))?;
    let key = (asset.name, kind);
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    {
        let mut entries = cache.lock().map_err(|_| "predefined CMap cache poisoned")?;
        if let Some(index) = entries.iter().position(|entry| entry.key == key) {
            let entry = entries.remove(index).ok_or("predefined CMap cache index")?;
            let program = Arc::clone(&entry.program);
            entries.push_back(entry);
            return Ok(program);
        }
    }
    let _guard = Loading::enter(key)?;
    let bytes = decoded(asset)?;
    let program = Program::parse(&bytes, kind, None, false)?;
    if program.name.as_deref() != Some(asset.name)
        || (kind == Kind::Cid
            && asset.collection != format!("{}-{}", asset.registry, asset.ordering))
        || program.space.fixed_length().unwrap_or(0) != asset.code_size
        || (kind == Kind::Cid && program.wmode.unwrap_or(0) != u8::from(asset.vertical))
        || !program.system.as_ref().is_some_and(|system| {
            system.registry == asset.registry.as_bytes()
                && system.ordering == asset.ordering.as_bytes()
                && system.supplement == asset.supplement
        })
    {
        return Err("bundled CMap metadata disagrees with parsed resource".into());
    }
    // Conservative retained-storage accounting, not a measured RSS claim.
    let weight = (program.cids.len() + program.notdef.len() + program.unicode.len())
        .saturating_mul(128)
        .saturating_add(program.unicode.values().map(String::len).sum::<usize>())
        .saturating_add(2 * 1024 * 1024);
    let program = Arc::new(program);
    let mut entries = cache.lock().map_err(|_| "predefined CMap cache poisoned")?;
    if let Some(entry) = entries.iter().find(|entry| entry.key == key) {
        return Ok(Arc::clone(&entry.program));
    }
    while !entries.is_empty()
        && (entries.len() >= 16
            || entries.iter().map(|e| e.weight).sum::<usize>() + weight > 32 * 1024 * 1024)
    {
        entries.pop_front();
    }
    if weight <= 32 * 1024 * 1024 {
        entries.push_back(Cached {
            key,
            program: Arc::clone(&program),
            weight,
        });
    }
    Ok(program)
}
