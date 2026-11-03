//! Font library access and source-font identification.
//!
//! The editor never searches the host for fonts: every face it embeds comes
//! from the caller's library (the browser fetches the self-hosted files; the
//! server reads the same files from its font directory) by id.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object};
use serde::Serialize;

use crate::ops::valid_font_id;

/// Library fonts by id.
pub trait FontSource {
    fn font_bytes(&self, id: &str) -> Option<Vec<u8>>;
}

/// An in-memory library (the browser registers fetched files here).
#[derive(Default, Clone)]
pub struct MemoryFonts {
    fonts: BTreeMap<String, Vec<u8>>,
}

impl MemoryFonts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, id: &str, bytes: Vec<u8>) -> bool {
        if !valid_font_id(id) || bytes.is_empty() {
            return false;
        }
        self.fonts.insert(id.to_owned(), bytes);
        true
    }
    pub fn contains(&self, id: &str) -> bool {
        self.fonts.contains_key(id)
    }
}

impl FontSource for MemoryFonts {
    fn font_bytes(&self, id: &str) -> Option<Vec<u8>> {
        self.fonts.get(id).cloned()
    }
}

/// A directory of `<id>.ttf` / `<id>.otf` files (the server's library).
#[cfg(not(target_arch = "wasm32"))]
pub struct DirectoryFonts {
    pub directory: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FontSource for DirectoryFonts {
    fn font_bytes(&self, id: &str) -> Option<Vec<u8>> {
        if !valid_font_id(id) {
            return None;
        }
        for extension in ["ttf", "otf"] {
            let path = self.directory.join(format!("{id}.{extension}"));
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.is_file() && metadata.len() <= 64 * 1024 * 1024 {
                    return std::fs::read(path).ok();
                }
            }
        }
        None
    }
}

/// Whether `bytes` has a glyph for every non-whitespace character of `text`.
pub fn covers(bytes: &[u8], text: &str) -> bool {
    let Ok(face) = ttf_parser::Face::parse(bytes, 0) else {
        return false;
    };
    text.chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| face.glyph_index(c).is_some_and(|glyph| glyph.0 != 0))
}

/// Ascender as a fraction of the em (for placing the first baseline).
pub fn ascent_ratio(bytes: &[u8]) -> f64 {
    ttf_parser::Face::parse(bytes, 0)
        .ok()
        .map(|face| {
            let units = f64::from(face.units_per_em().max(1));
            (f64::from(face.ascender()) / units).clamp(0.5, 1.2)
        })
        .unwrap_or(0.8)
}

/// What the editor shows about a page font resource.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFont {
    pub resource: String,
    pub base_font: String,
    /// Family name without the subset tag and style suffix.
    pub family: String,
    pub subtype: String,
    pub embedded: bool,
    /// A `ABCDEF+` subset tag: only the glyphs the document used are present.
    pub subset: bool,
    pub bold: bool,
    pub italic: bool,
    /// The library font that replaces it when the embedded font lacks a glyph.
    pub library_match: Option<LibraryMatch>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMatch {
    pub font: String,
    /// `exact` (same family), `metric` (metric-compatible substitute) or
    /// `similar` (same classification only).
    pub kind: &'static str,
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve(document, object)?.as_dict().ok()
}

fn name(dictionary: &Dictionary, key: &[u8]) -> Option<String> {
    dictionary
        .get(key)
        .ok()
        .and_then(|value| value.as_name().ok())
        .map(|value| String::from_utf8_lossy(value).into_owned())
}

/// The page's `/Resources` (own or inherited).
pub fn page_resources(document: &Document, page: lopdf::ObjectId) -> Option<Dictionary> {
    let mut current = document.get_dictionary(page).ok()?;
    for _ in 0..32 {
        if let Ok(resources) = current.get(b"Resources") {
            return dict(document, resources).cloned();
        }
        let parent = current.get(b"Parent").ok()?.as_reference().ok()?;
        current = document.get_dictionary(parent).ok()?;
    }
    None
}

/// The fonts of one page's resources.
pub fn page_fonts(document: &Document, page_number: u32) -> Vec<SourceFont> {
    let Some(page) = document.get_pages().get(&page_number).copied() else {
        return Vec::new();
    };
    let Some(resources) = page_resources(document, page) else {
        return Vec::new();
    };
    let Some(fonts) = resources.get(b"Font").ok().and_then(|f| dict(document, f)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, value) in fonts.iter() {
        let Some(font) = dict(document, value) else {
            continue;
        };
        let base_font = name(font, b"BaseFont").unwrap_or_default();
        let subtype = name(font, b"Subtype").unwrap_or_default();
        let descriptor_owner = if subtype == "Type0" {
            font.get(b"DescendantFonts")
                .ok()
                .and_then(|d| resolve(document, d))
                .and_then(|d| d.as_array().ok())
                .and_then(|a| a.first())
                .and_then(|d| dict(document, d))
        } else {
            Some(font)
        };
        let descriptor = descriptor_owner
            .and_then(|owner| owner.get(b"FontDescriptor").ok())
            .and_then(|d| dict(document, d));
        let embedded = descriptor
            .is_some_and(|d| d.has(b"FontFile") || d.has(b"FontFile2") || d.has(b"FontFile3"));
        let flags = descriptor
            .and_then(|d| d.get(b"Flags").ok())
            .and_then(|f| f.as_i64().ok())
            .unwrap_or(0);
        let subset = base_font.len() > 7
            && base_font.as_bytes()[6] == b'+'
            && base_font.as_bytes()[..6].iter().all(u8::is_ascii_uppercase);
        let plain = if subset {
            &base_font[7..]
        } else {
            base_font.as_str()
        };
        let lower = plain.to_ascii_lowercase();
        let bold = lower.contains("bold")
            || lower.contains("black")
            || lower.contains("heavy")
            || lower.contains("semibold")
            || lower.contains(",bold");
        let italic = lower.contains("italic") || lower.contains("oblique") || flags & 64 != 0;
        let family = family_of(plain);
        let library_match = library_match(&family, bold, italic, flags);
        out.push(SourceFont {
            resource: String::from_utf8_lossy(key).into_owned(),
            base_font,
            family,
            subtype,
            embedded,
            subset,
            bold,
            italic,
            library_match,
        });
    }
    out
}

/// `Calibri-BoldItalic` / `ArialMT,Bold` / `TimesNewRomanPSMT` -> family.
pub fn family_of(base: &str) -> String {
    let mut family = base.split([',', '-']).next().unwrap_or(base).to_owned();
    for suffix in ["PSMT", "MT", "PS"] {
        if family.len() > suffix.len() && family.ends_with(suffix) {
            family.truncate(family.len() - suffix.len());
            break;
        }
    }
    family
}

fn style_suffix(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "regular",
        (true, false) => "bold",
        (false, true) => "italic",
        (true, true) => "bold-italic",
    }
}

/// Metric-compatible substitutes for the standard 14 and common Office
/// fonts, then same-family library fonts, then a classification fallback.
/// Ids follow the library catalogue (`<family>-<style>`).
pub fn library_match(family: &str, bold: bool, italic: bool, flags: i64) -> Option<LibraryMatch> {
    let key: String = family
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();
    let style = style_suffix(bold, italic);
    let metric = |base: &str| {
        Some(LibraryMatch {
            font: format!("{base}-{style}"),
            kind: "metric",
        })
    };
    let exact = |base: &str| {
        Some(LibraryMatch {
            font: format!("{base}-{style}"),
            kind: "exact",
        })
    };
    match key.as_str() {
        "helvetica" | "arial" | "arialmt" | "helveticaneue" | "liberationsans" | "arimo" => {
            return if key.starts_with("liberation") {
                exact("liberation-sans")
            } else {
                metric("liberation-sans")
            };
        }
        "times" | "timesroman" | "timesnewroman" | "liberationserif" | "tinos" => {
            return if key.starts_with("liberation") {
                exact("liberation-serif")
            } else {
                metric("liberation-serif")
            };
        }
        "courier" | "couriernew" | "liberationmono" | "cousine" => {
            return if key.starts_with("liberation") {
                exact("liberation-mono")
            } else {
                metric("liberation-mono")
            };
        }
        "calibri" | "carlito" => {
            return if key == "carlito" {
                exact("carlito")
            } else {
                metric("carlito")
            }
        }
        "cambria" | "caladea" => {
            return if key == "caladea" {
                exact("caladea")
            } else {
                metric("caladea")
            }
        }
        "georgia" | "gelasio" => {
            return if key == "gelasio" {
                exact("gelasio")
            } else {
                metric("gelasio")
            }
        }
        "verdana" | "tahoma" | "segoeui" | "dejavusans" | "bitstreamverasans" => {
            return if key == "dejavusans" {
                exact("dejavu-sans")
            } else {
                Some(LibraryMatch {
                    font: format!("dejavu-sans-{style}"),
                    kind: "similar",
                })
            };
        }
        _ => {}
    }
    let known = [
        ("roboto", "roboto"),
        ("inter", "inter"),
        ("lato", "lato"),
        ("opensans", "open-sans"),
        ("montserrat", "montserrat"),
        ("ibmplexsans", "ibm-plex-sans"),
        ("ibmplexserif", "ibm-plex-serif"),
        ("ibmplexmono", "ibm-plex-mono"),
        ("sourcesans3", "source-sans-3"),
        ("sourcesanspro", "source-sans-3"),
        ("notosans", "noto-sans"),
        ("notoserif", "noto-serif"),
        ("dejavuserif", "dejavu-serif"),
        ("merriweather", "merriweather"),
        ("sourceserif4", "source-serif-4"),
        ("sourceserifpro", "source-serif-4"),
        ("jetbrainsmono", "jetbrains-mono"),
    ];
    for (prefix, base) in known {
        if key.starts_with(prefix) {
            return exact(base);
        }
    }
    // PDF font flags: bit 1 fixed pitch, bit 2 serif.
    let base = if flags & 1 != 0 {
        "liberation-mono"
    } else if flags & 2 != 0 {
        "noto-serif"
    } else {
        "noto-sans"
    };
    Some(LibraryMatch {
        font: format!("{base}-{style}"),
        kind: "similar",
    })
}
