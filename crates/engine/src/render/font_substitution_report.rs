//! Deterministic font substitution reporting (RB-11).
//!
//! Records events when the renderer uses a bundled fallback font instead of an
//! embedded or system font at render time. Events are structured, bounded, and
//! survive cache/state transfers so that callers can inspect substitution
//! decisions after a render pass without altering output semantics.

use serde::Serialize;

/// Maximum number of font substitution events retained per render pass.
/// This bounds memory usage regardless of how many unique fonts a document
/// references. Once the cap is reached, additional substitution events are
/// counted but not stored.
const MAX_FONT_SUBSTITUTION_EVENTS: usize = 1024;

/// Reason a font substitution occurred during rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FontSubstitutionReason {
    /// The PDF font resource did not embed a font program and no system font
    /// was available, so a bundled fallback was selected.
    MissingFont,
    /// The PDF font resource's embedded program could not be decoded or was
    /// empty, so a bundled fallback was selected.
    BundledFallback,
}

/// Metric/coverage posture for a font substitution event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FontSubstitutionMetricPosture {
    /// The bundled fallback has reasonable metric compatibility for the
    /// requested font family class (e.g. serif→Liberation Serif).
    MetricCompatible,
    /// The bundled fallback covers the requested glyphs but metrics may differ
    /// significantly (e.g. symbolic font → DejaVu Sans).
    CoverageOnly,
    /// Metric compatibility is unknown or the fallback is a generic last-resort.
    Unknown,
}

/// A single font substitution event recorded during rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontSubstitutionEvent {
    /// The font name requested by the PDF resource (as declared in the font
    /// dictionary, typically /BaseFont or the resource key).
    pub requested_font: String,
    /// The bundled fallback font file that was actually selected.
    pub selected_fallback: String,
    /// Why the substitution occurred.
    pub reason: FontSubstitutionReason,
    /// The 1-based page number where this substitution was first observed.
    pub page: usize,
    /// Metric/coverage posture of the substitution.
    pub metric_posture: FontSubstitutionMetricPosture,
}

/// Bounded container for font substitution events collected during a render
/// pass. Survives transfer between `RenderState` and `RenderDocumentCache`.
#[derive(Debug, Clone, Default)]
pub struct FontSubstitutionLog {
    events: Vec<FontSubstitutionEvent>,
    /// Number of events that were dropped because the log was at capacity.
    overflow_count: usize,
}

impl FontSubstitutionLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            overflow_count: 0,
        }
    }

    /// Record a font substitution event. If the log is at capacity, the event
    /// is counted but not stored.
    pub fn record(&mut self, event: FontSubstitutionEvent) {
        if self.events.len() < MAX_FONT_SUBSTITUTION_EVENTS {
            self.events.push(event);
        } else {
            self.overflow_count = self.overflow_count.saturating_add(1);
        }
    }

    /// Absorb all events from another log (used during child state merge).
    pub fn absorb(&mut self, other: FontSubstitutionLog) {
        for event in other.events {
            self.record(event);
        }
        self.overflow_count = self.overflow_count.saturating_add(other.overflow_count);
    }

    /// All recorded events.
    pub fn events(&self) -> &[FontSubstitutionEvent] {
        &self.events
    }

    /// Number of events that were dropped because the log was at capacity.
    pub fn overflow_count(&self) -> usize {
        self.overflow_count
    }

    /// Total number of substitution occurrences (stored + overflowed).
    pub fn total_count(&self) -> usize {
        self.events.len().saturating_add(self.overflow_count)
    }

    /// Whether any substitution was recorded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.overflow_count == 0
    }

    /// Clear all events and reset overflow.
    pub fn clear(&mut self) {
        self.events.clear();
        self.overflow_count = 0;
    }
}

/// Classify the metric posture of a fallback font selection based on the
/// requested font name and the selected fallback name.
pub fn classify_metric_posture(
    requested_font: &str,
    selected_fallback: &str,
) -> FontSubstitutionMetricPosture {
    let req_lower = requested_font.to_lowercase();
    let sel_lower = selected_fallback.to_lowercase();

    // Symbolic fonts mapped to DejaVu are coverage-only.
    if req_lower.contains("symbol")
        || req_lower.contains("dingbat")
        || req_lower.contains("wingding")
        || req_lower.contains("webding")
    {
        return FontSubstitutionMetricPosture::CoverageOnly;
    }

    // Liberation family is metric-compatible with its target families.
    if sel_lower.contains("liberation") {
        if sel_lower.contains("sans")
            && (req_lower.contains("arial")
                || req_lower.contains("helvetica")
                || req_lower.contains("sans"))
        {
            return FontSubstitutionMetricPosture::MetricCompatible;
        }
        if sel_lower.contains("serif")
            && (req_lower.contains("times")
                || req_lower.contains("serif")
                || req_lower.contains("georgia")
                || req_lower.contains("palatino"))
        {
            return FontSubstitutionMetricPosture::MetricCompatible;
        }
        if sel_lower.contains("mono")
            && (req_lower.contains("courier")
                || req_lower.contains("mono")
                || req_lower.contains("consolas"))
        {
            return FontSubstitutionMetricPosture::MetricCompatible;
        }
    }

    FontSubstitutionMetricPosture::Unknown
}

/// Determine the human-readable name of the fallback font selected for a given
/// font name. This mirrors the logic in `get_fallback_font` without returning
/// the bytes.
pub fn fallback_font_display_name(font_name: &str) -> &'static str {
    let raw = font_name.trim_start_matches('/');
    let raw = raw.find('+').map_or(raw, |idx| &raw[idx + 1..]);
    let name = raw.to_lowercase();

    let is_bold = name.contains("bold")
        || name.contains("-b")
        || name.ends_with('b')
        || name.contains("heavy")
        || name.contains("black");
    let is_italic = name.contains("italic")
        || name.contains("oblique")
        || name.contains("slant")
        || name.ends_with("-i")
        || name.ends_with("-o");

    if name.contains("symbol")
        || name.contains("dingbat")
        || name.contains("wingding")
        || name.contains("webding")
    {
        return "DejaVuSans";
    }

    if name.contains("courier")
        || name.contains("mono")
        || name.contains("typewriter")
        || name.contains("consolas")
        || name.contains("inconsolata")
        || name.contains("sourcecodemono")
        || name.contains("lucidaconsole")
    {
        return match (is_bold, is_italic) {
            (true, true) => "LiberationMono-BoldItalic",
            (true, false) => "LiberationMono-Bold",
            (false, true) => "LiberationMono-Italic",
            (false, false) => "LiberationMono-Regular",
        };
    }

    if name.contains("times")
        || name.contains("serif")
        || name.contains("georgia")
        || name.contains("palatino")
        || name.contains("bookman")
        || name.contains("garamond")
        || name.contains("cambria")
        || name.contains("constantia")
    {
        return match (is_bold, is_italic) {
            (true, true) => "LiberationSerif-BoldItalic",
            (true, false) => "LiberationSerif-Bold",
            (false, true) => "LiberationSerif-Italic",
            (false, false) => "LiberationSerif-Regular",
        };
    }

    match (is_bold, is_italic) {
        (true, true) => "LiberationSans-BoldItalic",
        (true, false) => "LiberationSans-Bold",
        (false, true) => "LiberationSans-Italic",
        (false, false) => "LiberationSans-Regular",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_records_events_up_to_capacity() {
        let mut log = FontSubstitutionLog::new();
        assert!(log.is_empty());
        assert_eq!(log.total_count(), 0);

        log.record(FontSubstitutionEvent {
            requested_font: "Helvetica".to_string(),
            selected_fallback: "LiberationSans-Regular".to_string(),
            reason: FontSubstitutionReason::MissingFont,
            page: 1,
            metric_posture: FontSubstitutionMetricPosture::MetricCompatible,
        });

        assert!(!log.is_empty());
        assert_eq!(log.events().len(), 1);
        assert_eq!(log.total_count(), 1);
        assert_eq!(log.overflow_count(), 0);
    }

    #[test]
    fn log_bounds_events_at_capacity() {
        let mut log = FontSubstitutionLog::new();
        for i in 0..MAX_FONT_SUBSTITUTION_EVENTS + 10 {
            log.record(FontSubstitutionEvent {
                requested_font: format!("Font{i}"),
                selected_fallback: "LiberationSans-Regular".to_string(),
                reason: FontSubstitutionReason::BundledFallback,
                page: 1,
                metric_posture: FontSubstitutionMetricPosture::Unknown,
            });
        }
        assert_eq!(log.events().len(), MAX_FONT_SUBSTITUTION_EVENTS);
        assert_eq!(log.overflow_count(), 10);
        assert_eq!(log.total_count(), MAX_FONT_SUBSTITUTION_EVENTS + 10);
    }

    #[test]
    fn log_absorb_merges_child_events() {
        let mut parent = FontSubstitutionLog::new();
        parent.record(FontSubstitutionEvent {
            requested_font: "TimesNewRoman".to_string(),
            selected_fallback: "LiberationSerif-Regular".to_string(),
            reason: FontSubstitutionReason::MissingFont,
            page: 1,
            metric_posture: FontSubstitutionMetricPosture::MetricCompatible,
        });

        let mut child = FontSubstitutionLog::new();
        child.record(FontSubstitutionEvent {
            requested_font: "CourierNew".to_string(),
            selected_fallback: "LiberationMono-Regular".to_string(),
            reason: FontSubstitutionReason::BundledFallback,
            page: 2,
            metric_posture: FontSubstitutionMetricPosture::MetricCompatible,
        });

        parent.absorb(child);
        assert_eq!(parent.events().len(), 2);
        assert_eq!(parent.events()[0].requested_font, "TimesNewRoman");
        assert_eq!(parent.events()[1].requested_font, "CourierNew");
    }

    #[test]
    fn classify_metric_posture_returns_expected_values() {
        assert_eq!(
            classify_metric_posture("Helvetica", "LiberationSans-Regular"),
            FontSubstitutionMetricPosture::MetricCompatible
        );
        assert_eq!(
            classify_metric_posture("Symbol", "DejaVuSans"),
            FontSubstitutionMetricPosture::CoverageOnly
        );
        assert_eq!(
            classify_metric_posture("CustomFont", "LiberationSans-Regular"),
            FontSubstitutionMetricPosture::Unknown
        );
        assert_eq!(
            classify_metric_posture("Times-Roman", "LiberationSerif-Regular"),
            FontSubstitutionMetricPosture::MetricCompatible
        );
        assert_eq!(
            classify_metric_posture("Courier", "LiberationMono-Regular"),
            FontSubstitutionMetricPosture::MetricCompatible
        );
    }

    #[test]
    fn fallback_font_display_name_matches_get_fallback_font_logic() {
        assert_eq!(
            fallback_font_display_name("Helvetica"),
            "LiberationSans-Regular"
        );
        assert_eq!(
            fallback_font_display_name("Helvetica-Bold"),
            "LiberationSans-Bold"
        );
        assert_eq!(
            fallback_font_display_name("Times-Italic"),
            "LiberationSerif-Italic"
        );
        assert_eq!(
            fallback_font_display_name("Courier"),
            "LiberationMono-Regular"
        );
        assert_eq!(fallback_font_display_name("Symbol"), "DejaVuSans");
        assert_eq!(fallback_font_display_name("ZapfDingbats"), "DejaVuSans");
        assert_eq!(
            fallback_font_display_name("ABCDEF+ArialMT"),
            "LiberationSans-Regular"
        );
    }

    #[test]
    fn log_clear_resets_state() {
        let mut log = FontSubstitutionLog::new();
        log.record(FontSubstitutionEvent {
            requested_font: "Test".to_string(),
            selected_fallback: "LiberationSans-Regular".to_string(),
            reason: FontSubstitutionReason::MissingFont,
            page: 1,
            metric_posture: FontSubstitutionMetricPosture::Unknown,
        });
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.events().len(), 0);
        assert_eq!(log.overflow_count(), 0);
    }
}
