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

/// Maximum number of missing glyphs sampled in one event. Counts remain exact
/// for the observed text run; samples are bounded to keep report size stable.
pub(crate) const MAX_MISSING_GLYPH_SAMPLES: usize = 8;

/// Reason a font substitution occurred during rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FontSubstitutionReason {
    /// The PDF referenced a Standard 14 font that is valid without embedding;
    /// the deterministic compatible face bundled with the renderer was used.
    Standard14,
    /// A caller-registered or document-provided deterministic font face was
    /// used instead of an embedded program.
    DocumentProvided,
    /// A configured deterministic system mapping was used.
    DeterministicSystemMapping,
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

/// Bounded coverage summary for the text run that first observed a fallback
/// font selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FontSubstitutionGlyphCoverage {
    /// Paintable decoded glyphs observed for the first text run using this
    /// fallback selection. Control characters and spaces are not counted.
    pub observed_glyphs: usize,
    /// Observed paintable glyphs that map to a non-.notdef glyph in the selected
    /// fallback font.
    pub covered_glyphs: usize,
    /// Observed paintable glyphs that do not map to a usable fallback glyph.
    pub missing_glyphs: usize,
    /// Bounded ASCII samples for missing glyphs, e.g. `U+0041 code=65`.
    pub missing_glyph_samples: Vec<String>,
}

/// A single font substitution event recorded during rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontSubstitutionEvent {
    /// The font name requested by the PDF resource (as declared in the font
    /// dictionary, typically /BaseFont or the resource key).
    pub requested_font: String,
    /// The resolved PDF font identity used for policy reporting. This is the
    /// `/BaseFont` or descendant CID font name when available, otherwise the
    /// resource name from the content stream.
    pub requested_pdf_font: String,
    /// The bundled fallback font file that was actually selected.
    pub selected_fallback: String,
    /// The selected replacement font identity. Mirrors `selected_fallback` for
    /// the current bundled fallback policy and leaves a stable field name for
    /// future deterministic system mappings.
    pub selected_replacement: String,
    /// Why the substitution occurred.
    pub reason: FontSubstitutionReason,
    /// Stable provider-side reason for the selected replacement face, such as
    /// a direct name match, style hint, or symbolic-descriptor routing.
    pub selection_reason: String,
    /// Deterministic source embedded-font state observed before replacement.
    pub embedded_state: String,
    /// Compact PDF encoding summary for the substituted font resource.
    pub encoding: String,
    /// Deterministic replacement source, e.g. bundled fallback.
    pub resolution_source: String,
    /// The 1-based page number where this substitution was first observed.
    pub page: usize,
    /// Metric/coverage posture of the substitution.
    pub metric_posture: FontSubstitutionMetricPosture,
    /// Bounded coverage summary for the first observed text run using this
    /// fallback selection.
    pub glyph_coverage: FontSubstitutionGlyphCoverage,
    /// Policy-named mirror of `glyph_coverage` for callers that need explicit
    /// required-glyph coverage without knowing the older field name.
    pub required_glyph_coverage: FontSubstitutionGlyphCoverage,
    /// Convenience copy of `glyph_coverage.missing_glyphs`.
    pub missing_glyphs: usize,
    /// Deterministic visual-risk bucket derived from metric posture and glyph
    /// coverage.
    pub visual_risk_category: String,
    /// Deterministic extraction impact statement. Rendering substitution does
    /// not mutate PDF text extraction, but missing visual glyphs are recorded.
    pub extraction_impact: String,
    /// Deterministic editing/reflow impact statement for callers that surface
    /// replacement risks in editing workflows.
    pub editing_impact: String,
    /// Stable identity of the active font-resolution policy and render contract.
    pub font_policy_identity: String,
    /// Deterministic risk flags for callers that need an explicit policy view.
    pub risk_flags: Vec<String>,
}

impl FontSubstitutionEvent {
    /// Construct a deterministic bundled-fallback event. The initial event may
    /// not yet know glyph coverage; call [`Self::set_glyph_coverage`] after the
    /// text run is decoded.
    #[allow(clippy::too_many_arguments)]
    pub fn bundled_fallback(
        requested_font: impl Into<String>,
        requested_pdf_font: impl Into<String>,
        selected_fallback: impl Into<String>,
        reason: FontSubstitutionReason,
        embedded_state: impl Into<String>,
        encoding: impl Into<String>,
        page: usize,
        metric_posture: FontSubstitutionMetricPosture,
        font_policy_identity: impl Into<String>,
    ) -> Self {
        let glyph_coverage = FontSubstitutionGlyphCoverage::default();
        let risk_flags = font_substitution_risk_flags(&reason, &metric_posture, &glyph_coverage);
        let selected_fallback = selected_fallback.into();
        let mut event = Self {
            requested_font: requested_font.into(),
            requested_pdf_font: requested_pdf_font.into(),
            selected_replacement: selected_fallback.clone(),
            selected_fallback,
            reason,
            selection_reason: "default_policy".to_string(),
            embedded_state: embedded_state.into(),
            encoding: encoding.into(),
            resolution_source: "bundled_fallback".to_string(),
            page,
            metric_posture,
            glyph_coverage: glyph_coverage.clone(),
            required_glyph_coverage: glyph_coverage,
            missing_glyphs: 0,
            visual_risk_category: String::new(),
            extraction_impact: String::new(),
            editing_impact: String::new(),
            font_policy_identity: font_policy_identity.into(),
            risk_flags,
        };
        event.refresh_policy_fields();
        event
    }

    /// Override the resolution source for deterministic non-generic
    /// replacements such as Standard 14 compatible faces. The constructor keeps
    /// the historical bundled-fallback default for compatibility with existing
    /// callers that create events directly.
    pub fn with_resolution_source(mut self, source: impl Into<String>) -> Self {
        self.resolution_source = source.into();
        self
    }

    /// Override the provider-side selection reason for the replacement face.
    pub fn with_selection_reason(mut self, reason: impl Into<String>) -> Self {
        self.selection_reason = reason.into();
        self
    }

    /// Update glyph coverage and all derived risk/impact fields together.
    pub fn set_glyph_coverage(&mut self, glyph_coverage: FontSubstitutionGlyphCoverage) {
        self.glyph_coverage = glyph_coverage.clone();
        self.required_glyph_coverage = glyph_coverage;
        self.refresh_policy_fields();
    }

    fn same_single_flight_selection(&self, other: &Self) -> bool {
        self.requested_font == other.requested_font
            && self.requested_pdf_font == other.requested_pdf_font
            && self.selected_fallback == other.selected_fallback
            && self.selected_replacement == other.selected_replacement
            && self.reason == other.reason
            && self.selection_reason == other.selection_reason
            && self.embedded_state == other.embedded_state
            && self.encoding == other.encoding
            && self.resolution_source == other.resolution_source
            && self.metric_posture == other.metric_posture
            && self.font_policy_identity == other.font_policy_identity
    }

    fn refresh_policy_fields(&mut self) {
        self.missing_glyphs = self.glyph_coverage.missing_glyphs;
        self.visual_risk_category =
            font_substitution_visual_risk_category(&self.metric_posture, &self.glyph_coverage)
                .to_string();
        self.extraction_impact = font_substitution_extraction_impact(&self.glyph_coverage);
        self.editing_impact =
            font_substitution_editing_impact(&self.metric_posture, &self.glyph_coverage);
        self.risk_flags =
            font_substitution_risk_flags(&self.reason, &self.metric_posture, &self.glyph_coverage);
    }
}

/// Bounded container for font substitution events collected during a render
/// pass. Survives transfer between `RenderState` and `RenderDocumentCache`.
#[derive(Debug, Clone, Default, Serialize)]
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
        let _ = self.record_indexed(event);
    }

    /// Record a font substitution event and return its stored index when the
    /// event fits inside the bounded log. Used by render code that enriches the
    /// event after text-run decoding.
    pub(crate) fn record_indexed(&mut self, event: FontSubstitutionEvent) -> Option<usize> {
        if self.events.len() < MAX_FONT_SUBSTITUTION_EVENTS {
            let index = self.events.len();
            self.events.push(event);
            Some(index)
        } else {
            self.overflow_count = self.overflow_count.saturating_add(1);
            None
        }
    }

    /// Record a renderer-originated substitution event only if an equivalent
    /// policy/font selection has not already been observed. The first event
    /// keeps its page number and first-run glyph coverage.
    pub(crate) fn record_single_flight_indexed(
        &mut self,
        event: FontSubstitutionEvent,
    ) -> Option<usize> {
        if let Some(existing) = self
            .events
            .iter_mut()
            .find(|existing| existing.same_single_flight_selection(&event))
        {
            if existing.glyph_coverage == FontSubstitutionGlyphCoverage::default()
                && event.glyph_coverage != FontSubstitutionGlyphCoverage::default()
            {
                existing.set_glyph_coverage(event.glyph_coverage);
            }
            return None;
        }
        self.record_indexed(event)
    }

    /// Mutably access one stored event by index.
    pub(crate) fn event_mut(&mut self, index: usize) -> Option<&mut FontSubstitutionEvent> {
        self.events.get_mut(index)
    }

    /// Absorb all events from another log (used during child state merge).
    pub fn absorb(&mut self, other: FontSubstitutionLog) {
        for event in other.events {
            let _ = self.record_single_flight_indexed(event);
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

/// Build deterministic risk flags from fallback reason, metric posture, and
/// bounded glyph coverage.
pub(crate) fn font_substitution_risk_flags(
    reason: &FontSubstitutionReason,
    metric_posture: &FontSubstitutionMetricPosture,
    coverage: &FontSubstitutionGlyphCoverage,
) -> Vec<String> {
    let mut flags = Vec::new();
    match reason {
        FontSubstitutionReason::Standard14 => {
            flags.push("source_standard14_compatible_face".to_string());
        }
        FontSubstitutionReason::DocumentProvided => {
            flags.push("source_document_provided_font".to_string());
        }
        FontSubstitutionReason::DeterministicSystemMapping => {
            flags.push("source_deterministic_system_mapping".to_string());
        }
        FontSubstitutionReason::MissingFont => {
            flags.push("source_font_program_missing".to_string());
        }
        FontSubstitutionReason::BundledFallback => {
            flags.push("source_font_program_unusable_or_unembedded".to_string());
        }
    }
    match metric_posture {
        FontSubstitutionMetricPosture::MetricCompatible => {}
        FontSubstitutionMetricPosture::CoverageOnly => {
            flags.push("fallback_metrics_may_differ".to_string());
        }
        FontSubstitutionMetricPosture::Unknown => {
            flags.push("fallback_metric_compatibility_unknown".to_string());
        }
    }
    if coverage.observed_glyphs == 0 {
        flags.push("no_paintable_glyphs_observed".to_string());
    }
    if coverage.missing_glyphs > 0 {
        flags.push("fallback_missing_observed_glyphs".to_string());
    }
    flags
}

fn font_substitution_visual_risk_category(
    metric_posture: &FontSubstitutionMetricPosture,
    coverage: &FontSubstitutionGlyphCoverage,
) -> &'static str {
    if coverage.missing_glyphs > 0 {
        return "high_missing_fallback_glyphs";
    }
    match metric_posture {
        FontSubstitutionMetricPosture::MetricCompatible => "low_metric_compatible",
        FontSubstitutionMetricPosture::CoverageOnly => "medium_coverage_only",
        FontSubstitutionMetricPosture::Unknown => "medium_metric_compatibility_unknown",
    }
}

fn font_substitution_extraction_impact(coverage: &FontSubstitutionGlyphCoverage) -> String {
    if coverage.missing_glyphs > 0 {
        "source_text_extraction_unchanged_visual_fallback_missing_glyphs".to_string()
    } else {
        "source_text_extraction_unchanged_render_substitution_only".to_string()
    }
}

fn font_substitution_editing_impact(
    metric_posture: &FontSubstitutionMetricPosture,
    coverage: &FontSubstitutionGlyphCoverage,
) -> String {
    if coverage.missing_glyphs > 0 {
        return "editing_requires_review_missing_fallback_glyphs".to_string();
    }
    match metric_posture {
        FontSubstitutionMetricPosture::MetricCompatible => {
            "editing_low_risk_metric_compatible_replacement".to_string()
        }
        FontSubstitutionMetricPosture::CoverageOnly => {
            "editing_reflow_metrics_may_differ".to_string()
        }
        FontSubstitutionMetricPosture::Unknown => {
            "editing_requires_review_metric_compatibility_unknown".to_string()
        }
    }
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

        log.record(FontSubstitutionEvent::bundled_fallback(
            "Helvetica",
            "Helvetica",
            "LiberationSans-Regular",
            FontSubstitutionReason::MissingFont,
            "missing_font_dictionary",
            "unknown",
            1,
            FontSubstitutionMetricPosture::MetricCompatible,
            "test-policy",
        ));

        assert!(!log.is_empty());
        assert_eq!(log.events().len(), 1);
        assert_eq!(log.total_count(), 1);
        assert_eq!(log.overflow_count(), 0);
    }

    #[test]
    fn log_bounds_events_at_capacity() {
        let mut log = FontSubstitutionLog::new();
        for i in 0..MAX_FONT_SUBSTITUTION_EVENTS + 10 {
            log.record(FontSubstitutionEvent::bundled_fallback(
                format!("Font{i}"),
                format!("Font{i}"),
                "LiberationSans-Regular",
                FontSubstitutionReason::BundledFallback,
                "embedded_font_unusable_or_empty",
                "unknown",
                1,
                FontSubstitutionMetricPosture::Unknown,
                "test-policy",
            ));
        }
        assert_eq!(log.events().len(), MAX_FONT_SUBSTITUTION_EVENTS);
        assert_eq!(log.overflow_count(), 10);
        assert_eq!(log.total_count(), MAX_FONT_SUBSTITUTION_EVENTS + 10);
    }

    #[test]
    fn log_absorb_merges_child_events() {
        let mut parent = FontSubstitutionLog::new();
        parent.record(FontSubstitutionEvent::bundled_fallback(
            "TimesNewRoman",
            "TimesNewRoman",
            "LiberationSerif-Regular",
            FontSubstitutionReason::MissingFont,
            "missing_font_dictionary",
            "unknown",
            1,
            FontSubstitutionMetricPosture::MetricCompatible,
            "test-policy",
        ));

        let mut child = FontSubstitutionLog::new();
        child.record(FontSubstitutionEvent::bundled_fallback(
            "CourierNew",
            "CourierNew",
            "LiberationMono-Regular",
            FontSubstitutionReason::BundledFallback,
            "embedded_font_unusable_or_empty",
            "unknown",
            2,
            FontSubstitutionMetricPosture::MetricCompatible,
            "test-policy",
        ));

        parent.absorb(child);
        assert_eq!(parent.events().len(), 2);
        assert_eq!(parent.events()[0].requested_font, "TimesNewRoman");
        assert_eq!(parent.events()[1].requested_font, "CourierNew");
    }

    #[test]
    fn log_single_flight_keeps_first_page_and_coverage() {
        let mut log = FontSubstitutionLog::new();
        let index = log
            .record_single_flight_indexed(FontSubstitutionEvent::bundled_fallback(
                "Helvetica",
                "Helvetica",
                "LiberationSans-Regular",
                FontSubstitutionReason::MissingFont,
                "missing_font_dictionary",
                "implicit_standard_encoding",
                1,
                FontSubstitutionMetricPosture::MetricCompatible,
                "test-policy",
            ))
            .expect("first event should be retained");
        log.event_mut(index)
            .expect("first event should be mutable")
            .set_glyph_coverage(FontSubstitutionGlyphCoverage {
                observed_glyphs: 2,
                covered_glyphs: 2,
                missing_glyphs: 0,
                missing_glyph_samples: Vec::new(),
            });

        let mut duplicate = FontSubstitutionEvent::bundled_fallback(
            "Helvetica",
            "Helvetica",
            "LiberationSans-Regular",
            FontSubstitutionReason::MissingFont,
            "missing_font_dictionary",
            "implicit_standard_encoding",
            9,
            FontSubstitutionMetricPosture::MetricCompatible,
            "test-policy",
        );
        duplicate.set_glyph_coverage(FontSubstitutionGlyphCoverage {
            observed_glyphs: 3,
            covered_glyphs: 2,
            missing_glyphs: 1,
            missing_glyph_samples: vec!["U+2603 code=9731".to_string()],
        });

        assert_eq!(log.record_single_flight_indexed(duplicate), None);
        assert_eq!(log.events().len(), 1);
        assert_eq!(log.total_count(), 1);
        assert_eq!(log.events()[0].page, 1);
        assert_eq!(
            log.events()[0].glyph_coverage,
            FontSubstitutionGlyphCoverage {
                observed_glyphs: 2,
                covered_glyphs: 2,
                missing_glyphs: 0,
                missing_glyph_samples: Vec::new(),
            }
        );
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
        log.record(FontSubstitutionEvent::bundled_fallback(
            "Test",
            "Test",
            "LiberationSans-Regular",
            FontSubstitutionReason::MissingFont,
            "missing_font_dictionary",
            "unknown",
            1,
            FontSubstitutionMetricPosture::Unknown,
            "test-policy",
        ));
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.events().len(), 0);
        assert_eq!(log.overflow_count(), 0);
    }

    #[test]
    fn risk_flags_report_missing_glyph_and_metric_risks() {
        let coverage = FontSubstitutionGlyphCoverage {
            observed_glyphs: 2,
            covered_glyphs: 1,
            missing_glyphs: 1,
            missing_glyph_samples: vec!["U+1F600 code=0".to_string()],
        };
        let flags = font_substitution_risk_flags(
            &FontSubstitutionReason::BundledFallback,
            &FontSubstitutionMetricPosture::CoverageOnly,
            &coverage,
        );
        assert!(flags.contains(&"source_font_program_unusable_or_unembedded".to_string()));
        assert!(flags.contains(&"fallback_metrics_may_differ".to_string()));
        assert!(flags.contains(&"fallback_missing_observed_glyphs".to_string()));
    }

    #[test]
    fn event_policy_fields_track_coverage_and_risk() {
        let mut event = FontSubstitutionEvent::bundled_fallback(
            "F1",
            "Helvetica",
            "LiberationSans-Regular",
            FontSubstitutionReason::BundledFallback,
            "embedded_font_unusable_or_empty",
            "WinAnsiEncoding",
            3,
            FontSubstitutionMetricPosture::MetricCompatible,
            "render_contract:abc",
        );
        event.set_glyph_coverage(FontSubstitutionGlyphCoverage {
            observed_glyphs: 2,
            covered_glyphs: 1,
            missing_glyphs: 1,
            missing_glyph_samples: vec!["U+0041 code=65".to_string()],
        });

        assert_eq!(event.requested_pdf_font, "Helvetica");
        assert_eq!(event.selected_replacement, "LiberationSans-Regular");
        assert_eq!(event.resolution_source, "bundled_fallback");
        assert_eq!(event.selection_reason, "default_policy");
        assert_eq!(event.encoding, "WinAnsiEncoding");
        assert_eq!(event.missing_glyphs, 1);
        assert_eq!(
            event.required_glyph_coverage,
            FontSubstitutionGlyphCoverage {
                observed_glyphs: 2,
                covered_glyphs: 1,
                missing_glyphs: 1,
                missing_glyph_samples: vec!["U+0041 code=65".to_string()],
            }
        );
        assert_eq!(event.visual_risk_category, "high_missing_fallback_glyphs");
        assert_eq!(
            event.extraction_impact,
            "source_text_extraction_unchanged_visual_fallback_missing_glyphs"
        );
        assert_eq!(
            event.editing_impact,
            "editing_requires_review_missing_fallback_glyphs"
        );
        assert_eq!(event.font_policy_identity, "render_contract:abc");
        assert!(event
            .risk_flags
            .contains(&"fallback_missing_observed_glyphs".to_string()));
    }
}
