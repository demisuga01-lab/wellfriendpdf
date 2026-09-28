//! Prepared authored rows retain logical line plans across continuation pages.
//! No raster clipping, string slicing at arbitrary bytes or reshaping a cell's
//! continuation as a new paragraph. The flow caller owns append rollback.
use super::*;
use std::ops::Range;

#[cfg(test)]
#[path = "authoring_tables_tests.rs"]
mod tests;

const EPS: f64 = 1e-7;
const MAX_LINES: usize = 1_000_000;
const MAX_CELLS: usize = 1_000_000;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FRAGMENTS: usize = 100_000;
const CAPTION_GAP: f64 = 4.0;
const TYPED_TABLE_CONTINUATION_MARKER: &str = "WFTableContinuation";
const TYPED_TABLE_RELOCATION_MARKER: &str = "WFTableRelocation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableRowSplitPolicy {
    /// Move a row intact; reject if row plus repeated header cannot fit a page.
    KeepTogether,
    /// Only rows too tall for a fresh page split. Every nonfinal cell fragment
    /// must contain this many lines; reserve the requested final-line minimum.
    /// Already-complete cells paint an empty continuation background.
    Lines {
        min_fragment_lines: usize,
        min_final_lines: usize,
    },
}

impl Default for TableRowSplitPolicy {
    fn default() -> Self {
        Self::Lines {
            min_fragment_lines: 1,
            min_final_lines: 1,
        }
    }
}

impl TableRowSplitPolicy {
    pub(super) fn validate(self) -> Result<()> {
        if let Self::Lines {
            min_fragment_lines,
            min_final_lines,
        } = self
        {
            if min_fragment_lines == 0
                || min_final_lines == 0
                || min_fragment_lines > 100_000
                || min_final_lines > 100_000
            {
                return Err(fail("invalid authored row line minima"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableFragmentInfo {
    /// One-based PDF page number. Row indexes below are zero-based.
    pub page: usize,
    /// None denotes the table's header row.
    pub row: Option<usize>,
    pub repeated_header: bool,
    pub top: f64,
    pub height: f64,
    /// Original cell UTF-8 ranges, including logical hard separators. Completed
    /// cells report an empty range at their original end, not repeated text.
    /// This compatibility projection follows logical placed-cell order.
    pub cell_utf8_ranges: Vec<[usize; 2]>,
    pub cells: Vec<TableCellFragmentInfo>,
    pub continues: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCellFragmentInfo {
    /// Index in the caller's source row. `None` denotes an implicit trailing
    /// empty cell used to complete the fixed grid.
    pub source_cell: Option<usize>,
    pub column_start: usize,
    pub column_span: usize,
    pub row_start: usize,
    pub row_span: usize,
    pub utf8_range: [usize; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowPageBreakInfo {
    /// Zero-based source row index.
    pub row: usize,
    pub policy: FlowPageBreak,
    /// One-based physical page occupied immediately before the transition.
    pub from_page: usize,
    /// One-based physical page on which the row starts.
    pub to_page: usize,
    /// Includes parity blanks and the destination page.
    pub added_pages: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCaptionInfo {
    pub page: usize,
    pub top: f64,
    pub height: f64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TableFlowReport {
    pub caption: Option<TableCaptionInfo>,
    pub fragments: Vec<TableFragmentInfo>,
    pub page_breaks: Vec<TableRowPageBreakInfo>,
    pub added_pages: usize,
    pub evaluated_values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AuthoredTypedCellAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AuthoredTypedCellRole {
    #[default]
    Data,
    Header,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredTypedHeaderScope {
    Row,
    Column,
    Both,
}

impl From<TableHeaderScope> for AuthoredTypedHeaderScope {
    fn from(value: TableHeaderScope) -> Self {
        match value {
            TableHeaderScope::Row => Self::Row,
            TableHeaderScope::Column => Self::Column,
            TableHeaderScope::Both => Self::Both,
        }
    }
}

impl From<AuthoredTypedHeaderScope> for TableHeaderScope {
    fn from(value: AuthoredTypedHeaderScope) -> Self {
        match value {
            AuthoredTypedHeaderScope::Row => Self::Row,
            AuthoredTypedHeaderScope::Column => Self::Column,
            AuthoredTypedHeaderScope::Both => Self::Both,
        }
    }
}

impl From<TextAlign> for AuthoredTypedCellAlignment {
    fn from(value: TextAlign) -> Self {
        match value {
            TextAlign::Left => Self::Left,
            TextAlign::Center => Self::Center,
            TextAlign::Right => Self::Right,
        }
    }
}

impl From<AuthoredTypedCellAlignment> for crate::GeneratedTextAlignment {
    fn from(value: AuthoredTypedCellAlignment) -> Self {
        match value {
            AuthoredTypedCellAlignment::Left => Self::Left,
            AuthoredTypedCellAlignment::Center => Self::Center,
            AuthoredTypedCellAlignment::Right => Self::Right,
        }
    }
}

impl From<AuthoredTypedCellAlignment> for TextAlign {
    fn from(value: AuthoredTypedCellAlignment) -> Self {
        match value {
            AuthoredTypedCellAlignment::Left => Self::Left,
            AuthoredTypedCellAlignment::Center => Self::Center,
            AuthoredTypedCellAlignment::Right => Self::Right,
        }
    }
}

/// Layout facts that cannot be reconstructed uniquely from painted glyphs.
/// New authored tables retain these values in their revision-bound registry;
/// older v1 registries deserialize without them and remain readable, but a
/// cross-fragment mutation fails closed instead of guessing line geometry.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedCellLayout {
    pub font_size: f64,
    pub line_spacing: f64,
    #[serde(default)]
    pub alignment: AuthoredTypedCellAlignment,
    /// Exact paint facts needed when a later edit allocates another owned
    /// fragment. Older registries remain readable, but cannot authorize page
    /// growth because PDF appearance is not recoverable uniquely from text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<AuthoredTypedCellPaint>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "space", content = "components", rename_all = "snake_case")]
pub enum AuthoredTypedColor {
    DeviceGray([f64; 1]),
    DeviceRgb([f64; 3]),
    DeviceCmyk([f64; 4]),
}

impl AuthoredTypedColor {
    fn from_color(color: &Color) -> Result<Self> {
        let invalid = || fail("authored typed-table paint uses an invalid device colour");
        if color
            .components
            .iter()
            .any(|component| !component.is_finite())
        {
            return Err(invalid());
        }
        match (&color.space, color.components.as_slice()) {
            (ColorSpace::DeviceGray, [gray]) => Ok(Self::DeviceGray([*gray])),
            (ColorSpace::DeviceRGB, [red, green, blue]) => {
                Ok(Self::DeviceRgb([*red, *green, *blue]))
            }
            (ColorSpace::DeviceCMYK, [cyan, magenta, yellow, black]) => {
                Ok(Self::DeviceCmyk([*cyan, *magenta, *yellow, *black]))
            }
            _ => Err(WellfriendError::UnsupportedFeature(
                "authored typed-table continuation paint requires a device colour".into(),
            )),
        }
    }

    fn to_color(&self) -> Color {
        match self {
            Self::DeviceGray([gray]) => Color::device_gray(*gray),
            Self::DeviceRgb([red, green, blue]) => Color::device_rgb(*red, *green, *blue),
            Self::DeviceCmyk([cyan, magenta, yellow, black]) => {
                Color::device_cmyk(*cyan, *magenta, *yellow, *black)
            }
        }
    }

    fn validate(&self) -> bool {
        match self {
            Self::DeviceGray(values) => values.iter().all(|value| value.is_finite()),
            Self::DeviceRgb(values) => values.iter().all(|value| value.is_finite()),
            Self::DeviceCmyk(values) => values.iter().all(|value| value.is_finite()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedCellPaint {
    pub background: AuthoredTypedColor,
    pub border: AuthoredTypedColor,
    pub text: AuthoredTypedColor,
    pub padding: f64,
    pub line_width: f64,
}

/// A font identity that can be resolved from the saved PDF without depending
/// on an installed system font. Embedded faces use the exact authored PDF base
/// name; subset prefixes are ignored only after all matching programs are
/// proven byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoredRetainedFont {
    Standard { base_name: String },
    Embedded { base_name: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedHeaderCellModel {
    pub text: String,
    pub column: usize,
    pub column_span: usize,
    pub layout: AuthoredTypedCellLayout,
    pub font: AuthoredRetainedFont,
}

/// Exact source facts needed to reproduce the visual repeatable header on a
/// later page. Repeated headers remain artifacts; the original TH ownership is
/// not duplicated into the logical structure tree.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedHeaderModel {
    pub height: f64,
    pub cells: Vec<AuthoredTypedHeaderCellModel>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedTablePagination {
    pub page_size: [f64; 2],
    /// left, right, top, bottom in PDF points.
    pub margins: [f64; 4],
    pub column_widths: Vec<f64>,
    pub row_split_policy: TableRowSplitPolicy,
    pub has_repeatable_header: bool,
    /// Present only when the repeatable header can be reconstructed from exact
    /// retained layout/paint and a saved-PDF-resolvable font. Older registries
    /// and fallback-stack headers remain readable but fail closed on growth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeatable_header: Option<AuthoredTypedHeaderModel>,
    pub body_rows: usize,
    /// Exact explicit break policy for every retained body row. `Some(None)`
    /// distinguishes a known absence from an older registry that never
    /// retained parity-sensitive row-break provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_page_breaks: Option<Vec<Option<FlowPageBreak>>>,
    /// True only when a blank continuation page needs no first/odd/even
    /// master, mirrored-margin or reserved-footnote materialization.
    #[serde(default)]
    pub direct_growth_safe: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedCellModel {
    pub id: String,
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub value: crate::typed_tables::TableValue,
    pub evaluated: String,
    #[serde(default)]
    pub structure_role: AuthoredTypedCellRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_scope: Option<AuthoredTypedHeaderScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<AuthoredTypedCellLayout>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedTableModel {
    pub schema_version: String,
    pub id: String,
    pub cells: Vec<AuthoredTypedCellModel>,
    /// Source-authoritative continuation geometry. It is optional only for
    /// registries emitted before page-growth support existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<AuthoredTypedTablePagination>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthoredTypedCellSourceFragment {
    pub page: usize,
    /// Page-logical Unicode-scalar range consumed by the canonical source
    /// editor. Empty cells retain a zero-width provenance-bearing operand.
    pub logical_range: [usize; 2],
    pub logical_text: String,
    pub source_span_ids: Vec<String>,
    pub region: Option<[f64; 4]>,
    /// 0 = horizontal, 1 = vertical, -1 = mixed/unsupported for one-shot
    /// post-reopen mutation.
    pub writing_mode: i32,
    /// Exact source text state observed inside this owned fragment. These are
    /// reported and cross-checked against the retained authoring contract;
    /// they are never inferred from glyph bounding boxes.
    pub font_resource: String,
    pub font_size: f64,
    pub character_spacing: f64,
    pub word_spacing: f64,
    pub horizontal_scaling: f64,
    pub uniform_text_state: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthoredTypedCellSourceBinding {
    pub table_id: String,
    pub cell_id: String,
    pub evaluated: String,
    pub fragments: Vec<AuthoredTypedCellSourceFragment>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthoredTypedTableSourceReport {
    pub schema_version: String,
    pub input_sha256: String,
    pub cells: Vec<AuthoredTypedCellSourceBinding>,
    pub exact_source_ownership: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthoredTypedGridPaintFragment {
    pub page: usize,
    pub stream_object: [u32; 2],
    /// Exact decoded-stream range including the private Artifact BDC/EMC
    /// wrapper. It is revision-bound and must be rebound after every rewrite.
    pub decoded_range: [usize; 2],
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub rect: [f64; 4],
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AuthoredTypedTableGridPaintReport {
    pub schema_version: String,
    pub input_sha256: String,
    pub table_id: String,
    pub fragments: Vec<AuthoredTypedGridPaintFragment>,
    /// True only when the distinct retained typed-cell topology and distinct
    /// grid-paint topology are identical. This exact source contract can
    /// authorize bounded row relocation; partial typed ownership cannot.
    pub complete_typed_grid_ownership: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthoredTypedTableMutationRequest {
    pub input_sha256: String,
    pub table_id: String,
    pub updates: BTreeMap<String, crate::typed_tables::TableValue>,
    #[serde(default)]
    pub signature_policy_override: bool,
    /// Remove only empty pages carrying this table transaction's exact
    /// continuation provenance. Original/unmarked or dependency-bearing pages
    /// are retained and reported.
    #[serde(default)]
    pub prune_empty_continuations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuthoredTypedTableRetainedContinuation {
    /// One-based page number in the pre-pruning edited revision.
    pub page: usize,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthoredTypedTableMutationReport {
    pub schema_version: String,
    pub table_id: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub previous_values: BTreeMap<String, String>,
    pub values: BTreeMap<String, String>,
    pub changed_cells: Vec<String>,
    pub changed_pages: Vec<usize>,
    /// One-based page numbers in the edited revision immediately before page
    /// removal. Final `changed_pages` values use the post-pruning numbering.
    pub removed_pages: Vec<usize>,
    /// Owned continuation pages that could not be pruned and relocation
    /// destination pages that could not be compacted, each with an exact
    /// machine-readable reason.
    pub retained_continuation_pages: Vec<AuthoredTypedTableRetainedContinuation>,
    pub source_report: AuthoredTypedTableSourceReport,
    pub signature_policy: crate::secure_mutation::EditPolicyReport,
    pub original_prefix_preserved: bool,
    pub output_reopened: bool,
    pub exact_limits: Vec<String>,
}

impl AuthoredTypedTableModel {
    fn validate(&self) -> Result<BTreeMap<String, String>> {
        if self.schema_version != "wellfriend.authored_typed_table.v1"
            || self.id.is_empty()
            || self.id.len() > 16 * 1024
            || self.cells.is_empty()
            || self.cells.len() > 4096
        {
            return Err(fail("invalid authored typed-table registry model"));
        }
        let values = self
            .cells
            .iter()
            .map(|cell| (cell.id.as_str(), cell.row, cell.column, &cell.value))
            .collect::<Vec<_>>();
        if self.cells.iter().any(|cell| {
            cell.row_span == 0
                || cell.column_span == 0
                || cell.evaluated.len() > 64_000
                || cell.id.is_empty()
                || cell.id.len() > 16 * 1024
                || cell.id.chars().any(|ch| ch == '\0')
                || match cell.structure_role {
                    AuthoredTypedCellRole::Data => cell.header_scope.is_some(),
                    AuthoredTypedCellRole::Header => cell.header_scope.is_none(),
                }
                || cell.layout.as_ref().is_some_and(|layout| {
                    !layout.font_size.is_finite()
                        || layout.font_size <= 0.0
                        || !layout.line_spacing.is_finite()
                        || layout.line_spacing <= 0.0
                        || !(layout.font_size * layout.line_spacing).is_finite()
                        || layout.paint.as_ref().is_some_and(|paint| {
                            !paint.background.validate()
                                || !paint.border.validate()
                                || !paint.text.validate()
                                || !paint.padding.is_finite()
                                || paint.padding < 0.0
                                || !paint.line_width.is_finite()
                                || paint.line_width < 0.0
                        })
                })
        }) {
            return Err(fail("invalid authored typed-table cell metadata"));
        }
        if let Some(pagination) = &self.pagination {
            pagination.row_split_policy.validate()?;
            let [width, height] = pagination.page_size;
            let [left, right, top, bottom] = pagination.margins;
            let content_width = width - left - right;
            let content_height = height - top - bottom;
            let columns = pagination.column_widths.iter().sum::<f64>();
            if !pagination
                .page_size
                .iter()
                .chain(pagination.margins.iter())
                .chain(pagination.column_widths.iter())
                .all(|value| value.is_finite())
                || width <= 0.0
                || height <= 0.0
                || [left, right, top, bottom]
                    .into_iter()
                    .any(|value| value < 0.0)
                || content_width <= 0.0
                || content_height <= 0.0
                || pagination.column_widths.is_empty()
                || pagination.column_widths.len() > 4096
                || pagination.column_widths.iter().any(|width| *width <= 0.0)
                || !columns.is_finite()
                || columns > content_width + EPS
                || pagination.body_rows == 0
                || pagination.body_rows > 1_000_000
            {
                return Err(fail("invalid authored typed-table continuation geometry"));
            }
            if !pagination.has_repeatable_header && pagination.repeatable_header.is_some() {
                return Err(fail(
                    "authored typed-table retains a header model without a repeatable header",
                ));
            }
            if pagination
                .row_page_breaks
                .as_ref()
                .is_some_and(|breaks| breaks.len() != pagination.body_rows)
            {
                return Err(fail(
                    "authored typed-table retained row-break topology is invalid",
                ));
            }
            if self.cells.iter().any(|cell| {
                cell.row >= pagination.body_rows
                    || cell
                        .row
                        .checked_add(cell.row_span)
                        .is_none_or(|end| end > pagination.body_rows)
                    || cell.column >= pagination.column_widths.len()
                    || cell
                        .column
                        .checked_add(cell.column_span)
                        .is_none_or(|end| end > pagination.column_widths.len())
            }) {
                return Err(fail(
                    "authored typed-table cell topology escapes retained pagination",
                ));
            }
            if let Some(header) = &pagination.repeatable_header {
                let mut cells = header.cells.iter().collect::<Vec<_>>();
                cells.sort_by_key(|cell| (cell.column, cell.column_span));
                let mut next_column = 0usize;
                let mut text_bytes = 0usize;
                for cell in cells {
                    text_bytes = text_bytes.checked_add(cell.text.len()).ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "authored repeatable-header text bytes".into(),
                        )
                    })?;
                    let paint = cell.layout.paint.as_ref();
                    let font_name = match &cell.font {
                        AuthoredRetainedFont::Standard { base_name }
                        | AuthoredRetainedFont::Embedded { base_name } => base_name,
                    };
                    if cell.column != next_column
                        || cell.column_span == 0
                        || cell
                            .column
                            .checked_add(cell.column_span)
                            .is_none_or(|end| end > pagination.column_widths.len())
                        || cell.text.len() > 64_000
                        || !cell.layout.font_size.is_finite()
                        || cell.layout.font_size <= 0.0
                        || !cell.layout.line_spacing.is_finite()
                        || cell.layout.line_spacing <= 0.0
                        || paint.is_none_or(|paint| {
                            !paint.background.validate()
                                || !paint.border.validate()
                                || !paint.text.validate()
                                || !paint.padding.is_finite()
                                || paint.padding < 0.0
                                || !paint.line_width.is_finite()
                                || paint.line_width < 0.0
                        })
                        || font_name.is_empty()
                        || font_name.len() > 255
                        || font_name.chars().any(|ch| ch == '\0' || ch == '/')
                        || matches!(
                            &cell.font,
                            AuthoredRetainedFont::Standard { base_name }
                                if standard_font_from_pdf_name(base_name).is_none()
                        )
                    {
                        return Err(fail("invalid authored repeatable-header cell metadata"));
                    }
                    next_column = cell.column + cell.column_span;
                }
                if !header.height.is_finite()
                    || header.height <= 0.0
                    || header.height >= content_height - EPS
                    || header.cells.is_empty()
                    || header.cells.len() > 4096
                    || next_column != pagination.column_widths.len()
                    || text_bytes > 4 * 1024 * 1024
                {
                    return Err(fail("invalid authored repeatable-header geometry"));
                }
            }
        }
        let evaluated = crate::typed_tables::evaluate_values(&self.id, &values)?;
        if self
            .cells
            .iter()
            .any(|cell| evaluated.get(&cell.id) != Some(&cell.evaluated))
        {
            return Err(fail(
                "authored typed-table stored result differs from exact reevaluation",
            ));
        }
        Ok(evaluated)
    }
}

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

fn region_within_page_box(region: [f64; 4], page_box: [f64; 4]) -> bool {
    if !region
        .iter()
        .chain(page_box.iter())
        .all(|value| value.is_finite())
    {
        return false;
    }
    let left = page_box[0].min(page_box[2]);
    let right = page_box[0].max(page_box[2]);
    let bottom = page_box[1].min(page_box[3]);
    let top = page_box[1].max(page_box[3]);
    left < right
        && bottom < top
        && region[0] < region[2]
        && region[1] < region[3]
        && region[0] >= left - EPS
        && region[1] >= bottom - EPS
        && region[2] <= right + EPS
        && region[3] <= top + EPS
}

fn authored_typed_model(
    table: &TableBuilder,
    evaluated: &BTreeMap<String, String>,
    pagination: AuthoredTypedTablePagination,
) -> Result<Option<AuthoredTypedTableModel>> {
    if evaluated.is_empty() {
        return Ok(None);
    }
    let id = table.identity.clone().ok_or_else(|| {
        fail("authored typed-table metadata requires the validated table identity")
    })?;
    let grid = table.body_cell_placements()?;
    let mut cells = Vec::with_capacity(evaluated.len());
    for placements in &grid {
        for placement in placements {
            let Some((_, cell)) = placement.source else {
                continue;
            };
            let (Some(cell_id), Some(value)) = (&cell.typed_id, &cell.typed_value) else {
                continue;
            };
            let style = table.cell_style(Some(cell), false);
            let alignment = cell
                .align
                .unwrap_or(table.columns[placement.columns.start].align);
            let background = cell
                .background
                .clone()
                .or_else(|| table.style.row_fill.clone())
                .unwrap_or_else(|| Color::device_gray(1.0));
            cells.push(AuthoredTypedCellModel {
                id: cell_id.clone(),
                row: placement.row_start,
                column: placement.columns.start,
                row_span: placement.row_span,
                column_span: placement.columns.len(),
                value: value.clone(),
                evaluated: evaluated.get(cell_id).cloned().ok_or_else(|| {
                    fail("authored typed-table result missing during metadata capture")
                })?,
                structure_role: if cell.header_scope.is_some() {
                    AuthoredTypedCellRole::Header
                } else {
                    AuthoredTypedCellRole::Data
                },
                header_scope: cell.header_scope.map(Into::into),
                layout: Some(AuthoredTypedCellLayout {
                    font_size: style.size,
                    line_spacing: table.style.paragraph.line_height,
                    alignment: alignment.into(),
                    paint: Some(AuthoredTypedCellPaint {
                        background: AuthoredTypedColor::from_color(&background)?,
                        border: AuthoredTypedColor::from_color(&table.style.border_color)?,
                        text: AuthoredTypedColor::from_color(&style.fill)?,
                        padding: table.style.padding,
                        line_width: table.style.line_width,
                    }),
                }),
            });
        }
    }
    let model = AuthoredTypedTableModel {
        schema_version: "wellfriend.authored_typed_table.v1".into(),
        id,
        cells,
        pagination: Some(pagination),
    };
    model.validate()?;
    Ok(Some(model))
}

fn retain_authored_typed_model(
    builder: &mut PdfBuilder,
    table: &TableBuilder,
    evaluated: &BTreeMap<String, String>,
    pagination: AuthoredTypedTablePagination,
) -> Result<()> {
    let Some(model) = authored_typed_model(table, evaluated, pagination)? else {
        return Ok(());
    };
    if builder.authored_typed_tables.len() >= 128
        || builder
            .authored_typed_tables
            .iter()
            .any(|existing| existing.id == model.id)
    {
        return Err(fail(
            "authored typed-table registry budget or identity uniqueness violated",
        ));
    }
    builder.authored_typed_tables.push(model);
    Ok(())
}

fn retained_header_font(
    page: &PdfPageBuilder,
    font: FontFace,
) -> Result<Option<AuthoredRetainedFont>> {
    Ok(match font {
        FontFace::Standard(font) => Some(AuthoredRetainedFont::Standard {
            base_name: font.base_font_name().to_string(),
        }),
        FontFace::BuiltinUnicode => Some(AuthoredRetainedFont::Embedded {
            base_name: BUILTIN_UNICODE_RESOURCE_NAME.to_string(),
        }),
        FontFace::Custom(id) => {
            let custom = page
                .custom_fonts
                .get(id.0 as usize)
                .ok_or_else(|| fail("authored repeatable-header custom font is not registered"))?;
            let embedding = crate::fonts::pdf_embedding::EmbeddingInfo::parse(&custom.bytes)?;
            Some(AuthoredRetainedFont::Embedded {
                base_name: embedding
                    .cff
                    .map_or_else(|| custom.base_name.clone(), |cff| cff.postscript_name),
            })
        }
        // A contextual fallback can produce more than one physical run in one
        // cell. The v1 retained header model deliberately does not collapse it
        // to a guessed single face.
        FontFace::Fallback(_) => None,
    })
}

fn retained_repeatable_header(
    flow: &FlowDocument,
    table: &TableBuilder,
) -> Result<Option<AuthoredTypedHeaderModel>> {
    let Some(header) = table.header.as_ref() else {
        return Ok(None);
    };
    let placements = table.cell_placements(header)?;
    if placements.iter().any(|(source, _)| {
        matches!(
            table
                .cell_style(source.as_ref().map(|(_, cell)| *cell), true)
                .font,
            FontFace::Fallback(_)
        )
    }) {
        return Ok(None);
    }
    let page = flow.current_page_ref();
    let prepared_row = PreparedRow::new(table, page, header, true, &mut Budget::default())?;
    if placements.len() != prepared_row.cells.len() || prepared_row.cells.is_empty() {
        return Err(fail(
            "authored repeatable-header topology changed during retention",
        ));
    }
    let fragment = prepared_row.whole(&vec![0; prepared_row.cell_count()])?;
    let mut cells = Vec::with_capacity(prepared_row.cells.len());
    for ((source, columns), prepared_cell) in placements.into_iter().zip(&prepared_row.cells) {
        let Some(font) = retained_header_font(page, prepared_cell.style.font)? else {
            return Ok(None);
        };
        let (Ok(background), Ok(border), Ok(text)) = (
            AuthoredTypedColor::from_color(&prepared_cell.fill),
            AuthoredTypedColor::from_color(&prepared_row.border),
            AuthoredTypedColor::from_color(&prepared_cell.style.fill),
        ) else {
            return Ok(None);
        };
        cells.push(AuthoredTypedHeaderCellModel {
            text: source.map_or_else(String::new, |(_, cell)| cell.text.clone()),
            column: columns.start,
            column_span: columns.len(),
            layout: AuthoredTypedCellLayout {
                font_size: prepared_cell.style.size,
                line_spacing: table.style.paragraph.line_height,
                alignment: prepared_cell.align.into(),
                paint: Some(AuthoredTypedCellPaint {
                    background,
                    border,
                    text,
                    padding: prepared_row.padding,
                    line_width: prepared_row.line_width,
                }),
            },
            font,
        });
    }
    Ok(Some(AuthoredTypedHeaderModel {
        height: fragment.height,
        cells,
    }))
}

fn retained_table_pagination(
    flow: &FlowDocument,
    table: &TableBuilder,
) -> Result<AuthoredTypedTablePagination> {
    let pagination = AuthoredTypedTablePagination {
        page_size: [flow.page_size.width, flow.page_size.height],
        margins: [
            flow.margins.left,
            flow.margins.right,
            flow.margins.top,
            flow.margins.bottom,
        ],
        column_widths: table.columns.iter().map(|column| column.width).collect(),
        row_split_policy: table.row_split_policy,
        has_repeatable_header: table.header.is_some(),
        repeatable_header: retained_repeatable_header(flow, table)?,
        body_rows: table.rows.len(),
        row_page_breaks: Some(table.rows.iter().map(|row| row.page_break_before).collect()),
        direct_growth_safe: flow.builder.sections.get(flow.current_section).is_some_and(
            |section| {
                !section.mirror_margins
                    && section.first.is_none()
                    && section.odd == SectionPageMaster::default()
                    && section.even == SectionPageMaster::default()
            },
        ) && flow.current_page_ref().footnote_reserved_height <= EPS,
    };
    // Reuse the registry validator's geometry rules before any bytes are
    // published. A temporary model is unnecessary; keep this local check
    // explicit so authoring errors remain attributable to the flow contract.
    let [width, height] = pagination.page_size;
    let [left, right, top, bottom] = pagination.margins;
    let columns = pagination.column_widths.iter().sum::<f64>();
    pagination.row_split_policy.validate()?;
    if !pagination
        .page_size
        .iter()
        .chain(pagination.margins.iter())
        .chain(pagination.column_widths.iter())
        .all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
        || [left, right, top, bottom]
            .into_iter()
            .any(|value| value < 0.0)
        || width - left - right <= 0.0
        || height - top - bottom <= 0.0
        || pagination.column_widths.is_empty()
        || pagination.column_widths.iter().any(|width| *width <= 0.0)
        || columns > width - left - right + EPS
        || pagination.body_rows == 0
    {
        return Err(fail(
            "authored typed-table continuation contract is invalid",
        ));
    }
    Ok(pagination)
}

pub(super) struct AuthoredTypedTableRegistry {
    pub root: Option<u32>,
    pub objects: Vec<OutputObject>,
}

pub(super) fn build_typed_table_registry(
    builder: &PdfBuilder,
    next: &mut u32,
) -> Result<AuthoredTypedTableRegistry> {
    if builder.authored_typed_tables.is_empty() {
        return Ok(AuthoredTypedTableRegistry {
            root: None,
            objects: Vec::new(),
        });
    }
    if builder.authored_typed_tables.len() > 128 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-table registry count".into(),
        ));
    }
    let mut identities = BTreeSet::new();
    for model in &builder.authored_typed_tables {
        model.validate()?;
        if !identities.insert(model.id.as_str()) {
            return Err(fail("duplicate authored typed-table registry identity"));
        }
    }
    let raw = serde_json::to_vec(&builder.authored_typed_tables)
        .map_err(|error| fail(&format!("authored typed-table JSON: {error}")))?;
    if raw.len() > 16 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-table registry bytes".into(),
        ));
    }
    let number = alloc(next);
    Ok(AuthoredTypedTableRegistry {
        root: Some(number),
        objects: vec![OutputObject {
            number,
            object: PdfObject::Stream {
                dict: dict(&[
                    (
                        "Type",
                        PdfObject::Name("WellfriendAuthoredTypedTables".into()),
                    ),
                    (
                        "SchemaVersion",
                        PdfObject::String(b"wellfriend.authored_typed_table.v1".to_vec()),
                    ),
                ]),
                raw,
            },
        }],
    })
}

pub fn load_authored_typed_tables(input: &[u8]) -> Result<Vec<AuthoredTypedTableModel>> {
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let catalog = engine.document().get_catalog()?;
    let Some(registry) = catalog.get("WellfriendAuthoredTypedTables") else {
        return Ok(Vec::new());
    };
    let object = engine.document().reader().resolve(registry.clone())?;
    let decoded = crate::filters::decode_stream_lossless_with_limits(
        &object,
        engine.document().reader(),
        &crate::filters::DecodeLimits {
            max_decoded_bytes_per_stream: 16 * 1024 * 1024,
            ..Default::default()
        },
    )?;
    if decoded.status != crate::filters::StreamDecodeStatus::Complete
        || decoded.data.len() > 16 * 1024 * 1024
    {
        return Err(fail("authored typed-table registry is opaque or oversized"));
    }
    let models: Vec<AuthoredTypedTableModel> = serde_json::from_slice(&decoded.data)
        .map_err(|error| fail(&format!("authored typed-table registry JSON: {error}")))?;
    if models.len() > 128 {
        return Err(fail("authored typed-table registry count exceeds limit"));
    }
    let mut identities = BTreeSet::new();
    for model in &models {
        crate::cancel::check_current_cancel("authored typed-table registry validation")?;
        if !identities.insert(model.id.as_str()) {
            return Err(fail("duplicate authored typed-table registry identity"));
        }
        model.validate()?;
    }
    Ok(models)
}

/// Reopen the typed-table registry and bind every logical cell to the exact
/// marked-content carriers emitted by fresh authoring. Duplicated visible
/// words are irrelevant: only the paired private table/cell identities count.
pub fn inspect_authored_typed_table_sources(
    input: &[u8],
) -> Result<AuthoredTypedTableSourceReport> {
    #[derive(Default)]
    struct Pending {
        key: Option<(String, String)>,
        page: usize,
        start: usize,
        end: usize,
        logical_text: String,
        actual_text_sources: BTreeSet<String>,
        span_ids: Vec<String>,
        writing_mode: Option<i32>,
        region: Option<[f64; 4]>,
        font_resource: Option<String>,
        font_size: Option<f64>,
        character_spacing: Option<f64>,
        word_spacing: Option<f64>,
        horizontal_scaling: Option<f64>,
        uniform_text_state: bool,
    }

    fn flush(
        pending: &mut Pending,
        found: &mut BTreeMap<(String, String), Vec<AuthoredTypedCellSourceFragment>>,
    ) {
        let Some(key) = pending.key.take() else {
            return;
        };
        found
            .entry(key)
            .or_default()
            .push(AuthoredTypedCellSourceFragment {
                page: pending.page,
                logical_range: [pending.start, pending.end],
                logical_text: std::mem::take(&mut pending.logical_text),
                source_span_ids: std::mem::take(&mut pending.span_ids),
                region: pending.region.take(),
                writing_mode: pending.writing_mode.take().unwrap_or(-1),
                font_resource: pending.font_resource.take().unwrap_or_default(),
                font_size: pending.font_size.take().unwrap_or(0.0),
                character_spacing: pending.character_spacing.take().unwrap_or(0.0),
                word_spacing: pending.word_spacing.take().unwrap_or(0.0),
                horizontal_scaling: pending.horizontal_scaling.take().unwrap_or(0.0),
                uniform_text_state: pending.uniform_text_state,
            });
        pending.actual_text_sources.clear();
        pending.uniform_text_state = false;
    }

    let models = load_authored_typed_tables(input)?;
    let mut expected = BTreeMap::new();
    for table in &models {
        for cell in &table.cells {
            if expected
                .insert((table.id.clone(), cell.id.clone()), cell.evaluated.clone())
                .is_some()
            {
                return Err(fail(
                    "duplicate authored typed table/cell identity in registry",
                ));
            }
        }
    }
    if expected.is_empty() {
        return Ok(AuthoredTypedTableSourceReport {
            schema_version: "wellfriend.authored_typed_table_sources.v1".into(),
            input_sha256: format!("{:x}", Sha256::digest(input)),
            cells: Vec::new(),
            exact_source_ownership: true,
        });
    }

    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let mut found = BTreeMap::<(String, String), Vec<AuthoredTypedCellSourceFragment>>::new();
    for page in 1..=engine.page_count()? {
        crate::cancel::check_current_cancel("authored typed-table source inspection")?;
        let page_box = engine.document().get_page(page)?.crop_box;
        let model = crate::advanced_editing::analyze_multi_run_text_range(input, page)?;
        let mut pending = Pending::default();
        for span in model.source_spans {
            let key = span
                .authored_typed_table
                .clone()
                .zip(span.authored_typed_cell.clone());
            let Some(key) = key else {
                flush(&mut pending, &mut found);
                continue;
            };
            if !expected.contains_key(&key) {
                return Err(fail(
                    "authored typed cell source owner is absent from the validated registry",
                ));
            }
            if span
                .authored_typed_region
                .is_some_and(|region| !region_within_page_box(region, page_box))
            {
                return Err(fail(
                    "authored typed-cell content region escapes its effective page box",
                ));
            }
            let continues = pending.key.as_ref() == Some(&key)
                && pending.page == page
                && pending.end == span.logical_range[0];
            if !continues {
                flush(&mut pending, &mut found);
                pending.key = Some(key);
                pending.page = page;
                pending.start = span.logical_range[0];
                pending.end = span.logical_range[0];
                pending.writing_mode = Some(span.writing_mode);
                pending.region = span.authored_typed_region;
                pending.font_resource = Some(span.font_resource.clone());
                pending.font_size = Some(span.font_size);
                pending.character_spacing = Some(span.character_spacing);
                pending.word_spacing = Some(span.word_spacing);
                pending.horizontal_scaling = Some(span.horizontal_scaling);
                pending.uniform_text_state = true;
            } else if pending.writing_mode != Some(span.writing_mode) {
                pending.writing_mode = Some(-1);
            }
            if continues && pending.region != span.authored_typed_region {
                return Err(fail(
                    "authored typed cell source fragment has inconsistent content regions",
                ));
            }
            if continues
                && (pending.font_resource.as_deref() != Some(span.font_resource.as_str())
                    || pending.font_size != Some(span.font_size)
                    || pending.character_spacing != Some(span.character_spacing)
                    || pending.word_spacing != Some(span.word_spacing)
                    || pending.horizontal_scaling != Some(span.horizontal_scaling))
            {
                pending.uniform_text_state = false;
            }
            pending.end = span.logical_range[1];
            pending.span_ids.push(span.span_id);
            match (span.direct_actual_text_source, span.direct_actual_text) {
                (Some(source), Some(text)) => {
                    if pending.actual_text_sources.insert(source) {
                        pending.logical_text.push_str(&text);
                    }
                }
                (None, None) => pending.logical_text.push_str(&span.text),
                _ => {
                    return Err(fail(
                        "authored typed cell has an incomplete ActualText provenance pair",
                    ));
                }
            }
        }
        flush(&mut pending, &mut found);
    }

    let mut cells = Vec::with_capacity(expected.len());
    for ((table_id, cell_id), evaluated) in expected {
        let fragments = found
            .remove(&(table_id.clone(), cell_id.clone()))
            .ok_or_else(|| {
                fail("authored typed cell has no exact marked-content source carrier")
            })?;
        let observed = fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>();
        if observed != evaluated
            || fragments.iter().any(|fragment| {
                fragment.source_span_ids.is_empty()
                    || fragment.logical_range[0] > fragment.logical_range[1]
                    || (fragment.uniform_text_state
                        && (fragment.font_resource.is_empty()
                            || !fragment.font_size.is_finite()
                            || fragment.font_size <= 0.0
                            || !fragment.character_spacing.is_finite()
                            || !fragment.word_spacing.is_finite()
                            || !fragment.horizontal_scaling.is_finite()
                            || fragment.horizontal_scaling <= 0.0))
            })
        {
            return Err(fail(
                "authored typed cell source differs from its exact registry evaluation",
            ));
        }
        cells.push(AuthoredTypedCellSourceBinding {
            table_id,
            cell_id,
            evaluated,
            fragments,
        });
    }
    if !found.is_empty() {
        return Err(fail("unclaimed authored typed cell source ownership"));
    }
    Ok(AuthoredTypedTableSourceReport {
        schema_version: "wellfriend.authored_typed_table_sources.v1".into(),
        input_sha256: format!("{:x}", Sha256::digest(input)),
        cells,
        exact_source_ownership: true,
    })
}

/// Bind the exact static grid/fill artifact scopes emitted for a retained
/// authored typed table. Row relocation consumes this source provenance; it
/// never infers ownership from rectangles, colours or extracted text.
pub fn inspect_authored_typed_table_grid_paint(
    input: &[u8],
    table_id: &str,
) -> Result<AuthoredTypedTableGridPaintReport> {
    use crate::content::operation::Operand;

    #[derive(Debug)]
    struct Scope {
        table: String,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        start: usize,
        rect: Option<[f64; 4]>,
        painted: bool,
    }

    fn property<'a>(
        dictionary: &'a [(String, Operand)],
        name: &str,
    ) -> Result<Option<&'a Operand>> {
        let mut values = dictionary
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value);
        let value = values.next();
        if values.next().is_some() {
            return Err(fail("duplicate authored grid-paint property"));
        }
        Ok(value)
    }

    fn bounded_index(value: Option<&Operand>, name: &str, maximum: usize) -> Result<usize> {
        value
            .and_then(Operand::as_integer)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= maximum)
            .ok_or_else(|| fail(&format!("invalid authored grid-paint {name}")))
    }

    if table_id.is_empty() || table_id.len() > 16 * 1024 || table_id.contains('\0') {
        return Err(fail(
            "invalid authored typed-table grid inspection identity",
        ));
    }
    let models = load_authored_typed_tables(input)?;
    let model = models
        .iter()
        .find(|model| model.id == table_id)
        .ok_or_else(|| fail("authored typed-table grid inspection identity is absent"))?;
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let pages = engine.document().get_pages()?;
    if pages.len() > 100_000 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-table grid page count".into(),
        ));
    }
    let mut decoded_bytes = 0usize;
    let mut fragments = Vec::new();
    let mut exact_ranges = BTreeSet::new();
    for (page_index, page) in pages.iter().enumerate() {
        if page.contents.len() > 4096 {
            return Err(WellfriendError::ResourceLimit(
                "authored typed-table grid content-stream count".into(),
            ));
        }
        for &(number, generation) in &page.contents {
            crate::cancel::check_current_cancel("authored typed-table grid inspection")?;
            let object = reader.get_object(number, generation)?;
            let decoded = crate::filters::decode_stream_lossless_with_limits(
                &object,
                reader,
                &crate::filters::DecodeLimits {
                    max_decoded_bytes_per_stream: 64 * 1024 * 1024,
                    ..Default::default()
                },
            )?;
            decoded_bytes = decoded_bytes
                .checked_add(decoded.data.len())
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored typed-table grid decoded bytes".into())
                })?;
            if decoded.status != crate::filters::StreamDecodeStatus::Complete
                || decoded_bytes > 256 * 1024 * 1024
            {
                return Err(fail(
                    "authored typed-table grid stream is opaque or exceeds its decode budget",
                ));
            }
            let mut scopes: Vec<Option<Scope>> = Vec::new();
            crate::image_fragments::operations(&decoded.data, |start, end, operation, inline| {
                if inline {
                    if scopes.iter().any(Option::is_some) {
                        return Err(fail("inline image appears inside authored grid ownership"));
                    }
                    return Ok(());
                }
                match operation.operator.as_str() {
                    "BDC" | "BMC" => {
                        if scopes.len() >= 4096 {
                            return Err(WellfriendError::ResourceLimit(
                                "authored typed-table grid marked-content depth".into(),
                            ));
                        }
                        if scopes.iter().any(Option::is_some) {
                            return Err(fail("nested content inside authored grid ownership"));
                        }
                        let dictionary = operation.operands.get(1).and_then(Operand::as_dictionary);
                        let private_property = dictionary.is_some_and(|dictionary| {
                            dictionary.iter().any(|(key, _)| {
                                matches!(
                                    key.as_str(),
                                    "WFRowGrid"
                                        | "WFGridTableID"
                                        | "WFRow"
                                        | "WFColumn"
                                        | "WFRowSpan"
                                        | "WFColSpan"
                                )
                            })
                        });
                        let owned = if private_property {
                            if operation.operator != "BDC" || operation.name(0) != Some("Artifact")
                            {
                                return Err(fail("authored grid ownership has the wrong tag"));
                            }
                            let dictionary = dictionary.ok_or_else(|| {
                                fail("authored grid ownership dictionary is missing")
                            })?;
                            if property(dictionary, "WFRowGrid")?.and_then(Operand::as_bool)
                                != Some(true)
                            {
                                return Err(fail("authored grid ownership marker is invalid"));
                            }
                            let table = property(dictionary, "WFGridTableID")?
                                .and_then(Operand::as_bytes)
                                .map(crate::info::decode_pdf_text_string)
                                .filter(|table| {
                                    !table.is_empty()
                                        && table.len() <= 16 * 1024
                                        && !table.contains('\0')
                                })
                                .ok_or_else(|| fail("authored grid table identity is invalid"))?;
                            let row =
                                bounded_index(property(dictionary, "WFRow")?, "row", 1_000_000)?;
                            let column =
                                bounded_index(property(dictionary, "WFColumn")?, "column", 10_000)?;
                            let row_span = bounded_index(
                                property(dictionary, "WFRowSpan")?,
                                "row span",
                                1_000_000,
                            )?;
                            let column_span = bounded_index(
                                property(dictionary, "WFColSpan")?,
                                "column span",
                                10_000,
                            )?;
                            if row_span == 0 || column_span == 0 {
                                return Err(fail("authored grid ownership span is empty"));
                            }
                            Some(Scope {
                                table,
                                row,
                                column,
                                row_span,
                                column_span,
                                start,
                                rect: None,
                                painted: false,
                            })
                        } else {
                            None
                        };
                        scopes.push(owned);
                    }
                    "EMC" => {
                        let scope = scopes
                            .pop()
                            .ok_or_else(|| fail("unbalanced authored grid marked content"))?;
                        if let Some(scope) = scope {
                            let rect = scope.rect.filter(|_| scope.painted).ok_or_else(|| {
                                fail("authored grid scope has no exact painted rectangle")
                            })?;
                            if !region_within_page_box(rect, page.crop_box) {
                                return Err(fail(
                                    "authored grid rectangle escapes its effective page box",
                                ));
                            }
                            if scope.table == table_id {
                                if !exact_ranges.insert((
                                    page_index + 1,
                                    number,
                                    generation,
                                    scope.start,
                                    end,
                                )) {
                                    return Err(fail("duplicate authored grid source range"));
                                }
                                fragments.push(AuthoredTypedGridPaintFragment {
                                    page: page_index + 1,
                                    stream_object: [number, u32::from(generation)],
                                    decoded_range: [scope.start, end],
                                    row: scope.row,
                                    column: scope.column,
                                    row_span: scope.row_span,
                                    column_span: scope.column_span,
                                    rect,
                                });
                            }
                        }
                    }
                    "re" => {
                        if let Some(scope) = scopes.last_mut().and_then(Option::as_mut) {
                            if scope.rect.is_some() || operation.operands.len() != 4 {
                                return Err(fail("authored grid scope has multiple rectangles"));
                            }
                            let x = operation
                                .number(0)
                                .ok_or_else(|| fail("grid x is invalid"))?;
                            let y = operation
                                .number(1)
                                .ok_or_else(|| fail("grid y is invalid"))?;
                            let width = operation
                                .number(2)
                                .ok_or_else(|| fail("grid width is invalid"))?;
                            let height = operation
                                .number(3)
                                .ok_or_else(|| fail("grid height is invalid"))?;
                            if ![x, y, width, height].iter().all(|value| value.is_finite())
                                || width <= 0.0
                                || height <= 0.0
                            {
                                return Err(fail("authored grid rectangle is invalid"));
                            }
                            let rect = [x, y, x + width, y + height];
                            if !rect.iter().all(|value| value.is_finite()) {
                                return Err(fail("authored grid rectangle extent overflow"));
                            }
                            scope.rect = Some(rect);
                        }
                    }
                    "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                        if let Some(scope) = scopes.last_mut().and_then(Option::as_mut) {
                            if scope.painted || scope.rect.is_none() {
                                return Err(fail("authored grid scope has invalid paint order"));
                            }
                            scope.painted = true;
                        }
                    }
                    "q" | "Q" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" | "CS" | "cs"
                    | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => {}
                    _ if scopes.last().is_some_and(Option::is_some) => {
                        return Err(fail("unsupported operation inside authored grid ownership"));
                    }
                    _ => {}
                }
                Ok(())
            })?;
            if !scopes.is_empty() {
                return Err(fail("unterminated authored grid marked content"));
            }
        }
    }
    fragments.sort_by_key(|fragment| {
        (
            fragment.page,
            fragment.stream_object,
            fragment.decoded_range,
        )
    });
    let typed_topology = model
        .cells
        .iter()
        .map(|cell| (cell.row, cell.column, cell.row_span, cell.column_span))
        .collect::<BTreeSet<_>>();
    let paint_topology = fragments
        .iter()
        .map(|cell| (cell.row, cell.column, cell.row_span, cell.column_span))
        .collect::<BTreeSet<_>>();
    Ok(AuthoredTypedTableGridPaintReport {
        schema_version: "wellfriend.authored_typed_table_grid_paint.v1".into(),
        input_sha256: format!("{:x}", Sha256::digest(input)),
        table_id: table_id.to_string(),
        complete_typed_grid_ownership: !typed_topology.is_empty()
            && typed_topology == paint_topology,
        fragments,
    })
}

fn locate_current_authored_typed_owner(
    input: &[u8],
    table_identity: &str,
    cell_identity: &str,
) -> Result<Vec<AuthoredTypedCellSourceFragment>> {
    fn flush(
        current: &mut Option<AuthoredTypedCellSourceFragment>,
        output: &mut Vec<AuthoredTypedCellSourceFragment>,
    ) {
        if let Some(fragment) = current.take() {
            output.push(fragment);
        }
    }

    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let mut output = Vec::new();
    for page in 1..=engine.page_count()? {
        crate::cancel::check_current_cancel("authored typed-table owner rebinding")?;
        let page_box = engine.document().get_page(page)?.crop_box;
        let model = crate::advanced_editing::analyze_multi_run_text_range(input, page)?;
        let mut current = None::<AuthoredTypedCellSourceFragment>;
        for span in model.source_spans {
            let matches = span.authored_typed_table.as_deref() == Some(table_identity)
                && span.authored_typed_cell.as_deref() == Some(cell_identity);
            if !matches {
                flush(&mut current, &mut output);
                continue;
            }
            if span
                .authored_typed_region
                .is_some_and(|region| !region_within_page_box(region, page_box))
            {
                return Err(fail(
                    "authored typed-cell mutation region escapes its effective page box",
                ));
            }
            let continues = current.as_ref().is_some_and(|fragment| {
                fragment.page == page && fragment.logical_range[1] == span.logical_range[0]
            });
            if !continues {
                flush(&mut current, &mut output);
                current = Some(AuthoredTypedCellSourceFragment {
                    page,
                    logical_range: [span.logical_range[0], span.logical_range[0]],
                    logical_text: String::new(),
                    source_span_ids: Vec::new(),
                    region: span.authored_typed_region,
                    writing_mode: span.writing_mode,
                    font_resource: span.font_resource.clone(),
                    font_size: span.font_size,
                    character_spacing: span.character_spacing,
                    word_spacing: span.word_spacing,
                    horizontal_scaling: span.horizontal_scaling,
                    uniform_text_state: true,
                });
            }
            let fragment = current.as_mut().expect("authored owner fragment");
            fragment.logical_range[1] = span.logical_range[1];
            fragment.source_span_ids.push(span.span_id);
            if fragment.writing_mode != span.writing_mode {
                fragment.writing_mode = -1;
            }
            if fragment.region != span.authored_typed_region {
                return Err(fail(
                    "authored typed cell changed content region during mutation rebinding",
                ));
            }
            if fragment.font_resource != span.font_resource
                || fragment.font_size != span.font_size
                || fragment.character_spacing != span.character_spacing
                || fragment.word_spacing != span.word_spacing
                || fragment.horizontal_scaling != span.horizontal_scaling
            {
                fragment.uniform_text_state = false;
            }
        }
        flush(&mut current, &mut output);
    }
    if output.is_empty()
        || output
            .iter()
            .any(|fragment| fragment.source_span_ids.is_empty())
    {
        return Err(fail(
            "authored typed-table owner disappeared during mutation rebinding",
        ));
    }
    Ok(output)
}

fn save_authored_typed_table_models(
    input: &[u8],
    models: &[AuthoredTypedTableModel],
) -> Result<Vec<u8>> {
    use crate::writer::{write_incremental_update, IncrementalObject};

    if models.is_empty() || models.len() > 128 {
        return Err(fail("authored typed-table registry model count is invalid"));
    }
    let mut tables = BTreeSet::new();
    for model in models {
        model.validate()?;
        if !tables.insert(model.id.as_str()) {
            return Err(fail("duplicate authored typed-table registry identity"));
        }
    }
    let raw = serde_json::to_vec(models)
        .map_err(|error| fail(&format!("authored typed-table JSON: {error}")))?;
    if raw.len() > 16 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-table registry bytes".into(),
        ));
    }
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let catalog = engine.document().get_catalog()?;
    let (number, generation) = catalog
        .get("WellfriendAuthoredTypedTables")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| fail("authored typed-table registry reference is missing"))?;
    let object = reader.get_object(number, generation)?;
    let PdfObject::Stream { mut dict, .. } = object else {
        return Err(fail("authored typed-table registry is not a stream"));
    };
    if dict.get_name("Type") != Some("WellfriendAuthoredTypedTables") {
        return Err(fail("authored typed-table registry has the wrong type"));
    }
    dict.remove("Filter");
    dict.remove("DecodeParms");
    dict.insert(
        "Length",
        PdfObject::Integer(
            i64::try_from(raw.len())
                .map_err(|_| WellfriendError::ResourceLimit("typed-table JSON length".into()))?,
        ),
    );
    write_incremental_update(
        reader,
        vec![IncrementalObject {
            number,
            generation,
            object: PdfObject::Stream { dict, raw },
        }],
    )
}

fn enforce_authored_table_signature_policy(
    policy: &crate::secure_mutation::EditPolicyReport,
    override_requested: bool,
) -> Result<()> {
    use crate::secure_mutation::EditPolicyDecision;
    if matches!(
        policy.decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !override_requested
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-table mutation is blocked by signature policy; explicit override required"
                .into(),
        ));
    }
    if policy.full_rewrite_required {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-table mutation requires a full rewrite but this transaction is incremental"
                .into(),
        ));
    }
    Ok(())
}

fn authored_mutation_mode(text: &str, writing_mode: i32) -> Result<crate::AdvancedTextMode> {
    if writing_mode == 1 {
        return Ok(crate::AdvancedTextMode::ParagraphReflowVertical);
    }
    if writing_mode != 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-cell source mixes horizontal and vertical writing modes".into(),
        ));
    }
    let rtl = text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x0590..=0x08FF
                | 0xFB1D..=0xFDFF
                | 0xFE70..=0xFEFF
                | 0x202A..=0x202E
                | 0x2066..=0x2069
        )
    });
    Ok(if rtl {
        crate::AdvancedTextMode::ParagraphReflowRtl
    } else {
        crate::AdvancedTextMode::ParagraphReflowHorizontal
    })
}

#[derive(Debug)]
struct AuthoredFragmentTextPlan {
    page: usize,
    region: [f64; 4],
    lines: Vec<crate::ExplicitLayoutLine>,
}

#[derive(Debug)]
struct AuthoredRedistributionPlan {
    fragments: Vec<AuthoredFragmentTextPlan>,
    generated_font: Option<Vec<u8>>,
    force_generated_style: bool,
    consumed_bytes: usize,
}

fn authored_horizontal_line_capacity(
    region: [f64; 4],
    layout: &AuthoredTypedCellLayout,
) -> Result<usize> {
    let height = region[3] - region[1];
    let advance = layout.font_size * layout.line_spacing;
    if !height.is_finite() || height <= 0.0 || !advance.is_finite() || advance <= 0.0 {
        return Err(fail("invalid retained authored typed-cell line geometry"));
    }
    let available = height - layout.font_size;
    if available < -EPS {
        return Ok(0);
    }
    let intervals = (available.max(0.0) / advance).floor();
    Ok(1usize.saturating_add(intervals.min(9_999.0) as usize))
}

fn authored_source_line_width(
    resolver: &crate::fonts::FontResolver,
    text: &str,
    fragment: &AuthoredTypedCellSourceFragment,
) -> Result<f64> {
    if text.is_empty() {
        return Ok(0.0);
    }
    let (encoded, ambiguous) = resolver.try_encode_existing(text).map_err(|reason| {
        WellfriendError::UnsupportedFeature(format!(
            "authored typed-cell source font cannot encode planned line: {reason}"
        ))
    })?;
    if ambiguous {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-cell source font has an ambiguous reverse mapping; use one approved generated font"
                .into(),
        ));
    }
    let mut width = 0.0;
    for decoded in resolver.codes(&encoded) {
        let code = decoded.map_err(WellfriendError::UnsupportedFeature)?.code;
        width += resolver.width_for_code(code) / 1000.0 * fragment.font_size;
        width += fragment.character_spacing;
        if code.is_word_space() {
            width += fragment.word_spacing;
        }
    }
    width *= fragment.horizontal_scaling / 100.0;
    if !width.is_finite() || width < 0.0 {
        return Err(fail("authored typed-cell source width overflow"));
    }
    Ok(width)
}

fn font_covers_authored_text(font: &[u8], text: &str) -> Result<bool> {
    if ttf_parser::Face::parse(font, 0).is_err() {
        return Ok(false);
    }
    for hard_line in crate::fonts::hard_break::logical_lines(text) {
        let hard_line = hard_line?;
        let visible = &text[hard_line.visible];
        if visible.is_empty() {
            continue;
        }
        let run = match crate::fonts::TextShaper::shape(
            font,
            visible,
            crate::fonts::ShapeOptions::default(),
        ) {
            Ok(run) => run,
            Err(_) => return Ok(false),
        };
        if crate::fonts::shaper::has_missing_glyphs(font, visible, &run)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn plan_authored_text_across_existing_fragments(
    input: &[u8],
    fragments: &[AuthoredTypedCellSourceFragment],
    additional_regions: &[(usize, [f64; 4])],
    replacement: &str,
    layout: &AuthoredTypedCellLayout,
    approved_font: Option<&[u8]>,
) -> Result<AuthoredRedistributionPlan> {
    if fragments.is_empty() || fragments.len() > 4096 || replacement.len() > 64_000 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-cell redistribution exceeds fragment or text budget".into(),
        ));
    }
    if fragments.iter().any(|fragment| fragment.writing_mode != 0) {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-cell cross-fragment redistribution currently requires uniform horizontal source owners"
                .into(),
        ));
    }
    if fragments.iter().any(|fragment| {
        !fragment.uniform_text_state
            || fragment.font_resource.is_empty()
            || !fragment.font_size.is_finite()
            || (fragment.font_size - layout.font_size).abs() > EPS
            || !fragment.character_spacing.is_finite()
            || !fragment.word_spacing.is_finite()
            || !fragment.horizontal_scaling.is_finite()
            || fragment.horizontal_scaling <= 0.0
            || fragment.region.is_none()
    }) {
        return Err(fail(
            "authored typed-cell source state differs from its retained layout contract",
        ));
    }

    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    // One generated font is selected before pagination. Without this rule,
    // fragments whose old subsets happen to cover different characters could
    // be measured with different metrics and later serialize different breaks.
    let contains_contextual_text = replacement.chars().any(|ch| {
        matches!(
            ch as u32,
            0x0590..=0x08FF
                | 0xFB1D..=0xFDFF
                | 0xFE70..=0xFEFF
                | 0x202A..=0x202E
                | 0x2066..=0x2069
        )
    });
    let mut every_source_font_encodes = !contains_contextual_text;
    if every_source_font_encodes {
        'fragment: for fragment in fragments {
            let page = engine.document().get_page(fragment.page)?;
            let resources = crate::PageResources::from_dict(&page.resources, reader);
            let font = resources
                .fonts
                .get(&fragment.font_resource)
                .ok_or_else(|| {
                    fail(&format!(
                        "authored typed-cell source font resource /{} disappeared from page {}; available={:?}",
                        fragment.font_resource,
                        fragment.page,
                        resources.fonts.keys().collect::<Vec<_>>()
                    ))
                })?;
            let resolver = crate::fonts::FontResolver::new(font, reader);
            // Standard 14 widths are the only source-CMap path whose advance
            // model is complete without the original shaping program. Custom
            // authored fonts may carry kerning, mark positioning or contextual
            // advances, so keep their full font program and force the shared
            // OpenType measurement/emission route even if reverse encoding is
            // technically available.
            if !resolver.has_standard14_metrics() {
                every_source_font_encodes = false;
                break 'fragment;
            }
            for hard_line in crate::fonts::hard_break::logical_lines(replacement) {
                let hard_line = hard_line?;
                let visible = &replacement[hard_line.visible];
                if visible.is_empty() {
                    continue;
                }
                match resolver.try_encode_existing(visible) {
                    Ok((_, false)) => {}
                    _ => {
                        every_source_font_encodes = false;
                        break 'fragment;
                    }
                }
            }
        }
    }
    let force_generated_style = !every_source_font_encodes;
    let generated_font = if force_generated_style {
        if let Some(font) = approved_font {
            if !font_covers_authored_text(font, replacement)? {
                return Err(WellfriendError::UnsupportedFeature(
                    "approved authored-table font is invalid or lacks replacement glyph coverage"
                        .into(),
                ));
            }
            Some(font.to_vec())
        } else {
            let first = &fragments[0];
            let page = engine.document().get_page(first.page)?;
            let resources = crate::PageResources::from_dict(&page.resources, reader);
            let embedded = resources
                .fonts
                .get(&first.font_resource)
                .and_then(|font| crate::fonts::provider::embedded_program(reader, font));
            if let Some(font) = embedded {
                if font_covers_authored_text(&font, replacement)? {
                    Some(font)
                } else {
                    let fallback = crate::render::get_fallback_font("Symbol").ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(
                            "authored-table generated shaping font unavailable".into(),
                        )
                    })?;
                    if !font_covers_authored_text(fallback, replacement)? {
                        return Err(WellfriendError::UnsupportedFeature(
                            "authored-table replacement requires an explicitly approved font"
                                .into(),
                        ));
                    }
                    Some(fallback.to_vec())
                }
            } else {
                let fallback = crate::render::get_fallback_font("Symbol").ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "authored-table generated shaping font unavailable".into(),
                    )
                })?;
                if !font_covers_authored_text(fallback, replacement)? {
                    return Err(WellfriendError::UnsupportedFeature(
                        "authored-table replacement requires an explicitly approved font".into(),
                    ));
                }
                Some(fallback.to_vec())
            }
        }
    } else {
        None
    };

    let mode = authored_mutation_mode(replacement, 0)?;
    let shape_options = crate::fonts::ShapeOptions {
        direction: Some(if mode == crate::AdvancedTextMode::ParagraphReflowRtl {
            crate::fonts::TextDirection::RightToLeft
        } else {
            crate::fonts::TextDirection::LeftToRight
        }),
    };
    let paragraph = crate::fonts::line_layout::PreparedParagraph::new(replacement, shape_options)?;
    let mut cursor = 0usize;
    let template = &fragments[0];
    let mut planning = fragments
        .iter()
        .map(|fragment| {
            (
                fragment.page,
                fragment.region.expect("validated authored region"),
                fragment,
            )
        })
        .collect::<Vec<_>>();
    planning.extend(
        additional_regions
            .iter()
            .map(|(page, region)| (*page, *region, template)),
    );
    let mut planned = Vec::with_capacity(planning.len());
    for (index, (output_page, region, fragment)) in planning.into_iter().enumerate() {
        crate::cancel::check_current_cancel("authored typed-cell redistribution planning")?;
        if index >= fragments.len() && cursor == replacement.len() {
            break;
        }
        if !region.iter().all(|value| value.is_finite())
            || region[0] >= region[2]
            || region[1] >= region[3]
        {
            return Err(fail("invalid authored typed-cell planned region"));
        }
        let capacity = authored_horizontal_line_capacity(region, layout)?;
        if capacity == 0 {
            planned.push(AuthoredFragmentTextPlan {
                page: output_page,
                region,
                lines: Vec::new(),
            });
            continue;
        }
        let page = engine.document().get_page(fragment.page)?;
        let resources = crate::PageResources::from_dict(&page.resources, reader);
        let source_font = resources
            .fonts
            .get(&fragment.font_resource)
            .ok_or_else(|| {
                fail(&format!(
                    "authored typed-cell source font resource /{} disappeared from page {}; available={:?}",
                    fragment.font_resource,
                    fragment.page,
                    resources.fonts.keys().collect::<Vec<_>>()
                ))
            })?;
        let resolver = crate::fonts::FontResolver::new(source_font, reader);
        let width = region[2] - region[0];
        let batch = paragraph.break_lines_measured_prefix(cursor, width, capacity, |range| {
            let visible = replacement[range.clone()]
                .trim_end_matches(crate::fonts::hard_break::is_hard_break);
            if force_generated_style {
                let bidi = paragraph
                    .bidi
                    .line(range.start..range.start + visible.len())?;
                crate::advanced_editing::measure_generated_preserved_line(
                    visible,
                    &bidi,
                    generated_font.as_deref().expect("generated font selected"),
                    fragment.font_size,
                    fragment.character_spacing,
                    fragment.word_spacing,
                    fragment.horizontal_scaling,
                )
            } else {
                authored_source_line_width(&resolver, visible, fragment)
            }
        })?;
        let mut lines = Vec::with_capacity(batch.lines.len());
        for line in batch.lines {
            let logical = &replacement[line.bytes.clone()];
            let visual = logical.trim_end_matches(crate::fonts::hard_break::is_hard_break);
            let bidi = paragraph
                .bidi
                .line(line.bytes.start..line.bytes.start + visual.len())?;
            cursor = line.bytes.end;
            lines.push(crate::ExplicitLayoutLine {
                logical_text: logical.to_owned(),
                visual_text: visual.to_owned(),
                bidi: Some(bidi),
                inserted_visual_hyphen: false,
            });
        }
        planned.push(AuthoredFragmentTextPlan {
            page: output_page,
            region,
            lines,
        });
        if cursor == replacement.len() {
            // Retain every remaining owner as an empty addressable carrier.
            continue;
        }
    }
    if planned
        .iter()
        .flat_map(|fragment| &fragment.lines)
        .map(|line| line.logical_text.as_str())
        .collect::<String>()
        != replacement[..cursor]
    {
        return Err(fail(
            "authored typed-cell redistribution lost logical source coverage",
        ));
    }
    Ok(AuthoredRedistributionPlan {
        fragments: planned,
        generated_font,
        force_generated_style,
        consumed_bytes: cursor,
    })
}

fn standard_font_from_pdf_name(name: &str) -> Option<StandardFont> {
    let name = name.strip_prefix('/').unwrap_or(name);
    let name = name
        .split_once('+')
        .filter(|(prefix, _)| {
            prefix.len() == 6 && prefix.bytes().all(|byte| byte.is_ascii_uppercase())
        })
        .map_or(name, |(_, base)| base);
    Some(match name {
        "Helvetica" => StandardFont::Helvetica,
        "Helvetica-Bold" => StandardFont::HelveticaBold,
        "Helvetica-Oblique" => StandardFont::HelveticaOblique,
        "Helvetica-BoldOblique" => StandardFont::HelveticaBoldOblique,
        "Times-Roman" => StandardFont::TimesRoman,
        "Times-Bold" => StandardFont::TimesBold,
        "Times-Italic" => StandardFont::TimesItalic,
        "Times-BoldItalic" => StandardFont::TimesBoldItalic,
        "Courier" => StandardFont::Courier,
        "Courier-Bold" => StandardFont::CourierBold,
        "Courier-Oblique" => StandardFont::CourierOblique,
        "Courier-BoldOblique" => StandardFont::CourierBoldOblique,
        "Symbol" => StandardFont::Symbol,
        "ZapfDingbats" => StandardFont::ZapfDingbats,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
enum AuthoredContinuationFont {
    Standard(StandardFont),
    Embedded(Vec<u8>),
}

#[derive(Debug, Clone)]
struct AuthoredContinuationHeaderCell {
    model: AuthoredTypedHeaderCellModel,
    font: AuthoredContinuationFont,
}

#[derive(Debug, Clone)]
struct AuthoredContinuationCell {
    model: AuthoredTypedCellModel,
    layout: AuthoredTypedCellLayout,
    rect: [f64; 4],
    region: [f64; 4],
    font: AuthoredContinuationFont,
    /// Exact decoded page-resource name used by the original carrier. The
    /// source page retains that resource when its row scopes are removed, so a
    /// later provenance-bound backward compaction can recreate empty carriers
    /// without importing or guessing a font program.
    source_font_resource: String,
}

#[derive(Debug, Clone)]
struct AuthoredTypedRelocationCell {
    id: String,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
    role: AuthoredTypedCellRole,
    rect: [f64; 4],
    region: [f64; 4],
    font_resource: String,
}

#[derive(Debug, Clone)]
struct AuthoredTypedRelocationReceipt {
    origin_page: usize,
    destination_page: usize,
    target_row: usize,
    static_digest: String,
    cells: Vec<AuthoredTypedRelocationCell>,
}

fn retained_base_font_name(name: &str) -> &str {
    let name = name.strip_prefix('/').unwrap_or(name);
    name.split_once('+')
        .filter(|(prefix, _)| {
            prefix.len() == 6 && prefix.bytes().all(|byte| byte.is_ascii_uppercase())
        })
        .map_or(name, |(_, base)| base)
}

fn resolve_retained_header_font(
    document: &crate::PdfDocument,
    font: &AuthoredRetainedFont,
) -> Result<AuthoredContinuationFont> {
    let AuthoredRetainedFont::Embedded { base_name } = font else {
        let AuthoredRetainedFont::Standard { base_name } = font else {
            unreachable!("retained font variants are exhaustive")
        };
        return standard_font_from_pdf_name(base_name)
            .map(AuthoredContinuationFont::Standard)
            .ok_or_else(|| fail("authored repeatable-header Standard-14 face is unsupported"));
    };
    let reader = document.reader();
    let pages = document.get_pages()?;
    if pages.len() > 100_000 {
        return Err(WellfriendError::ResourceLimit(
            "authored repeatable-header font page scan".into(),
        ));
    }
    let mut resolved: Option<Vec<u8>> = None;
    let mut candidates = 0usize;
    for page in pages {
        crate::cancel::check_current_cancel("authored repeatable-header font resolution")?;
        let resources = crate::PageResources::from_dict(&page.resources, reader);
        for object in resources.fonts.values() {
            if object.get_name("BaseFont").map(retained_base_font_name) != Some(base_name.as_str())
            {
                continue;
            }
            candidates = candidates.checked_add(1).ok_or_else(|| {
                WellfriendError::ResourceLimit("authored repeatable-header font candidates".into())
            })?;
            if candidates > 4096 {
                return Err(WellfriendError::ResourceLimit(
                    "authored repeatable-header font candidates".into(),
                ));
            }
            let bytes =
                crate::fonts::provider::embedded_program(reader, object).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "authored repeatable-header font resource is not embedded".into(),
                    )
                })?;
            if bytes.len() > 256 * 1024 * 1024 {
                return Err(WellfriendError::ResourceLimit(
                    "authored repeatable-header embedded font bytes".into(),
                ));
            }
            if resolved.as_ref().is_some_and(|known| known != &bytes) {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored repeatable-header font identity resolves to multiple embedded programs"
                        .into(),
                ));
            }
            resolved = Some(bytes);
        }
    }
    resolved
        .map(AuthoredContinuationFont::Embedded)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "authored repeatable-header embedded font disappeared from the saved PDF".into(),
            )
        })
}

fn authored_row_continuation_geometry(
    model: &AuthoredTypedTableModel,
    target_row: usize,
) -> Result<
    Vec<(
        AuthoredTypedCellModel,
        AuthoredTypedCellLayout,
        [f64; 4],
        [f64; 4],
    )>,
> {
    let pagination = model.pagination.as_ref().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "authored typed-table source predates retained continuation geometry".into(),
        )
    })?;
    if target_row >= pagination.body_rows
        || !pagination.direct_growth_safe
        || pagination.row_split_policy != TableRowSplitPolicy::default()
        || model.cells.len() > 4096
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-table page growth requires a retained body row on a simple page template"
                .into(),
        ));
    }
    let header_height = if pagination.has_repeatable_header {
        pagination
            .repeatable_header
            .as_ref()
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "authored typed-table repeatable header predates the retained reconstruction contract or uses a contextual fallback stack"
                        .into(),
                )
            })?
            .height
    } else {
        0.0
    };
    let mut cells = model
        .cells
        .iter()
        .filter(|cell| cell.row == target_row)
        .cloned()
        .collect::<Vec<_>>();
    cells.sort_by_key(|cell| (cell.column, cell.id.clone()));
    let mut next_column = 0usize;
    for cell in &cells {
        if cell.row_span != 1
            || cell.column != next_column
            || cell.column_span == 0
            || cell
                .column
                .checked_add(cell.column_span)
                .is_none_or(|end| end > pagination.column_widths.len())
        {
            return Err(WellfriendError::UnsupportedFeature(
                "authored typed-table row growth requires typed cells to tile every retained column exactly once"
                    .into(),
            ));
        }
        next_column = cell.column + cell.column_span;
    }
    if cells.is_empty() || next_column != pagination.column_widths.len() {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-table row growth requires complete typed grid coverage".into(),
        ));
    }
    let [width, height] = pagination.page_size;
    let [left, right, top, bottom] = pagination.margins;
    let mut offsets = Vec::with_capacity(pagination.column_widths.len() + 1);
    offsets.push(left);
    for column in &pagination.column_widths {
        offsets.push(offsets.last().copied().unwrap_or(left) + column);
    }
    let mut output = Vec::with_capacity(cells.len());
    for cell in cells {
        let layout = cell.layout.clone().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "authored typed-cell source predates retained continuation layout".into(),
            )
        })?;
        let paint = layout.paint.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "authored typed-cell source predates retained continuation paint".into(),
            )
        })?;
        let end = cell.column + cell.column_span;
        let rect = [
            offsets[cell.column],
            bottom,
            offsets[end],
            height - top - header_height,
        ];
        let region = [
            rect[0] + paint.padding,
            rect[1] + paint.padding,
            rect[2] - paint.padding,
            rect[3] - paint.padding,
        ];
        if !region
            .iter()
            .chain(rect.iter())
            .all(|value| value.is_finite())
            || rect[0] < 0.0
            || rect[1] < 0.0
            || rect[2] > width + EPS
            || rect[3] > height + EPS
            || region[0] >= region[2]
            || region[1] >= region[3]
            || right < 0.0
        {
            return Err(fail(
                "authored typed-cell continuation rectangle is outside its retained page",
            ));
        }
        output.push((cell, layout, rect, region));
    }
    Ok(output)
}

fn build_authored_typed_row_continuation_page(
    table_id: &str,
    pagination: &AuthoredTypedTablePagination,
    cells: &[AuthoredContinuationCell],
    header_cells: &[AuthoredContinuationHeaderCell],
) -> Result<Vec<u8>> {
    if cells.is_empty() || cells.len() > 4096 {
        return Err(fail(
            "authored typed-row continuation cell count is invalid",
        ));
    }
    let mut builder = PdfBuilder::new();
    let mut header_fonts = Vec::with_capacity(header_cells.len());
    for (index, cell) in header_cells.iter().enumerate() {
        header_fonts.push(match &cell.font {
            AuthoredContinuationFont::Standard(font) => FontFace::Standard(*font),
            AuthoredContinuationFont::Embedded(bytes) => builder.register_font_bytes(
                format!("WellfriendTypedHeaderContinuation{}", index + 1),
                bytes.clone(),
            )?,
        });
    }
    let mut fonts = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        fonts.push(match &cell.font {
            AuthoredContinuationFont::Standard(font) => FontFace::Standard(*font),
            AuthoredContinuationFont::Embedded(bytes) => builder.register_font_bytes(
                format!("WellfriendTypedCellContinuation{}", index + 1),
                bytes.clone(),
            )?,
        });
    }
    let table = structure::register_table(&mut builder, None)?;
    let body = structure::register_table_group(&mut builder, table, structure::Role::TableBody)?;
    let row = structure::register(&mut builder, structure::Role::TableRow, Some(body), None)?;
    let mut owners = Vec::with_capacity(cells.len());
    for cell in cells {
        let role = match cell.model.structure_role {
            AuthoredTypedCellRole::Data => structure::Role::TableData,
            AuthoredTypedCellRole::Header => structure::Role::TableHeader,
        };
        let owner = structure::register_table_cell(&mut builder, role, row, true)?;
        if let Some(scope) = cell.model.header_scope {
            structure::set_table_header_scope(&mut builder, owner, scope.into())?;
        }
        if cell.model.column_span > 1 {
            structure::set_table_column_span(&mut builder, owner, cell.model.column_span)?;
        }
        structure::set_typed_table_cell_owner(&mut builder, owner, table_id, &cell.model.id)?;
        owners.push(owner);
    }
    let [width, height] = pagination.page_size;
    let [left, right, top, bottom] = pagination.margins;
    let page = builder.add_page_with_margins(
        PageSize::custom(width, height),
        Margins {
            left,
            right,
            top,
            bottom,
        },
    );
    if let Some(header) = &pagination.repeatable_header {
        if header.cells.len() != header_cells.len() || header_cells.len() != header_fonts.len() {
            return Err(fail(
                "authored repeatable-header reconstruction topology changed",
            ));
        }
        let first_paint = header_cells
            .first()
            .and_then(|cell| cell.model.layout.paint.as_ref())
            .ok_or_else(|| fail("authored repeatable-header paint is missing"))?;
        let mut retained = TableBuilder::new(
            pagination
                .column_widths
                .iter()
                .copied()
                .map(TableColumn::new)
                .collect(),
        );
        retained.style = TableStyle {
            border_color: first_paint.border.to_color(),
            header_fill: first_paint.background.to_color(),
            row_fill: None,
            padding: first_paint.padding,
            line_width: first_paint.line_width,
            paragraph: ParagraphStyle::new().line_height(header_cells[0].model.layout.line_spacing),
        };
        let mut retained_cells = Vec::with_capacity(header_cells.len());
        for (cell, font) in header_cells.iter().zip(header_fonts) {
            let paint = cell
                .model
                .layout
                .paint
                .as_ref()
                .ok_or_else(|| fail("authored repeatable-header cell paint is missing"))?;
            if paint.padding != first_paint.padding
                || paint.line_width != first_paint.line_width
                || paint.border != first_paint.border
                || cell.model.layout.line_spacing != header_cells[0].model.layout.line_spacing
            {
                return Err(fail("authored repeatable-header shared row style changed"));
            }
            retained_cells.push(
                TableCell::text(cell.model.text.clone())
                    .style(
                        TextStyle::new(font, cell.model.layout.font_size)
                            .fill(paint.text.to_color()),
                    )
                    .background(paint.background.to_color())
                    .align(cell.model.layout.alignment.into())
                    .column_span(cell.model.column_span),
            );
        }
        retained.header = Some(TableRow::new(retained_cells));
        retained.validate()?;
        let retained_header = retained
            .header
            .as_ref()
            .ok_or_else(|| fail("authored repeatable-header row disappeared"))?;
        let prepared = PreparedRow::new(
            &retained,
            page,
            retained_header,
            true,
            &mut Budget::default(),
        )?;
        let fragment = Fragment {
            height: header.height,
            ranges: prepared
                .cells
                .iter()
                .map(|cell| 0..cell.lines.len())
                .collect(),
            complete: true,
        };
        prepared.render(page, left, height - top, &fragment, true, None, None)?;
    } else if !header_cells.is_empty() {
        return Err(fail(
            "authored continuation supplied header cells without a retained header",
        ));
    }
    for ((cell, owner), font) in cells.iter().zip(owners).zip(fonts) {
        let paint = cell
            .layout
            .paint
            .as_ref()
            .ok_or_else(|| fail("authored typed-cell continuation paint is missing"))?;
        page.commands
            .push(PageCommand::BeginOwnedTableCellArtifact {
                table: table_id.to_string(),
                row: cell.model.row,
                column: cell.model.column,
                row_span: cell.model.row_span,
                column_span: cell.model.column_span,
            });
        page.commands.push(PageCommand::Rect {
            x: cell.rect[0],
            y: cell.rect[1],
            width: cell.rect[2] - cell.rect[0],
            height: cell.rect[3] - cell.rect[1],
            style: GraphicsStyle::fill_stroke(
                paint.background.to_color(),
                paint.border.to_color(),
                paint.line_width,
            ),
        });
        page.commands.push(PageCommand::EndArtifact);
        page.commands.push(PageCommand::BeginTypedCellStructure {
            element: owner,
            region: cell.region,
        });
        page.commands.push(empty_owned_cell_command(
            cell.region[0],
            cell.region[3] - cell.layout.font_size,
            &TextStyle::new(font, cell.layout.font_size).fill(paint.text.to_color()),
        ));
        page.commands.push(PageCommand::EndStructure(owner));
    }
    builder.to_bytes()
}

fn authored_typed_table_continuation_row(
    dictionary: &PdfDictionary,
    table_id: &str,
) -> Option<usize> {
    let marker = dictionary
        .get(TYPED_TABLE_CONTINUATION_MARKER)
        .and_then(PdfObject::as_dict)?;
    if marker.get("Table").and_then(PdfObject::as_string) != Some(table_id.as_bytes()) {
        return None;
    }
    match marker.get_integer("Version")? {
        // Version 1 predates row-scoped provenance and was emitted only for
        // the former single-body-row transaction.
        1 => Some(0),
        2 => marker
            .get_integer("Row")
            .and_then(|value| usize::try_from(value).ok())
            .filter(|row| *row <= 1_000_000),
        _ => None,
    }
}

fn is_authored_typed_table_continuation(
    dictionary: &PdfDictionary,
    table_id: &str,
    row: usize,
    cell_count: usize,
) -> bool {
    authored_typed_table_continuation_row(dictionary, table_id) == Some(row)
        && dictionary
            .get(TYPED_TABLE_CONTINUATION_MARKER)
            .and_then(PdfObject::as_dict)
            .and_then(|marker| marker.get_integer("Cells"))
            .and_then(|value| usize::try_from(value).ok())
            == Some(cell_count)
}

fn authored_typed_table_continuation_digest<'a>(
    dictionary: &'a PdfDictionary,
    table_id: &str,
    row: usize,
    cell_count: usize,
) -> Option<&'a [u8]> {
    is_authored_typed_table_continuation(dictionary, table_id, row, cell_count)
        .then(|| {
            dictionary
                .get(TYPED_TABLE_CONTINUATION_MARKER)?
                .as_dict()?
                .get("StaticDigest")?
                .as_string()
                .filter(|digest| digest.len() == 64 && digest.iter().all(u8::is_ascii_hexdigit))
        })
        .flatten()
}

/// Stamp only pages allocated by the current typed-row transaction. Future
/// contraction must require this exact marker before it may even consider a
/// page for removal; visual emptiness or matching table geometry is never
/// sufficient ownership evidence.
fn stamp_authored_typed_table_continuations(
    input: &[u8],
    table_id: &str,
    row: usize,
    cell_ids: &BTreeSet<String>,
    page_numbers: &[usize],
) -> Result<Vec<u8>> {
    let cell_count = cell_ids.len();
    if table_id.is_empty()
        || row > 1_000_000
        || cell_count == 0
        || cell_count > 4096
        || page_numbers.is_empty()
        || page_numbers.len() > 10_000
        || page_numbers.windows(2).any(|pages| pages[0] >= pages[1])
    {
        return Err(fail("invalid authored typed-table continuation provenance"));
    }
    let page_set = page_numbers.iter().copied().collect::<BTreeSet<_>>();
    if page_set.len() != page_numbers.len() {
        return Err(fail(
            "authored typed-table continuation provenance repeats a page",
        ));
    }
    let inspections =
        crate::tagged_structure::story::inspect_authored_typed_cell_continuation_pages(
            input, table_id, cell_ids, &page_set,
        )?;
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut updates = Vec::with_capacity(page_numbers.len());
    for &page_number in page_numbers {
        crate::cancel::check_current_cancel("authored typed-table provenance")?;
        let page = engine.document().get_page(page_number)?;
        let mut dictionary = reader
            .get_object(page.object_number, page.generation_number)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("authored typed-table continuation page is not a dictionary"))?;
        if dictionary.contains_key(TYPED_TABLE_CONTINUATION_MARKER) {
            return Err(fail(
                "authored typed-table continuation page already has ownership provenance",
            ));
        }
        let mut marker = PdfDictionary::empty();
        marker.insert("Version", PdfObject::Integer(2));
        marker.insert("Table", PdfObject::String(table_id.as_bytes().to_vec()));
        marker.insert(
            "Row",
            PdfObject::Integer(
                i64::try_from(row)
                    .map_err(|_| fail("authored typed-table continuation row overflow"))?,
            ),
        );
        marker.insert(
            "Cells",
            PdfObject::Integer(
                i64::try_from(cell_count)
                    .map_err(|_| fail("authored typed-table continuation cell count overflow"))?,
            ),
        );
        let inspection = inspections.get(&page_number).ok_or_else(|| {
            fail("authored typed-table continuation provenance inspection is missing")
        })?;
        if !inspection.empty {
            return Err(fail(
                "new authored typed-table continuation is not initially empty",
            ));
        }
        marker.insert(
            "StaticDigest",
            PdfObject::String(inspection.static_digest.as_bytes().to_vec()),
        );
        dictionary.insert(
            TYPED_TABLE_CONTINUATION_MARKER,
            PdfObject::Dictionary(marker),
        );
        updates.push(crate::writer::IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dictionary),
        });
    }
    let output = crate::writer::write_incremental_update(reader, updates)?;
    let reopened = crate::ContentEngine::open_bytes(output.clone())?;
    for &page_number in page_numbers {
        let page = reopened.document().get_page(page_number)?;
        let page_object = reopened
            .document()
            .reader()
            .get_object(page.object_number, page.generation_number)?;
        let dictionary = page_object
            .as_dict()
            .ok_or_else(|| fail("authored typed-table continuation marker page disappeared"))?;
        if !is_authored_typed_table_continuation(dictionary, table_id, row, cell_count) {
            return Err(fail(
                "authored typed-table continuation provenance failed reopen validation",
            ));
        }
        if authored_typed_table_continuation_digest(dictionary, table_id, row, cell_count)
            != inspections
                .get(&page_number)
                .map(|inspection| inspection.static_digest.as_bytes())
        {
            return Err(fail(
                "authored typed-table continuation static digest failed reopen validation",
            ));
        }
    }
    Ok(output)
}

fn relocation_number(value: f64) -> PdfObject {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        PdfObject::Integer(value as i64)
    } else {
        PdfObject::Real(value)
    }
}

fn relocation_rect_object(rect: [f64; 4]) -> PdfObject {
    PdfObject::Array(rect.into_iter().map(relocation_number).collect())
}

fn relocation_rect(dictionary: &PdfDictionary, key: &str) -> Result<[f64; 4]> {
    let values = dictionary
        .get(key)
        .and_then(PdfObject::as_array)
        .filter(|values| values.len() == 4)
        .ok_or_else(|| fail("authored typed-row relocation rectangle is malformed"))?;
    let mut rect = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        rect[index] = value
            .as_number()
            .filter(|number| number.is_finite())
            .ok_or_else(|| fail("authored typed-row relocation rectangle is nonfinite"))?;
    }
    if rect[0] >= rect[2] || rect[1] >= rect[3] {
        return Err(fail("authored typed-row relocation rectangle is empty"));
    }
    Ok(rect)
}

fn inspect_authored_typed_table_relocations(
    input: &[u8],
    model: &AuthoredTypedTableModel,
) -> Result<Vec<AuthoredTypedRelocationReceipt>> {
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let pages = engine.document().get_pages()?;
    let reader = engine.document().reader();
    let page_numbers = pages
        .iter()
        .map(|page| {
            (
                (page.object_number, page.generation_number),
                page.page_number,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let pagination = model
        .pagination
        .as_ref()
        .ok_or_else(|| fail("authored typed-row relocation lost pagination metadata"))?;
    let mut receipts = Vec::new();
    let mut destinations = BTreeSet::new();
    for page in &pages {
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let dictionary = page_object
            .as_dict()
            .ok_or_else(|| fail("authored typed-row relocation origin is not a page dictionary"))?;
        let Some(marker) = dictionary
            .get(TYPED_TABLE_RELOCATION_MARKER)
            .and_then(PdfObject::as_dict)
        else {
            continue;
        };
        if marker.get("Table").and_then(PdfObject::as_string) != Some(model.id.as_bytes()) {
            continue;
        }
        if marker.get_integer("Version") != Some(1) {
            return Err(fail("authored typed-row relocation version is unsupported"));
        }
        let target_row = marker
            .get_integer("TargetRow")
            .and_then(|value| usize::try_from(value).ok())
            .filter(|row| *row < pagination.body_rows)
            .ok_or_else(|| fail("authored typed-row relocation target row is malformed"))?;
        let destination_id = match marker.get("Destination") {
            Some(PdfObject::Reference { number, generation }) => (*number, *generation),
            _ => {
                return Err(fail(
                    "authored typed-row relocation destination is not an indirect page reference",
                ));
            }
        };
        let destination_page = page_numbers
            .get(&destination_id)
            .copied()
            .ok_or_else(|| fail("authored typed-row relocation destination page disappeared"))?;
        if destination_page <= page.page_number || !destinations.insert(destination_page) {
            return Err(fail(
                "authored typed-row relocation page order or destination uniqueness changed",
            ));
        }
        let static_digest = marker
            .get("StaticDigest")
            .and_then(PdfObject::as_string)
            .filter(|digest| digest.len() == 64 && digest.iter().all(u8::is_ascii_hexdigit))
            .map(|digest| String::from_utf8_lossy(digest).into_owned())
            .ok_or_else(|| fail("authored typed-row relocation digest is malformed"))?;
        let cell_objects = marker
            .get("Cells")
            .and_then(PdfObject::as_array)
            .filter(|cells| !cells.is_empty() && cells.len() <= 4096)
            .ok_or_else(|| fail("authored typed-row relocation cell receipt is malformed"))?;
        let mut cells = Vec::with_capacity(cell_objects.len());
        let mut identities = BTreeSet::new();
        for object in cell_objects {
            let cell = object
                .as_dict()
                .ok_or_else(|| fail("authored typed-row relocation cell is not a dictionary"))?;
            let id = cell
                .get("ID")
                .and_then(PdfObject::as_string)
                .map(crate::info::decode_pdf_text_string)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| fail("authored typed-row relocation cell identity is malformed"))?;
            let number = |key: &str, upper: usize| {
                cell.get_integer(key)
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value <= upper)
                    .ok_or_else(|| fail("authored typed-row relocation cell topology is malformed"))
            };
            let row = number("Row", 1_000_000)?;
            let column = number("Column", 10_000)?;
            let row_span = number("RowSpan", 1_000_000)?;
            let column_span = number("ColSpan", 10_000)?;
            if row_span == 0
                || column_span == 0
                || column >= pagination.column_widths.len()
                || column
                    .checked_add(column_span)
                    .is_none_or(|end| end > pagination.column_widths.len())
            {
                return Err(fail(
                    "authored typed-row relocation cell span escapes retained columns",
                ));
            }
            let role = match cell.get_name("Role") {
                Some("TH") => AuthoredTypedCellRole::Header,
                Some("TD") => AuthoredTypedCellRole::Data,
                _ => return Err(fail("authored typed-row relocation cell role is malformed")),
            };
            let font_resource = cell
                .get_name("Font")
                .filter(|name| !name.is_empty() && name.len() <= 255)
                .map(str::to_owned)
                .ok_or_else(|| fail("authored typed-row relocation font resource is malformed"))?;
            let rect = relocation_rect(cell, "Rect")?;
            let region = relocation_rect(cell, "Region")?;
            let retained = model
                .cells
                .iter()
                .find(|candidate| candidate.id == id)
                .ok_or_else(|| fail("authored typed-row relocation cell model disappeared"))?;
            let paint = retained
                .layout
                .as_ref()
                .and_then(|layout| layout.paint.as_ref())
                .ok_or_else(|| fail("authored typed-row relocation paint metadata disappeared"))?;
            let expected_left =
                pagination.margins[0] + pagination.column_widths[..column].iter().sum::<f64>();
            let expected_right = expected_left
                + pagination.column_widths[column..column + column_span]
                    .iter()
                    .sum::<f64>();
            if !identities.insert(id.clone())
                || row <= target_row
                || retained.row != row
                || retained.column != column
                || retained.row_span != row_span
                || retained.column_span != column_span
                || retained.structure_role != role
                || region[0] < rect[0] - EPS
                || region[1] < rect[1] - EPS
                || region[2] > rect[2] + EPS
                || region[3] > rect[3] + EPS
                || rect[0] < -EPS
                || rect[1] < pagination.margins[3] - EPS
                || rect[2] > pagination.page_size[0] + EPS
                || rect[3] > pagination.page_size[1] - pagination.margins[2] + EPS
                || (rect[0] - expected_left).abs() > EPS
                || (rect[2] - expected_right).abs() > EPS
                || (region[0] - (rect[0] + paint.padding)).abs() > EPS
                || (region[1] - (rect[1] + paint.padding)).abs() > EPS
                || (region[2] - (rect[2] - paint.padding)).abs() > EPS
                || (region[3] - (rect[3] - paint.padding)).abs() > EPS
            {
                return Err(fail(
                    "authored typed-row relocation receipt drifted from retained topology",
                ));
            }
            cells.push(AuthoredTypedRelocationCell {
                id,
                row,
                column,
                row_span,
                column_span,
                role,
                rect,
                region,
                font_resource,
            });
        }
        cells.sort_by_key(|cell| (cell.row, cell.column, cell.column_span, cell.id.clone()));
        let mut row_boxes = BTreeMap::<usize, [f64; 2]>::new();
        for cell in &cells {
            if let Some(existing) = row_boxes.insert(cell.row, [cell.rect[1], cell.rect[3]]) {
                if (existing[0] - cell.rect[1]).abs() > EPS
                    || (existing[1] - cell.rect[3]).abs() > EPS
                {
                    return Err(fail(
                        "authored typed-row relocation row cells disagree on vertical geometry",
                    ));
                }
            }
        }
        let ordered_boxes = row_boxes.into_values().collect::<Vec<_>>();
        if ordered_boxes
            .windows(2)
            .any(|rows| (rows[0][0] - rows[1][1]).abs() > EPS)
        {
            return Err(fail(&format!(
                "authored typed-row relocation receipt no longer describes contiguous rows: {ordered_boxes:?}",
            )));
        }
        receipts.push(AuthoredTypedRelocationReceipt {
            origin_page: page.page_number,
            destination_page,
            target_row,
            static_digest,
            cells,
        });
    }
    receipts.sort_by_key(|receipt| (receipt.origin_page, receipt.destination_page));
    Ok(receipts)
}

fn stamp_authored_typed_table_relocation(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    origin_page: usize,
    destination_page: usize,
    target_row: usize,
    cells: &[AuthoredTypedRelocationCell],
) -> Result<Vec<u8>> {
    if cells.is_empty()
        || cells.len() > 4096
        || origin_page == 0
        || destination_page <= origin_page
        || target_row > 1_000_000
    {
        return Err(fail("invalid authored typed-row relocation receipt"));
    }
    let cell_ids = cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    if cell_ids.len() != cells.len() {
        return Err(fail(
            "authored typed-row relocation repeats a cell identity",
        ));
    }
    let destination_set = BTreeSet::from([destination_page]);
    let inspections =
        crate::tagged_structure::story::inspect_authored_typed_cell_continuation_pages(
            input,
            &model.id,
            &cell_ids,
            &destination_set,
        )?;
    let static_digest = inspections
        .get(&destination_page)
        .map(|inspection| inspection.static_digest.clone())
        .ok_or_else(|| fail("authored typed-row relocation destination inspection is missing"))?;
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let origin = engine.document().get_page(origin_page)?;
    let destination = engine.document().get_page(destination_page)?;
    let reader = engine.document().reader();
    let mut dictionary = reader
        .get_object(origin.object_number, origin.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("authored typed-row relocation origin is not a dictionary"))?;
    if dictionary.contains_key(TYPED_TABLE_RELOCATION_MARKER) {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-row origin already owns an active relocation receipt".into(),
        ));
    }
    let mut marker = PdfDictionary::empty();
    marker.insert("Version", PdfObject::Integer(1));
    marker.insert("Table", PdfObject::String(model.id.as_bytes().to_vec()));
    marker.insert(
        "TargetRow",
        PdfObject::Integer(
            i64::try_from(target_row)
                .map_err(|_| fail("authored typed-row relocation target row overflow"))?,
        ),
    );
    marker.insert(
        "Destination",
        PdfObject::Reference {
            number: destination.object_number,
            generation: destination.generation_number,
        },
    );
    marker.insert(
        "StaticDigest",
        PdfObject::String(static_digest.as_bytes().to_vec()),
    );
    let mut cell_objects = Vec::with_capacity(cells.len());
    for cell in cells {
        if !cell
            .rect
            .iter()
            .chain(cell.region.iter())
            .all(|value| value.is_finite())
            || cell.font_resource.is_empty()
            || cell.font_resource.len() > 255
        {
            return Err(fail("invalid authored typed-row relocation cell receipt"));
        }
        let mut item = PdfDictionary::empty();
        item.insert("ID", PdfObject::String(pdf_text_string(&cell.id)));
        item.insert("Row", PdfObject::Integer(cell.row as i64));
        item.insert("Column", PdfObject::Integer(cell.column as i64));
        item.insert("RowSpan", PdfObject::Integer(cell.row_span as i64));
        item.insert("ColSpan", PdfObject::Integer(cell.column_span as i64));
        item.insert(
            "Role",
            PdfObject::Name(
                match cell.role {
                    AuthoredTypedCellRole::Header => "TH",
                    AuthoredTypedCellRole::Data => "TD",
                }
                .into(),
            ),
        );
        item.insert("Rect", relocation_rect_object(cell.rect));
        item.insert("Region", relocation_rect_object(cell.region));
        item.insert("Font", PdfObject::Name(cell.font_resource.clone()));
        cell_objects.push(PdfObject::Dictionary(item));
    }
    marker.insert("Cells", PdfObject::Array(cell_objects));
    dictionary.insert(TYPED_TABLE_RELOCATION_MARKER, PdfObject::Dictionary(marker));
    let output = crate::writer::write_incremental_update(
        reader,
        vec![crate::writer::IncrementalObject {
            number: origin.object_number,
            generation: origin.generation_number,
            object: PdfObject::Dictionary(dictionary),
        }],
    )?;
    let receipts = inspect_authored_typed_table_relocations(&output, model)?;
    let matching = receipts
        .iter()
        .filter(|receipt| {
            receipt.origin_page == origin_page
                && receipt.destination_page == destination_page
                && receipt.target_row == target_row
        })
        .collect::<Vec<_>>();
    if matching.len() != 1
        || matching[0].static_digest != static_digest
        || matching[0]
            .cells
            .iter()
            .map(|cell| cell.id.as_str())
            .collect::<BTreeSet<_>>()
            != cell_ids.iter().map(String::as_str).collect::<BTreeSet<_>>()
    {
        return Err(fail(
            "authored typed-row relocation receipt failed reopen validation",
        ));
    }
    Ok(output)
}

fn authored_structural_tail_only(data: &[u8]) -> Result<bool> {
    let mut structural = true;
    crate::image_fragments::operations(data, |_, _, operation, inline| {
        if inline || !matches!(operation.operator.as_str(), "Q" | "EMC") {
            structural = false;
        }
        Ok(())
    })?;
    Ok(structural)
}

fn remove_relocated_authored_rows_from_page(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    page_number: usize,
    rows: &BTreeSet<usize>,
) -> Result<Vec<u8>> {
    if rows.is_empty() || rows.len() > 4096 || page_number == 0 {
        return Err(fail("invalid authored typed-row relocation removal"));
    }
    let cell_ids = model
        .cells
        .iter()
        .filter(|cell| rows.contains(&cell.row))
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    if cell_ids.is_empty() || cell_ids.len() > 4096 {
        return Err(fail(
            "authored typed-row relocation has no bounded typed-cell owners",
        ));
    }
    let expected = cell_ids
        .iter()
        .map(|cell| (page_number, model.id.clone(), cell.clone()))
        .collect::<Vec<_>>();
    let detached =
        crate::tagged_structure::story::detach_authored_typed_cell_fragments(input, &expected)?;
    let pages = BTreeSet::from([page_number]);
    let text_scopes = crate::tagged_structure::story::inspect_authored_typed_cell_source_scopes(
        &detached, &model.id, &cell_ids, &pages,
    )?;
    if text_scopes
        .iter()
        .map(|scope| scope.cell_id.clone())
        .collect::<BTreeSet<_>>()
        != cell_ids
    {
        return Err(fail(
            "authored typed-row relocation text ownership is incomplete",
        ));
    }
    let grid = inspect_authored_typed_table_grid_paint(&detached, &model.id)?;
    let mut ranges = BTreeMap::<(u32, u16), Vec<Range<usize>>>::new();
    for scope in text_scopes {
        ranges
            .entry(scope.stream)
            .or_default()
            .push(scope.decoded_range[0]..scope.decoded_range[1]);
    }
    let expected_grid = model
        .cells
        .iter()
        .filter(|cell| rows.contains(&cell.row))
        .map(|cell| (cell.row, cell.column, cell.row_span, cell.column_span))
        .collect::<BTreeSet<_>>();
    let mut removed_grid = BTreeSet::new();
    for fragment in grid
        .fragments
        .iter()
        .filter(|fragment| fragment.page == page_number && rows.contains(&fragment.row))
    {
        let generation = u16::try_from(fragment.stream_object[1])
            .map_err(|_| fail("authored grid stream generation overflow"))?;
        ranges
            .entry((fragment.stream_object[0], generation))
            .or_default()
            .push(fragment.decoded_range[0]..fragment.decoded_range[1]);
        removed_grid.insert((
            fragment.row,
            fragment.column,
            fragment.row_span,
            fragment.column_span,
        ));
    }
    if removed_grid != expected_grid {
        return Err(fail(
            "authored typed-row relocation grid ownership is incomplete",
        ));
    }

    let engine = crate::ContentEngine::open_bytes(detached)?;
    let reader = engine.document().reader();
    let page = engine.document().get_page(page_number)?;
    let page_streams = page.contents.iter().copied().collect::<BTreeSet<_>>();
    let stream_indexes = page
        .contents
        .iter()
        .copied()
        .enumerate()
        .map(|(index, stream)| (stream, index))
        .collect::<BTreeMap<_, _>>();
    let last_owned_stream = ranges
        .keys()
        .map(|stream| {
            stream_indexes
                .get(stream)
                .copied()
                .ok_or_else(|| fail("authored typed-row relocation stream left its page"))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .ok_or_else(|| fail("authored typed-row relocation has no owned stream"))?;
    for &stream in &page.contents[last_owned_stream + 1..] {
        let object = reader.get_object(stream.0, stream.1)?;
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &object,
            reader,
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 64 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        if decoded.status != crate::filters::StreamDecodeStatus::Complete
            || !authored_structural_tail_only(&decoded.data)?
        {
            return Err(WellfriendError::UnsupportedFeature(
                "authored typed-row relocation cannot move ahead of later page content".into(),
            ));
        }
    }
    let mut occurrences = BTreeMap::<(u32, u16), Vec<usize>>::new();
    for source_page in engine.document().get_pages()? {
        for &stream in &source_page.contents {
            occurrences
                .entry(stream)
                .or_default()
                .push(source_page.page_number);
        }
    }
    let mut updates = Vec::with_capacity(ranges.len());
    for (stream, mut stream_ranges) in ranges {
        crate::cancel::check_current_cancel("authored typed-row source relocation")?;
        if !page_streams.contains(&stream)
            || occurrences
                .get(&stream)
                .is_none_or(|pages| pages.len() != 1 || pages.first().copied() != Some(page_number))
        {
            return Err(WellfriendError::UnsupportedFeature(
                "authored typed-row relocation requires page-private content streams".into(),
            ));
        }
        stream_ranges.sort_by_key(|range| (range.start, range.end));
        if stream_ranges
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start)
        {
            return Err(fail("authored typed-row relocation source ranges overlap"));
        }
        let object = reader.get_object(stream.0, stream.1)?;
        let (mut dictionary, raw) = match object {
            PdfObject::Stream { dict, raw } => (dict, raw),
            _ => return Err(fail("authored typed-row source is not a stream")),
        };
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &PdfObject::Stream {
                dict: dictionary.clone(),
                raw,
            },
            reader,
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 64 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        if decoded.status != crate::filters::StreamDecodeStatus::Complete
            || stream_ranges
                .iter()
                .any(|range| range.start >= range.end || range.end > decoded.data.len())
        {
            return Err(fail(
                "authored typed-row relocation stream is opaque or its provenance escaped",
            ));
        }
        if stream_indexes.get(&stream).copied() == Some(last_owned_stream) {
            let last_end = stream_ranges
                .iter()
                .map(|range| range.end)
                .max()
                .ok_or_else(|| fail("authored typed-row relocation range set is empty"))?;
            if !authored_structural_tail_only(&decoded.data[last_end..])? {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored typed-row relocation cannot move ahead of later page content".into(),
                ));
            }
        }
        let mut data = decoded.data;
        for range in stream_ranges.into_iter().rev() {
            data.splice(range, [b'\n']);
        }
        let compressed = crate::filters::flate_encode_cancellable(&data, 6)?;
        dictionary.insert("Filter", PdfObject::Name("FlateDecode".into()));
        dictionary.remove("DecodeParms");
        dictionary.insert("Length", PdfObject::Integer(compressed.len() as i64));
        updates.push(crate::writer::IncrementalObject {
            number: stream.0,
            generation: stream.1,
            object: PdfObject::Stream {
                dict: dictionary,
                raw: compressed,
            },
        });
    }
    if updates.is_empty() {
        return Err(fail(
            "authored typed-row relocation produced no source updates",
        ));
    }
    let output = crate::writer::write_incremental_update(reader, updates)?;
    let output = crate::tagged_structure::story::finalize_authored_typed_cell_transaction(&output)?;
    for cell in model.cells.iter().filter(|cell| rows.contains(&cell.row)) {
        if locate_current_authored_typed_owner(&output, &model.id, &cell.id)?
            .iter()
            .any(|fragment| fragment.page == page_number)
        {
            return Err(fail(
                "authored typed-row relocation left an original text owner reachable",
            ));
        }
    }
    let rebound_grid = inspect_authored_typed_table_grid_paint(&output, &model.id)?;
    if !rebound_grid.complete_typed_grid_ownership
        || rebound_grid
            .fragments
            .iter()
            .any(|fragment| fragment.page == page_number && rows.contains(&fragment.row))
    {
        return Err(fail(
            "authored typed-row relocation failed its grid-source postcondition",
        ));
    }
    crate::tagged_structure::validate_parent_tree(&output)?;
    Ok(output)
}

fn prepare_authored_row_continuation_cells(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    row: usize,
    grid: &AuthoredTypedTableGridPaintReport,
    source_engine: &crate::ContentEngine,
) -> Result<(Vec<AuthoredContinuationCell>, usize)> {
    let geometry = authored_row_continuation_geometry(model, row)?;
    let reader = source_engine.document().reader();
    let mut cells = Vec::with_capacity(geometry.len());
    let mut last_owned_page = 0usize;
    for (cell, layout, rect, region) in geometry {
        let owned = locate_current_authored_typed_owner(input, &model.id, &cell.id)?;
        let owned_pages = owned
            .iter()
            .map(|fragment| fragment.page)
            .collect::<BTreeSet<_>>();
        let grid_pages = grid
            .fragments
            .iter()
            .filter(|fragment| {
                fragment.row == cell.row
                    && fragment.column == cell.column
                    && fragment.row_span == cell.row_span
                    && fragment.column_span == cell.column_span
            })
            .map(|fragment| fragment.page)
            .collect::<BTreeSet<_>>();
        if owned_pages.is_empty() || owned_pages != grid_pages {
            return Err(fail(
                "authored typed-row text and grid ownership disagree across pages",
            ));
        }
        last_owned_page = last_owned_page.max(
            owned
                .iter()
                .map(|fragment| fragment.page)
                .max()
                .ok_or_else(|| fail("authored typed-row cell has no source owner"))?,
        );
        if owned.iter().any(|fragment| {
            fragment.region.is_none_or(|source| {
                (source[0] - region[0]).abs() > EPS || (source[2] - region[2]).abs() > EPS
            })
        }) {
            return Err(WellfriendError::UnsupportedFeature(
                "authored typed-row source geometry no longer matches its retained columns".into(),
            ));
        }
        let first = &owned[0];
        let page = source_engine.document().get_page(first.page)?;
        let resources = crate::PageResources::from_dict(&page.resources, reader);
        let font_object = resources
            .fonts
            .get(&first.font_resource)
            .ok_or_else(|| fail("authored typed-row continuation font disappeared"))?;
        let resolver = crate::fonts::FontResolver::new(font_object, reader);
        let font = if resolver.has_standard14_metrics() {
            let name = font_object
                .get_name("BaseFont")
                .and_then(standard_font_from_pdf_name)
                .ok_or_else(|| {
                    fail("authored typed-row Standard-14 font has no supported BaseFont")
                })?;
            AuthoredContinuationFont::Standard(name)
        } else {
            let bytes =
                crate::fonts::provider::embedded_program(reader, font_object).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "authored typed-row growth requires every custom source font to be embedded"
                            .into(),
                    )
                })?;
            AuthoredContinuationFont::Embedded(bytes)
        };
        cells.push(AuthoredContinuationCell {
            model: cell,
            layout,
            rect,
            region,
            font,
            source_font_resource: first.font_resource.clone(),
        });
    }
    Ok((cells, last_owned_page))
}

fn allocate_authored_typed_row_fragments(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    cell: &AuthoredTypedCellModel,
    fragments: &[AuthoredTypedCellSourceFragment],
    replacement: &str,
    layout: &AuthoredTypedCellLayout,
    initial: &AuthoredRedistributionPlan,
    approved_font: Option<&[u8]>,
) -> Result<(Vec<u8>, Vec<usize>, Vec<String>)> {
    const MAX_GROWTH_PAGES: usize = 10_000;
    if initial.consumed_bytes == replacement.len() {
        return Ok((input.to_vec(), Vec::new(), Vec::new()));
    }
    let pagination = model.pagination.as_ref().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "authored typed-table growth requires retained pagination metadata".into(),
        )
    })?;
    let retained_row_breaks = if cell.row + 1 < pagination.body_rows {
        Some(pagination.row_page_breaks.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "authored multi-row table predates retained row-break provenance".into(),
            )
        })?)
    } else {
        None
    };
    let geometry = authored_row_continuation_geometry(model, cell.row)?;
    let grid = inspect_authored_typed_table_grid_paint(input, &model.id)?;
    if !grid.complete_typed_grid_ownership {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-row growth requires complete source-owned grid paint".into(),
        ));
    }
    let target_last_grid_page = grid
        .fragments
        .iter()
        .filter(|fragment| fragment.row == cell.row)
        .map(|fragment| fragment.page)
        .max()
        .ok_or_else(|| fail("authored typed-row grid ownership is missing"))?;
    let mut relocated_rows = BTreeSet::new();
    let mut later_page_seen = false;
    for row in cell.row + 1..pagination.body_rows {
        let pages = grid
            .fragments
            .iter()
            .filter(|fragment| fragment.row == row)
            .map(|fragment| fragment.page)
            .collect::<BTreeSet<_>>();
        if pages.is_empty() {
            return Err(fail(
                "authored typed-table downstream grid ownership is missing",
            ));
        }
        if pages.contains(&target_last_grid_page) {
            if later_page_seen || pages != BTreeSet::from([target_last_grid_page]) {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored typed-row relocation requires each displaced downstream row to be wholly owned by the target row's final page"
                        .into(),
                ));
            }
            relocated_rows.insert(row);
        } else {
            if pages.iter().any(|page| *page < target_last_grid_page) {
                return Err(fail(
                    "authored typed-table downstream row order is not monotone",
                ));
            }
            later_page_seen = true;
        }
    }
    if !relocated_rows.is_empty()
        && !parent_relocation_digest_is_current(input, model, target_last_grid_page)?
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored nested row relocation destination changed before the child transaction"
                .into(),
        ));
    }
    let target_region = geometry
        .iter()
        .find(|(candidate, _, _, _)| candidate.id == cell.id)
        .map(|(_, retained_layout, _, region)| {
            if retained_layout != layout {
                return Err(fail(
                    "authored typed-cell retained layout changed during row growth",
                ));
            }
            Ok(*region)
        })
        .transpose()?
        .ok_or_else(|| fail("authored typed-cell is absent from retained row geometry"))?;
    let source_engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let _reader = source_engine.document().reader();
    let mut retained_header_cells = Vec::new();
    if let Some(header) = &pagination.repeatable_header {
        let mut resolved_fonts = BTreeMap::<String, AuthoredContinuationFont>::new();
        retained_header_cells.reserve(header.cells.len());
        for cell in &header.cells {
            let key = match &cell.font {
                AuthoredRetainedFont::Standard { base_name } => format!("standard:{base_name}"),
                AuthoredRetainedFont::Embedded { base_name } => format!("embedded:{base_name}"),
            };
            let font = if let Some(font) = resolved_fonts.get(&key) {
                font.clone()
            } else {
                let font = resolve_retained_header_font(source_engine.document(), &cell.font)?;
                resolved_fonts.insert(key, font.clone());
                font
            };
            retained_header_cells.push(AuthoredContinuationHeaderCell {
                model: cell.clone(),
                font,
            });
        }
    }
    let (row_cells, last_owned_page) =
        prepare_authored_row_continuation_cells(input, model, cell.row, &grid, &source_engine)?;
    if last_owned_page != target_last_grid_page {
        return Err(fail(
            "authored typed-row final text and grid ownership pages disagree",
        ));
    }
    let mut relocated_row_cells = Vec::with_capacity(relocated_rows.len());
    for &row in &relocated_rows {
        let (cells, final_page) =
            prepare_authored_row_continuation_cells(input, model, row, &grid, &source_engine)?;
        if final_page != target_last_grid_page {
            return Err(fail(
                "authored typed-row relocation source escaped the target page",
            ));
        }
        relocated_row_cells.push((row, cells));
    }
    // The relocation receipt describes the exact source geometry that was
    // removed from the origin page. `prepare_authored_row_continuation_cells`
    // deliberately gives every cell the full continuation-page geometry, so
    // recording those placeholder rectangles here would collapse all source
    // rows onto the same box and make provenance-bound backward compaction
    // impossible. Bind every receipt cell to its unique painted source grid
    // fragment instead and derive the text region from the retained padding.
    let mut relocation_receipt_cells = Vec::new();
    for (_, cells) in &relocated_row_cells {
        for cell in cells {
            let mut matches = grid.fragments.iter().filter(|fragment| {
                fragment.page == target_last_grid_page
                    && fragment.row == cell.model.row
                    && fragment.column == cell.model.column
                    && fragment.row_span == cell.model.row_span
                    && fragment.column_span == cell.model.column_span
            });
            let rect = matches
                .next()
                .map(|fragment| fragment.rect)
                .ok_or_else(|| fail("authored typed-row relocation source grid is missing"))?;
            if matches.next().is_some() {
                return Err(fail(
                    "authored typed-row relocation source grid is ambiguous",
                ));
            }
            let paint =
                cell.layout.paint.as_ref().ok_or_else(|| {
                    fail("authored typed-row relocation retained paint is missing")
                })?;
            let region = [
                rect[0] + paint.padding,
                rect[1] + paint.padding,
                rect[2] - paint.padding,
                rect[3] - paint.padding,
            ];
            if region[0] >= region[2] || region[1] >= region[3] {
                return Err(fail(
                    "authored typed-row relocation source content region collapsed",
                ));
            }
            relocation_receipt_cells.push(AuthoredTypedRelocationCell {
                id: cell.model.id.clone(),
                row: cell.model.row,
                column: cell.model.column,
                row_span: cell.model.row_span,
                column_span: cell.model.column_span,
                role: cell.model.structure_role,
                rect,
                region,
                font_resource: cell.source_font_resource.clone(),
            });
        }
    }
    let header_height = pagination
        .repeatable_header
        .as_ref()
        .map_or(0.0, |header| header.height);
    let mut relocation_top = pagination.page_size[1] - pagination.margins[2] - header_height;
    for (row, cells) in &mut relocated_row_cells {
        let original_rects = grid
            .fragments
            .iter()
            .filter(|fragment| fragment.page == target_last_grid_page && fragment.row == *row)
            .map(|fragment| fragment.rect)
            .collect::<Vec<_>>();
        let first = original_rects
            .first()
            .copied()
            .ok_or_else(|| fail("authored typed-row relocation geometry is missing"))?;
        let row_height = first[3] - first[1];
        if !row_height.is_finite()
            || row_height <= 0.0
            || original_rects
                .iter()
                .any(|rect| (rect[1] - first[1]).abs() > EPS || (rect[3] - first[3]).abs() > EPS)
        {
            return Err(fail(
                "authored typed-row relocation requires one consistent retained row height",
            ));
        }
        let bottom = relocation_top - row_height;
        if bottom < pagination.margins[3] - EPS {
            return Err(fail(
                "authored typed-row relocation rows exceed one retained continuation page",
            ));
        }
        for cell in cells {
            let paint =
                cell.layout.paint.as_ref().ok_or_else(|| {
                    fail("authored typed-row relocation retained paint is missing")
                })?;
            cell.rect[1] = bottom;
            cell.rect[3] = relocation_top;
            cell.region[1] = bottom + paint.padding;
            cell.region[3] = relocation_top - paint.padding;
            if cell.region[1] >= cell.region[3] {
                return Err(fail(
                    "authored typed-row relocation content region collapsed",
                ));
            }
        }
        relocation_top = bottom;
    }
    let relocated_page_cells = relocated_row_cells
        .iter()
        .flat_map(|(_, cells)| cells.iter().cloned())
        .collect::<Vec<_>>();
    let template_page_number = std::iter::once(last_owned_page)
        .chain(fragments.iter().map(|fragment| fragment.page))
        .max()
        .ok_or_else(|| fail("authored typed-cell continuation template is missing"))?;
    let template_page = source_engine.document().get_page(template_page_number)?;
    let expected_box = [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]];
    let same_box = |actual: [f64; 4]| {
        actual
            .iter()
            .zip(expected_box)
            .all(|(actual, expected)| (*actual - expected).abs() <= EPS)
    };
    if !same_box(template_page.media_box)
        || !same_box(template_page.crop_box)
        || template_page.rotate != 0
        || (template_page.user_unit - 1.0).abs() > EPS
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-table continuation geometry no longer matches its retained simple page template"
                .into(),
        ));
    }
    let insert_at = last_owned_page
        .checked_add(1)
        .ok_or_else(|| fail("authored typed-cell continuation insertion point overflow"))?;
    let additional = (0..MAX_GROWTH_PAGES)
        .map(|offset| {
            insert_at
                .checked_add(offset)
                .map(|page| (page, target_region))
                .ok_or_else(|| fail("authored typed-cell continuation page overflow"))
        })
        .collect::<Result<Vec<_>>>()?;
    let expanded = plan_authored_text_across_existing_fragments(
        input,
        fragments,
        &additional,
        replacement,
        layout,
        approved_font,
    )?;
    if expanded.consumed_bytes != replacement.len() {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-cell replacement requires more than 10000 continuation pages".into(),
        ));
    }
    let existing = fragments.len();
    let last_used = expanded
        .fragments
        .iter()
        .enumerate()
        .skip(existing)
        .filter(|(_, fragment)| !fragment.lines.is_empty())
        .map(|(index, _)| index)
        .max()
        .ok_or_else(|| fail("authored typed-cell growth plan made no progress"))?;
    let page_count = last_used - existing + 1;
    let total_inserted_pages = page_count
        .checked_add(usize::from(!relocated_page_cells.is_empty()))
        .ok_or_else(|| fail("authored typed-row relocation page count overflow"))?;
    if let Some(breaks) = retained_row_breaks {
        for (row, _) in &relocated_row_cells {
            let destination = insert_at
                .checked_add(page_count)
                .ok_or_else(|| fail("authored typed-row relocation destination overflow"))?;
            let accepted = match breaks[*row] {
                None | Some(FlowPageBreak::NextPage) => true,
                Some(FlowPageBreak::NextOddPage) => destination % 2 == 1,
                Some(FlowPageBreak::NextEvenPage) => destination % 2 == 0,
            };
            if !accepted {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored typed-row relocation would violate its retained odd/even page break"
                        .into(),
                ));
            }
        }
        let later_parity_sensitive =
            breaks
                .iter()
                .enumerate()
                .skip(cell.row + 1)
                .any(|(row, policy)| {
                    !relocated_rows.contains(&row)
                        && matches!(
                            policy,
                            Some(FlowPageBreak::NextOddPage | FlowPageBreak::NextEvenPage)
                        )
                });
        if later_parity_sensitive && total_inserted_pages % 2 != 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "authored typed-row growth requires an even inserted-page count to preserve a downstream odd/even row break"
                    .into(),
            ));
        }
    }
    if total_inserted_pages > MAX_GROWTH_PAGES {
        return Err(WellfriendError::UnsupportedFeature(
            "authored typed-row growth and relocation exceed the bounded page transaction".into(),
        ));
    }
    let relocation_carriers = relocated_row_cells
        .iter()
        .try_fold(0usize, |count, (_, cells)| count.checked_add(cells.len()))
        .ok_or_else(|| {
            WellfriendError::ResourceLimit("authored typed-row relocation carrier count".into())
        })?;
    if page_count
        .checked_mul(row_cells.len())
        .and_then(|count| count.checked_add(relocation_carriers))
        .is_none_or(|count| count > 1_000_000)
    {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-row continuation carrier count".into(),
        ));
    }
    crate::cancel::check_current_cancel("authored typed-row page allocation")?;
    let continuation =
        crate::ContentEngine::open_bytes(build_authored_typed_row_continuation_page(
            &model.id,
            pagination,
            &row_cells,
            &retained_header_cells,
        )?)?;
    let relocated_continuation = if relocated_page_cells.is_empty() {
        None
    } else {
        Some(crate::ContentEngine::open_bytes(
            build_authored_typed_row_continuation_page(
                &model.id,
                pagination,
                &relocated_page_cells,
                &retained_header_cells,
            )?,
        )?)
    };
    let geometry = Some(crate::writer::AuthoredPageGeometry {
        media_box: [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]],
        crop_box: [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]],
        bleed_box: [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]],
        trim_box: [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]],
        art_box: [0.0, 0.0, pagination.page_size[0], pagination.page_size[1]],
        rotate: 0,
        user_unit: 1.0,
    });
    let mut pages = Vec::with_capacity(total_inserted_pages);
    pages.extend((0..page_count).map(|_| (continuation.document(), geometry)));
    if let Some(relocated) = &relocated_continuation {
        pages.push((relocated.document(), geometry));
    }
    let inserted = crate::writer::insert_authored_pages_preserving_catalog(
        source_engine.document(),
        &pages,
        insert_at,
    )?;
    let page_numbers = (0..page_count)
        .map(|offset| insert_at + offset)
        .collect::<Vec<_>>();
    let mut expected = page_numbers
        .iter()
        .flat_map(|page| {
            row_cells
                .iter()
                .map(move |cell| (*page, model.id.clone(), cell.model.id.clone()))
        })
        .collect::<Vec<_>>();
    if !relocated_page_cells.is_empty() {
        let page = insert_at + page_count;
        expected.extend(
            relocated_page_cells
                .iter()
                .map(|cell| (page, model.id.clone(), cell.model.id.clone())),
        );
    }
    let output =
        crate::tagged_structure::story::attach_authored_typed_cell_fragments(&inserted, &expected)?;
    let cell_ids = row_cells
        .iter()
        .map(|cell| cell.model.id.clone())
        .collect::<BTreeSet<_>>();
    let output = stamp_authored_typed_table_continuations(
        &output,
        &model.id,
        cell.row,
        &cell_ids,
        &page_numbers,
    )?;
    let output = if relocation_receipt_cells.is_empty() {
        output
    } else {
        stamp_authored_typed_table_relocation(
            &output,
            model,
            target_last_grid_page,
            insert_at + page_count,
            cell.row,
            &relocation_receipt_cells,
        )?
    };
    let output = if relocated_rows.is_empty() {
        output
    } else {
        let output = remove_relocated_authored_rows_from_page(
            &output,
            model,
            target_last_grid_page,
            &relocated_rows,
        )?;
        refresh_parent_relocation_digest(&output, model, target_last_grid_page)?
    };
    let relocated_cells = relocated_row_cells
        .iter()
        .flat_map(|(_, cells)| cells.iter().map(|cell| cell.model.id.clone()))
        .collect();
    let inserted_pages = (0..total_inserted_pages)
        .map(|offset| insert_at + offset)
        .collect();
    Ok((output, inserted_pages, relocated_cells))
}

#[derive(Debug)]
struct AuthoredTypedTablePruningResult {
    output: Vec<u8>,
    removed_pages: Vec<usize>,
    retained_pages: Vec<AuthoredTypedTableRetainedContinuation>,
}

fn authored_typed_continuation_has_page_features(dictionary: &PdfDictionary) -> bool {
    dictionary.iter().any(|(key, _)| {
        key != TYPED_TABLE_CONTINUATION_MARKER
            && !matches!(
                key.as_str(),
                "Type"
                    | "Parent"
                    | "MediaBox"
                    | "CropBox"
                    | "BleedBox"
                    | "TrimBox"
                    | "ArtBox"
                    | "Rotate"
                    | "UserUnit"
                    | "Resources"
                    | "Contents"
                    | "Annots"
                    | "StructParents"
                    | "Tabs"
            )
    })
}

fn encoded_pdf_name(name: &str) -> String {
    let mut output = String::new();
    for byte in name.as_bytes() {
        if (0x21..=0x7e).contains(byte)
            && !matches!(
                *byte,
                b'#' | b'%' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/'
            )
        {
            output.push(char::from(*byte));
        } else {
            output.push_str(&format!("#{byte:02X}"));
        }
    }
    output
}

fn authored_relocation_origin_tail_safe(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    receipt: &AuthoredTypedRelocationReceipt,
) -> Result<bool> {
    let target_cells = model
        .cells
        .iter()
        .filter(|cell| cell.row == receipt.target_row)
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    if target_cells.is_empty() {
        return Err(fail("authored relocation target row has no retained cells"));
    }
    let page_numbers = BTreeSet::from([receipt.origin_page]);
    let text = crate::tagged_structure::story::inspect_authored_typed_cell_source_scopes(
        input,
        &model.id,
        &target_cells,
        &page_numbers,
    )?;
    let grid = inspect_authored_typed_table_grid_paint(input, &model.id)?;
    let target_rects = grid
        .fragments
        .iter()
        .filter(|fragment| {
            fragment.page == receipt.origin_page && fragment.row == receipt.target_row
        })
        .map(|fragment| fragment.rect)
        .collect::<Vec<_>>();
    let target_bottom = target_rects
        .first()
        .map(|rect| rect[1])
        .ok_or_else(|| fail("authored relocation target grid disappeared"))?;
    let first_relocated_row = receipt
        .cells
        .iter()
        .map(|cell| cell.row)
        .min()
        .ok_or_else(|| fail("authored relocation receipt has no rows"))?;
    let first_relocated_top = receipt
        .cells
        .iter()
        .find(|cell| cell.row == first_relocated_row)
        .map(|cell| cell.rect[3])
        .ok_or_else(|| fail("authored relocation first row disappeared"))?;
    if target_rects
        .iter()
        .any(|rect| (rect[1] - target_bottom).abs() > EPS)
        || (target_bottom - first_relocated_top).abs() > EPS
    {
        return Ok(false);
    }
    let mut ranges = BTreeMap::<(u32, u16), Vec<Range<usize>>>::new();
    for scope in text {
        ranges
            .entry(scope.stream)
            .or_default()
            .push(scope.decoded_range[0]..scope.decoded_range[1]);
    }
    for fragment in grid.fragments.iter().filter(|fragment| {
        fragment.page == receipt.origin_page && fragment.row == receipt.target_row
    }) {
        ranges
            .entry((
                fragment.stream_object[0],
                u16::try_from(fragment.stream_object[1])
                    .map_err(|_| fail("authored relocation grid generation overflow"))?,
            ))
            .or_default()
            .push(fragment.decoded_range[0]..fragment.decoded_range[1]);
    }
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(receipt.origin_page)?;
    let reader = engine.document().reader();
    let indexes = page
        .contents
        .iter()
        .copied()
        .enumerate()
        .map(|(index, stream)| (stream, index))
        .collect::<BTreeMap<_, _>>();
    let last_stream = ranges
        .keys()
        .map(|stream| {
            indexes
                .get(stream)
                .copied()
                .ok_or_else(|| fail("authored relocation target stream left its origin page"))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .ok_or_else(|| fail("authored relocation target row has no source ranges"))?;
    for (index, &stream) in page.contents.iter().enumerate().skip(last_stream) {
        let object = reader.get_object(stream.0, stream.1)?;
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &object,
            reader,
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 64 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        if decoded.status != crate::filters::StreamDecodeStatus::Complete {
            return Ok(false);
        }
        let start = if index == last_stream {
            ranges
                .get(&stream)
                .and_then(|owned| owned.iter().map(|range| range.end).max())
                .ok_or_else(|| fail("authored relocation target tail is missing"))?
        } else {
            0
        };
        if start > decoded.data.len() || !authored_structural_tail_only(&decoded.data[start..])? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn append_authored_relocation_carriers(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    receipt: &AuthoredTypedRelocationReceipt,
) -> Result<Vec<u8>> {
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(receipt.origin_page)?;
    let reader = engine.document().reader();
    let resources = crate::PageResources::from_dict(&page.resources, reader);
    for cell in &receipt.cells {
        if !resources.fonts.contains_key(&cell.font_resource) {
            return Err(WellfriendError::UnsupportedFeature(
                "authored relocation origin no longer retains its exact font resource".into(),
            ));
        }
    }
    let mut next_mcid = crate::tagged_structure::story::next_page_mcid(input, receipt.origin_page)?;
    let mut content = b"Q\nq\n".to_vec();
    for cell in &receipt.cells {
        let retained = model
            .cells
            .iter()
            .find(|candidate| candidate.id == cell.id)
            .ok_or_else(|| fail("authored relocation retained cell disappeared"))?;
        let layout = retained
            .layout
            .as_ref()
            .ok_or_else(|| fail("authored relocation retained layout disappeared"))?;
        let paint = layout
            .paint
            .as_ref()
            .ok_or_else(|| fail("authored relocation retained paint disappeared"))?;
        content.extend_from_slice(
            format!(
                "/Artifact << /WFRowGrid true /WFGridTableID <{}> /WFRow {} /WFColumn {} /WFRowSpan {} /WFColSpan {} >> BDC\nq\n",
                hex_string(&pdf_text_string(&model.id)),
                cell.row,
                cell.column,
                cell.row_span,
                cell.column_span,
            )
            .as_bytes(),
        );
        write_fill_color(&mut content, &paint.background.to_color());
        write_stroke_color(&mut content, &paint.border.to_color());
        content.extend_from_slice(
            format!(
                "{} w\n{} {} {} {} re B\nQ\nEMC\n",
                fmt_num(paint.line_width),
                fmt_num(cell.rect[0]),
                fmt_num(cell.rect[1]),
                fmt_num(cell.rect[2] - cell.rect[0]),
                fmt_num(cell.rect[3] - cell.rect[1]),
            )
            .as_bytes(),
        );
        let role = match cell.role {
            AuthoredTypedCellRole::Header => "TH",
            AuthoredTypedCellRole::Data => "TD",
        };
        content.extend_from_slice(
            format!(
                "/{role} << /MCID {next_mcid} /WFTableID <{}> /WFCellID <{}> /WFLeft {} /WFBottom {} /WFRight {} /WFTop {} >> BDC\nq\n",
                hex_string(&pdf_text_string(&model.id)),
                hex_string(&pdf_text_string(&cell.id)),
                fmt_num(cell.region[0]),
                fmt_num(cell.region[1]),
                fmt_num(cell.region[2]),
                fmt_num(cell.region[3]),
            )
            .as_bytes(),
        );
        write_fill_color(&mut content, &paint.text.to_color());
        content.extend_from_slice(
            format!(
                "BT /{} {} Tf\n0 Tc 0 Tw 100 Tz 0 Ts\n1 0 0 1 {} {} Tm <> Tj\nET\nQ\nEMC\n",
                encoded_pdf_name(&cell.font_resource),
                fmt_num(layout.font_size),
                fmt_num(cell.region[0]),
                fmt_num(cell.region[3] - layout.font_size),
            )
            .as_bytes(),
        );
        next_mcid = next_mcid.checked_add(1).ok_or_else(|| {
            WellfriendError::ResourceLimit("authored relocation MCID space".into())
        })?;
    }
    content.extend_from_slice(b"Q\n");
    let base = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("authored relocation object space".into()))?;
    let content_number = base
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("authored relocation object space".into()))?;
    let stream = |raw: Vec<u8>| {
        let mut dictionary = PdfDictionary::empty();
        dictionary.insert("Length", PdfObject::Integer(raw.len() as i64));
        PdfObject::Stream {
            dict: dictionary,
            raw,
        }
    };
    let mut dictionary = reader
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("authored relocation origin is not a page dictionary"))?;
    let mut contents = vec![PdfObject::Reference {
        number: base,
        generation: 0,
    }];
    contents.extend(
        page.contents
            .iter()
            .map(|&(number, generation)| PdfObject::Reference { number, generation }),
    );
    contents.push(PdfObject::Reference {
        number: content_number,
        generation: 0,
    });
    dictionary.insert("Contents", PdfObject::Array(contents));
    let output = crate::writer::write_incremental_update(
        reader,
        vec![
            crate::writer::IncrementalObject {
                number: base,
                generation: 0,
                object: stream(b"q\n".to_vec()),
            },
            crate::writer::IncrementalObject {
                number: content_number,
                generation: 0,
                object: stream(content),
            },
            crate::writer::IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(dictionary),
            },
        ],
    )?;
    let expected = receipt
        .cells
        .iter()
        .map(|cell| (receipt.origin_page, model.id.clone(), cell.id.clone()))
        .collect::<Vec<_>>();
    crate::tagged_structure::story::attach_authored_typed_cell_fragments_to_existing_pages(
        &output, &expected,
    )
}

fn compact_one_authored_typed_table_relocation(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    signature_policy_override: bool,
    receipt: &AuthoredTypedRelocationReceipt,
) -> Result<(
    Vec<u8>,
    Vec<usize>,
    Vec<String>,
    Vec<AuthoredTypedTableRetainedContinuation>,
)> {
    let retained = |reason: &str| {
        (
            input.to_vec(),
            Vec::new(),
            Vec::new(),
            vec![AuthoredTypedTableRetainedContinuation {
                page: receipt.destination_page,
                reason: reason.into(),
            }],
        )
    };
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let pages = engine.document().get_pages()?;
    let reader = engine.document().reader();
    let cell_ids = receipt
        .cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    let destination_set = BTreeSet::from([receipt.destination_page]);
    let destination_inspection =
        crate::tagged_structure::story::inspect_authored_typed_cell_continuation_pages(
            input,
            &model.id,
            &cell_ids,
            &destination_set,
        )?;
    if destination_inspection
        .get(&receipt.destination_page)
        .is_none_or(|inspection| {
            inspection.static_digest != receipt.static_digest || inspection.owned_nontext
        })
    {
        return Ok(retained("relocation_destination_changed"));
    }
    let destination = pages
        .get(receipt.destination_page.saturating_sub(1))
        .filter(|page| page.page_number == receipt.destination_page)
        .ok_or_else(|| fail("authored relocation destination page disappeared"))?;
    let destination_object =
        reader.get_object(destination.object_number, destination.generation_number)?;
    let destination_dictionary = destination_object
        .as_dict()
        .ok_or_else(|| fail("authored relocation destination is not a page dictionary"))?;
    if destination_dictionary.contains_key(TYPED_TABLE_CONTINUATION_MARKER)
        || destination_dictionary.contains_key(TYPED_TABLE_RELOCATION_MARKER)
        || authored_typed_continuation_has_page_features(destination_dictionary)
        || destination_dictionary
            .get("Annots")
            .is_some_and(|annotations| {
                reader
                    .resolve(annotations.clone())
                    .ok()
                    .and_then(|object| object.as_array().map(|items| items.to_vec()))
                    .is_none_or(|annotations| !annotations.is_empty())
            })
    {
        return Ok(retained("relocation_destination_page_features"));
    }
    if !authored_relocation_origin_tail_safe(input, model, receipt)? {
        return Ok(retained("relocation_origin_tail_changed"));
    }
    let target_cell_ids = model
        .cells
        .iter()
        .filter(|cell| cell.row == receipt.target_row)
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    let mut target_continuations = BTreeSet::new();
    for page in &pages {
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let dictionary = page_object
            .as_dict()
            .ok_or_else(|| fail("authored continuation candidate is not a page dictionary"))?;
        if authored_typed_table_continuation_row(dictionary, &model.id) == Some(receipt.target_row)
        {
            target_continuations.insert(page.page_number);
        }
    }
    if !target_continuations.is_empty()
        && crate::tagged_structure::story::inspect_authored_typed_cell_continuation_pages(
            input,
            &model.id,
            &target_cell_ids,
            &target_continuations,
        )?
        .values()
        .any(|inspection| !inspection.empty)
    {
        return Ok(retained("target_continuation_still_has_content"));
    }
    // Consume the only intentional incoming page reference before asking the
    // canonical page-pruning guard about every remaining reachable reference.
    // This intermediate revision is never published independently.
    let origin = engine.document().get_page(receipt.origin_page)?;
    let mut origin_dictionary = reader
        .get_object(origin.object_number, origin.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("authored relocation origin disappeared"))?;
    if origin_dictionary
        .remove(TYPED_TABLE_RELOCATION_MARKER)
        .is_none()
    {
        return Err(fail(
            "authored relocation receipt disappeared before consumption",
        ));
    }
    let markerless = crate::writer::write_incremental_update(
        reader,
        vec![crate::writer::IncrementalObject {
            number: origin.object_number,
            generation: origin.generation_number,
            object: PdfObject::Dictionary(origin_dictionary),
        }],
    )?;
    let guard_engine = crate::ContentEngine::open_bytes(markerless.clone())?;
    let guarded_destination = guard_engine.document().get_page(receipt.destination_page)?;
    let candidates = BTreeMap::from([(
        (
            guarded_destination.object_number,
            guarded_destination.generation_number,
        ),
        receipt.destination_page,
    )]);
    let mut departures = crate::writer::page_pruning::Departures::default();
    departures.tags = crate::tagged_structure::story::authored_typed_cell_owner_nodes(
        &markerless,
        &model.id,
        &cell_ids,
    )?;
    if !crate::writer::page_pruning::protected_pages(
        guard_engine.document(),
        &candidates,
        &departures,
    )?
    .is_empty()
    {
        return Ok(retained("relocation_live_page_dependency"));
    }
    // Prove every retained value fits its original carrier and exact retained
    // font before adding any source objects.
    for cell in &receipt.cells {
        let retained_cell = model
            .cells
            .iter()
            .find(|candidate| candidate.id == cell.id)
            .ok_or_else(|| fail("authored relocation retained cell disappeared"))?;
        let layout = retained_cell
            .layout
            .as_ref()
            .ok_or_else(|| fail("authored relocation retained layout disappeared"))?;
        let current = locate_current_authored_typed_owner(&markerless, &model.id, &cell.id)?;
        let mut synthetic = current
            .iter()
            .find(|fragment| fragment.page == receipt.destination_page)
            .cloned()
            .ok_or_else(|| fail("authored relocation destination carrier disappeared"))?;
        synthetic.page = receipt.origin_page;
        synthetic.region = Some(cell.region);
        synthetic.font_resource = cell.font_resource.clone();
        synthetic.logical_range = [0, 0];
        synthetic.logical_text.clear();
        let plan = plan_authored_text_across_existing_fragments(
            &markerless,
            std::slice::from_ref(&synthetic),
            &[],
            &retained_cell.evaluated,
            layout,
            None,
        )?;
        if plan.consumed_bytes != retained_cell.evaluated.len() {
            return Ok(retained("relocation_value_no_longer_fits_original"));
        }
    }
    let mut output = append_authored_relocation_carriers(&markerless, model, receipt)?;
    for cell in &receipt.cells {
        let retained = model
            .cells
            .iter()
            .find(|candidate| candidate.id == cell.id)
            .ok_or_else(|| fail("authored relocation retained cell disappeared"))?;
        let layout = retained
            .layout
            .as_ref()
            .ok_or_else(|| fail("authored relocation retained layout disappeared"))?;
        let current = locate_current_authored_typed_owner(&output, &model.id, &cell.id)?;
        let fragment = current
            .iter()
            .find(|fragment| {
                fragment.page == receipt.origin_page && fragment.region == Some(cell.region)
            })
            .ok_or_else(|| fail("restored authored relocation carrier disappeared"))?;
        let plan = plan_authored_text_across_existing_fragments(
            &output,
            std::slice::from_ref(fragment),
            &[],
            &retained.evaluated,
            layout,
            None,
        )?;
        if plan.consumed_bytes != retained.evaluated.len() || plan.fragments.len() != 1 {
            return Err(fail(
                "authored relocation restoration plan changed after staging",
            ));
        }
        let generated_font = plan.generated_font.as_deref();
        let force_generated_style = plan.force_generated_style;
        let fragment_text = plan.fragments[0]
            .lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>();
        let mut options = crate::AdvancedTextEditOptions::default();
        options.signature_policy_override = signature_policy_override;
        options.region = cell.region;
        options.font_size = layout.font_size;
        options.line_spacing = layout.line_spacing;
        options.max_lines_or_columns =
            authored_horizontal_line_capacity(cell.region, layout)?.max(1);
        options.alignment = layout.alignment.into();
        let edit = crate::MultiRunTextRangeRequest {
            page: receipt.origin_page,
            logical_start: fragment.logical_range[0],
            logical_end: fragment.logical_range[1],
            replacement_text: fragment_text.clone(),
            mode: authored_mutation_mode(&fragment_text, fragment.writing_mode)?,
            style_policy: crate::MultiRunStylePolicy::PreservePerSegment,
            options,
            final_lines: (!fragment_text.is_empty()).then_some(plan.fragments[0].lines.clone()),
        };
        output = crate::advanced_editing::edit_multi_run_text_range_for_authored_owner(
            &output,
            &edit,
            generated_font,
            &model.id,
            &cell.id,
            force_generated_style,
        )?
        .0;
    }
    let expected = receipt
        .cells
        .iter()
        .map(|cell| (receipt.destination_page, model.id.clone(), cell.id.clone()))
        .collect::<Vec<_>>();
    output =
        crate::tagged_structure::story::detach_authored_typed_cell_fragments(&output, &expected)?;
    let staged = crate::ContentEngine::open_bytes(output)?;
    output = crate::writer::page_pruning::remove(
        staged.document(),
        &BTreeSet::from([receipt.destination_page]),
    )?;
    output = crate::tagged_structure::story::finalize_authored_typed_cell_transaction(&output)?;
    let sources = inspect_authored_typed_table_sources(&output)?;
    for cell in &receipt.cells {
        let source = sources
            .cells
            .iter()
            .find(|candidate| candidate.table_id == model.id && candidate.cell_id == cell.id)
            .ok_or_else(|| fail("restored authored relocation source disappeared"))?;
        if source.evaluated.as_str()
            != model
                .cells
                .iter()
                .find(|candidate| candidate.id == cell.id)
                .map(|candidate| candidate.evaluated.as_str())
                .unwrap_or_default()
            || source.fragments.iter().any(|fragment| {
                fragment.page != receipt.origin_page && !fragment.logical_text.is_empty()
            })
        {
            return Err(fail("authored relocation restoration postcondition failed"));
        }
    }
    if !inspect_authored_typed_table_grid_paint(&output, &model.id)?.complete_typed_grid_ownership {
        return Err(fail("authored relocation grid restoration is incomplete"));
    }
    crate::tagged_structure::validate_parent_tree(&output)?;
    Ok((
        output,
        vec![receipt.destination_page],
        receipt.cells.iter().map(|cell| cell.id.clone()).collect(),
        Vec::new(),
    ))
}

fn authored_relocation_key(receipt: &AuthoredTypedRelocationReceipt) -> (usize, Vec<String>) {
    let mut cells = receipt
        .cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<Vec<_>>();
    cells.sort();
    (receipt.target_row, cells)
}

fn page_before_prior_removals(current: usize, removed: &[usize]) -> Result<usize> {
    let mut original = current;
    for &page in removed {
        if page <= original {
            original = original
                .checked_add(1)
                .ok_or_else(|| fail("authored relocation page projection overflow"))?;
        }
    }
    Ok(original)
}

/// A nested relocation can restore owned grid scopes onto a page that is also
/// the destination of an older receipt. Cell scopes were already excluded from
/// that receipt's digest, but the restored grid artifacts intentionally change
/// its static bytes. Refresh only that directly affected parent receipt inside
/// the same atomic transaction; arbitrary external page changes never call this
/// path and therefore remain detectable.
fn refresh_parent_relocation_digest(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    changed_destination_page: usize,
) -> Result<Vec<u8>> {
    let receipts = inspect_authored_typed_table_relocations(input, model)?;
    let matching = receipts
        .iter()
        .filter(|receipt| receipt.destination_page == changed_destination_page)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok(input.to_vec());
    }
    if matching.len() != 1 {
        return Err(fail(
            "nested authored relocation destination has conflicting receipts",
        ));
    }
    let receipt = matching[0];
    let cell_ids = receipt
        .cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    let inspections =
        crate::tagged_structure::story::inspect_authored_typed_cell_continuation_page_subset(
            input,
            &model.id,
            &cell_ids,
            &BTreeSet::from([changed_destination_page]),
        )?;
    let digest = inspections
        .get(&changed_destination_page)
        .filter(|inspection| !inspection.owned_nontext)
        .map(|inspection| inspection.static_digest.clone())
        .ok_or_else(|| fail("nested authored relocation destination is unsafe"))?;
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let origin = engine.document().get_page(receipt.origin_page)?;
    let reader = engine.document().reader();
    let mut dictionary = reader
        .get_object(origin.object_number, origin.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("nested authored relocation origin disappeared"))?;
    let mut marker = dictionary
        .get(TYPED_TABLE_RELOCATION_MARKER)
        .and_then(PdfObject::as_dict)
        .cloned()
        .ok_or_else(|| fail("nested authored relocation receipt disappeared"))?;
    if marker.get("Table").and_then(PdfObject::as_string) != Some(model.id.as_bytes()) {
        return Err(fail("nested authored relocation receipt changed identity"));
    }
    marker.insert(
        "StaticDigest",
        PdfObject::String(digest.as_bytes().to_vec()),
    );
    dictionary.insert(TYPED_TABLE_RELOCATION_MARKER, PdfObject::Dictionary(marker));
    let output = crate::writer::write_incremental_update(
        reader,
        vec![crate::writer::IncrementalObject {
            number: origin.object_number,
            generation: origin.generation_number,
            object: PdfObject::Dictionary(dictionary),
        }],
    )?;
    let refreshed = inspect_authored_typed_table_relocations(&output, model)?;
    if refreshed
        .iter()
        .filter(|candidate| {
            candidate.origin_page == receipt.origin_page
                && candidate.destination_page == changed_destination_page
                && candidate.static_digest == digest
        })
        .count()
        != 1
    {
        return Err(fail(
            "nested authored relocation digest failed reopen validation",
        ));
    }
    Ok(output)
}

fn parent_relocation_digest_is_current(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    destination_page: usize,
) -> Result<bool> {
    let receipts = inspect_authored_typed_table_relocations(input, model)?;
    let matching = receipts
        .iter()
        .filter(|receipt| receipt.destination_page == destination_page)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok(true);
    }
    if matching.len() != 1 {
        return Err(fail(
            "nested authored relocation destination has conflicting parent receipts",
        ));
    }
    let receipt = matching[0];
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(destination_page)?;
    let reader = engine.document().reader();
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let dictionary = page_object
        .as_dict()
        .ok_or_else(|| fail("nested authored relocation destination is not a page dictionary"))?;
    // The child's relocation marker is expected on this shared page until the
    // child transaction consumes it. Everything else follows the same strict
    // page-feature policy as ordinary relocation compaction.
    let unexpected_page_feature = dictionary.iter().any(|(key, _)| {
        key != TYPED_TABLE_CONTINUATION_MARKER
            && key != TYPED_TABLE_RELOCATION_MARKER
            && !matches!(
                key.as_str(),
                "Type"
                    | "Parent"
                    | "MediaBox"
                    | "CropBox"
                    | "BleedBox"
                    | "TrimBox"
                    | "ArtBox"
                    | "Rotate"
                    | "UserUnit"
                    | "Resources"
                    | "Contents"
                    | "Annots"
                    | "StructParents"
                    | "Tabs"
            )
    });
    let has_annotations = dictionary.get("Annots").is_some_and(|annotations| {
        reader
            .resolve(annotations.clone())
            .ok()
            .and_then(|object| object.as_array().map(|items| items.to_vec()))
            .is_none_or(|annotations| !annotations.is_empty())
    });
    if unexpected_page_feature || has_annotations {
        return Ok(false);
    }
    let cell_ids = receipt
        .cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    let inspections =
        crate::tagged_structure::story::inspect_authored_typed_cell_continuation_page_subset(
            input,
            &model.id,
            &cell_ids,
            &BTreeSet::from([destination_page]),
        )?;
    Ok(inspections
        .get(&destination_page)
        .is_some_and(|inspection| {
            inspection.static_digest == receipt.static_digest && !inspection.owned_nontext
        }))
}

fn compact_authored_typed_table_relocation(
    input: &[u8],
    model: &AuthoredTypedTableModel,
    signature_policy_override: bool,
) -> Result<(
    Vec<u8>,
    Vec<usize>,
    Vec<String>,
    Vec<AuthoredTypedTableRetainedContinuation>,
)> {
    let initial = inspect_authored_typed_table_relocations(input, model)?;
    if initial.is_empty() {
        return Ok((input.to_vec(), Vec::new(), Vec::new(), Vec::new()));
    }
    if initial.len() > 4096 {
        return Err(WellfriendError::ResourceLimit(
            "authored row-relocation receipt count".into(),
        ));
    }
    // A destination may itself be the origin of a later relocation. Process
    // the furthest destination first so the child receipt is consumed before
    // its parent page is considered for removal.
    let mut scheduled = initial
        .iter()
        .map(|receipt| (receipt.destination_page, authored_relocation_key(receipt)))
        .collect::<Vec<_>>();
    let unique_keys = scheduled
        .iter()
        .map(|(_, key)| key.clone())
        .collect::<BTreeSet<_>>();
    if unique_keys.len() != scheduled.len() {
        return Err(fail(
            "authored row-relocation receipts do not have stable unique row identities",
        ));
    }
    scheduled.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    let mut output = input.to_vec();
    let mut removed_original_pages = Vec::new();
    let mut restored_cells = Vec::new();
    let mut retained_pages = Vec::new();
    let mut blocked_receipts = BTreeSet::new();
    for (_, key) in scheduled {
        crate::cancel::check_current_cancel("authored multi-relocation compaction")?;
        let current = inspect_authored_typed_table_relocations(&output, model)?;
        let matching = current
            .iter()
            .filter(|receipt| authored_relocation_key(receipt) == key)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(fail(
                "authored row-relocation receipt lost its stable identity during compaction",
            ));
        }
        if blocked_receipts.contains(&key) {
            retained_pages.push(AuthoredTypedTableRetainedContinuation {
                page: page_before_prior_removals(
                    matching[0].destination_page,
                    &removed_original_pages,
                )?,
                reason: "child_relocation_retained".into(),
            });
            continue;
        }
        let restored_origin = matching[0].origin_page;
        // A nested child's origin is the parent receipt's destination. Verify
        // the parent's stored digest before the child restores any owned
        // carriers onto that page. Refreshing the digest after a child-owned
        // rewrite is safe only when the parent page was current beforehand;
        // otherwise the refresh would launder unrelated external changes.
        if !parent_relocation_digest_is_current(&output, model, restored_origin)? {
            blocked_receipts.extend(
                current
                    .iter()
                    .filter(|receipt| receipt.destination_page == restored_origin)
                    .map(authored_relocation_key),
            );
            retained_pages.push(AuthoredTypedTableRetainedContinuation {
                page: page_before_prior_removals(
                    matching[0].destination_page,
                    &removed_original_pages,
                )?,
                reason: "parent_relocation_destination_changed".into(),
            });
            continue;
        }
        let (mut next, removed, restored, retained) = compact_one_authored_typed_table_relocation(
            &output,
            model,
            signature_policy_override,
            matching[0],
        )?;
        if removed.is_empty() && !retained.is_empty() {
            // This receipt is a child when its origin is another receipt's
            // destination. A typed retention is a valid fail-closed result,
            // but it means the parent destination is still intentionally
            // incomplete. Never run the parent's strict compaction against
            // that partial page in the same transaction.
            blocked_receipts.extend(
                current
                    .iter()
                    .filter(|receipt| receipt.destination_page == restored_origin)
                    .map(authored_relocation_key),
            );
        }
        if !removed.is_empty() {
            next = refresh_parent_relocation_digest(&next, model, restored_origin)?;
        }
        for page in removed {
            removed_original_pages.push(page_before_prior_removals(page, &removed_original_pages)?);
            removed_original_pages.sort_unstable();
        }
        for mut retained in retained {
            retained.page = page_before_prior_removals(retained.page, &removed_original_pages)?;
            retained_pages.push(retained);
        }
        restored_cells.extend(restored);
        output = next;
    }
    removed_original_pages.sort_unstable();
    removed_original_pages.dedup();
    restored_cells.sort();
    restored_cells.dedup();
    retained_pages.sort_by_key(|retained| (retained.page, retained.reason.clone()));
    Ok((
        output,
        removed_original_pages,
        restored_cells,
        retained_pages,
    ))
}

fn prune_empty_authored_typed_table_continuations(
    input: &[u8],
    model: &AuthoredTypedTableModel,
) -> Result<AuthoredTypedTablePruningResult> {
    let all_cell_ids = model
        .cells
        .iter()
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    if all_cell_ids.len() != model.cells.len()
        || all_cell_ids.is_empty()
        || all_cell_ids.len() > 4096
    {
        return Err(fail(
            "authored typed-table pruning requires unique bounded cell identities",
        ));
    }
    let mut row_cell_ids = BTreeMap::<usize, BTreeSet<String>>::new();
    for cell in &model.cells {
        row_cell_ids
            .entry(cell.row)
            .or_default()
            .insert(cell.id.clone());
    }
    let engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let pages = engine.document().get_pages()?;
    let reader = engine.document().reader();
    let mut marked_by_row = BTreeMap::<usize, BTreeSet<usize>>::new();
    let mut page_rows = BTreeMap::<usize, usize>::new();
    for page in &pages {
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let dictionary = page_object
            .as_dict()
            .ok_or_else(|| fail("authored typed-table candidate page is not a dictionary"))?;
        let Some(marker) = dictionary
            .get(TYPED_TABLE_CONTINUATION_MARKER)
            .and_then(PdfObject::as_dict)
        else {
            continue;
        };
        if marker.get("Table").and_then(PdfObject::as_string) != Some(model.id.as_bytes()) {
            continue;
        }
        if marker.get_integer("Version") == Some(1)
            && model
                .pagination
                .as_ref()
                .is_none_or(|pagination| pagination.body_rows != 1)
        {
            return Err(fail(
                "legacy authored typed-table continuation cannot identify a row in a multi-row table",
            ));
        }
        let row = authored_typed_table_continuation_row(dictionary, &model.id)
            .ok_or_else(|| fail("authored typed-table continuation row is malformed"))?;
        let cell_ids = row_cell_ids.get(&row).ok_or_else(|| {
            fail("authored typed-table continuation names an unknown retained row")
        })?;
        if !is_authored_typed_table_continuation(dictionary, &model.id, row, cell_ids.len())
            || authored_typed_table_continuation_digest(dictionary, &model.id, row, cell_ids.len())
                .is_none()
        {
            return Err(fail(
                "authored typed-table continuation provenance is malformed",
            ));
        }
        marked_by_row
            .entry(row)
            .or_default()
            .insert(page.page_number);
        if page_rows.insert(page.page_number, row).is_some() {
            return Err(fail(
                "authored typed-table continuation page has conflicting row provenance",
            ));
        }
    }
    if page_rows.is_empty() {
        return Ok(AuthoredTypedTablePruningResult {
            output: input.to_vec(),
            removed_pages: Vec::new(),
            retained_pages: Vec::new(),
        });
    }
    let mut inspections = BTreeMap::new();
    for (row, marked) in &marked_by_row {
        let cell_ids = row_cell_ids
            .get(row)
            .ok_or_else(|| fail("authored typed-table continuation row disappeared"))?;
        for (page, inspection) in
            crate::tagged_structure::story::inspect_authored_typed_cell_continuation_pages(
                input, &model.id, cell_ids, marked,
            )?
        {
            if inspections.insert(page, inspection).is_some() {
                return Err(fail(
                    "authored typed-table continuation inspection repeats a page",
                ));
            }
        }
    }
    let mut candidates = BTreeMap::<(u32, u16), usize>::new();
    let mut retained_pages = Vec::new();
    for (&page_number, &row) in &page_rows {
        crate::cancel::check_current_cancel("authored typed-table pruning plan")?;
        let cell_ids = row_cell_ids
            .get(&row)
            .ok_or_else(|| fail("authored typed-table continuation row disappeared"))?;
        let page = pages
            .get(page_number.saturating_sub(1))
            .filter(|page| page.page_number == page_number)
            .ok_or_else(|| fail("authored typed-table continuation page disappeared"))?;
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let dictionary = page_object
            .as_dict()
            .ok_or_else(|| fail("authored typed-table continuation is not a page dictionary"))?;
        let inspection = inspections.get(&page_number).ok_or_else(|| {
            fail("authored typed-table continuation inspection result is missing")
        })?;
        let stored_digest =
            authored_typed_table_continuation_digest(dictionary, &model.id, row, cell_ids.len())
                .ok_or_else(|| fail("authored typed-table continuation digest is missing"))?;
        let reason = if stored_digest != inspection.static_digest.as_bytes() {
            Some("static_page_content_changed")
        } else if !inspection.empty {
            Some("continuation_still_has_content")
        } else if authored_typed_continuation_has_page_features(dictionary) {
            Some("additional_page_features")
        } else if let Some(value) = dictionary.get("Annots") {
            let value = reader.resolve(value.clone())?;
            if value
                .as_array()
                .is_none_or(|annotations| !annotations.is_empty())
            {
                Some("page_annotations")
            } else {
                None
            }
        } else {
            None
        };
        if let Some(reason) = reason {
            retained_pages.push(AuthoredTypedTableRetainedContinuation {
                page: page_number,
                reason: reason.into(),
            });
        } else {
            candidates.insert((page.object_number, page.generation_number), page_number);
        }
    }
    if candidates.len() >= pages.len() {
        if let Some((&page_id, &page_number)) = candidates.iter().next() {
            candidates.remove(&page_id);
            retained_pages.push(AuthoredTypedTableRetainedContinuation {
                page: page_number,
                reason: "last_document_page".into(),
            });
        }
    }
    if !candidates.is_empty() {
        let mut departures = crate::writer::page_pruning::Departures::default();
        departures.tags = crate::tagged_structure::story::authored_typed_cell_owner_nodes(
            input,
            &model.id,
            &all_cell_ids,
        )?;
        let protected = crate::writer::page_pruning::protected_pages(
            engine.document(),
            &candidates,
            &departures,
        )?;
        candidates.retain(|_, page| {
            if let Some(reason) = protected.get(page) {
                retained_pages.push(AuthoredTypedTableRetainedContinuation {
                    page: *page,
                    reason: reason.clone(),
                });
                false
            } else {
                true
            }
        });
    }
    let mut removed_pages = candidates.values().copied().collect::<Vec<_>>();
    removed_pages.sort_unstable();
    retained_pages.sort_by_key(|page| page.page);
    if removed_pages.is_empty() {
        return Ok(AuthoredTypedTablePruningResult {
            output: input.to_vec(),
            removed_pages,
            retained_pages,
        });
    }
    let mut expected = Vec::new();
    for &page in &removed_pages {
        let row = page_rows
            .get(&page)
            .ok_or_else(|| fail("removed continuation page lost its row provenance"))?;
        let cell_ids = row_cell_ids
            .get(row)
            .ok_or_else(|| fail("removed continuation row lost its cell provenance"))?;
        expected.extend(
            cell_ids
                .iter()
                .map(|cell| (page, model.id.clone(), cell.clone())),
        );
    }
    let detached =
        crate::tagged_structure::story::detach_authored_typed_cell_fragments(input, &expected)?;
    let detached_engine = crate::ContentEngine::open_bytes(detached)?;
    let output = crate::writer::page_pruning::remove(
        detached_engine.document(),
        &removed_pages.iter().copied().collect(),
    )?;
    let output = crate::tagged_structure::story::finalize_authored_typed_cell_transaction(&output)?;
    let reopened = crate::ContentEngine::open_bytes(output.clone())?;
    if reopened.page_count()? != pages.len() - removed_pages.len() {
        return Err(fail(
            "authored typed-table continuation pruning changed the wrong page count",
        ));
    }
    crate::tagged_structure::validate_parent_tree(&output)?;
    Ok(AuthoredTypedTablePruningResult {
        output,
        removed_pages,
        retained_pages,
    })
}

/// Update exact values/formulas in a freshly authored table after reopen.
/// The operation is revision-bound and publishes no partial output. Source
/// mutation prepares one paragraph against the retained layout contract and
/// redistributes complete lines across all existing exact owner rectangles.
/// A fully typed simple row may grow through canonical owned continuations and
/// may opt into provenance-bound empty-page contraction. Contiguous downstream
/// rows wholly owned by the target's final page are source-relocated together;
/// a later shrink can restore that exact page tail from a versioned relocation
/// receipt before canonical page removal. Split/ambiguous rows still fail closed
/// rather than clipping or overpainting.
pub fn mutate_authored_typed_table(
    input: &[u8],
    request: &AuthoredTypedTableMutationRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, AuthoredTypedTableMutationReport)> {
    let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    if request.input_sha256 != digest(input)
        || request.table_id.is_empty()
        || request.updates.is_empty()
        || request.updates.len() > 4096
    {
        return Err(fail(
            "authored typed-table mutation requires the exact input revision and bounded updates",
        ));
    }
    let mut models = load_authored_typed_tables(input)?;
    let table_index = models
        .iter()
        .position(|model| model.id == request.table_id)
        .ok_or_else(|| fail("authored typed-table mutation target is missing"))?;
    let previous_values = models[table_index]
        .cells
        .iter()
        .map(|cell| (cell.id.clone(), cell.evaluated.clone()))
        .collect::<BTreeMap<_, _>>();
    for (id, value) in &request.updates {
        let cell = models[table_index]
            .cells
            .iter_mut()
            .find(|cell| cell.id == *id)
            .ok_or_else(|| fail("authored typed-table update names an unknown cell"))?;
        cell.value = value.clone();
    }
    let entries = models[table_index]
        .cells
        .iter()
        .map(|cell| (cell.id.as_str(), cell.row, cell.column, &cell.value))
        .collect::<Vec<_>>();
    let values = crate::typed_tables::evaluate_values(&request.table_id, &entries)?;
    for cell in &mut models[table_index].cells {
        cell.evaluated = values
            .get(&cell.id)
            .cloned()
            .ok_or_else(|| fail("reevaluated authored typed cell is missing"))?;
    }
    models[table_index].validate()?;

    let source_before = inspect_authored_typed_table_sources(input)?;
    let mut edits = Vec::new();
    for cell in &source_before.cells {
        if cell.table_id != request.table_id
            || previous_values.get(&cell.cell_id) == values.get(&cell.cell_id)
        {
            continue;
        }
        let fragment = cell
            .fragments
            .first()
            .ok_or_else(|| fail("authored typed cell has no source fragment"))?;
        let cell_model = models[table_index]
            .cells
            .iter()
            .find(|model| model.id == cell.cell_id)
            .ok_or_else(|| fail("authored typed-cell retained model disappeared"))?;
        let layout = cell_model.layout.clone().ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "authored typed-cell source predates retained layout metadata; cross-fragment mutation cannot guess its line geometry"
                        .into(),
                )
            })?;
        edits.push((
            fragment.page,
            fragment.logical_range,
            cell.cell_id.clone(),
            values[&cell.cell_id].clone(),
            layout,
            cell_model.row,
            true,
        ));
    }
    if edits.len() > 256 {
        return Err(WellfriendError::ResourceLimit(
            "authored typed-table visible mutation count exceeds 256".into(),
        ));
    }
    edits.sort_by_key(|edit| (edit.5, std::cmp::Reverse((edit.0, edit.1[0], edit.1[1]))));
    let operation = if request.prune_empty_continuations {
        crate::secure_mutation::EditOperation::PageDelete
    } else if edits.is_empty() {
        crate::secure_mutation::EditOperation::MetadataUpdate
    } else {
        crate::secure_mutation::EditOperation::ContentEdit
    };
    let policy_engine = crate::ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = crate::secure_mutation::analyze_edit_policy(&policy_engine, operation)?;
    enforce_authored_table_signature_policy(&signature_policy, request.signature_policy_override)?;

    let mut output = input.to_vec();
    let mut changed_pages = BTreeSet::new();
    let mut changed_cells = Vec::new();
    let mut allocated_pages = Vec::new();
    let mut pending = std::collections::VecDeque::from(edits);
    let mut relocation_enqueues = 0usize;
    while let Some((_, _, cell_id, replacement, layout, _, allow_approved_font)) =
        pending.pop_front()
    {
        crate::cancel::check_current_cancel("authored typed-table source mutation")?;
        let cell_font_bytes = if allow_approved_font {
            font_bytes
        } else {
            None
        };
        let mut rebound =
            locate_current_authored_typed_owner(&output, &request.table_id, &cell_id)?;
        rebound.sort_by_key(|fragment| {
            (
                fragment.page,
                fragment.logical_range[0],
                fragment.logical_range[1],
            )
        });
        let mut plan = plan_authored_text_across_existing_fragments(
            &output,
            &rebound,
            &[],
            &replacement,
            &layout,
            cell_font_bytes,
        )?;
        if plan.consumed_bytes != replacement.len() {
            let cell_model = models[table_index]
                .cells
                .iter()
                .find(|cell| cell.id == cell_id)
                .cloned()
                .ok_or_else(|| fail("authored typed-cell growth model disappeared"))?;
            let (grown, pages, relocated_cells) = allocate_authored_typed_row_fragments(
                &output,
                &models[table_index],
                &cell_model,
                &rebound,
                &replacement,
                &layout,
                &plan,
                cell_font_bytes,
            )?;
            output = grown;
            for page in &pages {
                changed_pages.insert(*page);
            }
            allocated_pages.extend(pages);
            relocation_enqueues = relocation_enqueues
                .checked_add(relocated_cells.len())
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "authored typed-row relocation enqueue count".into(),
                    )
                })?;
            if relocation_enqueues > 4096 {
                return Err(WellfriendError::ResourceLimit(
                    "authored typed-row relocation enqueue count".into(),
                ));
            }
            for relocated in relocated_cells {
                pending.retain(|(_, _, queued, _, _, _, _)| queued != &relocated);
                let relocated_model = models[table_index]
                    .cells
                    .iter()
                    .find(|cell| cell.id == relocated)
                    .ok_or_else(|| fail("relocated authored typed-cell model disappeared"))?;
                let relocated_layout = relocated_model
                    .layout
                    .clone()
                    .ok_or_else(|| fail("relocated authored typed-cell layout disappeared"))?;
                pending.push_back((
                    0,
                    [0, 0],
                    relocated.clone(),
                    relocated_model.evaluated.clone(),
                    relocated_layout,
                    relocated_model.row,
                    false,
                ));
            }
            rebound = locate_current_authored_typed_owner(&output, &request.table_id, &cell_id)?;
            rebound.sort_by_key(|fragment| {
                (
                    fragment.page,
                    fragment.logical_range[0],
                    fragment.logical_range[1],
                )
            });
            plan = plan_authored_text_across_existing_fragments(
                &output,
                &rebound,
                &[],
                &replacement,
                &layout,
                cell_font_bytes,
            )?;
        }
        let AuthoredRedistributionPlan {
            fragments,
            generated_font,
            force_generated_style,
            consumed_bytes,
        } = plan;
        if consumed_bytes != replacement.len() {
            return Err(fail(
                "authored typed-cell continuation allocation did not close shaped content",
            ));
        }
        let edit_font = generated_font.as_deref().or(cell_font_bytes);
        for fragment_plan in fragments {
            crate::cancel::check_current_cancel("authored typed-table fragment redistribution")?;
            let current =
                locate_current_authored_typed_owner(&output, &request.table_id, &cell_id)?;
            let matches = current
                .iter()
                .filter(|fragment| {
                    fragment.page == fragment_plan.page
                        && fragment.region == Some(fragment_plan.region)
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(fail(
                    "authored typed-cell fragment lost its unique page/region identity",
                ));
            }
            let fragment = matches[0];
            let fragment_text = fragment_plan
                .lines
                .iter()
                .map(|line| line.logical_text.as_str())
                .collect::<String>();
            if fragment.logical_range[0] == fragment.logical_range[1] && fragment_text.is_empty() {
                continue;
            }
            let mut options = crate::AdvancedTextEditOptions::default();
            options.signature_policy_override = request.signature_policy_override;
            options.region = fragment_plan.region;
            options.font_size = layout.font_size;
            options.line_spacing = layout.line_spacing;
            options.max_lines_or_columns =
                authored_horizontal_line_capacity(fragment_plan.region, &layout)?.max(1);
            options.alignment = layout.alignment.into();
            let edit = crate::MultiRunTextRangeRequest {
                page: fragment_plan.page,
                logical_start: fragment.logical_range[0],
                logical_end: fragment.logical_range[1],
                replacement_text: fragment_text.clone(),
                mode: authored_mutation_mode(&fragment_text, fragment.writing_mode)?,
                style_policy: crate::MultiRunStylePolicy::PreservePerSegment,
                options,
                final_lines: (!fragment_text.is_empty()).then_some(fragment_plan.lines),
            };
            output = crate::advanced_editing::edit_multi_run_text_range_for_authored_owner(
                &output,
                &edit,
                edit_font,
                &request.table_id,
                &cell_id,
                force_generated_style,
            )?
            .0;
            changed_pages.insert(fragment_plan.page);
        }
        changed_cells.push(cell_id);
    }
    // Publish the reevaluated registry inside the still-private transaction
    // before compaction reopens the complete table source. Otherwise the exact
    // source inspector would correctly reject the newly painted target value
    // against the previous registry revision.
    output = save_authored_typed_table_models(&output, &models)?;
    let page_count_before_pruning = if request.prune_empty_continuations {
        Some(crate::ContentEngine::open_bytes(output.clone())?.page_count()?)
    } else {
        None
    };
    let mut relocation_removed_pages = Vec::new();
    let mut relocation_retained_pages = Vec::new();
    if request.prune_empty_continuations {
        let (compacted, removed, restored_cells, retained) =
            compact_authored_typed_table_relocation(
                &output,
                &models[table_index],
                request.signature_policy_override,
            )?;
        output = compacted;
        relocation_removed_pages = removed;
        relocation_retained_pages = retained;
        if !restored_cells.is_empty() {
            let restored = restored_cells.iter().cloned().collect::<BTreeSet<_>>();
            changed_pages.extend(
                inspect_authored_typed_table_relocations(input, &models[table_index])?
                    .into_iter()
                    .filter(|receipt| receipt.cells.iter().any(|cell| restored.contains(&cell.id)))
                    .map(|receipt| receipt.origin_page),
            );
            changed_cells.extend(restored_cells);
        }
    }
    let mut removed_pages = relocation_removed_pages.clone();
    let mut retained_continuation_pages = relocation_retained_pages;
    if request.prune_empty_continuations {
        let pruning =
            prune_empty_authored_typed_table_continuations(&output, &models[table_index])?;
        output = pruning.output;
        let restore_pre_compaction_number = |page: usize| {
            let mut restored = page;
            for &removed in &relocation_removed_pages {
                if restored >= removed {
                    restored += 1;
                }
            }
            restored
        };
        removed_pages.extend(
            pruning
                .removed_pages
                .into_iter()
                .map(restore_pre_compaction_number),
        );
        removed_pages.sort_unstable();
        removed_pages.dedup();
        retained_continuation_pages.extend(pruning.retained_pages.into_iter().map(
            |mut retained| {
                retained.page = restore_pre_compaction_number(retained.page);
                retained
            },
        ));
        retained_continuation_pages.sort_by_key(|retained| retained.page);
        if let Some(&first_removed) = removed_pages.first() {
            let removed = removed_pages.iter().copied().collect::<BTreeSet<_>>();
            changed_pages = changed_pages
                .into_iter()
                .filter(|page| !removed.contains(page))
                .map(|page| page - removed_pages.partition_point(|removed| *removed < page))
                .collect();
            let final_page_count = page_count_before_pruning
                .ok_or_else(|| fail("authored table pruning page count is missing"))?
                .checked_sub(removed_pages.len())
                .ok_or_else(|| fail("authored table pruning removed too many pages"))?;
            if final_page_count > 0 {
                changed_pages.extend(first_removed.min(final_page_count)..=final_page_count);
            }
        }
    }
    let source_report = inspect_authored_typed_table_sources(&output)?;
    let output_sha256 = digest(&output);
    if (allocated_pages.is_empty() && removed_pages.is_empty() && !output.starts_with(input))
        || source_report.input_sha256 != output_sha256
        || source_report.cells.iter().any(|cell| {
            cell.table_id == request.table_id && values.get(&cell.cell_id) != Some(&cell.evaluated)
        })
    {
        return Err(fail(
            "authored typed-table mutation failed its reopen/source postconditions",
        ));
    }
    changed_cells.sort();
    changed_cells.dedup();
    Ok((
        output,
        AuthoredTypedTableMutationReport {
            schema_version: "wellfriend.authored_typed_table_mutation.v1".into(),
            table_id: request.table_id.clone(),
            input_sha256: request.input_sha256.clone(),
            output_sha256,
            previous_values,
            values,
            changed_cells,
            changed_pages: changed_pages.into_iter().collect(),
            removed_pages: removed_pages.clone(),
            retained_continuation_pages,
            source_report,
            signature_policy,
            original_prefix_preserved: allocated_pages.is_empty() && removed_pages.is_empty(),
            output_reopened: true,
            exact_limits: vec![
                "a changed cell is shaped once and redistributed across retained owned rectangles using its font size, line spacing, alignment and exact source font state; a fully typed body row can allocate canonical continuation pages for every cell owner, while contiguous later rows wholly owned by the same final page are removed by exact source ranges, packed at retained heights on one following page and rebound to their original TH/TD owners; repeatable headers remain exact artifacts".into(),
                "positioned replacement is injected into the first source owner operand of each fragment, preserving its MCID, private owner and original paint order while removing trailing old line operands".into(),
                "opt-in contraction removes only empty typed-row pages whose exact table/row/cell-count provenance and non-cell content digest survive reopen; live references, annotations, page features and original or changed pages are retained".into(),
                "same-page row relocation requires complete typed text/grid ownership in page-private losslessly decoded streams and a structural-only page tail after the displaced rows; it writes a versioned origin/destination/geometry/font/digest receipt, removes only exact BDC/EMC scopes, preserves unrelated stream bytes and revalidates grid ownership plus the ParentTree after rebinding".into(),
                "opt-in backward compaction restores a receipt-bound relocated page tail only when every target continuation is empty, the destination digest and page-reference guard survive, every retained value fits its original exact font/region, and the origin is still a structural tail; it recreates native grid/text carriers, rebinds the original TH/TD owners and canonically removes the relocation page".into(),
                "multiple relocation receipts rebind after every rewrite and nested groups compact furthest-child first; downstream rows already split across pages, row-spanning target rows, partially typed or mixed-object rows, non-page fragment compaction and general row-height changes remain outside this retained pagination transaction".into(),
                "existing-fragment edits are incremental; page growth performs one canonical full-document page-tree rewrite before incremental owner/registry updates, and neither mode is a sanitizing-redaction guarantee".into(),
                "font substitution and signature-policy overrides remain explicit caller decisions".into(),
            ],
        },
    ))
}

#[derive(Default)]
pub(super) struct Budget {
    lines: usize,
    cells: usize,
    bytes: usize,
    fragments: usize,
    output_cells: usize,
    output_lines: usize,
}

impl Budget {
    fn text(&mut self, bytes: usize, lines: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        self.lines = self.lines.saturating_add(lines);
        if self.bytes > MAX_SOURCE_BYTES || self.lines > MAX_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authored table text/line budget".into(),
            ));
        }
        Ok(())
    }

    fn cell(&mut self, bytes: usize) -> Result<()> {
        self.cells = self.cells.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.cells > MAX_CELLS || self.bytes > MAX_SOURCE_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "authored table source/cell budget".into(),
            ));
        }
        Ok(())
    }
    fn lines(&mut self, lines: usize) -> Result<()> {
        self.lines = self.lines.saturating_add(lines);
        if self.lines > MAX_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authored table line budget".into(),
            ));
        }
        Ok(())
    }
    fn fragment(&mut self, fragment: &Fragment) -> Result<()> {
        crate::cancel::check_current_cancel("authored table pagination")?;
        self.fragments += 1;
        self.output_cells = self.output_cells.saturating_add(fragment.ranges.len());
        for range in &fragment.ranges {
            self.output_lines = self.output_lines.saturating_add(range.len());
        }
        if self.output_cells > MAX_CELLS || self.output_lines > MAX_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authored table output cell/line budget".into(),
            ));
        }
        if self.fragments > MAX_FRAGMENTS {
            return Err(WellfriendError::ResourceLimit(
                "authored table fragment budget".into(),
            ));
        }
        Ok(())
    }

    fn grid_fragment(&mut self, ranges: &[Range<usize>]) -> Result<()> {
        crate::cancel::check_current_cancel("authored row-span pagination")?;
        self.fragments = self.fragments.saturating_add(1);
        self.output_cells = self.output_cells.saturating_add(ranges.len());
        for range in ranges {
            self.output_lines = self.output_lines.saturating_add(range.len());
        }
        if self.output_cells > MAX_CELLS || self.output_lines > MAX_LINES {
            return Err(WellfriendError::ResourceLimit(
                "authored row-span output cell/line budget".into(),
            ));
        }
        if self.fragments > MAX_FRAGMENTS {
            return Err(WellfriendError::ResourceLimit(
                "authored row-span fragment budget".into(),
            ));
        }
        Ok(())
    }
}

struct Cell {
    source_cell: Option<usize>,
    column_start: usize,
    column_span: usize,
    row_start: usize,
    row_span: usize,
    left: f64,
    width: f64,
    align: TextAlign,
    style: TextStyle,
    typed: bool,
    fill: Color,
    lines: Vec<layout::Line>,
    /// Monotone height and UTF-8 prefixes permit bounded binary-search cuts.
    heights: Vec<f64>,
    bytes: Vec<usize>,
}

pub(super) struct PreparedRow {
    cells: Vec<Cell>,
    padding: f64,
    minimum_height: f64,
    border: Color,
    line_width: f64,
}

struct PreparedGrid {
    rows: Vec<PreparedRow>,
    prefix: Vec<f64>,
}

struct GridSegmentRow {
    row: usize,
    top_offset: f64,
    height: f64,
    ranges: Vec<Range<usize>>,
    continues: bool,
}

pub(super) struct Fragment {
    pub(super) height: f64,
    ranges: Vec<Range<usize>>,
    complete: bool,
}

struct TableStructure {
    caption: Option<u64>,
    header_cells: Option<Vec<u64>>,
    body_cells: Vec<Vec<u64>>,
}

struct PreparedCaption {
    lines: Vec<layout::Line>,
    line_height: f64,
    height: f64,
}

impl PreparedCaption {
    fn new(
        table: &TableBuilder,
        page: &PdfPageBuilder,
        budget: &mut Budget,
    ) -> Result<Option<Self>> {
        let Some(text) = table.caption.as_deref() else {
            return Ok(None);
        };
        let width = table.total_width();
        let lines = layout::prepare(page, text, width, &table.caption_style)?;
        if lines.is_empty() {
            return Err(fail("authored table caption has no logical lines"));
        }
        budget.text(text.len(), lines.len())?;
        let line_height = table
            .caption_paragraph
            .line_height_points(table.caption_style.size)?;
        let height = lines.iter().try_fold(CAPTION_GAP, |height, line| {
            let next = height + line.occupied_height(line_height);
            if next.is_finite() {
                Ok(next)
            } else {
                Err(fail("authored table caption height overflow"))
            }
        })?;
        Ok(Some(Self {
            lines,
            line_height,
            height,
        }))
    }

    fn render(
        &self,
        page: &mut PdfPageBuilder,
        table: &TableBuilder,
        x: f64,
        top: f64,
        structure: Option<u64>,
    ) -> Result<()> {
        let mut cursor = top;
        for line in &self.lines {
            let origin = line.aligned_x(x, table.total_width(), table.caption_paragraph.align);
            if let Some(structure) = structure {
                page.commands.push(PageCommand::BeginStructure(structure));
            }
            page.commands.push(line.command(
                origin,
                cursor - line.metrics.ascent,
                &table.caption_style,
            )?);
            if let Some(structure) = structure {
                page.commands.push(PageCommand::EndStructure(structure));
            }
            cursor -= line.occupied_height(self.line_height);
        }
        if (top - cursor + CAPTION_GAP - self.height).abs() > EPS {
            return Err(fail("authored table caption extent mismatch"));
        }
        Ok(())
    }
}

fn register_structure(
    flow: &mut FlowDocument,
    table: &TableBuilder,
) -> Result<Option<TableStructure>> {
    if table.header.is_none() && table.rows.is_empty() {
        return Ok(None);
    }
    let table_structure = structure::register_table(&mut flow.builder, table.summary.as_deref())?;
    let caption = table
        .caption
        .as_ref()
        .map(|_| structure::register_caption(&mut flow.builder, table_structure))
        .transpose()?;
    let head = table
        .header
        .as_ref()
        .map(|_| {
            structure::register_table_group(
                &mut flow.builder,
                table_structure,
                structure::Role::TableHead,
            )
        })
        .transpose()?;
    let body = (!table.rows.is_empty())
        .then(|| {
            structure::register_table_group(
                &mut flow.builder,
                table_structure,
                structure::Role::TableBody,
            )
        })
        .transpose()?;
    let mut register_row = |source: &TableRow, header: bool, parent: u64| -> Result<Vec<u64>> {
        let row = structure::register(
            &mut flow.builder,
            structure::Role::TableRow,
            Some(parent),
            None,
        )?;
        table
            .cell_placements(source)?
            .into_iter()
            .map(|(source, columns)| {
                let scope = source
                    .map(|(_, cell)| cell)
                    .and_then(|cell| cell.header_scope)
                    .or(header.then_some(TableHeaderScope::Column));
                let cell = structure::register_table_cell(
                    &mut flow.builder,
                    if scope.is_some() {
                        structure::Role::TableHeader
                    } else {
                        structure::Role::TableData
                    },
                    row,
                    source.is_none_or(|(_, cell)| cell.text.is_empty()),
                )?;
                if let Some(scope) = scope {
                    structure::set_table_header_scope(&mut flow.builder, cell, scope)?;
                }
                if columns.len() > 1 {
                    structure::set_table_column_span(&mut flow.builder, cell, columns.len())?;
                }
                Ok(cell)
            })
            .collect()
    };
    let header_cells = table
        .header
        .as_ref()
        .map(|row| register_row(row, true, head.expect("header group")))
        .transpose()?;
    let body_grid = table.body_cell_placements()?;
    let mut body_cells = Vec::with_capacity(body_grid.len());
    for placements in &body_grid {
        let row = structure::register(
            &mut flow.builder,
            structure::Role::TableRow,
            Some(body.expect("body group")),
            None,
        )?;
        let mut registered = Vec::with_capacity(placements.len());
        for placement in placements {
            let scope = placement
                .source
                .map(|(_, cell)| cell)
                .and_then(|cell| cell.header_scope);
            let cell = structure::register_table_cell(
                &mut flow.builder,
                if scope.is_some() {
                    structure::Role::TableHeader
                } else {
                    structure::Role::TableData
                },
                row,
                placement
                    .source
                    .is_none_or(|(_, cell)| cell.text.is_empty()),
            )?;
            if let Some(scope) = scope {
                structure::set_table_header_scope(&mut flow.builder, cell, scope)?;
            }
            if placement.columns.len() > 1 {
                structure::set_table_column_span(&mut flow.builder, cell, placement.columns.len())?;
            }
            if placement.row_span > 1 {
                structure::set_table_row_span(&mut flow.builder, cell, placement.row_span)?;
            }
            if let Some((_, source)) = placement.source {
                match (&source.typed_id, &source.typed_value) {
                    (Some(cell_identity), Some(_)) => {
                        let table_identity = table.identity.as_deref().ok_or_else(|| {
                            fail("authored typed table structure requires a table identity")
                        })?;
                        structure::set_typed_table_cell_owner(
                            &mut flow.builder,
                            cell,
                            table_identity,
                            cell_identity,
                        )?;
                    }
                    (None, None) => {}
                    _ => {
                        return Err(fail(
                            "authored typed table structure requires paired cell identity/value",
                        ));
                    }
                }
            }
            registered.push(cell);
        }
        body_cells.push(registered);
    }
    let mut column_headers = vec![None; table.columns.len()];
    if let (Some(source), Some(headers)) = (table.header.as_ref(), header_cells.as_ref()) {
        let placements = table.cell_placements(source)?;
        if placements.len() != headers.len() {
            return Err(fail(
                "authored table header topology changed during structure planning",
            ));
        }
        for ((source, columns), header) in placements.into_iter().zip(headers) {
            let scope = source
                .map(|(_, cell)| cell)
                .and_then(|cell| cell.header_scope)
                .unwrap_or(TableHeaderScope::Column);
            if matches!(scope, TableHeaderScope::Column | TableHeaderScope::Both) {
                for column in columns {
                    column_headers[column] = Some(*header);
                }
            }
        }
    }
    let mut row_headers_by_row = vec![Vec::new(); table.rows.len()];
    for (cells, placements) in body_cells.iter().zip(&body_grid) {
        for (cell, placement) in cells.iter().zip(placements) {
            if placement
                .source
                .map(|(_, source)| source)
                .and_then(|source| source.header_scope)
                .is_some_and(|scope| {
                    matches!(scope, TableHeaderScope::Row | TableHeaderScope::Both)
                })
            {
                for row in placement.row_start..placement.row_start + placement.row_span {
                    row_headers_by_row[row].push(*cell);
                }
            }
        }
    }
    for (row_index, cells) in body_cells.iter().enumerate() {
        let placements = &body_grid[row_index];
        if placements.len() != cells.len() {
            return Err(fail(
                "authored table body topology changed during structure planning",
            ));
        }
        for (placement, cell) in placements.iter().zip(cells) {
            if placement
                .source
                .map(|(_, source)| source)
                .and_then(|source| source.header_scope)
                .is_some_and(|scope| {
                    matches!(scope, TableHeaderScope::Column | TableHeaderScope::Both)
                })
            {
                for column in placement.columns.clone() {
                    column_headers[column] = Some(*cell);
                }
            }
        }
        for (cell, placement) in cells.iter().zip(placements) {
            let is_header = placement
                .source
                .map(|(_, source)| source)
                .and_then(|source| source.header_scope)
                .is_some();
            if is_header {
                continue;
            }
            let mut headers = Vec::new();
            for column in placement.columns.clone() {
                if let Some(header) = column_headers[column] {
                    headers.push(header);
                }
            }
            for row in placement.row_start..placement.row_start + placement.row_span {
                headers.extend(row_headers_by_row[row].iter().copied());
            }
            let mut seen = BTreeSet::new();
            headers.retain(|header| seen.insert(*header));
            structure::set_table_headers(&mut flow.builder, *cell, headers)?;
        }
    }
    Ok(Some(TableStructure {
        caption,
        header_cells,
        body_cells,
    }))
}

struct Continuations {
    policy: TableRowSplitPolicy,
    /// Greatest viable continuation boundary at or before each line boundary.
    /// usize::MAX denotes none. The terminal boundary is handled separately.
    previous: Vec<Vec<usize>>,
}

impl PreparedRow {
    pub(super) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub(super) fn new(
        table: &TableBuilder,
        page: &PdfPageBuilder,
        row: &TableRow,
        header: bool,
        budget: &mut Budget,
    ) -> Result<Self> {
        let placements = table
            .cell_placements(row)?
            .into_iter()
            .map(|(source, columns)| TableCellPlacement {
                source,
                columns,
                row_start: 0,
                row_span: 1,
            })
            .collect::<Vec<_>>();
        Self::new_with_placements(table, page, &placements, header, budget)
    }

    fn new_with_placements(
        table: &TableBuilder,
        page: &PdfPageBuilder,
        placements: &[TableCellPlacement<'_>],
        header: bool,
        budget: &mut Budget,
    ) -> Result<Self> {
        let mut column_offsets = Vec::with_capacity(table.columns.len() + 1);
        column_offsets.push(0.0);
        for column in &table.columns {
            let next = column_offsets.last().copied().unwrap_or(0.0) + column.width;
            if !next.is_finite() {
                return Err(fail("authored table column offset overflow"));
            }
            column_offsets.push(next);
        }
        let mut cells = Vec::with_capacity(placements.len());
        let padding = table.style.padding;
        let mut minimum_height = padding * 2.0 + table.body_style.size;
        for placement in placements {
            crate::cancel::check_current_cancel("authored table cell preparation")?;
            let source = placement.source;
            let columns = placement.columns.clone();
            let source_cell = source.map(|(index, _)| index);
            let cell = source.map(|(_, cell)| cell);
            let column_start = columns.start;
            let column_span = columns.len();
            let left = column_offsets[column_start];
            let width = column_offsets[columns.end] - left;
            if column_span == 0 || !left.is_finite() || !width.is_finite() || width <= 0.0 {
                return Err(fail("invalid authored table cell span geometry"));
            }
            let text = cell.map_or("", |cell| cell.text.as_str());
            budget.cell(text.len())?;
            let style = table.cell_style(cell, header);
            let line_height = table.style.paragraph.line_height_points(style.size)?;
            let lines = layout::prepare_with_tabs(
                page,
                text,
                width - padding * 2.0,
                &style,
                &table.style.paragraph.tab_stops,
            )?;
            budget.lines(lines.len())?;
            let mut heights = Vec::with_capacity(lines.len() + 1);
            let mut bytes = Vec::with_capacity(lines.len() + 1);
            heights.push(0.0f64);
            bytes.push(0usize);
            for line in &lines {
                crate::cancel::check_current_cancel("authored table line index")?;
                let next = heights.last().unwrap() + line.occupied_height(line_height);
                let end = bytes
                    .last()
                    .unwrap()
                    .checked_add(line.logical.len())
                    .ok_or_else(|| fail("authored cell source range overflow"))?;
                if !next.is_finite() || next <= *heights.last().unwrap() {
                    return Err(fail("invalid authored cell line heights"));
                }
                if text.get(*bytes.last().unwrap()..end) != Some(line.logical.as_str()) {
                    return Err(fail("authored cell plan lost logical source ownership"));
                }
                heights.push(next);
                bytes.push(end);
            }
            if bytes.last().copied() != Some(text.len()) {
                return Err(fail("authored cell plan did not consume its source"));
            }
            if lines.is_empty() {
                minimum_height = minimum_height.max(line_height + padding * 2.0);
            }
            let fill = cell
                .and_then(|cell| cell.background.clone())
                .or_else(|| header.then(|| table.style.header_fill.clone()))
                .or_else(|| table.style.row_fill.clone())
                .unwrap_or_else(|| Color::device_gray(1.0));
            cells.push(Cell {
                source_cell,
                column_start,
                column_span,
                row_start: placement.row_start,
                row_span: placement.row_span,
                left,
                width,
                align: cell
                    .and_then(|c| c.align)
                    .unwrap_or(table.columns[column_start].align),
                style,
                typed: cell.is_some_and(|cell| cell.typed_id.is_some()),
                fill,
                lines,
                heights,
                bytes,
            });
        }
        if !minimum_height.is_finite() || minimum_height <= 0.0 {
            return Err(fail("invalid authored row minimum"));
        }
        Ok(Self {
            cells,
            padding,
            minimum_height,
            border: table.style.border_color.clone(),
            line_width: table.style.line_width,
        })
    }

    fn validate_cursor(&self, starts: &[usize]) -> Result<()> {
        if starts.len() != self.cells.len()
            || self
                .cells
                .iter()
                .zip(starts)
                .any(|(c, &s)| s > c.lines.len())
        {
            return Err(fail("invalid authored row continuation cursor"));
        }
        Ok(())
    }

    pub(super) fn remaining_height(&self, starts: &[usize]) -> Result<f64> {
        self.validate_cursor(starts)?;
        let mut height = self.minimum_height;
        for (cell, &start) in self.cells.iter().zip(starts) {
            height =
                height.max(cell.heights.last().unwrap() - cell.heights[start] + self.padding * 2.0);
        }
        if !height.is_finite() || height <= 0.0 {
            return Err(fail("authored row height overflow"));
        }
        Ok(height)
    }

    pub(super) fn whole(&self, starts: &[usize]) -> Result<Fragment> {
        let height = self.remaining_height(starts)?;
        Ok(Fragment {
            height,
            ranges: self
                .cells
                .iter()
                .zip(starts)
                .map(|(c, &s)| s..c.lines.len())
                .collect(),
            complete: true,
        })
    }

    fn continuations(&self, capacity: f64, policy: TableRowSplitPolicy) -> Result<Continuations> {
        policy.validate()?;
        let mut previous = Vec::new();
        if let TableRowSplitPolicy::Lines {
            min_fragment_lines,
            min_final_lines,
        } = policy
        {
            let inner = capacity - self.padding * 2.0;
            for cell in &self.cells {
                let n = cell.lines.len();
                let mut viable = vec![false; n + 1];
                let mut next = vec![usize::MAX; n + 1];
                viable[n] = true;
                next[n] = n;
                // Backward feasibility prevents a greedy prefix from stranding
                // a later tall line beside an under-minimum final fragment.
                // Prefix-height and nearest-viable queries keep this O(n log n).
                for start in (0..n).rev() {
                    if start % 256 == 0 {
                        crate::cancel::check_current_cancel("authored continuation feasibility")?;
                    }
                    let fits = cell.heights[start + 1..]
                        .partition_point(|h| *h - cell.heights[start] <= inner + EPS);
                    let end = start + fits;
                    if end == n && (start == 0 || n - start >= min_final_lines) {
                        viable[start] = true;
                    } else {
                        let lo = start.saturating_add(min_fragment_lines);
                        let hi = end.min(n.saturating_sub(min_final_lines));
                        viable[start] = lo <= hi && lo <= n && next[lo] <= hi;
                    }
                    next[start] = if viable[start] {
                        start
                    } else {
                        next[start + 1]
                    };
                }
                if !viable[0] || capacity + EPS < self.minimum_height {
                    return Err(WellfriendError::UnsupportedFeature("authored cell has no complete pagination satisfying line minima and page capacity".into()));
                }
                let mut last = usize::MAX;
                previous.push(
                    viable
                        .into_iter()
                        .enumerate()
                        .map(|(index, valid)| {
                            if valid {
                                last = index;
                            }
                            last
                        })
                        .collect(),
                );
            }
        }
        Ok(Continuations { policy, previous })
    }

    fn fragment(
        &self,
        starts: &[usize],
        available: f64,
        continuation: &Continuations,
    ) -> Result<Option<Fragment>> {
        self.validate_cursor(starts)?;
        if !available.is_finite() {
            return Err(fail("invalid authored fragment capacity"));
        }
        if available + EPS < self.minimum_height {
            return Ok(None);
        }
        let whole = self.whole(starts)?;
        if whole.height <= available + EPS {
            return Ok(Some(whole));
        }
        let TableRowSplitPolicy::Lines {
            min_fragment_lines,
            min_final_lines,
        } = continuation.policy
        else {
            return Ok(None);
        };
        let inner = available - self.padding * 2.0;
        let mut ranges = Vec::with_capacity(self.cells.len());
        let mut height = self.minimum_height;
        let mut progress = false;
        for (index, (cell, &start)) in self.cells.iter().zip(starts).enumerate() {
            crate::cancel::check_current_cancel("authored row fragment selection")?;
            let remaining = cell.lines.len() - start;
            if remaining == 0 {
                ranges.push(start..start);
                continue;
            }
            let prefix = cell.heights[start];
            let count = cell.heights[start + 1..].partition_point(|h| *h - prefix <= inner + EPS);
            let mut take = count;
            if take < remaining {
                let bound = (start + take).min(cell.lines.len().saturating_sub(min_final_lines));
                let end = continuation.previous[index][bound];
                if end == usize::MAX || end < start.saturating_add(min_fragment_lines) {
                    return Ok(None);
                }
                take = end - start;
            }
            if take == 0 {
                return Ok(None);
            }
            let end = start + take;
            height = height.max(cell.heights[end] - prefix + self.padding * 2.0);
            ranges.push(start..end);
            progress = true;
        }
        if !progress || height > available + EPS {
            return Ok(None);
        }
        Ok(Some(Fragment {
            height,
            ranges,
            complete: false,
        }))
    }

    fn info(
        &self,
        fragment: &Fragment,
        page: usize,
        row: Option<usize>,
        top: f64,
        repeated_header: bool,
    ) -> TableFragmentInfo {
        TableFragmentInfo {
            page,
            row,
            repeated_header,
            top,
            height: fragment.height,
            cell_utf8_ranges: self
                .cells
                .iter()
                .zip(&fragment.ranges)
                .map(|(c, r)| [c.bytes[r.start], c.bytes[r.end]])
                .collect(),
            cells: self
                .cells
                .iter()
                .zip(&fragment.ranges)
                .map(|(cell, range)| TableCellFragmentInfo {
                    source_cell: cell.source_cell,
                    column_start: cell.column_start,
                    column_span: cell.column_span,
                    row_start: row.unwrap_or(cell.row_start),
                    row_span: cell.row_span,
                    utf8_range: [cell.bytes[range.start], cell.bytes[range.end]],
                })
                .collect(),
            continues: !fragment.complete,
        }
    }

    pub(super) fn render(
        &self,
        page: &mut PdfPageBuilder,
        x: f64,
        top: f64,
        fragment: &Fragment,
        artifact: bool,
        paint_owner: Option<(&str, usize)>,
        cell_structures: Option<&[u64]>,
    ) -> Result<()> {
        if fragment.ranges.len() != self.cells.len()
            || ![x, top, fragment.height, top - fragment.height]
                .iter()
                .all(|v| v.is_finite())
            || fragment.height <= 0.0
        {
            return Err(fail("invalid authored row fragment geometry"));
        }
        if let Some(cell_structures) = cell_structures {
            if artifact || cell_structures.len() != self.cells.len() {
                return Err(fail("invalid authored table-cell structure plan"));
            }
        }
        let mut commands = Vec::new();
        if artifact {
            commands.push(PageCommand::BeginArtifact);
        }
        for (cell_index, (cell, range)) in self.cells.iter().zip(&fragment.ranges).enumerate() {
            crate::cancel::check_current_cancel("authored fragment emission")?;
            let left = x + cell.left;
            if range.start > range.end
                || range.end > cell.lines.len()
                || !(left + cell.width).is_finite()
                || cell.heights[range.end] - cell.heights[range.start] + self.padding * 2.0
                    > fragment.height + EPS
            {
                return Err(fail(
                    "authored fragment exceeds declared geometry or cell source",
                ));
            }
            if !artifact {
                commands.push(if let Some((table, row)) = paint_owner {
                    PageCommand::BeginOwnedTableCellArtifact {
                        table: table.to_string(),
                        row,
                        column: cell.column_start,
                        row_span: cell.row_span,
                        column_span: cell.column_span,
                    }
                } else {
                    PageCommand::BeginArtifact
                });
            }
            commands.push(PageCommand::Rect {
                x: left,
                y: top - fragment.height,
                width: cell.width,
                height: fragment.height,
                style: GraphicsStyle::fill_stroke(
                    cell.fill.clone(),
                    self.border.clone(),
                    self.line_width,
                ),
            });
            if !artifact {
                commands.push(PageCommand::EndArtifact);
            }
            let mut cursor = top - self.padding;
            let content_region = [
                left + self.padding,
                top - fragment.height + self.padding,
                left + cell.width - self.padding,
                top - self.padding,
            ];
            for index in range.clone() {
                crate::cancel::check_current_cancel("authored continued cell line")?;
                let line = &cell.lines[index];
                let origin = line.aligned_x(
                    left + self.padding,
                    cell.width - self.padding * 2.0,
                    cell.align,
                );
                if let Some(cell_structures) = cell_structures {
                    commands.push(begin_cell_structure(
                        cell_structures[cell_index],
                        cell.typed,
                        content_region,
                    ));
                }
                commands.push(line.command(origin, cursor - line.metrics.ascent, &cell.style)?);
                if let Some(cell_structures) = cell_structures {
                    commands.push(PageCommand::EndStructure(cell_structures[cell_index]));
                }
                cursor -= cell.heights[index + 1] - cell.heights[index];
            }
            if range.is_empty() && cell.typed && cell.bytes.last().copied() == Some(0) {
                if let Some(cell_structures) = cell_structures {
                    commands.push(begin_cell_structure(
                        cell_structures[cell_index],
                        cell.typed,
                        content_region,
                    ));
                    commands.push(empty_owned_cell_command(
                        left + self.padding,
                        cursor - cell.style.size,
                        &cell.style,
                    ));
                    commands.push(PageCommand::EndStructure(cell_structures[cell_index]));
                }
            }
            if cursor < top - fragment.height + self.padding - EPS {
                return Err(fail("authored cell escaped fragment bounds"));
            }
        }
        if artifact {
            commands.push(PageCommand::EndArtifact);
        }
        page.commands.extend(commands);
        Ok(())
    }

    fn render_spanned(
        &self,
        page: &mut PdfPageBuilder,
        x: f64,
        table_top: f64,
        prefix: &[f64],
        artifact: bool,
        paint_owner: Option<&str>,
        cell_structures: Option<&[u64]>,
    ) -> Result<()> {
        if let Some(cell_structures) = cell_structures {
            if artifact || cell_structures.len() != self.cells.len() {
                return Err(fail("invalid authored spanned-cell structure plan"));
            }
        }
        let mut commands = Vec::new();
        if artifact {
            commands.push(PageCommand::BeginArtifact);
        }
        for (cell_index, cell) in self.cells.iter().enumerate() {
            let end = cell
                .row_start
                .checked_add(cell.row_span)
                .ok_or_else(|| fail("authored row span overflow during painting"))?;
            if end >= prefix.len() || cell.row_start >= end {
                return Err(fail("authored row span escaped prepared grid"));
            }
            let top = table_top - prefix[cell.row_start];
            let height = prefix[end] - prefix[cell.row_start];
            let left = x + cell.left;
            let content_height = cell.heights.last().copied().unwrap_or(0.0);
            if ![top, height, left, left + cell.width]
                .iter()
                .all(|value| value.is_finite())
                || height <= 0.0
                || content_height + self.padding * 2.0 > height + EPS
            {
                return Err(fail("authored spanned cell exceeds solved geometry"));
            }
            if !artifact {
                commands.push(if let Some(table) = paint_owner {
                    PageCommand::BeginOwnedTableCellArtifact {
                        table: table.to_string(),
                        row: cell.row_start,
                        column: cell.column_start,
                        row_span: cell.row_span,
                        column_span: cell.column_span,
                    }
                } else {
                    PageCommand::BeginArtifact
                });
            }
            commands.push(PageCommand::Rect {
                x: left,
                y: top - height,
                width: cell.width,
                height,
                style: GraphicsStyle::fill_stroke(
                    cell.fill.clone(),
                    self.border.clone(),
                    self.line_width,
                ),
            });
            if !artifact {
                commands.push(PageCommand::EndArtifact);
            }
            let mut cursor = top - self.padding;
            let content_region = [
                left + self.padding,
                top - height + self.padding,
                left + cell.width - self.padding,
                top - self.padding,
            ];
            for (index, line) in cell.lines.iter().enumerate() {
                let origin = line.aligned_x(
                    left + self.padding,
                    cell.width - self.padding * 2.0,
                    cell.align,
                );
                if let Some(cell_structures) = cell_structures {
                    commands.push(begin_cell_structure(
                        cell_structures[cell_index],
                        cell.typed,
                        content_region,
                    ));
                }
                commands.push(line.command(origin, cursor - line.metrics.ascent, &cell.style)?);
                if let Some(cell_structures) = cell_structures {
                    commands.push(PageCommand::EndStructure(cell_structures[cell_index]));
                }
                cursor -= cell.heights[index + 1] - cell.heights[index];
            }
            if cell.lines.is_empty() && cell.typed {
                if let Some(cell_structures) = cell_structures {
                    commands.push(begin_cell_structure(
                        cell_structures[cell_index],
                        cell.typed,
                        content_region,
                    ));
                    commands.push(empty_owned_cell_command(
                        left + self.padding,
                        cursor - cell.style.size,
                        &cell.style,
                    ));
                    commands.push(PageCommand::EndStructure(cell_structures[cell_index]));
                }
            }
            if cursor < top - height + self.padding - EPS {
                return Err(fail(
                    "authored spanned cell text escaped its solved rectangle",
                ));
            }
        }
        if artifact {
            commands.push(PageCommand::EndArtifact);
        }
        page.commands.extend(commands);
        Ok(())
    }
}

impl PreparedGrid {
    fn new(table: &TableBuilder, page: &PdfPageBuilder, budget: &mut Budget) -> Result<Self> {
        let placements = table.body_cell_placements()?;
        let mut rows = Vec::with_capacity(placements.len());
        for row in &placements {
            rows.push(PreparedRow::new_with_placements(
                table, page, row, false, budget,
            )?);
        }
        let mut endings = vec![Vec::<(usize, f64)>::new(); rows.len() + 1];
        for (row_index, row) in rows.iter().enumerate() {
            for cell in &row.cells {
                if cell.row_start != row_index {
                    return Err(fail("authored span origin disagrees with prepared row"));
                }
                let end = row_index.checked_add(cell.row_span).ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored row-span constraint overflow".into())
                })?;
                if end > rows.len() {
                    return Err(fail("authored row-span constraint escaped body grid"));
                }
                endings[end].push((
                    row_index,
                    cell.heights.last().copied().unwrap_or(0.0) + row.padding * 2.0,
                ));
            }
        }
        let mut prefix = vec![0.0f64; rows.len() + 1];
        for end in 1..prefix.len() {
            prefix[end] = prefix[end - 1] + rows[end - 1].minimum_height;
            for &(start, required) in &endings[end] {
                prefix[end] = prefix[end].max(prefix[start] + required);
            }
            if !prefix[end].is_finite() || prefix[end] <= prefix[end - 1] {
                return Err(fail("authored row-span height solver overflow"));
            }
        }
        Ok(Self { rows, prefix })
    }

    fn height(&self, rows: Range<usize>) -> Result<f64> {
        if rows.start >= rows.end || rows.end >= self.prefix.len() {
            return Err(fail("invalid authored spanned-row group"));
        }
        Ok(self.prefix[rows.end] - self.prefix[rows.start])
    }

    fn groups(&self) -> Vec<Range<usize>> {
        let mut crossed = vec![false; self.rows.len() + 1];
        for row in &self.rows {
            for cell in &row.cells {
                for boundary in cell.row_start + 1..cell.row_start + cell.row_span {
                    crossed[boundary] = true;
                }
            }
        }
        let mut groups = Vec::new();
        let mut start = 0usize;
        for end in 1..=self.rows.len() {
            if end == self.rows.len() || !crossed[end] {
                groups.push(start..end);
                start = end;
            }
        }
        groups
    }

    fn safe_cut(
        &self,
        rows: Range<usize>,
        start: f64,
        available: f64,
        policy: TableRowSplitPolicy,
    ) -> Result<Option<f64>> {
        policy.validate()?;
        let group_start = self.prefix[rows.start];
        let group_end = self.prefix[rows.end];
        if ![start, available, group_start, group_end]
            .iter()
            .all(|value| value.is_finite())
            || start < group_start - EPS
            || start >= group_end - EPS
            || available <= EPS
        {
            return Err(fail("invalid authored row-span continuation geometry"));
        }
        let target = (start + available).min(group_end);
        if target >= group_end - EPS {
            return Ok(Some(group_end));
        }
        if policy == TableRowSplitPolicy::KeepTogether {
            return Ok(None);
        }
        let TableRowSplitPolicy::Lines {
            min_fragment_lines,
            min_final_lines,
        } = policy
        else {
            unreachable!()
        };
        let mut candidates = vec![target];
        candidates.extend(
            self.prefix[rows.start + 1..rows.end]
                .iter()
                .copied()
                .filter(|cut| *cut > start + EPS && *cut <= target + EPS),
        );
        for row in &self.rows[rows.clone()] {
            for cell in &row.cells {
                let cell_top = self.prefix[cell.row_start] + row.padding;
                for height in &cell.heights {
                    let boundary = cell_top + *height;
                    if boundary > start + EPS && boundary <= target + EPS {
                        candidates.push(boundary);
                    }
                }
            }
        }
        candidates.sort_by(f64::total_cmp);
        candidates.dedup_by(|left, right| (*left - *right).abs() <= EPS);
        for cut in candidates.into_iter().rev() {
            if cut <= start + EPS || cut > target + EPS {
                continue;
            }
            let mut safe = true;
            let completes_row = self.prefix[rows.start + 1..=rows.end]
                .iter()
                .any(|boundary| (*boundary - cut).abs() <= EPS);
            let mut consumes_line = false;
            for row in &self.rows[rows.clone()] {
                for cell in &row.cells {
                    let cell_top = self.prefix[cell.row_start] + row.padding;
                    let before_start = cell
                        .heights
                        .iter()
                        .skip(1)
                        .take_while(|height| cell_top + **height <= start + EPS)
                        .count();
                    let before_cut = cell
                        .heights
                        .iter()
                        .skip(1)
                        .take_while(|height| cell_top + **height <= cut + EPS)
                        .count();
                    if cell.heights.windows(2).any(|window| {
                        let line_top = cell_top + window[0];
                        let line_bottom = cell_top + window[1];
                        line_top < cut - EPS && line_bottom > cut + EPS
                    }) {
                        safe = false;
                        break;
                    }
                    let taken = before_cut.saturating_sub(before_start);
                    let remaining = cell.lines.len().saturating_sub(before_cut);
                    consumes_line |= taken > 0;
                    if taken > 0
                        && remaining > 0
                        && (taken < min_fragment_lines || remaining < min_final_lines)
                    {
                        safe = false;
                        break;
                    }
                }
                if !safe {
                    break;
                }
            }
            // A geometric cut inside cell padding is not forward progress: it
            // paints an empty fragment, leaves all shaped lines behind and can
            // strand a row-span group on the preceding page. Only accept a cut
            // that consumes at least one complete shaped line or lands on an
            // actual retained row boundary (which also permits empty rows).
            if safe && (consumes_line || completes_row) {
                return Ok(Some(cut));
            }
        }
        Ok(None)
    }

    fn cell_line_range(
        &self,
        row: &PreparedRow,
        cell: &Cell,
        start: f64,
        end: f64,
    ) -> Range<usize> {
        let text_top = self.prefix[cell.row_start] + row.padding;
        let before = |position: f64| {
            cell.heights
                .iter()
                .skip(1)
                .take_while(|height| text_top + **height <= position + EPS)
                .count()
        };
        before(start)..before(end)
    }

    fn segment_rows(
        &self,
        rows: Range<usize>,
        start: f64,
        end: f64,
    ) -> Result<Vec<GridSegmentRow>> {
        if start >= end - EPS
            || start < self.prefix[rows.start] - EPS
            || end > self.prefix[rows.end] + EPS
        {
            return Err(fail("invalid authored row-span segment"));
        }
        let mut result = Vec::new();
        for row_index in rows {
            let row = &self.rows[row_index];
            let mut union_start = f64::INFINITY;
            let mut union_end = f64::NEG_INFINITY;
            let mut intersects = false;
            let mut ranges = Vec::with_capacity(row.cells.len());
            let mut continues = false;
            for cell in &row.cells {
                let cell_start = self.prefix[cell.row_start];
                let cell_end = self.prefix[cell.row_start + cell.row_span];
                let intersection_start = cell_start.max(start);
                let intersection_end = cell_end.min(end);
                if intersection_end > intersection_start + EPS {
                    intersects = true;
                    union_start = union_start.min(intersection_start);
                    union_end = union_end.max(intersection_end);
                }
                let range = self.cell_line_range(row, cell, start, end);
                continues |= range.end < cell.lines.len() || cell_end > end + EPS;
                ranges.push(range);
            }
            if intersects {
                result.push(GridSegmentRow {
                    row: row_index,
                    top_offset: union_start - start,
                    height: union_end - union_start,
                    ranges,
                    continues,
                });
            }
        }
        Ok(result)
    }

    fn render_segment(
        &self,
        page: &mut PdfPageBuilder,
        x: f64,
        top: f64,
        rows: Range<usize>,
        start: f64,
        end: f64,
        paint_owner: Option<&str>,
        structures: Option<&[Vec<u64>]>,
    ) -> Result<Vec<GridSegmentRow>> {
        let segment_rows = self.segment_rows(rows.clone(), start, end)?;
        let mut commands = Vec::new();
        for info in &segment_rows {
            let row = &self.rows[info.row];
            let owners = structures.map(|plans| plans[info.row].as_slice());
            if owners.is_some_and(|owners| owners.len() != row.cells.len()) {
                return Err(fail("invalid authored row-span structure plan"));
            }
            for (cell_index, (cell, range)) in row.cells.iter().zip(&info.ranges).enumerate() {
                let cell_start = self.prefix[cell.row_start];
                let cell_end = self.prefix[cell.row_start + cell.row_span];
                let intersection_start = cell_start.max(start);
                let intersection_end = cell_end.min(end);
                if intersection_end <= intersection_start + EPS {
                    continue;
                }
                let left = x + cell.left;
                let rect_top = top - (intersection_start - start);
                let height = intersection_end - intersection_start;
                if ![left, rect_top, height, left + cell.width]
                    .iter()
                    .all(|value| value.is_finite())
                    || height <= 0.0
                {
                    return Err(fail("invalid authored row-span fragment geometry"));
                }
                commands.push(if let Some(table) = paint_owner {
                    PageCommand::BeginOwnedTableCellArtifact {
                        table: table.to_string(),
                        row: cell.row_start,
                        column: cell.column_start,
                        row_span: cell.row_span,
                        column_span: cell.column_span,
                    }
                } else {
                    PageCommand::BeginArtifact
                });
                commands.push(PageCommand::Rect {
                    x: left,
                    y: rect_top - height,
                    width: cell.width,
                    height,
                    style: GraphicsStyle::fill_stroke(
                        cell.fill.clone(),
                        row.border.clone(),
                        row.line_width,
                    ),
                });
                commands.push(PageCommand::EndArtifact);
                let content_region = [
                    left + row.padding,
                    rect_top - height + row.padding,
                    left + cell.width - row.padding,
                    rect_top - row.padding,
                ];
                for line_index in range.clone() {
                    let line = &cell.lines[line_index];
                    let line_top = cell_start + row.padding + cell.heights[line_index];
                    let baseline = top - (line_top - start) - line.metrics.ascent;
                    if baseline - line.metrics.descent < rect_top - height - EPS {
                        return Err(fail("authored row-span line escaped its page fragment"));
                    }
                    let origin = line.aligned_x(
                        left + row.padding,
                        cell.width - row.padding * 2.0,
                        cell.align,
                    );
                    if let Some(owners) = owners {
                        commands.push(begin_cell_structure(
                            owners[cell_index],
                            cell.typed,
                            content_region,
                        ));
                    }
                    commands.push(line.command(origin, baseline, &cell.style)?);
                    if let Some(owners) = owners {
                        commands.push(PageCommand::EndStructure(owners[cell_index]));
                    }
                }
                if range.is_empty()
                    && cell.typed
                    && cell.bytes.last().copied() == Some(0)
                    && intersection_start <= cell_start + EPS
                {
                    if let Some(owners) = owners {
                        commands.push(begin_cell_structure(
                            owners[cell_index],
                            cell.typed,
                            content_region,
                        ));
                        commands.push(empty_owned_cell_command(
                            left + row.padding,
                            rect_top - row.padding - cell.style.size,
                            &cell.style,
                        ));
                        commands.push(PageCommand::EndStructure(owners[cell_index]));
                    }
                }
            }
        }
        page.commands.extend(commands);
        Ok(segment_rows)
    }

    fn segment_info(
        &self,
        segment: &GridSegmentRow,
        page: usize,
        page_top: f64,
    ) -> TableFragmentInfo {
        let row = &self.rows[segment.row];
        let cells = row
            .cells
            .iter()
            .zip(&segment.ranges)
            .map(|(cell, range)| TableCellFragmentInfo {
                source_cell: cell.source_cell,
                column_start: cell.column_start,
                column_span: cell.column_span,
                row_start: cell.row_start,
                row_span: cell.row_span,
                utf8_range: [cell.bytes[range.start], cell.bytes[range.end]],
            })
            .collect::<Vec<_>>();
        TableFragmentInfo {
            page,
            row: Some(segment.row),
            repeated_header: false,
            top: page_top - segment.top_offset,
            height: segment.height,
            cell_utf8_ranges: cells.iter().map(|cell| cell.utf8_range).collect(),
            cells,
            continues: segment.continues,
        }
    }

    fn render_group(
        &self,
        page: &mut PdfPageBuilder,
        x: f64,
        top: f64,
        rows: Range<usize>,
        structures: Option<&[Vec<u64>]>,
    ) -> Result<()> {
        let offset = self.prefix[rows.start];
        for row_index in rows.clone() {
            let row = &self.rows[row_index];
            // Spans never cross a group boundary, so translate global row
            // origins by shifting the conceptual full-table top.
            let row_top = top + offset;
            row.render_spanned(
                page,
                x,
                row_top,
                &self.prefix,
                false,
                None,
                structures.map(|plans| plans[row_index].as_slice()),
            )?;
        }
        Ok(())
    }
}

fn empty_owned_cell_command(x: f64, y: f64, style: &TextStyle) -> PageCommand {
    PageCommand::Text {
        text: String::new(),
        x,
        y,
        style: style.clone(),
        bidi: None,
        logical_text: None,
        suppress_actual_text: false,
        font_asset: None,
        shaped: None,
    }
}

fn begin_cell_structure(element: u64, typed: bool, region: [f64; 4]) -> PageCommand {
    if typed {
        PageCommand::BeginTypedCellStructure { element, region }
    } else {
        PageCommand::BeginStructure(element)
    }
}

pub(super) fn draw_on_page(
    table: &TableBuilder,
    page: &mut PdfPageBuilder,
    x: f64,
    top: f64,
) -> Result<f64> {
    crate::cancel::check_current_cancel("authored table single-page transaction")?;
    let (resolved, _) = table.resolved_typed_values()?;
    let table = &resolved;
    if table.rows.iter().any(|row| row.page_break_before.is_some()) {
        return Err(WellfriendError::UnsupportedFeature(
            "a page-local table cannot own a row page break; use FlowDocument::add_table".into(),
        ));
    }
    if !x.is_finite() || !top.is_finite() {
        return Err(fail("invalid authored table anchor"));
    }
    let original = page.commands.len();
    let result = (|| {
        let mut budget = Budget::default();
        let mut cursor = top;
        if let Some(caption) = PreparedCaption::new(table, page, &mut budget)? {
            caption.render(page, table, x, cursor, None)?;
            cursor -= caption.height;
        }
        if table.has_row_spans() {
            if let Some(header) = &table.header {
                let prepared = PreparedRow::new(table, page, header, true, &mut budget)?;
                let whole = prepared.whole(&vec![0; prepared.cells.len()])?;
                budget.fragment(&whole)?;
                prepared.render(page, x, cursor, &whole, false, None, None)?;
                cursor -= whole.height;
            }
            let grid = PreparedGrid::new(table, page, &mut budget)?;
            if !grid.rows.is_empty() {
                let body_height = grid.height(0..grid.rows.len())?;
                for row in &grid.rows {
                    let whole = row.whole(&vec![0; row.cells.len()])?;
                    budget.fragment(&whole)?;
                }
                grid.render_group(page, x, cursor, 0..grid.rows.len(), None)?;
                cursor -= body_height;
            }
            let consumed = top - cursor;
            if !consumed.is_finite() {
                return Err(fail("authored spanned-table extent overflow"));
            }
            return Ok(consumed);
        }
        for (row, header) in table
            .header
            .iter()
            .map(|row| (row, true))
            .chain(table.rows.iter().map(|row| (row, false)))
        {
            let prepared = PreparedRow::new(table, page, row, header, &mut budget)?;
            let whole = prepared.whole(&vec![0; prepared.cells.len()])?;
            budget.fragment(&whole)?;
            prepared.render(page, x, cursor, &whole, false, None, None)?;
            cursor -= whole.height;
        }
        let consumed = top - cursor;
        if !consumed.is_finite() {
            return Err(fail("authored table extent overflow"));
        }
        Ok(consumed)
    })();
    if result.is_err() {
        page.commands.truncate(original);
    }
    result
}

fn fresh_page(flow: &mut FlowDocument, added: &mut usize) -> Result<()> {
    crate::cancel::check_current_cancel("authored table page allocation")?;
    if *added >= MAX_FRAGMENTS {
        return Err(WellfriendError::ResourceLimit(
            "authored table page budget".into(),
        ));
    }
    flow.add_page_break();
    *added += 1;
    Ok(())
}

fn forced_row_page(
    flow: &mut FlowDocument,
    row: usize,
    policy: FlowPageBreak,
    report: &mut TableFlowReport,
) -> Result<()> {
    crate::cancel::check_current_cancel("authored table forced row page")?;
    let from_page = flow.current_page + 1;
    let before = flow.builder.pages.len();
    let destination = before.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("authored table page index overflow".into())
    })?;
    let requested = match policy {
        FlowPageBreak::NextPage => 1,
        FlowPageBreak::NextOddPage if destination % 2 == 1 => 1,
        FlowPageBreak::NextEvenPage if destination % 2 == 0 => 1,
        FlowPageBreak::NextOddPage | FlowPageBreak::NextEvenPage => 2,
    };
    if report.added_pages.saturating_add(requested) > MAX_FRAGMENTS {
        return Err(WellfriendError::ResourceLimit(
            "authored table page budget".into(),
        ));
    }
    flow.add_page_break_to(policy);
    let added_pages = flow.builder.pages.len() - before;
    if added_pages != requested || flow.current_page < from_page {
        return Err(fail(
            "authored row page transition violated its physical policy",
        ));
    }
    report.added_pages += added_pages;
    report.page_breaks.push(TableRowPageBreakInfo {
        row,
        policy,
        from_page,
        to_page: flow.current_page + 1,
        added_pages,
    });
    Ok(())
}

fn append_spanned_flow(
    flow: &mut FlowDocument,
    table: &TableBuilder,
    evaluated_values: BTreeMap<String, String>,
) -> Result<TableFlowReport> {
    let width = flow.content_width()?;
    let page_top = flow.page_size.height - flow.margins.top;
    let usable = page_top - flow.margins.bottom;
    if flow.current_page >= flow.builder.pages.len()
        || ![page_top, usable, flow.cursor_y, flow.current_bottom()]
            .iter()
            .all(|value| value.is_finite())
        || usable <= 0.0
        || table.total_width() > width + EPS
    {
        return Err(fail("invalid authored spanned-table flow geometry"));
    }
    let structures = register_structure(flow, table)?;
    let mut budget = Budget::default();
    let caption = PreparedCaption::new(table, flow.current_page_ref(), &mut budget)?;
    let header = table
        .header
        .as_ref()
        .map(|row| PreparedRow::new(table, flow.current_page_ref(), row, true, &mut budget))
        .transpose()?;
    let header_fragment = header
        .as_ref()
        .map(|header| header.whole(&vec![0; header.cells.len()]))
        .transpose()?;
    let header_height = header_fragment
        .as_ref()
        .map_or(0.0, |fragment| fragment.height);
    let capacity = usable - header_height;
    if capacity <= 0.0 {
        return Err(WellfriendError::UnsupportedFeature(
            "authored table header leaves no spanned-row capacity".into(),
        ));
    }
    let grid = PreparedGrid::new(table, flow.current_page_ref(), &mut budget)?;
    let groups = grid.groups();
    if groups.is_empty() {
        return Err(fail("authored row-span table has no body group"));
    }
    let mut report = TableFlowReport {
        evaluated_values,
        ..TableFlowReport::default()
    };
    let mut first_break_consumed = false;
    if let Some(policy) = table.rows[groups[0].start].page_break_before {
        forced_row_page(flow, groups[0].start, policy, &mut report)?;
        first_break_consumed = true;
    }
    if let Some(caption) = &caption {
        let first = groups[0].clone();
        let available = usable - caption.height - header_height;
        if available <= EPS {
            return Err(WellfriendError::UnsupportedFeature(
                "authored table caption and header leave no row-span fragment capacity".into(),
            ));
        }
        let start = grid.prefix[first.start];
        let cut = grid
            .safe_cut(first, start, available, table.row_split_policy)?
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "authored table caption has no feasible first spanned-row fragment".into(),
                )
            })?;
        let required = caption.height + header_height + cut - start;
        if flow.cursor_y - required < flow.current_bottom() - EPS {
            fresh_page(flow, &mut report.added_pages)?;
        }
        let owner = structures.as_ref().and_then(|plan| plan.caption);
        if owner.is_none() {
            return Err(fail(
                "authored spanned-table caption lost its structure owner",
            ));
        }
        let cursor = flow.cursor_y;
        let left = flow.margins.left;
        caption.render(flow.current_page_mut(), table, left, cursor, owner)?;
        report.caption = Some(TableCaptionInfo {
            page: flow.current_page + 1,
            top: cursor,
            height: caption.height,
        });
        flow.cursor_y -= caption.height;
    }

    let mut header_pending = header.is_some();
    let mut header_emitted = false;
    for (group_index, group) in groups.iter().enumerate() {
        if let Some(policy) = table.rows[group.start]
            .page_break_before
            .filter(|_| !(group_index == 0 && first_break_consumed))
        {
            forced_row_page(flow, group.start, policy, &mut report)?;
            header_pending = header.is_some();
        }
        let group_end = grid.prefix[group.end];
        let mut position = grid.prefix[group.start];
        while position < group_end - EPS {
            let reserve = if header_pending { header_height } else { 0.0 };
            let available = flow.cursor_y - flow.current_bottom() - reserve;
            if available <= EPS {
                if available >= capacity - EPS {
                    return Err(WellfriendError::UnsupportedFeature(
                        "authored table header leaves no row-span continuation capacity".into(),
                    ));
                }
                fresh_page(flow, &mut report.added_pages)?;
                header_pending = header.is_some();
                continue;
            }
            let cut = grid.safe_cut(group.clone(), position, available, table.row_split_policy)?;
            let Some(cut) = cut else {
                if available >= capacity - EPS {
                    return Err(WellfriendError::UnsupportedFeature(
                        "authored row-span fragment has no safe shaped-line cut satisfying its line minima"
                            .into(),
                    ));
                }
                fresh_page(flow, &mut report.added_pages)?;
                header_pending = header.is_some();
                continue;
            };
            if header_pending {
                let header = header.as_ref().expect("pending authored table header");
                let fragment = header_fragment
                    .as_ref()
                    .expect("pending authored table header fragment");
                budget.fragment(fragment)?;
                let cursor = flow.cursor_y;
                let left = flow.margins.left;
                header.render(
                    flow.current_page_mut(),
                    left,
                    cursor,
                    fragment,
                    header_emitted,
                    None,
                    if header_emitted {
                        None
                    } else {
                        structures
                            .as_ref()
                            .and_then(|plan| plan.header_cells.as_deref())
                    },
                )?;
                report.fragments.push(header.info(
                    fragment,
                    flow.current_page + 1,
                    None,
                    cursor,
                    header_emitted,
                ));
                flow.cursor_y -= fragment.height;
                header_pending = false;
                header_emitted = true;
            }
            let top = flow.cursor_y;
            let left = flow.margins.left;
            let segment_rows = grid.render_segment(
                flow.current_page_mut(),
                left,
                top,
                group.clone(),
                position,
                cut,
                (!report.evaluated_values.is_empty())
                    .then_some(table.identity.as_deref())
                    .flatten(),
                structures.as_ref().map(|plan| plan.body_cells.as_slice()),
            )?;
            for segment in &segment_rows {
                budget.grid_fragment(&segment.ranges)?;
                report
                    .fragments
                    .push(grid.segment_info(segment, flow.current_page + 1, top));
            }
            flow.cursor_y -= cut - position;
            if flow.cursor_y < flow.current_bottom() - EPS {
                return Err(fail("authored row-span fragment escaped page bounds"));
            }
            position = cut;
            if position < group_end - EPS {
                fresh_page(flow, &mut report.added_pages)?;
                header_pending = header.is_some();
            }
        }
    }
    let pagination = retained_table_pagination(flow, table)?;
    retain_authored_typed_model(
        &mut flow.builder,
        table,
        &report.evaluated_values,
        pagination,
    )?;
    Ok(report)
}

pub(super) fn append_flow(
    flow: &mut FlowDocument,
    table: &TableBuilder,
) -> Result<TableFlowReport> {
    crate::cancel::check_current_cancel("authored table flow transaction")?;
    let (resolved, evaluated_values) = table.resolved_typed_values()?;
    let table = &resolved;
    if table.has_row_spans() {
        return append_spanned_flow(flow, table, evaluated_values);
    }
    let structures = register_structure(flow, table)?;
    let width = flow.content_width()?;
    let top = flow.page_size.height - flow.margins.top;
    let usable = top - flow.margins.bottom;
    let current_bottom = flow.current_bottom();
    if flow.current_page >= flow.builder.pages.len()
        || ![
            top,
            usable,
            flow.cursor_y,
            flow.margins.bottom,
            current_bottom,
        ]
        .iter()
        .all(|v| v.is_finite())
        || usable <= 0.0
        || flow.cursor_y > top + EPS
        || flow.cursor_y < current_bottom - EPS
    {
        return Err(fail("invalid authored table flow state"));
    }
    if table.total_width() > width + EPS {
        return Err(fail("authored table exceeds flow content width"));
    }
    let mut budget = Budget::default();
    let caption = PreparedCaption::new(table, flow.current_page_ref(), &mut budget)?;
    let header = table
        .header
        .as_ref()
        .map(|row| PreparedRow::new(table, flow.current_page_ref(), row, true, &mut budget))
        .transpose()?;
    let header_fragment = header
        .as_ref()
        .map(|header| header.whole(&vec![0; header.cells.len()]))
        .transpose()?;
    let header_height = header_fragment.as_ref().map_or(0.0, |h| h.height);
    let capacity = usable - header_height;
    if header_height > usable + EPS || (!table.rows.is_empty() && capacity <= 0.0) {
        return Err(WellfriendError::UnsupportedFeature(
            "authored table header leaves no body capacity".into(),
        ));
    }
    let mut report = TableFlowReport {
        evaluated_values,
        ..TableFlowReport::default()
    };
    let mut first_break_consumed = false;
    if let Some(caption) = &caption {
        if let Some(policy) = table.rows.first().and_then(|row| row.page_break_before) {
            forced_row_page(flow, 0, policy, &mut report)?;
            first_break_consumed = true;
        }
        let caption_height = caption.height;
        let required_body = if let Some(first) = table.rows.first() {
            let mut scratch = Budget::default();
            let row = PreparedRow::new(table, flow.current_page_ref(), first, false, &mut scratch)?;
            let starts = vec![0; row.cells.len()];
            let available = usable - caption_height - header_height;
            if available <= 0.0 {
                return Err(WellfriendError::UnsupportedFeature(
                    "authored table caption and header leave no first-row capacity".into(),
                ));
            }
            let remaining = row.remaining_height(&starts)?;
            match table.row_split_policy {
                TableRowSplitPolicy::KeepTogether if remaining > available + EPS => {
                    return Err(WellfriendError::UnsupportedFeature(
                        "authored table caption cannot stay with its keep-together first row"
                            .into(),
                    ));
                }
                TableRowSplitPolicy::KeepTogether => remaining,
                policy => {
                    let continuation = row.continuations(capacity, policy)?;
                    row.fragment(&starts, available, &continuation)?
                        .ok_or_else(|| {
                            WellfriendError::UnsupportedFeature(
                                "authored table caption has no feasible first-row fragment".into(),
                            )
                        })?
                        .height
                }
            }
        } else {
            0.0
        };
        let required = caption_height + header_height + required_body;
        if required > usable + EPS {
            return Err(WellfriendError::UnsupportedFeature(
                "authored table caption cannot stay with the first table fragment".into(),
            ));
        }
        if flow.cursor_y - required < flow.current_bottom() - EPS {
            fresh_page(flow, &mut report.added_pages)?;
        }
        let structure = structures.as_ref().and_then(|plan| plan.caption);
        if structure.is_none() {
            return Err(fail("authored table caption lost its structure owner"));
        }
        let left = flow.margins.left;
        let cursor = flow.cursor_y;
        caption.render(flow.current_page_mut(), table, left, cursor, structure)?;
        report.caption = Some(TableCaptionInfo {
            page: flow.current_page + 1,
            top: cursor,
            height: caption_height,
        });
        flow.cursor_y -= caption_height;
    }
    let mut header_pending = header.is_some();
    let mut header_emitted = false;
    if table.rows.is_empty() {
        if let (Some(header), Some(fragment)) = (&header, &header_fragment) {
            if flow.cursor_y - fragment.height < flow.current_bottom() - EPS {
                fresh_page(flow, &mut report.added_pages)?;
            }
            budget.fragment(fragment)?;
            let cursor = flow.cursor_y;
            let left = flow.margins.left;
            header.render(
                flow.current_page_mut(),
                left,
                cursor,
                fragment,
                false,
                None,
                structures
                    .as_ref()
                    .and_then(|plan| plan.header_cells.as_deref()),
            )?;
            report.fragments.push(header.info(
                fragment,
                flow.current_page + 1,
                None,
                cursor,
                false,
            ));
            flow.cursor_y -= fragment.height;
        }
        return Ok(report);
    }
    for (row_index, source) in table.rows.iter().enumerate() {
        if let Some(policy) = source
            .page_break_before
            .filter(|_| !(row_index == 0 && first_break_consumed))
        {
            forced_row_page(flow, row_index, policy, &mut report)?;
            header_pending = header.is_some();
        }
        let row = PreparedRow::new(table, flow.current_page_ref(), source, false, &mut budget)?;
        let continuation = row.continuations(capacity, table.row_split_policy)?;
        let mut starts = vec![0; row.cells.len()];
        loop {
            crate::cancel::check_current_cancel("authored table continuation")?;
            let available = flow.cursor_y
                - flow.current_bottom()
                - if header_pending { header_height } else { 0.0 };
            let remaining = row.remaining_height(&starts)?;
            // Keep rows whole when they fit a fresh frame; only oversized rows
            // use line fragmentation. Do not orphan an as-yet-unpainted header.
            let caption_first_fragment =
                caption.is_some() && row_index == 0 && starts.iter().all(|start| *start == 0);
            let chosen = if remaining > available + EPS
                && remaining <= capacity + EPS
                && !caption_first_fragment
            {
                None
            } else {
                row.fragment(&starts, available, &continuation)?
            };
            let Some(fragment) = chosen else {
                if available >= capacity - EPS {
                    return Err(WellfriendError::UnsupportedFeature("authored row cannot fit a permitted fragment with its line minima and repeated header".into()));
                }
                fresh_page(flow, &mut report.added_pages)?;
                header_pending = header.is_some();
                continue;
            };
            if header_pending {
                let (header, header_fragment) =
                    (header.as_ref().unwrap(), header_fragment.as_ref().unwrap());
                budget.fragment(header_fragment)?;
                let cursor = flow.cursor_y;
                let left = flow.margins.left;
                header.render(
                    flow.current_page_mut(),
                    left,
                    cursor,
                    header_fragment,
                    header_emitted,
                    None,
                    if header_emitted {
                        None
                    } else {
                        structures
                            .as_ref()
                            .and_then(|plan| plan.header_cells.as_deref())
                    },
                )?;
                report.fragments.push(header.info(
                    header_fragment,
                    flow.current_page + 1,
                    None,
                    cursor,
                    header_emitted,
                ));
                flow.cursor_y -= header_fragment.height;
                header_pending = false;
                header_emitted = true;
            }
            budget.fragment(&fragment)?;
            let cursor = flow.cursor_y;
            let left = flow.margins.left;
            row.render(
                flow.current_page_mut(),
                left,
                cursor,
                &fragment,
                false,
                (!report.evaluated_values.is_empty())
                    .then_some(table.identity.as_deref())
                    .flatten()
                    .map(|identity| (identity, row_index)),
                structures
                    .as_ref()
                    .map(|plan| plan.body_cells[row_index].as_slice()),
            )?;
            report.fragments.push(row.info(
                &fragment,
                flow.current_page + 1,
                Some(row_index),
                cursor,
                false,
            ));
            flow.cursor_y -= fragment.height;
            if flow.cursor_y < flow.current_bottom() - EPS {
                return Err(fail("authored table fragment escaped page bounds"));
            }
            if fragment.complete {
                break;
            }
            if starts
                .iter()
                .zip(&fragment.ranges)
                .all(|(&start, range)| start == range.end)
            {
                return Err(fail("authored row fragmentation made no progress"));
            }
            starts = fragment.ranges.iter().map(|range| range.end).collect();
            fresh_page(flow, &mut report.added_pages)?;
            header_pending = header.is_some();
        }
    }
    let pagination = retained_table_pagination(flow, table)?;
    retain_authored_typed_model(
        &mut flow.builder,
        table,
        &report.evaluated_values,
        pagination,
    )?;
    Ok(report)
}
