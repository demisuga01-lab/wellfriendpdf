//! Deterministic font-provider abstraction.
//!
//! PDF rendering and generated output need a common way to resolve embedded,
//! Standard 14, and fallback fonts without depending on whatever happens to be
//! installed on the host. This module exposes the stable provider seam used by
//! Codec Boundary while the existing renderer continues to consume byte slices.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::render::font_rasterizer::get_fallback_font;

/// A request for a substitute or generated-output font face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontMatchRequest {
    /// PDF `/BaseFont` or requested family/PostScript name.
    pub base_font: String,
    /// Whether a bold-style face is desired.
    pub bold: bool,
    /// Whether an italic/oblique-style face is desired.
    pub italic: bool,
    /// Whether the PDF font is symbolic.
    pub symbolic: bool,
}

impl FontMatchRequest {
    /// Build a request from a PDF font name. Style and symbolic hints can be
    /// refined by callers from `FontDescriptor` flags when available.
    pub fn new(base_font: impl Into<String>) -> Self {
        Self {
            base_font: base_font.into(),
            bold: false,
            italic: false,
            symbolic: false,
        }
    }
}

/// Where a resolved font face came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontProviderSource {
    /// The PDF carried an embedded font program.
    Embedded,
    /// A built-in Standard 14 compatible face was used.
    Standard14,
    /// A deterministic bundled fallback face was used.
    BundledFallback,
    /// Caller-registered or configured font bytes were used.
    UserRegistered,
    /// A configured deterministic system-font mapping was used.
    DeterministicSystemMapping,
}

/// A resolved font-program match.
#[derive(Debug, Clone)]
pub struct FontMatch {
    /// Stable family label for diagnostics and reports.
    pub family_name: &'static str,
    /// The deterministic bundled lookup name actually selected by the provider.
    /// This can differ from the requested name when descriptor/style hints are
    /// needed to choose the correct bundled face.
    pub lookup_name: String,
    /// Resolution source.
    pub source: FontProviderSource,
    /// Font program bytes. Bundled matches are `'static`; future providers may
    /// wrap this behind owned/cache-backed storage.
    pub bytes: Cow<'static, [u8]>,
    /// Whether the match requires synthetic bolding to satisfy the request.
    pub synthetic_bold: bool,
    /// Whether the match requires synthetic slanting to satisfy the request.
    pub synthetic_italic: bool,
    /// Stable explanation used by font diagnostics.
    pub match_reason: &'static str,
}

/// Pluggable font lookup interface.
pub trait FontProvider {
    /// Resolve a font match for a PDF/rendering/generated-output request.
    fn match_font(&self, request: &FontMatchRequest) -> Option<FontMatch>;
}

/// Deterministic provider backed by the bundled Liberation and DejaVu faces.
#[derive(Debug, Default, Clone, Copy)]
pub struct BundledFontProvider;

impl FontProvider for BundledFontProvider {
    fn match_font(&self, request: &FontMatchRequest) -> Option<FontMatch> {
        let (lookup_name, match_reason) = selected_lookup_name(request);
        let bytes = get_fallback_font(&lookup_name)?;
        let actual_bold = lookup_name_contains_bold(&lookup_name);
        let actual_italic = lookup_name_contains_italic(&lookup_name);
        Some(FontMatch {
            family_name: bundled_family_label(&lookup_name),
            lookup_name,
            source: source_for_name(request, match_reason),
            bytes: Cow::Borrowed(bytes),
            synthetic_bold: request.bold && !actual_bold,
            synthetic_italic: request.italic && !actual_italic,
            match_reason,
        })
    }
}

/// Deterministic caller-owned font provider used by [`ContentEngine`] when
/// clients register explicit replacement font bytes.
#[derive(Debug, Clone)]
pub struct RegisteredFontProvider {
    faces: BTreeMap<String, RegisteredFontFace>,
    fingerprint: String,
}

#[derive(Debug, Clone)]
struct RegisteredFontFace {
    display_name: String,
    bytes: Vec<u8>,
}

impl Default for RegisteredFontProvider {
    fn default() -> Self {
        Self {
            faces: BTreeMap::new(),
            fingerprint: "registered-fonts:none".to_string(),
        }
    }
}

impl RegisteredFontProvider {
    /// Register deterministic caller-owned bytes under a PDF family or
    /// PostScript name. Returns `false` for empty names or empty byte payloads.
    pub fn register_font_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> bool {
        let display_name = name.into();
        let key = registered_font_key(&display_name);
        let bytes = bytes.into();
        if key.is_empty() || bytes.is_empty() {
            return false;
        }
        self.faces.insert(
            key,
            RegisteredFontFace {
                display_name,
                bytes,
            },
        );
        self.refresh_fingerprint();
        true
    }

    /// Stable source identity for cache keys and public substitution reports.
    pub fn cache_fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    fn refresh_fingerprint(&mut self) {
        if self.faces.is_empty() {
            self.fingerprint = "registered-fonts:none".to_string();
            return;
        }
        let mut hash = FNV1A64_OFFSET;
        for (key, face) in &self.faces {
            fnv1a_update(&mut hash, key.as_bytes());
            fnv1a_update(&mut hash, face.display_name.as_bytes());
            fnv1a_update(&mut hash, &(face.bytes.len() as u64).to_le_bytes());
            fnv1a_update(&mut hash, &face.bytes);
        }
        self.fingerprint = format!("registered-fonts:{}:{hash:016x}", self.faces.len());
    }
}

impl FontProvider for RegisteredFontProvider {
    fn match_font(&self, request: &FontMatchRequest) -> Option<FontMatch> {
        let key = registered_font_key(&request.base_font);
        let face = self.faces.get(&key)?;
        let actual_bold = lookup_name_contains_bold(&face.display_name);
        let actual_italic = lookup_name_contains_italic(&face.display_name);
        Some(FontMatch {
            family_name: "User Registered",
            lookup_name: face.display_name.clone(),
            source: FontProviderSource::UserRegistered,
            bytes: Cow::Owned(face.bytes.clone()),
            synthetic_bold: request.bold && !actual_bold,
            synthetic_italic: request.italic && !actual_italic,
            match_reason: "user_registered_direct_match",
        })
    }
}

fn selected_lookup_name(request: &FontMatchRequest) -> (String, &'static str) {
    let mut name = request.base_font.clone();
    if request.symbolic && !is_symbolic_lookup_name(&name) {
        return ("Symbol".to_string(), "symbolic_flag");
    }
    if !is_standard14_name(&request.base_font) {
        if let Some(family) = deterministic_system_family(&request.base_font) {
            let bold = request.bold || lookup_name_contains_bold(&request.base_font);
            let italic = request.italic || lookup_name_contains_italic(&request.base_font);
            return (
                standard14_lookup_for_family(family, bold, italic).to_string(),
                "deterministic_system_mapping",
            );
        }
    }
    if request.bold && !lookup_name_contains_bold(&name) {
        name.push_str("-Bold");
    }
    if request.italic && !lookup_name_contains_italic(&name) {
        name.push_str("-Italic");
    }
    let reason = if name == request.base_font {
        "name"
    } else {
        "style_hint"
    };
    (name, reason)
}

fn lookup_name_contains_bold(name: &str) -> bool {
    let normalized = normalized_font_name(name);
    normalized.contains("bold")
        || normalized.contains("heavy")
        || normalized.contains("black")
        || normalized.ends_with('b')
}

fn lookup_name_contains_italic(name: &str) -> bool {
    let normalized = normalized_font_name(name);
    normalized.contains("italic")
        || normalized.contains("oblique")
        || normalized.contains("slant")
        || normalized.ends_with("-i")
        || normalized.ends_with("-o")
}

fn source_for_name(request: &FontMatchRequest, match_reason: &str) -> FontProviderSource {
    if is_standard14_name(&request.base_font) {
        FontProviderSource::Standard14
    } else if match_reason == "deterministic_system_mapping" {
        FontProviderSource::DeterministicSystemMapping
    } else {
        FontProviderSource::BundledFallback
    }
}

fn bundled_family_label(name: &str) -> &'static str {
    let normalized = normalized_font_name(name);
    if is_symbolic_lookup_name(&normalized) {
        "DejaVu Sans"
    } else if normalized.contains("courier")
        || normalized.contains("mono")
        || normalized.contains("typewriter")
    {
        "Liberation Mono"
    } else if normalized.contains("times")
        || normalized.contains("serif")
        || normalized.contains("palatino")
        || normalized.contains("bookman")
    {
        "Liberation Serif"
    } else {
        "Liberation Sans"
    }
}

fn is_symbolic_lookup_name(name: &str) -> bool {
    let normalized = normalized_font_name(name);
    normalized.contains("symbol")
        || normalized.contains("dingbat")
        || normalized.contains("wingding")
        || normalized.contains("webding")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeterministicSystemFamily {
    Sans,
    Serif,
    Mono,
}

fn deterministic_system_family(name: &str) -> Option<DeterministicSystemFamily> {
    let normalized = normalized_font_name(name);
    if is_symbolic_lookup_name(&normalized) {
        return None;
    }
    let compact = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if [
        "arial",
        "arialmt",
        "arialnarrow",
        "calibri",
        "segoeui",
        "tahoma",
        "verdana",
        "helveticaneue",
    ]
    .iter()
    .any(|token| compact.starts_with(token))
    {
        Some(DeterministicSystemFamily::Sans)
    } else if [
        "timesnewroman",
        "timesnewromanps",
        "georgia",
        "cambria",
        "constantia",
        "garamond",
    ]
    .iter()
    .any(|token| compact.starts_with(token))
    {
        Some(DeterministicSystemFamily::Serif)
    } else if ["couriernew", "consolas", "inconsolata", "lucidaconsole"]
        .iter()
        .any(|token| compact.starts_with(token))
    {
        Some(DeterministicSystemFamily::Mono)
    } else {
        None
    }
}

fn standard14_lookup_for_family(
    family: DeterministicSystemFamily,
    bold: bool,
    italic: bool,
) -> &'static str {
    match family {
        DeterministicSystemFamily::Sans => match (bold, italic) {
            (true, true) => "Helvetica-BoldOblique",
            (true, false) => "Helvetica-Bold",
            (false, true) => "Helvetica-Oblique",
            (false, false) => "Helvetica",
        },
        DeterministicSystemFamily::Serif => match (bold, italic) {
            (true, true) => "Times-BoldItalic",
            (true, false) => "Times-Bold",
            (false, true) => "Times-Italic",
            (false, false) => "Times-Roman",
        },
        DeterministicSystemFamily::Mono => match (bold, italic) {
            (true, true) => "Courier-BoldOblique",
            (true, false) => "Courier-Bold",
            (false, true) => "Courier-Oblique",
            (false, false) => "Courier",
        },
    }
}

fn registered_font_key(name: &str) -> String {
    normalized_font_name(name)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

const FNV1A64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
}

/// True for the PDF Standard 14 base fonts after removing subset prefixes.
pub fn is_standard14_name(name: &str) -> bool {
    matches!(
        normalized_font_name(name).as_str(),
        "courier"
            | "courier-bold"
            | "courier-oblique"
            | "courier-boldoblique"
            | "helvetica"
            | "helvetica-bold"
            | "helvetica-oblique"
            | "helvetica-boldoblique"
            | "times-roman"
            | "times-bold"
            | "times-italic"
            | "times-bolditalic"
            | "symbol"
            | "zapfdingbats"
    )
}

fn normalized_font_name(name: &str) -> String {
    let raw = name.trim_start_matches('/');
    let raw = raw.find('+').map_or(raw, |idx| &raw[idx + 1..]);
    raw.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_14_with_subset_prefix() {
        assert!(is_standard14_name("ABCDEE+Helvetica-Bold"));
        assert!(is_standard14_name("/Times-Roman"));
        assert!(is_standard14_name("ZapfDingbats"));
        assert!(!is_standard14_name("ABCDEE+SomeCorporateSans"));
    }

    #[test]
    fn bundled_provider_is_deterministic() {
        let provider = BundledFontProvider;
        let request = FontMatchRequest::new("Helvetica");
        let first = provider.match_font(&request).expect("bundled match");
        let second = provider.match_font(&request).expect("bundled match");

        assert_eq!(first.family_name, "Liberation Sans");
        assert_eq!(first.source, FontProviderSource::Standard14);
        assert_eq!(first.bytes.as_ptr(), second.bytes.as_ptr());
        assert!(!first.bytes.is_empty());
        assert_eq!(first.match_reason, "name");
    }

    #[test]
    fn symbolic_fonts_route_to_symbol_coverage_fallback() {
        let provider = BundledFontProvider;
        let result = provider
            .match_font(&FontMatchRequest::new("Symbol"))
            .expect("symbol match");
        assert_eq!(result.family_name, "DejaVu Sans");
        assert_eq!(result.source, FontProviderSource::Standard14);
    }

    #[test]
    fn explicit_style_hints_are_deterministic() {
        let provider = BundledFontProvider;
        let mut request = FontMatchRequest::new("Helvetica");
        request.bold = true;
        request.italic = true;
        let hinted = provider.match_font(&request).expect("hinted font");

        assert_eq!(hinted.family_name, "Liberation Sans");
        assert_eq!(hinted.lookup_name, "Helvetica-Bold-Italic");
        assert_eq!(hinted.match_reason, "style_hint");
        assert!(!hinted.synthetic_bold);
        assert!(!hinted.synthetic_italic);
    }

    #[test]
    fn deterministic_system_aliases_report_mapping_source() {
        let provider = BundledFontProvider;
        let arial = provider
            .match_font(&FontMatchRequest::new("ArialMT"))
            .expect("Arial alias should map deterministically");
        assert_eq!(arial.lookup_name, "Helvetica");
        assert_eq!(arial.family_name, "Liberation Sans");
        assert_eq!(arial.source, FontProviderSource::DeterministicSystemMapping);
        assert_eq!(arial.match_reason, "deterministic_system_mapping");

        let mut times = FontMatchRequest::new("TimesNewRomanPSMT");
        times.bold = true;
        times.italic = true;
        let mapped = provider
            .match_font(&times)
            .expect("Times New Roman alias should map deterministically");
        assert_eq!(mapped.lookup_name, "Times-BoldItalic");
        assert_eq!(mapped.family_name, "Liberation Serif");
        assert_eq!(
            mapped.source,
            FontProviderSource::DeterministicSystemMapping
        );
        assert_eq!(mapped.match_reason, "deterministic_system_mapping");
    }

    #[test]
    fn symbolic_system_aliases_do_not_report_system_mapping() {
        let provider = BundledFontProvider;
        let segoe_symbol = provider
            .match_font(&FontMatchRequest::new("SegoeUISymbol"))
            .expect("symbolic system alias should resolve through coverage fallback");
        assert_eq!(segoe_symbol.family_name, "DejaVu Sans");
        assert_eq!(segoe_symbol.source, FontProviderSource::BundledFallback);
        assert_eq!(segoe_symbol.match_reason, "name");

        let mut symbolic_arial = FontMatchRequest::new("ArialMT");
        symbolic_arial.symbolic = true;
        let symbolic = provider
            .match_font(&symbolic_arial)
            .expect("symbolic descriptor should prefer coverage fallback");
        assert_eq!(symbolic.lookup_name, "Symbol");
        assert_eq!(symbolic.family_name, "DejaVu Sans");
        assert_eq!(symbolic.source, FontProviderSource::BundledFallback);
        assert_eq!(symbolic.match_reason, "symbolic_flag");
    }

    #[test]
    fn registered_provider_matches_subset_names_and_fingerprints_bytes() {
        let mut provider = RegisteredFontProvider::default();
        let empty_fingerprint = provider.cache_fingerprint().to_string();
        assert!(!provider.register_font_bytes("", vec![1, 2, 3]));
        assert!(!provider.register_font_bytes("RegisteredSans", Vec::<u8>::new()));

        assert!(provider.register_font_bytes("Registered Sans", vec![1, 2, 3, 4]));
        assert_ne!(provider.cache_fingerprint(), empty_fingerprint);

        let matched = provider
            .match_font(&FontMatchRequest::new("ABCDEF+RegisteredSans"))
            .expect("subset-prefixed registered font should resolve");
        assert_eq!(matched.source, FontProviderSource::UserRegistered);
        assert_eq!(matched.lookup_name, "Registered Sans");
        assert_eq!(matched.bytes.as_ref(), &[1, 2, 3, 4]);
        assert_eq!(matched.match_reason, "user_registered_direct_match");

        let first_fingerprint = provider.cache_fingerprint().to_string();
        assert!(provider.register_font_bytes("Registered Sans", vec![4, 3, 2, 1]));
        assert_ne!(provider.cache_fingerprint(), first_fingerprint);
    }

    #[test]
    fn symbolic_descriptor_hint_routes_unknown_fonts_to_symbol_coverage_face() {
        let provider = BundledFontProvider;
        let mut request = FontMatchRequest::new("CorporatePi");
        request.symbolic = true;
        let matched = provider
            .match_font(&request)
            .expect("symbolic hinted font should resolve");

        assert_eq!(matched.lookup_name, "Symbol");
        assert_eq!(matched.family_name, "DejaVu Sans");
        assert_eq!(matched.source, FontProviderSource::BundledFallback);
        assert_eq!(matched.match_reason, "symbolic_flag");
        assert!(!matched.bytes.is_empty());
    }

    #[test]
    fn standard_14_families_resolve_to_deterministic_bundled_faces() {
        let provider = BundledFontProvider;
        for (name, family) in [
            ("Times-Roman", "Liberation Serif"),
            ("Times-BoldItalic", "Liberation Serif"),
            ("Helvetica", "Liberation Sans"),
            ("Helvetica-Oblique", "Liberation Sans"),
            ("Courier", "Liberation Mono"),
            ("Courier-BoldOblique", "Liberation Mono"),
            ("Symbol", "DejaVu Sans"),
            ("ZapfDingbats", "DejaVu Sans"),
        ] {
            let matched = provider
                .match_font(&FontMatchRequest::new(name))
                .unwrap_or_else(|| panic!("standard 14 font {name} should resolve"));
            assert_eq!(matched.source, FontProviderSource::Standard14, "{name}");
            assert_eq!(matched.family_name, family, "{name}");
            assert!(!matched.bytes.is_empty(), "{name}");
        }
    }
}
