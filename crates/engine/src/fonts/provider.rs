//! Deterministic font-provider abstraction.
//!
//! PDF rendering and generated output need a common way to resolve embedded,
//! Standard 14, and fallback fonts without depending on whatever happens to be
//! installed on the host. This module exposes the stable provider seam used by
//! Prompt 04 while the existing renderer continues to consume byte slices.

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
}

/// A resolved font-program match.
#[derive(Debug, Clone)]
pub struct FontMatch {
    /// Stable family label for diagnostics and reports.
    pub family_name: &'static str,
    /// Resolution source.
    pub source: FontProviderSource,
    /// Font program bytes. Bundled matches are `'static`; future providers may
    /// wrap this behind owned/cache-backed storage.
    pub bytes: &'static [u8],
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
        let lookup_name = styled_lookup_name(request);
        let bytes = get_fallback_font(&lookup_name)?;
        let actual_bold = lookup_name_contains_bold(&lookup_name);
        let actual_italic = lookup_name_contains_italic(&lookup_name);
        Some(FontMatch {
            family_name: bundled_family_label(&lookup_name),
            source: source_for_name(&request.base_font),
            bytes,
            synthetic_bold: request.bold && !actual_bold,
            synthetic_italic: request.italic && !actual_italic,
            match_reason: if lookup_name == request.base_font {
                "name"
            } else {
                "style_hint"
            },
        })
    }
}

fn styled_lookup_name(request: &FontMatchRequest) -> String {
    let mut name = request.base_font.clone();
    if request.bold && !lookup_name_contains_bold(&name) {
        name.push_str("-Bold");
    }
    if request.italic && !lookup_name_contains_italic(&name) {
        name.push_str("-Italic");
    }
    name
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

fn source_for_name(name: &str) -> FontProviderSource {
    if is_standard14_name(name) {
        FontProviderSource::Standard14
    } else {
        FontProviderSource::BundledFallback
    }
}

fn bundled_family_label(name: &str) -> &'static str {
    let normalized = normalized_font_name(name);
    if normalized.contains("symbol")
        || normalized.contains("dingbat")
        || normalized.contains("wingding")
        || normalized.contains("webding")
    {
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
        assert_eq!(hinted.match_reason, "style_hint");
        assert!(!hinted.synthetic_bold);
        assert!(!hinted.synthetic_italic);
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
