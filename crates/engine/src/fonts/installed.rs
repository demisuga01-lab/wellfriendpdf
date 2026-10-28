//! Fonts installed on the host: discovery, name matching and glyph-coverage
//! fallback.
//!
//! Faces are discovered through fontdb (the fontconfig directories on Linux,
//! the platform font folders on macOS and Windows) plus every directory in
//! `WELLFRIENDPDF_FONT_DIRS` (platform path-list syntax). Discovery is lazy:
//! nothing is scanned until a caller asks for an installed face, so documents
//! whose fonts are all embedded never pay for it.
//!
//! Installed fonts make output depend on the host. Set
//! `WELLFRIENDPDF_SYSTEM_FONTS=0`, or call [`set_installed_fonts_enabled`],
//! for host-independent output; the exact render contract never uses them.

use std::borrow::Cow;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, OnceLock};

use super::provider::{FontMatch, FontMatchRequest, FontProvider, FontProviderSource};

/// One installed face, as reported by [`installed_font_faces`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstalledFontFace {
    /// Preferred (typographic) family name.
    pub family: String,
    /// PostScript name (`name` ID 6).
    pub post_script_name: String,
    /// Font file, when the face came from a file.
    pub path: Option<PathBuf>,
    /// Face index inside a collection (`.ttc`/`.otc`), else 0.
    pub index: u32,
    /// OS/2 weight class (400 regular, 700 bold).
    pub weight: u16,
    /// Italic or oblique.
    pub italic: bool,
    /// Fixed pitch.
    pub monospaced: bool,
}

/// Style wanted from a coverage fallback (see [`installed_font_covering`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledFontStyle {
    /// The source font's name (`/BaseFont`, subset tag allowed). Faces of the
    /// same family are preferred, then faces of a similar design.
    pub family_hint: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub monospace: bool,
    /// `Some(true)` for a serif design, `Some(false)` for sans serif.
    pub serif: Option<bool>,
    /// Only faces whose OS/2 `fsType` allows embedding in an edited PDF.
    pub require_embeddable: bool,
}

/// A selected installed face with its standalone program bytes (a face of a
/// collection is extracted to a single-face sfnt).
#[derive(Debug, Clone)]
pub struct InstalledFontProgram {
    pub face: InstalledFontFace,
    pub bytes: &'static [u8],
}

const ENABLED_UNSET: u8 = 0;
const ENABLED_ON: u8 = 1;
const ENABLED_OFF: u8 = 2;
static ENABLED: AtomicU8 = AtomicU8::new(ENABLED_UNSET);

#[cfg(test)]
thread_local! {
    static TEST_THREAD_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The engine's own unit tests are host-independent: installed fonts are off
/// unless a test opts in for its thread.
#[cfg(test)]
pub(crate) fn enable_installed_fonts_for_test_thread() {
    TEST_THREAD_ENABLED.with(|enabled| enabled.set(true));
}

/// Whether installed fonts may be used. Defaults to on unless
/// `WELLFRIENDPDF_SYSTEM_FONTS` is `0`, `false`, `off` or `no`.
pub fn installed_fonts_enabled() -> bool {
    #[cfg(test)]
    if !TEST_THREAD_ENABLED.with(std::cell::Cell::get) {
        return false;
    }
    match ENABLED.load(Ordering::Relaxed) {
        ENABLED_ON => true,
        ENABLED_OFF => false,
        _ => {
            let enabled = !std::env::var("WELLFRIENDPDF_SYSTEM_FONTS").is_ok_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            });
            ENABLED.store(
                if enabled { ENABLED_ON } else { ENABLED_OFF },
                Ordering::Relaxed,
            );
            enabled
        }
    }
}

/// Allow or forbid installed fonts for the rest of the process.
pub fn set_installed_fonts_enabled(enabled: bool) {
    ENABLED.store(
        if enabled { ENABLED_ON } else { ENABLED_OFF },
        Ordering::Relaxed,
    );
}

#[cfg(not(target_arch = "wasm32"))]
/// Every installed face, sorted by family, then weight, italic and name.
pub fn installed_font_faces() -> Vec<InstalledFontFace> {
    let Some(db) = database() else {
        return Vec::new();
    };
    let mut faces: Vec<InstalledFontFace> = db.faces().map(face_record).collect();
    faces.sort_by(|a, b| {
        (
            a.family.as_str(),
            a.weight,
            a.italic,
            a.post_script_name.as_str(),
        )
            .cmp(&(
                b.family.as_str(),
                b.weight,
                b.italic,
                b.post_script_name.as_str(),
            ))
    });
    faces
}

#[cfg(not(target_arch = "wasm32"))]
/// The installed face named by a PDF `/BaseFont` (subset tag allowed), by
/// PostScript name first, then by family with the closest weight and slope.
pub fn installed_font_named(
    base_font: &str,
    bold: bool,
    italic: bool,
    require_embeddable: bool,
) -> Option<InstalledFontProgram> {
    let db = database()?;
    let key = normalize_font_key(base_font);
    if key.is_empty() {
        return None;
    }
    let cache_key = format!("named:{key}:{bold}:{italic}:{require_embeddable}");
    memoized(&cache_key, || {
        let wanted_bold = bold || name_is_bold(base_font);
        let wanted_italic = italic || name_is_italic(base_font);
        let mut exact = db
            .faces()
            .filter(|face| normalize_font_key(&face.post_script_name) == key)
            .collect::<Vec<_>>();
        exact.sort_by_key(|face| face_sort_key(face));
        for face in exact {
            if let Some(program) = load_program(db, face, require_embeddable) {
                return Some(program);
            }
        }
        let family = family_key(base_font);
        if family.is_empty() {
            return None;
        }
        let mut same_family = db
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| normalize_font_key(name) == family)
                    || family_key(&face.post_script_name) == family
            })
            .collect::<Vec<_>>();
        same_family.sort_by_key(|face| {
            (
                style_distance(face, wanted_bold, wanted_italic),
                face_sort_key(face),
            )
        });
        same_family
            .into_iter()
            .find_map(|face| load_program(db, face, require_embeddable))
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The installed face that draws every non-whitespace character of `text`,
/// preferring the hinted family, then the hinted design (serif, sans serif,
/// monospace) and style. When no face covers everything, the face covering
/// the most characters is returned; `None` when no face covers any.
pub fn installed_font_covering(
    text: &str,
    style: &InstalledFontStyle,
) -> Option<InstalledFontProgram> {
    let db = database()?;
    let mut chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    chars.sort_unstable();
    chars.dedup();
    if chars.is_empty() {
        return None;
    }
    let cache_key = format!(
        "cover:{}:{:?}:{}:{}:{}:{:?}:{}",
        chars.iter().collect::<String>(),
        style.family_hint.as_deref().map(family_key),
        style.bold,
        style.italic,
        style.monospace,
        style.serif,
        style.require_embeddable
    );
    memoized(&cache_key, || {
        let hint = style.family_hint.as_deref().map(family_key);
        let serif = style
            .serif
            .or_else(|| style.family_hint.as_deref().map(name_looks_serif));
        let monospace = style.monospace
            || style
                .family_hint
                .as_deref()
                .is_some_and(name_looks_monospace);
        let wanted_bold = style.bold || style.family_hint.as_deref().is_some_and(name_is_bold);
        let wanted_italic =
            style.italic || style.family_hint.as_deref().is_some_and(name_is_italic);

        let mut best: Option<(usize, (u32, u32, u32, String, u32), fontdb::ID)> = None;
        for face in db.faces() {
            let covered = db
                .with_face_data(face.id, |data, index| {
                    let parsed = ttf_parser::Face::parse(data, index).ok()?;
                    if !has_outlines(&parsed) || (style.require_embeddable && !embeddable(&parsed))
                    {
                        return None;
                    }
                    Some(
                        chars
                            .iter()
                            .filter(|ch| parsed.glyph_index(**ch).is_some_and(|gid| gid.0 != 0))
                            .count(),
                    )
                })
                .flatten()
                .unwrap_or(0);
            if covered == 0 {
                continue;
            }
            let family = face
                .families
                .first()
                .map(|(name, _)| name.as_str())
                .unwrap_or(&face.post_script_name);
            let family_rank = match &hint {
                Some(hint)
                    if face
                        .families
                        .iter()
                        .any(|(name, _)| normalize_font_key(name) == *hint)
                        || family_key(&face.post_script_name) == *hint =>
                {
                    0
                }
                _ => 1,
            };
            let design_rank = u32::from(face.monospaced != monospace)
                + u32::from(serif.is_some_and(|serif| name_looks_serif(family) != serif));
            let rank = (
                family_rank,
                design_rank,
                style_distance(face, wanted_bold, wanted_italic) + preferred_family_rank(family),
                face.post_script_name.clone(),
                face.index,
            );
            let better = match &best {
                None => true,
                Some((best_covered, best_rank, _)) => {
                    covered > *best_covered || (covered == *best_covered && rank < *best_rank)
                }
            };
            if better {
                best = Some((covered, rank, face.id));
            }
        }
        let (_, _, id) = best?;
        load_program(db, db.face(id)?, style.require_embeddable)
    })
}

#[cfg(target_arch = "wasm32")]
pub fn installed_font_faces() -> Vec<InstalledFontFace> {
    Vec::new()
}

#[cfg(target_arch = "wasm32")]
pub fn installed_font_named(
    _base_font: &str,
    _bold: bool,
    _italic: bool,
    _require_embeddable: bool,
) -> Option<InstalledFontProgram> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn installed_font_covering(
    _text: &str,
    _style: &InstalledFontStyle,
) -> Option<InstalledFontProgram> {
    None
}

/// Font for generated (non-source) text: the bundled DejaVu Sans when it
/// draws every character, else the installed embeddable face that does
/// (scripts DejaVu lacks: CJK, Indic, Thai...), else DejaVu Sans so callers
/// report the missing glyphs as before.
pub(crate) fn generated_text_font(text: &str) -> Option<&'static [u8]> {
    let bundled = crate::render::get_fallback_font("Symbol");
    if bundled.is_some_and(|font| draws_all(font, text)) {
        return bundled;
    }
    installed_font_covering(
        text,
        &InstalledFontStyle {
            require_embeddable: true,
            ..InstalledFontStyle::default()
        },
    )
    .map(|program| program.bytes)
    .filter(|font| draws_all(font, text))
    .or(bundled)
}

/// The installed embeddable face closest in design to `base_font` (same
/// family first, then serif/sans/mono and weight/slope) that draws every
/// character of `text`: the substitute for an edit whose source subset and
/// source family cannot draw the replacement.
pub(crate) fn similar_installed_font(base_font: &str, text: &str) -> Option<&'static [u8]> {
    installed_font_covering(
        text,
        &InstalledFontStyle {
            family_hint: Some(base_font.to_string()),
            require_embeddable: true,
            ..InstalledFontStyle::default()
        },
    )
    .map(|program| program.bytes)
    .filter(|font| draws_all(font, text))
}

/// Every visible character of `text` has a real glyph in `font`, and
/// shaping `text` with it leaves no missing-glyph cluster.
pub(crate) fn draws_all(font: &[u8], text: &str) -> bool {
    let mapped = ttf_parser::Face::parse(font, 0).is_ok_and(|face| {
        text.chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control() && !is_invisible_format(*ch))
            .all(|ch| face.glyph_index(ch).is_some_and(|glyph| glyph.0 != 0))
    });
    mapped
        && super::TextShaper::shape(font, text, super::ShapeOptions::default()).is_ok_and(|run| {
            super::shaper::has_missing_glyphs(font, text, &run).is_ok_and(|missing| !missing)
        })
}

/// Bidi controls, joiners and variation selectors, which shaping consumes
/// without needing a glyph of their own.
fn is_invisible_format(ch: char) -> bool {
    matches!(
        ch as u32,
        0x00AD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF
    )
}

/// Rendering/substitution provider over installed faces: exact name or
/// family matches only. Similar-design substitution stays with the bundled
/// metric-compatible faces, which are deterministic.
#[derive(Debug, Default, Clone, Copy)]
pub struct InstalledFontProvider;

impl FontProvider for InstalledFontProvider {
    fn match_font(&self, request: &FontMatchRequest) -> Option<FontMatch> {
        if !installed_fonts_enabled() || super::provider::is_standard14_name(&request.base_font) {
            return None;
        }
        let program =
            installed_font_named(&request.base_font, request.bold, request.italic, false)?;
        let actual_bold = program.face.weight >= 600;
        let actual_italic = program.face.italic;
        Some(FontMatch {
            family_name: "Installed",
            lookup_name: program.face.post_script_name.clone(),
            source: FontProviderSource::InstalledSystemFont,
            bytes: Cow::Borrowed(program.bytes),
            synthetic_bold: request.bold && !actual_bold,
            synthetic_italic: request.italic && !actual_italic,
            match_reason: "installed_font_name_match",
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The process-wide installed-font database, or `None` when installed fonts
/// are disabled or none were found.
pub(crate) fn database() -> Option<&'static fontdb::Database> {
    if !installed_fonts_enabled() {
        return None;
    }
    static DB: OnceLock<Option<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let signature = index_cache::directory_signature();
        if let Some(db) = index_cache::load(&signature) {
            return (!db.is_empty()).then_some(db);
        }
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        for dir in index_cache::extra_font_dirs() {
            db.load_fonts_dir(dir);
        }
        index_cache::store(&signature, &db);
        (!db.is_empty()).then_some(db)
    })
    .as_ref()
}

/// On-disk index of installed faces, so a process that needs one installed
/// font does not parse every font file on the host. The index is keyed by
/// the modification times of every font directory (adding or removing a
/// font changes its directory's time) and of the fontconfig configuration;
/// any difference rebuilds it. Set `WELLFRIENDPDF_FONT_INDEX=0` to disable.
#[cfg(not(target_arch = "wasm32"))]
mod index_cache {
    use std::path::{Path, PathBuf};

    const VERSION: &str = "wellfriendpdf-installed-fonts-v2";

    pub(super) fn extra_font_dirs() -> Vec<PathBuf> {
        std::env::var_os("WELLFRIENDPDF_FONT_DIRS")
            .map(|dirs| std::env::split_paths(&dirs).collect())
            .unwrap_or_default()
    }

    fn roots() -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if cfg!(target_os = "windows") {
            if let Some(windir) = std::env::var_os("WINDIR") {
                roots.push(PathBuf::from(windir).join("Fonts"));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                roots.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
            }
        } else if cfg!(target_os = "macos") {
            roots.extend(
                [
                    "/Library/Fonts",
                    "/System/Library/Fonts",
                    "/Network/Library/Fonts",
                ]
                .map(PathBuf::from),
            );
            roots.extend(home.iter().map(|home| home.join("Library/Fonts")));
        } else {
            roots.extend(
                ["/usr/share/fonts", "/usr/local/share/fonts", "/etc/fonts"].map(PathBuf::from),
            );
            if let Some(home) = &home {
                roots.push(home.join(".local/share/fonts"));
                roots.push(home.join(".fonts"));
            }
        }
        roots.extend(extra_font_dirs());
        roots
    }

    /// Path and modification time of every directory under the font roots.
    pub(super) fn directory_signature() -> String {
        fn walk(dir: &Path, out: &mut Vec<String>, depth: usize) {
            let Ok(metadata) = std::fs::metadata(dir) else {
                return;
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_nanos());
            out.push(format!("{}={modified}", dir.display()));
            if depth >= 12 {
                return;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut children: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect();
            children.sort();
            for child in children {
                walk(&child, out, depth + 1);
            }
        }
        let mut entries = vec![VERSION.to_string()];
        for root in roots() {
            walk(&root, &mut entries, 0);
        }
        format!(
            "{:016x}",
            crate::reader::xxh64(entries.join("\n").as_bytes(), 0)
        )
    }

    fn cache_file() -> Option<PathBuf> {
        if std::env::var("WELLFRIENDPDF_FONT_INDEX").is_ok_and(|value| value.trim() == "0") {
            return None;
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
        Some(base.join("wellfriendpdf").join("installed-fonts.idx"))
    }

    /// One line per face: path, index, PostScript name, weight, style
    /// (0 normal, 1 italic, 2 oblique), monospaced, width class (1-9), then
    /// families; fields
    /// separated by tabs. The first line is the directory signature.
    pub(super) fn store(signature: &str, db: &fontdb::Database) {
        let Some(path) = cache_file() else {
            return;
        };
        let mut text = format!("{signature}\n");
        for face in db.faces() {
            let file = match &face.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => path,
                fontdb::Source::Binary(_) => continue,
            };
            let fields = [
                file.to_string_lossy().into_owned(),
                face.index.to_string(),
                face.post_script_name.clone(),
                face.weight.0.to_string(),
                match face.style {
                    fontdb::Style::Normal => "0",
                    fontdb::Style::Italic => "1",
                    fontdb::Style::Oblique => "2",
                }
                .to_string(),
                u8::from(face.monospaced).to_string(),
                face.stretch.to_number().to_string(),
            ];
            if fields.iter().any(|field| field.contains(['\t', '\n'])) {
                continue;
            }
            text.push_str(&fields.join("\t"));
            for (family, _) in &face.families {
                if !family.contains(['\t', '\n']) {
                    text.push('\t');
                    text.push_str(family);
                }
            }
            text.push('\n');
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Write then rename, so a concurrent reader never sees half a file.
        let temporary = path.with_extension(format!("idx.{}", std::process::id()));
        if std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, &path).is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
    }

    pub(super) fn load(signature: &str) -> Option<fontdb::Database> {
        let text = std::fs::read_to_string(cache_file()?).ok()?;
        let mut lines = text.lines();
        if lines.next()? != signature {
            return None;
        }
        let mut db = fontdb::Database::new();
        for line in lines {
            let mut fields = line.split('\t');
            let (
                Some(path),
                Some(index),
                Some(post_script_name),
                Some(weight),
                Some(style),
                Some(mono),
                Some(stretch),
            ) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            )
            else {
                return None;
            };
            db.push_face_info(fontdb::FaceInfo {
                id: fontdb::ID::dummy(),
                source: fontdb::Source::File(PathBuf::from(path)),
                index: index.parse().ok()?,
                families: fields
                    .map(|family| (family.to_string(), fontdb::Language::English_UnitedStates))
                    .collect(),
                post_script_name: post_script_name.to_string(),
                style: match style {
                    "1" => fontdb::Style::Italic,
                    "2" => fontdb::Style::Oblique,
                    _ => fontdb::Style::Normal,
                },
                weight: fontdb::Weight(weight.parse().ok()?),
                stretch: match stretch {
                    "1" => fontdb::Stretch::UltraCondensed,
                    "2" => fontdb::Stretch::ExtraCondensed,
                    "3" => fontdb::Stretch::Condensed,
                    "4" => fontdb::Stretch::SemiCondensed,
                    "6" => fontdb::Stretch::SemiExpanded,
                    "7" => fontdb::Stretch::Expanded,
                    "8" => fontdb::Stretch::ExtraExpanded,
                    "9" => fontdb::Stretch::UltraExpanded,
                    _ => fontdb::Stretch::Normal,
                },
                monospaced: mono == "1",
            });
        }
        Some(db)
    }
}

/// Lowercase alphanumerics of a font name without its subset tag and the
/// style-neutral words producers append (`Regular`, `Roman`, `MT`, `PS`).
pub(crate) fn normalize_font_key(name: &str) -> String {
    name.rsplit('+')
        .next()
        .unwrap_or(name)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace("regular", "")
        .replace("roman", "")
        .replace("mt", "")
        .replace("ps", "")
}

/// The family part of a PDF font name: before the `-`/`,` style suffix, with
/// style words removed, normalized like [`normalize_font_key`].
fn family_key(name: &str) -> String {
    let name = name.rsplit('+').next().unwrap_or(name);
    let family = name.split([',', '-']).next().unwrap_or(name);
    let mut key = normalize_font_key(family);
    for word in [
        "semibold",
        "demibold",
        "extrabold",
        "ultrabold",
        "bold",
        "black",
        "heavy",
        "medium",
        "light",
        "thin",
        "italic",
        "oblique",
        "book",
    ] {
        key = key.replace(word, "");
    }
    key
}

fn name_is_bold(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["bold", "black", "heavy", "semibold", "demi"]
        .iter()
        .any(|word| lower.contains(word))
}

fn name_is_italic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("italic") || lower.contains("oblique") || lower.ends_with("-it")
}

fn name_looks_monospace(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["mono", "courier", "consol", "typewriter", "code", "fixed"]
        .iter()
        .any(|word| lower.contains(word))
}

fn name_looks_serif(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("sans") || lower.contains("gothic") || lower.contains("arial") {
        return false;
    }
    [
        "serif",
        "times",
        "roman",
        "georgia",
        "garamond",
        "cambria",
        "book",
        "minion",
        "palatino",
        "palladio",
        "baskerville",
        "caslon",
        "century",
        "bodoni",
        "didot",
        "mincho",
        "song",
        "ming",
        "batang",
        "myeongjo",
        "charter",
        "gelasio",
        "tinos",
        "libertin",
        "crimson",
        "merriweather",
        "constantia",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// Widely available, broad-coverage families first, so equally good matches
/// resolve to the same design on every host that has them.
fn preferred_family_rank(family: &str) -> u32 {
    let lower = family.to_ascii_lowercase();
    if lower.starts_with("noto ") {
        0
    } else if lower.starts_with("dejavu") || lower.starts_with("liberation") {
        1
    } else {
        2
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn style_distance(face: &fontdb::FaceInfo, bold: bool, italic: bool) -> u32 {
    let wanted_weight: i32 = if bold { 700 } else { 400 };
    let weight_distance = (i32::from(face.weight.0) - wanted_weight).unsigned_abs() / 100;
    let face_italic = !matches!(face.style, fontdb::Style::Normal);
    let width_distance = (i32::from(face.stretch.to_number()) - 5).unsigned_abs() * 2;
    weight_distance + width_distance + if face_italic != italic { 4 } else { 0 }
}

#[cfg(not(target_arch = "wasm32"))]
fn face_sort_key(face: &fontdb::FaceInfo) -> (String, u32) {
    (face.post_script_name.clone(), face.index)
}

#[cfg(not(target_arch = "wasm32"))]
fn face_record(face: &fontdb::FaceInfo) -> InstalledFontFace {
    InstalledFontFace {
        family: face
            .families
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| face.post_script_name.clone()),
        post_script_name: face.post_script_name.clone(),
        path: match &face.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            fontdb::Source::Binary(_) => None,
        },
        index: face.index,
        weight: face.weight.0,
        italic: !matches!(face.style, fontdb::Style::Normal),
        monospaced: face.monospaced,
    }
}

/// The face draws with outlines (TrueType, CFF or CFF2), not only bitmaps.
pub(crate) fn has_outlines(face: &ttf_parser::Face<'_>) -> bool {
    let tables = face.tables();
    tables.glyf.is_some() || tables.cff.is_some() || tables.cff2.is_some()
}

/// OS/2 `fsType` permits subset embedding in an edited PDF (installable,
/// preview/print or editable; not restricted, no-subsetting or bitmap-only).
pub(crate) fn embeddable(face: &ttf_parser::Face<'_>) -> bool {
    let fs_type = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"OS/2"))
        .and_then(|os2| os2.get(8..10))
        .map_or(0, |bytes| u16::from_be_bytes([bytes[0], bytes[1]]));
    matches!(fs_type & 0x000f, 0 | 4 | 8) && fs_type & 0x0100 == 0 && fs_type & 0x0200 == 0
}

#[cfg(not(target_arch = "wasm32"))]
/// Load one face as a standalone program, leaked for the process lifetime
/// (each face is loaded at most once and shared by every caller).
fn load_program(
    db: &fontdb::Database,
    face: &fontdb::FaceInfo,
    require_embeddable: bool,
) -> Option<InstalledFontProgram> {
    type Programs = Mutex<HashMap<(fontdb::ID, bool), Option<&'static [u8]>>>;
    static PROGRAMS: OnceLock<Programs> = OnceLock::new();
    let programs = PROGRAMS.get_or_init(Default::default);
    let key = (face.id, require_embeddable);
    if let Some(hit) = programs.lock().ok()?.get(&key) {
        return hit.map(|bytes| InstalledFontProgram {
            face: face_record(face),
            bytes,
        });
    }
    let bytes = db
        .with_face_data(face.id, |data, index| {
            let parsed = ttf_parser::Face::parse(data, index).ok()?;
            if !has_outlines(&parsed) || (require_embeddable && !embeddable(&parsed)) {
                return None;
            }
            if data.starts_with(b"ttcf") {
                let container = super::font_container::Container::parse(data).ok()?;
                container
                    .extract(data, index as usize, super::font_container::MAX_BYTES)
                    .ok()
            } else {
                Some(data.to_vec())
            }
        })
        .flatten()
        .map(|bytes| &*Box::leak(bytes.into_boxed_slice()));
    programs.lock().ok()?.insert(key, bytes);
    bytes.map(|bytes| InstalledFontProgram {
        face: face_record(face),
        bytes,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn memoized(
    key: &str,
    resolve: impl FnOnce() -> Option<InstalledFontProgram>,
) -> Option<InstalledFontProgram> {
    type Memo = Mutex<HashMap<String, Option<InstalledFontProgram>>>;
    static MEMO: OnceLock<Memo> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    if let Some(hit) = memo.lock().ok()?.get(key) {
        return hit.clone();
    }
    let resolved = resolve();
    memo.lock().ok()?.insert(key.to_string(), resolved.clone());
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_key_strips_subset_tag_and_style() {
        assert_eq!(
            family_key("ABCDEF+TimesNewRomanPS-BoldItalicMT"),
            "timesnew"
        );
        assert_eq!(family_key("Arial,Bold"), "arial");
        assert_eq!(family_key("NotoSerif-Regular"), "notoserif");
    }

    #[test]
    fn installed_faces_resolve_by_name_and_cover_text_when_present() {
        enable_installed_fonts_for_test_thread();
        let faces = installed_font_faces();
        // Host-dependent: a host without fonts has nothing to resolve.
        for face in faces.iter().filter(|face| face.path.is_some()).take(40) {
            if let Some(program) = installed_font_named(&face.post_script_name, false, false, false)
            {
                assert_eq!(
                    normalize_font_key(&program.face.post_script_name),
                    normalize_font_key(&face.post_script_name)
                );
                let parsed = ttf_parser::Face::parse(program.bytes, 0).expect("standalone face");
                assert!(has_outlines(&parsed));
            }
        }
        if let Some(program) = installed_font_covering("Hello", &InstalledFontStyle::default()) {
            assert!(draws_all(program.bytes, "Hello"));
        }
    }

    #[test]
    fn design_heuristics() {
        assert!(name_looks_serif("Times New Roman"));
        assert!(!name_looks_serif("Noto Sans"));
        assert!(name_looks_monospace("Courier New"));
        assert!(name_is_bold("Arial-BoldMT"));
        assert!(name_is_italic("Garamond-Italic"));
    }
}
