//! Back-of-document indexes for fresh authoring. Index occurrences bind to
//! already-declared anchors, are ordered deterministically, deduplicated by
//! physical page and emitted through the deferred clickable-field engine.
use super::*;
use std::cmp::Ordering;

#[cfg(test)]
#[path = "authoring_index_tests.rs"]
mod tests;

const MAX_INDEX_ENTRIES: usize = 100_000;
const MAX_INDEX_DEPTH: usize = 128;
const MAX_INDEX_ANCHORS_PER_ENTRY: usize = 2047;
const MAX_INDEX_LINKS: usize = 100_000;
const MAX_INDEX_PARTS_PER_ENTRY: usize = 4096;
const MAX_INDEX_TEXT_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DocumentIndexSort {
    /// Preserve caller order at every hierarchy level.
    Authored,
    /// Compare Unicode scalar lowercase expansions, then the original UTF-8.
    /// This is deterministic but intentionally not presented as locale collation.
    #[default]
    UnicodeScalar,
    /// Require and compare caller-supplied collation keys. Applications can
    /// provide ICU/CLDR sort keys without coupling the PDF engine to one locale.
    ExplicitKeys,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfIndexEntry {
    pub term: String,
    pub sort_key: Option<String>,
    pub anchors: Vec<String>,
    pub see: Option<String>,
    pub see_also: Vec<String>,
    pub children: Vec<PdfIndexEntry>,
}

impl PdfIndexEntry {
    pub fn new(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            sort_key: None,
            anchors: Vec::new(),
            see: None,
            see_also: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn sort_key(mut self, key: impl Into<String>) -> Self {
        self.sort_key = Some(key.into());
        self
    }

    pub fn anchor(mut self, anchor: impl Into<String>) -> Self {
        self.anchors.push(anchor.into());
        self
    }

    pub fn anchors<I, S>(mut self, anchors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.anchors.extend(anchors.into_iter().map(Into::into));
        self
    }

    pub fn see(mut self, term: impl Into<String>) -> Self {
        self.see = Some(term.into());
        self
    }

    pub fn see_also(mut self, term: impl Into<String>) -> Self {
        self.see_also.push(term.into());
        self
    }

    pub fn children(mut self, children: Vec<PdfIndexEntry>) -> Self {
        self.children = children;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentIndexStyle {
    pub text_style: TextStyle,
    pub paragraph: ParagraphStyle,
    pub indent_per_level: f64,
    pub row_gap: f64,
    pub max_page_characters: usize,
    pub section_page_numbers: bool,
    pub sort: DocumentIndexSort,
    pub compress_page_ranges: bool,
    pub minimum_range_pages: usize,
    pub page_separator: String,
    pub range_separator: String,
    pub reference_separator: String,
    pub see_label: String,
    pub see_also_label: String,
}

impl Default for DocumentIndexStyle {
    fn default() -> Self {
        Self {
            text_style: TextStyle::unicode(10.0),
            paragraph: ParagraphStyle::new(),
            indent_per_level: 14.0,
            row_gap: 1.5,
            max_page_characters: 8,
            section_page_numbers: false,
            sort: DocumentIndexSort::UnicodeScalar,
            compress_page_ranges: true,
            minimum_range_pages: 3,
            page_separator: ", ".into(),
            range_separator: "\u{2013}".into(),
            reference_separator: "; ".into(),
            see_label: "see ".into(),
            see_also_label: "see also ".into(),
        }
    }
}

impl DocumentIndexStyle {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate(&self) -> Result<()> {
        if !self.text_style.size.is_finite()
            || self.text_style.size <= 0.0
            || !self.paragraph.line_height.is_finite()
            || self.paragraph.line_height <= 0.0
            || !self.indent_per_level.is_finite()
            || self.indent_per_level < 0.0
            || !self.row_gap.is_finite()
            || self.row_gap < 0.0
            || self.max_page_characters == 0
            || self.max_page_characters > 64
            || self.minimum_range_pages < 2
            || self.minimum_range_pages > MAX_INDEX_ANCHORS_PER_ENTRY
        {
            return Err(WellfriendError::invalid_input(
                "invalid document-index style geometry or page capacity",
            ));
        }
        for text in [
            &self.page_separator,
            &self.range_separator,
            &self.reference_separator,
            &self.see_label,
            &self.see_also_label,
        ] {
            validate_text(text, false)?;
        }
        if self.page_separator.is_empty()
            || self.range_separator.is_empty()
            || self.reference_separator.is_empty()
            || self.see_label.is_empty()
            || self.see_also_label.is_empty()
        {
            return Err(WellfriendError::invalid_input(
                "document-index separators and labels must be nonempty",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentIndexRow {
    pub term: String,
    pub level: usize,
    pub anchors: Vec<String>,
    /// One-based physical source pages after same-page occurrence deduplication.
    pub source_pages: Vec<usize>,
    /// One-based physical pages occupied by the painted index row.
    pub output_pages: Vec<usize>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DocumentIndexReport {
    pub rows: Vec<DocumentIndexRow>,
}

#[derive(Clone)]
struct PreparedEntry {
    entry: PdfIndexEntry,
    level: usize,
    anchors: Vec<(String, usize, usize)>,
}

fn validate_text(text: &str, require_nonempty: bool) -> Result<()> {
    if (require_nonempty && text.is_empty())
        || text.len() > MAX_INDEX_TEXT_BYTES
        || text
            .chars()
            .any(|ch| ch == '\0' || crate::fonts::hard_break::is_hard_break(ch))
    {
        return Err(WellfriendError::invalid_input(
            "document-index text must be bounded and single-line",
        ));
    }
    Ok(())
}

fn scalar_key(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

fn prepare_level(
    builder: &PdfBuilder,
    entries: &[PdfIndexEntry],
    level: usize,
    style: &DocumentIndexStyle,
    output: &mut Vec<PreparedEntry>,
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if level >= MAX_INDEX_DEPTH {
        return Err(WellfriendError::ResourceLimit(
            "document-index nesting depth".into(),
        ));
    }
    let mut ordered = entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let key = match style.sort {
                DocumentIndexSort::Authored => Vec::new(),
                DocumentIndexSort::UnicodeScalar => {
                    scalar_key(entry.sort_key.as_deref().unwrap_or(&entry.term)).into_bytes()
                }
                DocumentIndexSort::ExplicitKeys => {
                    entry.sort_key.as_deref().unwrap_or("").as_bytes().to_vec()
                }
            };
            (position, entry, key)
        })
        .collect::<Vec<_>>();
    if style.sort != DocumentIndexSort::Authored {
        ordered.sort_by(|left, right| {
            left.2
                .cmp(&right.2)
                .then_with(|| left.1.term.as_bytes().cmp(right.1.term.as_bytes()))
                .then_with(|| left.0.cmp(&right.0))
        });
    }
    let mut sibling_terms = std::collections::BTreeSet::new();
    for (_, entry, _) in ordered {
        crate::cancel::check_current_cancel("document-index preparation")?;
        if output.len() >= MAX_INDEX_ENTRIES {
            return Err(WellfriendError::ResourceLimit(
                "document-index entry count".into(),
            ));
        }
        validate_text(&entry.term, true)?;
        if !sibling_terms.insert(entry.term.as_bytes()) {
            return Err(WellfriendError::invalid_input(
                "duplicate document-index term at one hierarchy level",
            ));
        }
        if style.sort == DocumentIndexSort::ExplicitKeys && entry.sort_key.is_none() {
            return Err(WellfriendError::invalid_input(
                "explicit document-index sorting requires every entry to have a sort key",
            ));
        }
        if let Some(key) = &entry.sort_key {
            validate_text(key, true)?;
        }
        if entry.anchors.len() > MAX_INDEX_ANCHORS_PER_ENTRY {
            return Err(WellfriendError::ResourceLimit(
                "document-index anchors per entry".into(),
            ));
        }
        let part_count = 1usize
            .saturating_add(entry.anchors.len().saturating_mul(2))
            .saturating_add(usize::from(entry.see.is_some()))
            .saturating_add(entry.see_also.len());
        if part_count > MAX_INDEX_PARTS_PER_ENTRY {
            return Err(WellfriendError::ResourceLimit(
                "document-index row part count".into(),
            ));
        }
        if entry.see.is_some() && (!entry.anchors.is_empty() || !entry.see_also.is_empty()) {
            return Err(WellfriendError::invalid_input(
                "a document-index see reference cannot also own pages or see-also references",
            ));
        }
        if entry.anchors.is_empty()
            && entry.see.is_none()
            && entry.see_also.is_empty()
            && entry.children.is_empty()
        {
            return Err(WellfriendError::invalid_input(
                "document-index entry has no occurrence, cross-reference or child",
            ));
        }
        if let Some(see) = &entry.see {
            validate_text(see, true)?;
        }
        let mut seen_also = std::collections::BTreeSet::new();
        for see_also in &entry.see_also {
            validate_text(see_also, true)?;
            if !seen_also.insert(see_also.as_bytes()) {
                return Err(WellfriendError::invalid_input(
                    "duplicate document-index see-also reference",
                ));
            }
        }
        let mut resolved = Vec::with_capacity(entry.anchors.len());
        let mut names = std::collections::BTreeSet::new();
        for anchor in &entry.anchors {
            if !names.insert(anchor.as_bytes()) {
                return Err(WellfriendError::invalid_input(
                    "duplicate document-index anchor on one entry",
                ));
            }
            let (page, section, y) = fields::anchor_position(builder, anchor)?;
            resolved.push((anchor.clone(), page, section, y));
        }
        resolved.sort_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| right.3.partial_cmp(&left.3).unwrap_or(Ordering::Equal))
                .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
        });
        let mut deduplicated = Vec::with_capacity(resolved.len());
        for occurrence in resolved {
            if deduplicated
                .last()
                .is_none_or(|(_, page, _, _): &(String, usize, usize, f64)| *page != occurrence.1)
            {
                deduplicated.push(occurrence);
            }
        }
        output.push(PreparedEntry {
            entry: entry.clone(),
            level,
            anchors: deduplicated
                .into_iter()
                .map(|(anchor, page, section, _)| (anchor, page, section))
                .collect(),
        });
        prepare_level(builder, &entry.children, level + 1, style, output)?;
    }
    Ok(())
}

fn anchor_field(anchor: &str, style: &DocumentIndexStyle) -> BodyFieldPart {
    let field = if style.section_page_numbers {
        BodyField::AnchorSectionPage(anchor.to_owned())
    } else {
        BodyField::AnchorDocumentPage(anchor.to_owned())
    };
    BodyFieldPart::field(
        field,
        BodyFieldFormat::new(style.max_page_characters)
            .link_to_anchor(true)
            .value_align(TextAlign::Left),
    )
}

fn parts(entry: &PreparedEntry, style: &DocumentIndexStyle) -> Vec<BodyFieldPart> {
    let mut parts = vec![BodyFieldPart::text(entry.entry.term.clone())];
    let mut first_page = true;
    let mut start = 0usize;
    while start < entry.anchors.len() {
        let mut end = start;
        while end + 1 < entry.anchors.len()
            && entry.anchors[end].1.checked_add(1) == Some(entry.anchors[end + 1].1)
            && (!style.section_page_numbers || entry.anchors[end + 1].2 == entry.anchors[end].2)
        {
            end += 1;
        }
        let compress = style.compress_page_ranges && end - start + 1 >= style.minimum_range_pages;
        if compress {
            parts.push(BodyFieldPart::text(if first_page {
                style.reference_separator.clone()
            } else {
                style.page_separator.clone()
            }));
            parts.push(anchor_field(&entry.anchors[start].0, style));
            parts.push(BodyFieldPart::text(style.range_separator.clone()));
            parts.push(anchor_field(&entry.anchors[end].0, style));
            first_page = false;
        } else {
            for index in start..=end {
                parts.push(BodyFieldPart::text(if first_page {
                    style.reference_separator.clone()
                } else {
                    style.page_separator.clone()
                }));
                parts.push(anchor_field(&entry.anchors[index].0, style));
                first_page = false;
            }
        }
        start = end + 1;
    }
    if let Some(see) = &entry.entry.see {
        parts.push(BodyFieldPart::text(format!(
            "{}{}{}",
            style.reference_separator, style.see_label, see
        )));
    }
    for (index, see_also) in entry.entry.see_also.iter().enumerate() {
        parts.push(BodyFieldPart::text(format!(
            "{}{}{}",
            if index == 0 {
                &style.reference_separator
            } else {
                &style.page_separator
            },
            style.see_also_label,
            see_also
        )));
    }
    parts
}

pub(super) fn append(
    flow: &mut FlowDocument,
    entries: &[PdfIndexEntry],
    style: &DocumentIndexStyle,
) -> Result<DocumentIndexReport> {
    style.validate()?;
    if entries.is_empty() {
        return Err(WellfriendError::invalid_input(
            "document index requires a nonempty entry list",
        ));
    }
    let usable_height = flow.page_size.height - flow.margins.top - flow.margins.bottom;
    if !usable_height.is_finite() || usable_height <= 0.0 || style.row_gap >= usable_height {
        return Err(WellfriendError::invalid_input(
            "document-index row gap must be smaller than the usable page height",
        ));
    }
    let mut prepared = Vec::new();
    prepare_level(&flow.builder, entries, 0, style, &mut prepared)?;
    let link_count = prepared.iter().try_fold(0usize, |count, entry| {
        count.checked_add(entry.anchors.len()).ok_or_else(|| {
            WellfriendError::ResourceLimit("document-index link count overflow".into())
        })
    })?;
    if link_count > MAX_INDEX_LINKS {
        return Err(WellfriendError::ResourceLimit(
            "document-index link count".into(),
        ));
    }
    let mut report = DocumentIndexReport::default();
    flow.append_transaction(|flow| {
        let index_structure = structure::register(
            &mut flow.builder,
            structure::Role::Index,
            None,
            Some("Index".into()),
        )?;
        for (entry_index, entry) in prepared.iter().enumerate() {
            crate::cancel::check_current_cancel("document-index layout")?;
            if entry_index > 0
                && style.row_gap > 0.0
                && flow.cursor_y - style.row_gap >= flow.current_bottom()
            {
                flow.cursor_y -= style.row_gap;
            }
            let indent = style.indent_per_level * entry.level as f64;
            let width = flow.content_width()? - indent;
            if !indent.is_finite() || !width.is_finite() || width <= 0.0 {
                return Err(WellfriendError::UnsupportedFeature(
                    "document-index level leaves no text region".into(),
                ));
            }
            let left = flow.margins.left + indent;
            let row_structure = structure::register(
                &mut flow.builder,
                structure::Role::Paragraph,
                Some(index_structure),
                Some(entry.entry.term.clone()),
            )?;
            let row = fields::append_paragraph_in_region_with_structure(
                flow,
                &parts(entry, style),
                &style.text_style,
                &style.paragraph,
                left,
                width,
                Some(row_structure),
            )?;
            let mut output_pages = row.line_pages;
            output_pages.dedup();
            report.rows.push(DocumentIndexRow {
                term: entry.entry.term.clone(),
                level: entry.level,
                anchors: entry
                    .anchors
                    .iter()
                    .map(|(anchor, _, _)| anchor.clone())
                    .collect(),
                source_pages: entry.anchors.iter().map(|(_, page, _)| page + 1).collect(),
                output_pages,
            });
        }
        Ok(())
    })?;
    Ok(report)
}
