//! document subsystems source-linked tables, mathematics, OCR, annotations, forms, and XFA.
//!
//! This is deliberately an adapter over the canonical engines. Table, math, and
//! approved OCR text edits compile through text reflow source reflow; annotation
//! appearances compile through annotation/media redaction; form values compile through the
//! canonical form exchange/editor path; XFA inventory remains byte-preserving.

use crate::annotation_media_redaction::{
    export_annotation_xfdf, generate_annotation_appearances_pdf, import_annotation_xfdf_pdf,
    move_resize_annotation_pdf, parse_annotation_xfdf, AnnotationAppearanceOptions,
    AnnotationDeletePolicy, AnnotationXfdfImportOptions,
};
use crate::content::Color;
use crate::form_exchange::{apply_form_data_pdf, FormDataFormat};
use crate::text_reflow::{
    analyze_geometric_region, analyze_semantic_layout, apply_reflow_document, apply_reflow_region,
    undo_reflow_from_replay, GeometricReflowRequest,
};
use crate::writer::{rewrite_document_objects, OutputObject, PdfWriter, WriterMode};
use crate::xfa::{
    extract_xfa, xfa_flatten_pdf, xfa_inventory, xfa_runtime_report, XfaFlattenMode,
    XfaFlattenOptions, XfaLimits, XfaRuntimeOptions,
};
use crate::{
    interactive_report, AnnotationOptions, ContentEngine, EditMode, EditRectStyle, EditTextStyle,
    ImageRect, OverlayLayer, PdfDictionary, PdfDocument, PdfEditor, PdfObject, PdfReader, Result,
    WellfriendError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION: &str =
    "document_subsystems.tables-math-ocr-forms-annotations.v1";
const MAX_DOCUMENT_SUBSYSTEM_ANALYSIS_PAGES: usize = 2;
const MAX_DOCUMENT_SUBSYSTEM_SAMPLE_ITEMS: usize = 256;
const MAX_OCR_SOURCE_LINKS_PER_PAGE: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSubsystemsSubsystem {
    Table,
    Math,
    OcrSearchableLayer,
    OcrReconstruction,
    AnnotationAppearance,
    FormData,
    XfaPreservation,
}

/// Concrete document subsystems mutation requests.  These deliberately carry only
/// source identifiers and user-approved values; they never embed a separate
/// table, mathematical, OCR, annotation, or form document model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentSubsystemsAction {
    /// Replace the text in one detected source-linked table cell.  The table
    /// geometry is resolved again from the current snapshot before text reflow
    /// performs the actual source rewrite.
    TableEditCell {
        table_id: String,
        row: usize,
        col: usize,
        replacement_text: String,
    },
    /// Replaces a born-digital, source-linked mathematical expression in a
    /// resolved table cell.  This shares table-cell provenance and text reflow
    /// reflow with the canonical math review boundary.
    TableEditMathCell {
        table_id: String,
        row: usize,
        col: usize,
        replacement_text: String,
        approved: bool,
    },
    /// Move a resolved annotation with the real bounds of one table cell.
    /// The annotation's canonical annotation/media redaction appearance is regenerated; the
    /// table itself remains source-identical.
    TableMoveLinkedAnnotation {
        table_id: String,
        row: usize,
        col: usize,
        annotation_id: String,
    },
    /// Reposition source-linked cell text with a bounded text reflow alignment
    /// policy. The text itself is retained; no overlay alignment substitute is
    /// painted over the table cell.
    TableSetCellAlignment {
        table_id: String,
        row: usize,
        col: usize,
        alignment: String,
    },
    /// Reflow source-linked cell text inside explicit cell padding. Padding
    /// changes the actual usable text region; no replacement rectangle or
    /// hidden text is introduced.
    TableSetCellPadding {
        table_id: String,
        row: usize,
        col: usize,
        padding: [f64; 4],
    },
    /// Add a real stroked border at one resolved table-cell boundary. The
    /// operation appends canonical page-content instructions; it never covers
    /// cell text or substitutes a rasterized table.
    TableAddCellBorder {
        table_id: String,
        row: usize,
        col: usize,
        line_width: f64,
    },
    /// Add a real underlay fill at one resolved table-cell boundary. Existing
    /// text remains above the new path and is never covered or replaced.
    TableSetCellFill {
        table_id: String,
        row: usize,
        col: usize,
        color_rgb: [f64; 3],
        opacity: f64,
    },
    /// Append one simple, ruled row in explicitly unoccupied page space.  The
    /// new cells and borders are real page-content instructions; merged or
    /// ambiguous grids remain an exact refusal rather than a visual cover-up.
    TableAppendRow {
        table_id: String,
        values: Vec<String>,
        #[serde(default)]
        row_height: Option<f64>,
    },
    /// Append one simple, ruled column in explicitly unoccupied page space.
    /// Existing columns are not rescaled or covered; unsupported grid topology
    /// is rejected before source mutation.
    TableAppendColumn {
        table_id: String,
        values: Vec<String>,
        #[serde(default)]
        column_width: Option<f64>,
    },
    /// Replace a resolved formula source range after the caller approves the
    /// inferred mathematical structure.  The resulting source text is shaped
    /// by the editing transactions/33 path rather than painted as a replacement overlay.
    MathReplace {
        replacement_text: String,
    },
    /// Move or resize one resolved born-digital mathematical source region.
    /// The expression text is retained and rewritten through text reflow rather
    /// than flattened or painted over.
    MathMoveResize {
        bounds: [f64; 4],
    },
    /// Replace a single cell in a resolved born-digital bracket-matrix
    /// expression (`[[a,b];[c,d]]`).  The complete expression is then
    /// rewritten through the same shaped source-reflow path as `math_replace`.
    /// Other matrix notations remain an exact `math_structure_not_resolved`
    /// boundary rather than being flattened to plain text.
    MathEditMatrixCell {
        row: usize,
        col: usize,
        replacement_text: String,
    },
    /// Insert or delete one row/column of a resolved bracket-matrix source.
    /// Supported operations are `insert_row`, `delete_row`, `insert_column`,
    /// and `delete_column`; malformed or topology-changing notation is refused.
    MathEditMatrixStructure {
        operation: String,
        index: usize,
        #[serde(default)]
        values: Vec<String>,
    },
    /// Replace the numerator or denominator in a resolved single-slash
    /// born-digital fraction. The reconstructed source is routed through the
    /// canonical shaped mathematical reflow path.
    MathEditFractionPart {
        part: String,
        replacement_text: String,
    },
    /// Replace a resolved single source superscript or subscript while
    /// retaining the base expression and using shaped source reflow.
    MathEditScript {
        script_kind: String,
        replacement_text: String,
    },
    /// Replace the inner source of one resolved single-layer fenced expression
    /// while retaining its original delimiter pair.
    MathEditFencedInner {
        replacement_text: String,
    },
    /// Replace the radicand of a resolved born-digital radical source while
    /// retaining its radical construction notation.
    MathEditRadicand {
        replacement_text: String,
    },
    /// Correct an existing searchable OCR text range while preserving the
    /// source scan.  Creation of a new recognition layer still requires an
    /// injected canonical OCR provider and is refused when unavailable.
    OcrCorrectText {
        replacement_text: String,
    },
    /// Move a provenance-resolved searchable OCR source range without
    /// replacing the original scan. text reflow rewrites the text instruction at
    /// the reviewed target geometry and preserves the logical text value.
    OcrCorrectGeometry {
        bounds: [f64; 4],
    },
    /// Add an explicit provider-produced searchable text record to an
    /// image-only page. The original scan remains the visible page content;
    /// the canonical editor writes an invisible (`Tr 3`) text instruction.
    /// This bounded path accepts only exact ASCII text because the standard
    /// fallback font has no generated `/ToUnicode` CMap.
    OcrAddSearchableText {
        page: usize,
        text: String,
        rect: [f64; 4],
        font_size: f64,
        provider_id: String,
        #[serde(default)]
        provider_version: Option<String>,
        confidence: f64,
    },
    /// Add reviewed invisible OCR text and a source-linked URI annotation at
    /// the exact same scan-space geometry in one canonical transaction.
    OcrAddSearchableTextWithLink {
        page: usize,
        text: String,
        rect: [f64; 4],
        font_size: f64,
        provider_id: String,
        #[serde(default)]
        provider_version: Option<String>,
        confidence: f64,
        uri: String,
    },
    /// Add an atomic batch of provider-recognized words to one scanned page.
    /// Each word remains individually source-mapped in the transaction report
    /// and is serialized as invisible text so the scan is never painted over.
    OcrAddSearchableWords {
        page: usize,
        words: Vec<OcrSearchableWord>,
        provider_id: String,
        #[serde(default)]
        provider_version: Option<String>,
        #[serde(default)]
        language: Option<String>,
    },
    /// Add a canonical supported annotation through PdfEditor, then regenerate
    /// the viewer-independent appearance state through annotation/media redaction.
    AnnotationCreate {
        page: usize,
        subtype: String,
        rect: [f64; 4],
        #[serde(default)]
        contents: String,
        #[serde(default)]
        uri: Option<String>,
    },
    /// Update contents of one page-local annotation index and regenerate its
    /// appearance.  The index is verified against the current snapshot.
    AnnotationEditContents {
        page: usize,
        annotation_index: usize,
        contents: String,
    },
    /// Move or resize an existing stable XFDF annotation.  Canonical annotation/media redaction
    /// transforms its rectangle-linked geometry and regenerates the supported
    /// appearance without changing unrelated annotations.
    AnnotationMoveResize {
        annotation_id: String,
        page: usize,
        rect: [f64; 4],
    },
    /// Create a source-linked text reply to an existing stable annotation ID.
    /// Parent existence is checked against the current snapshot before the
    /// canonical XFDF importer establishes `/IRT`.
    AnnotationCreateReply {
        parent_annotation_id: String,
        page: usize,
        rect: [f64; 4],
        contents: String,
    },
    /// Delete only annotations intersecting an explicit page-space rectangle.
    /// The canonical editor resolves the actual `/Annots` entries; no visual
    /// cover-up is used.
    AnnotationDeleteInRect {
        page: usize,
        rect: [f64; 4],
    },
    /// Import canonical secure XFDF records for supported annotation creation,
    /// update, replies, popup links, geometry, and appearance regeneration.
    /// Standalone widget creation and unsafe actions remain exact refusals in
    /// the canonical annotation/media redaction importer.
    AnnotationXfdf {
        xfdf: String,
        #[serde(default)]
        delete_ids: Vec<String>,
    },
    /// Flatten selected canonical annotation appearances rather than leaving a
    /// live widget or annotation over a painted substitute.
    AnnotationFlatten {
        #[serde(default)]
        subtypes: Vec<String>,
    },
    FormSetText {
        field_name: String,
        value: String,
    },
    FormSetChoice {
        field_name: String,
        value: String,
    },
    /// Replace the option array and selected value of an existing resolved
    /// choice field, then regenerate its canonical appearance.
    FormSetChoiceOptions {
        field_name: String,
        options: Vec<String>,
        #[serde(default)]
        selected: Option<String>,
        #[serde(default)]
        editable_combo: bool,
    },
    FormSetCheckbox {
        field_name: String,
        checked: bool,
    },
    /// Rename the terminal component of an existing non-signature AcroForm
    /// field.  Field hierarchy separators are intentionally not accepted here
    /// so parent-child ownership cannot be guessed or silently rebuilt.
    FormRename {
        field_name: String,
        new_name: String,
    },
    /// Remove a resolved non-signature field subtree and its widget annotation
    /// references.  The operation does not paint a substitute widget.
    FormDelete {
        field_name: String,
    },
    /// Create a viewer-independent root text field and widget on an existing
    /// page.  The widget gets a canonical normal appearance and a real
    /// AcroForm field-tree entry; no painted-only substitute is used.
    FormCreateText {
        field_name: String,
        page: usize,
        rect: [f64; 4],
        #[serde(default)]
        value: String,
    },
    /// Create a real AcroForm text widget using the bounds of a resolved table
    /// cell. The widget remains a normal field-tree object, not table artwork.
    FormCreateTextInTableCell {
        table_id: String,
        row: usize,
        col: usize,
        field_name: String,
        #[serde(default)]
        value: String,
    },
    /// Create a real checkbox widget using the bounds of a resolved table
    /// cell, with canonical `Off`/`Yes` appearances and field state.
    FormCreateCheckboxInTableCell {
        table_id: String,
        row: usize,
        col: usize,
        field_name: String,
        #[serde(default)]
        checked: bool,
    },
    /// Create a real choice widget using the bounds of a resolved table cell.
    FormCreateChoiceInTableCell {
        table_id: String,
        row: usize,
        col: usize,
        field_name: String,
        #[serde(default)]
        options: Vec<String>,
        #[serde(default)]
        selected: Option<String>,
        #[serde(default)]
        editable_combo: bool,
    },
    /// Create a root checkbox with explicit `Off` and `Yes` normal appearance
    /// states.  The field, widget, page annotation entry, and appearances are
    /// all written in the same canonical object-rewrite transaction.
    FormCreateCheckbox {
        field_name: String,
        page: usize,
        rect: [f64; 4],
        #[serde(default)]
        checked: bool,
    },
    /// Create a root list or editable-combo choice field with a concrete
    /// option array and a canonical normal appearance for its selected value.
    FormCreateChoice {
        field_name: String,
        page: usize,
        rect: [f64; 4],
        options: Vec<String>,
        #[serde(default)]
        selected: Option<String>,
        #[serde(default)]
        editable_combo: bool,
    },
    /// Create a root push button with canonical normal, rollover, and down
    /// appearance entries. The bounded action uses the caption as its visible
    /// normal content and does not execute arbitrary button actions.
    FormCreatePushButton {
        field_name: String,
        page: usize,
        rect: [f64; 4],
        caption: String,
    },
    /// Create a bounded one-widget radio field with an explicit export value
    /// and matching named normal appearance state. Adding additional widgets
    /// to an existing group remains a separate topology-sensitive operation.
    FormCreateRadio {
        field_name: String,
        page: usize,
        rect: [f64; 4],
        export_value: String,
        #[serde(default)]
        selected: bool,
    },
    /// Create an unsigned signature field and widget. Existing signature
    /// values remain immutable under the existing signature policy.
    FormCreateSignature {
        field_name: String,
        page: usize,
        rect: [f64; 4],
    },
    /// Move or resize one resolved widget while retaining its owning field.
    /// This changes the actual widget `/Rect` through the canonical annotation
    /// path and preserves a valid existing widget appearance.
    FormMoveResizeWidget {
        field_name: String,
        page: usize,
        rect: [f64; 4],
    },
    /// Update `/DV` on an existing non-signature field without changing the
    /// live value or relying on a viewer-side reset implementation.
    FormSetDefault {
        field_name: String,
        value: String,
    },
    /// Update `/DV` for an existing checkbox or radio field using its actual
    /// named appearance/export state without changing the current live value.
    FormSetButtonDefault {
        field_name: String,
        checked: bool,
    },
    /// Replace the AcroForm calculation-order (`/CO`) reference array with
    /// resolved non-signature fields in explicit caller order.
    FormSetCalculationOrder {
        field_names: Vec<String>,
    },
    /// Apply bounded scalar form data through the canonical exchange parser.
    /// Accepted formats are JSON, FDF, and XFDF; actions/scripts are never
    /// executed by an import.
    FormImportData {
        data: String,
        format: String,
    },
    /// Restore one field or every supported non-signature field to its
    /// canonical `/DV` value through the existing appearance writer.
    FormReset {
        #[serde(default)]
        field_name: Option<String>,
    },
    FormFlatten,
    /// Report byte-preserved XFA inventory; dynamic conversion remains an
    /// explicit unsupported boundary rather than a destructive best effort.
    XfaInventory,
    /// Replace only the resolved static XFA `/datasets` packet. The template
    /// and every other packet retain their exact decoded bytes; dynamic XFA
    /// remains an explicit no-change boundary.
    XfaImportDatasets {
        datasets_xml: String,
    },
    /// Materialize the canonical static-XFA layout into page content. Dynamic
    /// XFA remains an exact refusal and packet removal is explicit.
    XfaFlattenStatic {
        #[serde(default)]
        remove_original_packets: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSubsystemsRequest {
    pub subsystem: DocumentSubsystemsSubsystem,
    #[serde(default)]
    pub action: Option<DocumentSubsystemsAction>,
    #[serde(default)]
    pub reflow: Option<GeometricReflowRequest>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub form_data: Option<String>,
    #[serde(default)]
    pub form_data_format: Option<String>,
    #[serde(default)]
    pub use_semantic_document_flow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSubsystemsAnalysisReport {
    pub schema_version: String,
    pub source_sha256: String,
    pub table_evidence: Value,
    pub mathematical_content: Value,
    pub ocr_layers: Value,
    pub annotations: Value,
    pub forms: Value,
    pub xfa: Value,
    pub exact_limits: Vec<String>,
}

/// Canonical source-linked table projection. The structural cells are the
/// existing table-analysis cells; document subsystems adds stable page/table identity and
/// the transaction revision that owns subsequent edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableTableGraph {
    pub table_id: String,
    pub page: usize,
    pub source: crate::analysis::tables::Table,
    pub provenance: Value,
    pub confidence: f64,
}

/// Deterministic source-linked mathematical tree.  It deliberately models the
/// syntax actually observed in a text run; outlined/raster formula inference is
/// kept outside this type and cannot silently replace the original artwork.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MathNodeKind {
    Expression,
    Row,
    Fraction,
    Radical,
    Superscript,
    Subscript,
    SubSup,
    Under,
    Over,
    UnderOver,
    Matrix,
    MatrixRow,
    MatrixCell,
    Fenced,
    Accent,
    Identifier,
    Operator,
    Number,
    Text,
    Space,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MathNode {
    pub node_id: String,
    pub kind: MathNodeKind,
    pub source_text: String,
    pub children: Vec<MathNode>,
    pub provenance: Value,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MathExpression {
    pub expression_id: String,
    pub page: usize,
    pub bounds: [f64; 4],
    pub source_text: String,
    pub root: MathNode,
    pub source_kind: String,
    pub confidence: f64,
    pub review_required: bool,
}

/// A provider-produced OCR word in PDF user space.  The original scan remains
/// the visual source; this record is written as invisible searchable text only
/// after the caller supplies a reviewed provider identity and confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrSearchableWord {
    pub text: String,
    pub rect: [f64; 4],
    pub font_size: f64,
    pub confidence: f64,
    #[serde(default)]
    pub line_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSubsystemsOperationReport {
    pub schema_version: String,
    pub subsystem: DocumentSubsystemsSubsystem,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<DocumentSubsystemsAction>,
    pub operation: String,
    pub source_sha256: String,
    pub output_sha256: String,
    pub changed_pages: Vec<usize>,
    pub source_links: Value,
    pub transaction: Value,
    pub appearance_effect: Value,
    pub xfa_effect: Value,
    pub undo_available: bool,
    pub exact_limits: Vec<String>,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn value<T: Serialize>(item: &T) -> Result<Value> {
    serde_json::to_value(item).map_err(|error| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems report_serialization_failed: {error}"
        ))
    })
}

fn reflow_required<'a>(
    request: &'a DocumentSubsystemsRequest,
    typed: &str,
) -> Result<&'a GeometricReflowRequest> {
    request.reflow.as_ref().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems {typed}: a provenance-resolved TextReflow reflow request is required"
        ))
    })
}

fn no_change_limit(subsystem: &DocumentSubsystemsSubsystem) -> Vec<String> {
    match subsystem {
        DocumentSubsystemsSubsystem::Table => vec![
            "grid_ambiguous and decorative_layout_not_table leave source bytes unchanged".into(),
            "unsupported merged-cell/page-break topology returns table_overflow or continuation_ambiguous".into(),
        ],
        DocumentSubsystemsSubsystem::Math => vec![
            "formula_review_required preserves unresolved outlined or raster formulas".into(),
            "math_metrics_unavailable and delimiter_construction_unavailable never flatten math to text".into(),
        ],
        DocumentSubsystemsSubsystem::OcrSearchableLayer | DocumentSubsystemsSubsystem::OcrReconstruction => vec![
            "provider_unavailable and confidence_below_threshold preserve the scan and generated layer".into(),
            "reconstruction_review_required prevents destructive scan replacement".into(),
        ],
        DocumentSubsystemsSubsystem::AnnotationAppearance => vec![
            "unsupported_annotation_type and appearance_generation_failed retain the source annotation".into(),
        ],
        DocumentSubsystemsSubsystem::FormData => vec![
            "signature_permission_violation, validation_rejected, and unsupported_action preserve field state".into(),
        ],
        DocumentSubsystemsSubsystem::XfaPreservation => vec![
            "dynamic_xfa_unsupported and xfa_conversion_lossy never perform silent conversion".into(),
        ],
    }
}

fn editable_tables(input: &[u8]) -> Result<Vec<EditableTableGraph>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut tables = Vec::new();
    for page in 1..=engine
        .page_count()?
        .min(MAX_DOCUMENT_SUBSYSTEM_ANALYSIS_PAGES)
    {
        for (index, table) in engine.extract_tables(page)?.into_iter().enumerate() {
            let table_id = format!("p{page}:table:{index}");
            tables.push(EditableTableGraph {
                table_id: table_id.clone(),
                page,
                confidence: table.confidence,
                provenance: json!({
                    "page": page,
                    "coordinate_space": "pdf_user_space",
                    "semantic_and_graphics_evidence": true,
                    "table_id": table_id,
                    "source": table.source,
                }),
                source: table,
            });
        }
    }
    Ok(tables)
}

fn math_like(text: &str) -> bool {
    text.contains('=')
        || text.contains('±')
        || text.contains('∑')
        || text.contains('∫')
        || text.contains('√')
        || text.contains('≤')
        || text.contains('≥')
        || (text.trim_start().starts_with("sqrt(") && text.trim_end().ends_with(')'))
        || (text.contains('/') && text.chars().any(|ch| ch.is_ascii_digit()))
        || ((text.contains('^') || text.contains('_'))
            && text.chars().any(|ch| ch.is_ascii_alphanumeric()))
        || bracket_matrix_rows(text).is_some()
}

fn bracket_matrix_rows(text: &str) -> Option<Vec<Vec<&str>>> {
    let inner = text.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    let rows = inner
        .split(';')
        .map(|row| {
            row.trim()
                .trim_matches(['[', ']'])
                .split(',')
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let width = rows.first()?.len();
    if width == 0
        || rows.is_empty()
        || rows
            .iter()
            .any(|row| row.len() != width || row.iter().any(|cell| cell.trim().is_empty()))
    {
        return None;
    }
    Some(rows)
}

fn math_leaf(
    kind: MathNodeKind,
    text: impl Into<String>,
    id: &str,
    provenance: &Value,
) -> MathNode {
    MathNode {
        node_id: id.to_string(),
        kind,
        source_text: text.into(),
        children: Vec::new(),
        provenance: provenance.clone(),
        confidence: 1.0,
    }
}

fn parse_math_node(text: &str, id: &str, provenance: &Value) -> MathNode {
    parse_math_node_with_depth(text, id, provenance, 0)
}

fn parse_math_node_with_depth(text: &str, id: &str, provenance: &Value, depth: usize) -> MathNode {
    let trimmed = text.trim();
    if depth >= 32 {
        return math_leaf(MathNodeKind::Unknown, trimmed, id, provenance);
    }
    if let Some(rows) = bracket_matrix_rows(trimmed) {
        let children = rows
            .into_iter()
            .take(32)
            .enumerate()
            .map(|(row_index, row)| MathNode {
                node_id: format!("{id}:row:{row_index}"),
                kind: MathNodeKind::MatrixRow,
                source_text: row.join(","),
                children: row
                    .into_iter()
                    .take(64)
                    .enumerate()
                    .map(|(column_index, cell)| MathNode {
                        node_id: format!("{id}:row:{row_index}:cell:{column_index}"),
                        kind: MathNodeKind::MatrixCell,
                        source_text: cell.trim().to_string(),
                        children: vec![parse_math_node_with_depth(
                            cell.trim(),
                            &format!("{id}:row:{row_index}:cell:{column_index}:value"),
                            provenance,
                            depth + 1,
                        )],
                        provenance: provenance.clone(),
                        confidence: 0.9,
                    })
                    .collect(),
                provenance: provenance.clone(),
                confidence: 0.9,
            })
            .collect();
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Matrix,
            source_text: trimmed.to_string(),
            children,
            provenance: provenance.clone(),
            confidence: 0.9,
        };
    }
    if let Some((left, right)) = trimmed.split_once('=') {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Row,
            source_text: trimmed.to_string(),
            children: vec![
                parse_math_node_with_depth(left, &format!("{id}:left"), provenance, depth + 1),
                math_leaf(
                    MathNodeKind::Operator,
                    "=",
                    &format!("{id}:equals"),
                    provenance,
                ),
                parse_math_node_with_depth(right, &format!("{id}:right"), provenance, depth + 1),
            ],
            provenance: provenance.clone(),
            confidence: 0.9,
        };
    }
    if let Some((numerator, denominator)) = trimmed.split_once('/') {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Fraction,
            source_text: trimmed.to_string(),
            children: vec![
                parse_math_node_with_depth(
                    numerator,
                    &format!("{id}:numerator"),
                    provenance,
                    depth + 1,
                ),
                parse_math_node_with_depth(
                    denominator,
                    &format!("{id}:denominator"),
                    provenance,
                    depth + 1,
                ),
            ],
            provenance: provenance.clone(),
            confidence: 0.82,
        };
    }
    if let Some((base, script)) = trimmed.split_once('^') {
        return MathNode {
            node_id: id.to_string(),
            kind: if base.contains('_') {
                MathNodeKind::SubSup
            } else {
                MathNodeKind::Superscript
            },
            source_text: trimmed.to_string(),
            children: vec![
                parse_math_node_with_depth(base, &format!("{id}:base"), provenance, depth + 1),
                parse_math_node_with_depth(script, &format!("{id}:sup"), provenance, depth + 1),
            ],
            provenance: provenance.clone(),
            confidence: 0.78,
        };
    }
    if let Some((base, script)) = trimmed.split_once('_') {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Subscript,
            source_text: trimmed.to_string(),
            children: vec![
                parse_math_node_with_depth(base, &format!("{id}:base"), provenance, depth + 1),
                parse_math_node_with_depth(script, &format!("{id}:sub"), provenance, depth + 1),
            ],
            provenance: provenance.clone(),
            confidence: 0.78,
        };
    }
    if let Some(inner) = trimmed
        .strip_prefix("sqrt(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Radical,
            source_text: trimmed.to_string(),
            children: vec![parse_math_node_with_depth(
                inner,
                &format!("{id}:radicand"),
                provenance,
                depth + 1,
            )],
            provenance: provenance.clone(),
            confidence: 0.86,
        };
    }
    if let Some(rest) = trimmed.strip_prefix('√') {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Radical,
            source_text: trimmed.to_string(),
            children: vec![parse_math_node_with_depth(
                rest,
                &format!("{id}:radicand"),
                provenance,
                depth + 1,
            )],
            provenance: provenance.clone(),
            confidence: 0.86,
        };
    }
    if trimmed.starts_with('(') && trimmed.ends_with(')') && trimmed.len() > 1 {
        return MathNode {
            node_id: id.to_string(),
            kind: MathNodeKind::Fenced,
            source_text: trimmed.to_string(),
            children: vec![parse_math_node_with_depth(
                &trimmed[1..trimmed.len() - 1],
                &format!("{id}:inner"),
                provenance,
                depth + 1,
            )],
            provenance: provenance.clone(),
            confidence: 0.82,
        };
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return math_leaf(MathNodeKind::Number, trimmed, id, provenance);
    }
    if trimmed.chars().all(|ch| ch.is_alphabetic()) {
        return math_leaf(MathNodeKind::Identifier, trimmed, id, provenance);
    }
    if trimmed.chars().all(|ch| "=+-*×÷<>≤≥∑∫()[]{}".contains(ch)) {
        return math_leaf(MathNodeKind::Operator, trimmed, id, provenance);
    }
    if !trimmed.chars().any(char::is_whitespace) {
        return math_leaf(MathNodeKind::Unknown, trimmed, id, provenance);
    }
    let children = trimmed
        .split_whitespace()
        .take(MAX_DOCUMENT_SUBSYSTEM_SAMPLE_ITEMS)
        .enumerate()
        .map(|(index, part)| {
            parse_math_node_with_depth(part, &format!("{id}:token:{index}"), provenance, depth + 1)
        })
        .collect::<Vec<_>>();
    MathNode {
        node_id: id.to_string(),
        kind: MathNodeKind::Row,
        source_text: trimmed.to_string(),
        children,
        provenance: provenance.clone(),
        confidence: 0.7,
    }
}

pub fn analyze_math_expressions(input: &[u8]) -> Result<Vec<MathExpression>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut expressions = Vec::new();
    for page in 1..=engine
        .page_count()?
        .min(MAX_DOCUMENT_SUBSYSTEM_ANALYSIS_PAGES)
    {
        for (index, chunk) in engine
            .collect_page_text_chunks(page)?
            .into_iter()
            .enumerate()
        {
            if !math_like(&chunk.text) {
                continue;
            }
            let expression_id = format!("p{page}:math:{index}");
            let provenance = json!({
                "page": page,
                "coordinate_space": "pdf_user_space",
                "source_instruction_text": true,
                "marked_content": "preserved_by_text_reflow_source_rewrite",
            });
            let bounds = [
                chunk.x,
                chunk.y,
                chunk.x + chunk.width.max(0.0),
                chunk.y + chunk.font_size.max(0.0),
            ];
            let root = MathNode {
                node_id: format!("{expression_id}:root"),
                kind: MathNodeKind::Expression,
                source_text: chunk.text.clone(),
                children: vec![parse_math_node(
                    &chunk.text,
                    &format!("{expression_id}:row"),
                    &provenance,
                )],
                provenance,
                confidence: 0.78,
            };
            expressions.push(MathExpression {
                expression_id,
                page,
                bounds,
                source_text: chunk.text,
                root,
                source_kind: "born_digital_text_instruction".to_string(),
                confidence: 0.78,
                review_required: false,
            });
        }
    }
    Ok(expressions)
}

fn resolved_math_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let expression = analyze_math_expressions(input)?
        .into_iter()
        .find(|item| item.page == supplied.page && item.source_text == supplied.source_text)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems math_structure_not_resolved: source text is not a resolved born-digital mathematical expression"
                    .to_string(),
            )
        })?;
    let mut reflow = supplied.clone();
    if reflow.region.is_none() {
        reflow.region = Some(expression.bounds);
    }
    reflow.replacement_text = replacement_text.to_string();
    Ok(reflow)
}

fn matrix_cell_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    row: usize,
    col: usize,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let mut matrix = bracket_matrix_rows(&supplied.source_text).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: matrix cell editing requires resolved [[row];[row]] source syntax"
                .to_string(),
        )
    })?;
    let row_cells = matrix.get_mut(row).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems math_structure_not_resolved: matrix row {row} is outside the resolved matrix"
        ))
    })?;
    let cell = row_cells.get_mut(col).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems math_structure_not_resolved: matrix column {col} is outside the resolved matrix"
        ))
    })?;
    if replacement_text.trim().is_empty()
        || replacement_text.contains(';')
        || replacement_text.contains(',')
        || replacement_text.contains('[')
        || replacement_text.contains(']')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: matrix cell content must be a nonempty scalar source fragment"
                .to_string(),
        ));
    }
    *cell = replacement_text.trim();
    let rebuilt = format!(
        "[[{}]]",
        matrix
            .iter()
            .map(|cells| cells.join(","))
            .collect::<Vec<_>>()
            .join(";")
    );
    resolved_math_reflow(input, request, &rebuilt)
}

fn matrix_structure_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    operation: &str,
    index: usize,
    values: &[String],
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let matrix = bracket_matrix_rows(&supplied.source_text).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: matrix structure editing requires resolved [[row];[row]] source syntax"
                .to_string(),
        )
    })?;
    let mut matrix = matrix
        .into_iter()
        .map(|row| row.into_iter().map(str::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let rows = matrix.len();
    let columns = matrix.first().map(Vec::len).unwrap_or(0);
    let scalar_values = || {
        values.iter().all(|value| {
            !value.trim().is_empty()
                && !value.contains(';')
                && !value.contains(',')
                && !value.contains('[')
                && !value.contains(']')
                && !value.contains('\n')
                && !value.contains('\r')
        })
    };
    match operation {
        "insert_row" => {
            if index > rows || values.len() != columns || !scalar_values() {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems math_structure_not_resolved: inserted matrix row needs exactly {columns} nonempty scalar values"
                )));
            }
            matrix.insert(index, values.iter().map(|value| value.trim().to_string()).collect());
        }
        "delete_row" => {
            if index >= rows || rows <= 1 || !values.is_empty() {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems math_structure_not_resolved: delete_row requires an existing row, no values, and at least one remaining row"
                        .to_string(),
                ));
            }
            matrix.remove(index);
        }
        "insert_column" => {
            if index > columns || values.len() != rows || !scalar_values() {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems math_structure_not_resolved: inserted matrix column needs exactly {rows} nonempty scalar values"
                )));
            }
            for (row, value) in matrix.iter_mut().zip(values) {
                row.insert(index, value.trim().to_string());
            }
        }
        "delete_column" => {
            if index >= columns || columns <= 1 || !values.is_empty() {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems math_structure_not_resolved: delete_column requires an existing column, no values, and at least one remaining column"
                        .to_string(),
                ));
            }
            for row in &mut matrix {
                row.remove(index);
            }
        }
        _ => {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems math_structure_not_resolved: matrix operation must be insert_row, delete_row, insert_column, or delete_column"
                    .to_string(),
            ))
        }
    }
    let rebuilt = format!(
        "[[{}]]",
        matrix
            .iter()
            .map(|row| row.join(","))
            .collect::<Vec<_>>()
            .join(";")
    );
    resolved_math_reflow(input, request, &rebuilt)
}

fn fraction_part_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    part: &str,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let (numerator, denominator) = supplied.source_text.trim().split_once('/').ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: fraction editing requires one resolved numerator/denominator slash"
                .to_string(),
        )
    })?;
    if denominator.contains('/')
        || numerator.trim().is_empty()
        || denominator.trim().is_empty()
        || replacement_text.trim().is_empty()
        || replacement_text.contains('/')
        || replacement_text.contains('\n')
        || replacement_text.contains('\r')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: a fraction part requires one nonempty single-line scalar source fragment"
                .to_string(),
        ));
    }
    let rebuilt = match part {
        "numerator" => format!("{}/{}", replacement_text.trim(), denominator.trim()),
        "denominator" => format!("{}/{}", numerator.trim(), replacement_text.trim()),
        _ => return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: fraction part must be numerator or denominator"
                .to_string(),
        )),
    };
    resolved_math_reflow(input, request, &rebuilt)
}

fn math_script_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    kind: &str,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    if replacement_text.trim().is_empty()
        || replacement_text.contains('^')
        || replacement_text.contains('_')
        || replacement_text.contains('\n')
        || replacement_text.contains('\r')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: a script requires one nonempty scalar source fragment"
                .to_string(),
        ));
    }
    let separator = match kind {
        "superscript" => '^',
        "subscript" => '_',
        _ => return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: script kind must be superscript or subscript"
                .to_string(),
        )),
    };
    let (base, _) = supplied.source_text.trim().split_once(separator).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems math_structure_not_resolved: requested {kind} is not present in the resolved source"
        ))
    })?;
    if base.trim().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: script source has no resolved base expression"
                .to_string(),
        ));
    }
    let rebuilt = format!("{}{}{}", base.trim(), separator, replacement_text.trim());
    resolved_math_reflow(input, request, &rebuilt)
}

fn math_fenced_inner_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let source = supplied.source_text.trim();
    let mut chars = source.chars();
    let open = chars.next().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: fenced expression source is empty"
                .to_string(),
        )
    })?;
    let close = source.chars().last().expect("nonempty source");
    if !matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
        || source.chars().count() < 3
        || bracket_matrix_rows(source).is_some()
        || replacement_text.trim().is_empty()
        || replacement_text.contains('\n')
        || replacement_text.contains('\r')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: fenced editing requires one resolved non-matrix (), [], or {} source expression"
                .to_string(),
        ));
    }
    let rebuilt = format!("{open}{}{close}", replacement_text.trim());
    resolved_math_reflow(input, request, &rebuilt)
}

fn math_radicand_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    replacement_text: &str,
) -> Result<GeometricReflowRequest> {
    let supplied = reflow_required(request, "math_structure_not_resolved")?;
    let source = supplied.source_text.trim();
    if replacement_text.trim().is_empty()
        || replacement_text.contains('\n')
        || replacement_text.contains('\r')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: a radical radicand requires one nonempty single-line source fragment"
                .to_string(),
        ));
    }
    let rebuilt = if source.strip_prefix('√').is_some() {
        format!("√{}", replacement_text.trim())
    } else if source
        .strip_prefix("sqrt(")
        .and_then(|value| value.strip_suffix(')'))
        .is_some()
    {
        format!("sqrt({})", replacement_text.trim())
    } else {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems math_structure_not_resolved: radical editing requires resolved √radicand or sqrt(radicand) source notation"
                .to_string(),
        ));
    };
    resolved_math_reflow(input, request, &rebuilt)
}

fn analyze_ocr_layers(input: &[u8]) -> Result<Value> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let classifications =
        crate::classify_document(&engine, &[], &crate::ClassifyConfig::default())?;
    let mut pages = Vec::new();
    for classification in classifications
        .into_iter()
        .take(MAX_DOCUMENT_SUBSYSTEM_ANALYSIS_PAGES)
    {
        let page = classification.page as usize;
        let words = engine.extract_page_words(page)?;
        let source = match classification.source {
            crate::PageSource::DigitalBorn => "born_digital_text",
            crate::PageSource::DigitalBornOverImage => "searchable_scan_existing_layer",
            crate::PageSource::Scanned => "scan_provider_required",
        };
        pages.push(json!({
            "page": page,
            "classification": classification,
            "layer_state": source,
            "existing_word_count": words.len(),
            "source_link_count": words.len(),
            "source_links_sample": words.into_iter().take(MAX_OCR_SOURCE_LINKS_PER_PAGE).map(|word| json!({
                "text": word.text,
                "bounds": [word.x0, word.y0, word.x1, word.y1],
                "page": word.page,
            })).collect::<Vec<_>>(),
            "source_link_sample_limit": MAX_OCR_SOURCE_LINKS_PER_PAGE,
            "recognition_provider": if source == "scan_provider_required" {
                "provider_unavailable_without_explicit_OcrEngine"
            } else {
                "not_required"
            },
        }));
    }
    Ok(json!({
        "canonical_provider_interface": "ocr::OcrEngine",
        "canonical_preprocess": "ocr::preprocess::preprocess",
        "layers": ["original_scan", "searchable_text", "editable_reconstruction"],
        "analysis_scope": {
            "max_pages": MAX_DOCUMENT_SUBSYSTEM_ANALYSIS_PAGES,
            "source_link_sample_per_page": MAX_OCR_SOURCE_LINKS_PER_PAGE,
            "full_page_provider_rerun_available_from_scoped_ocr_apis": true
        },
        "pages": pages,
        "scan_preserved_by_default": true,
    }))
}

#[allow(clippy::too_many_arguments)]
fn supported_ocr_add_searchable_text(
    input: &[u8],
    page: usize,
    text: &str,
    bounds: [f64; 4],
    font_size: f64,
    provider_id: &str,
    provider_version: Option<&str>,
    confidence: f64,
) -> Result<(Vec<u8>, Value)> {
    supported_ocr_add_searchable_words(
        input,
        page,
        &[OcrSearchableWord {
            text: text.to_string(),
            rect: bounds,
            font_size,
            confidence,
            line_id: None,
        }],
        provider_id,
        provider_version,
        None,
    )
}

fn supported_ocr_add_searchable_words(
    input: &[u8],
    page: usize,
    words: &[OcrSearchableWord],
    provider_id: &str,
    provider_version: Option<&str>,
    language: Option<&str>,
) -> Result<(Vec<u8>, Value)> {
    const MAX_OCR_LAYER_WORDS: usize = 20_000;
    if provider_id.trim().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems provider_unavailable: searchable-layer creation requires an identified canonical OCR provider"
                .to_string(),
        ));
    }
    if words.is_empty() || words.len() > MAX_OCR_LAYER_WORDS {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems resource_limit_exceeded: searchable layer requires 1..=20000 recognized words"
                .to_string(),
        ));
    }
    if language.is_some_and(|tag| tag.trim().is_empty() || tag.len() > 35) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems unsupported_script: OCR language tag must be a bounded nonempty BCP 47 token"
                .to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page_source = crate::classify_document(&engine, &[], &crate::ClassifyConfig::default())?
        .into_iter()
        .find(|item| item.page as usize == page)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems scan_not_resolved: page {page} is not present in the current snapshot"
            ))
        })?;
    if page_source.source != crate::PageSource::Scanned {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems searchable_layer_conflict: a searchable or born-digital text layer already owns this page"
                .to_string(),
        ));
    }
    if !engine.extract_page_words(page)?.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems duplicate_text_layer: page already has extractable source text"
                .to_string(),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    let mut source_words = Vec::with_capacity(words.len());
    for (index, word) in words.iter().enumerate() {
        if word.text.trim().is_empty() || !word.text.is_ascii() {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems unsupported_script: OCR word {index} requires nonempty exact-ASCII text until a canonical ToUnicode-capable OCR font route is selected"
            )));
        }
        if !word.confidence.is_finite() || !(0.0..=1.0).contains(&word.confidence) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems confidence_below_threshold: OCR word {index} has an invalid confidence"
            )));
        }
        if word.confidence < 0.80 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems confidence_below_threshold: OCR word {index} requires review before searchable-layer creation"
            )));
        }
        if !word.font_size.is_finite() || word.font_size <= 0.0 || word.font_size > 288.0 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems invalid_geometry: OCR word {index} font size must be finite and within 0..288"
            )));
        }
        let rect = rect_from_pdf_bounds(word.rect)?;
        let baseline = rect.y + rect.height.min(word.font_size).max(word.font_size * 0.75);
        editor.draw_text(
            page,
            &word.text,
            rect.x,
            baseline,
            EditTextStyle::new(word.font_size).rendering_mode(3),
            OverlayLayer::Overlay,
        )?;
        source_words.push(json!({
            "word_index": index,
            "text_sha256": digest(word.text.as_bytes()),
            "rect": word.rect,
            "font_size": word.font_size,
            "confidence": word.confidence,
            "line_id": word.line_id,
        }));
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    Ok((
        output,
        json!({
            "operation": "create_invisible_searchable_text_layer",
            "provider": {"id": provider_id, "version": provider_version},
            "language": language,
            "word_count": words.len(),
            "words": source_words,
            "source_scan_preserved": true,
            "text_rendering_mode": 3,
            "source_image_identity": {"document_sha256": digest(input), "page": page},
            "encoding_boundary": "exact_ascii_standard_encoding_only; non-ASCII requires a canonical ToUnicode-capable OCR font route",
        }),
    ))
}

fn xfa_packet_fingerprint(input: &[u8]) -> Result<Value> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = xfa_inventory(&engine, &XfaLimits::default())?;
    Ok(json!(inventory
        .packets
        .into_iter()
        .map(|packet| json!({
            "order": packet.order,
            "name": packet.name,
            "decoded_byte_length": packet.decoded_byte_length,
            "content_sha256": packet.content_sha256,
        }))
        .collect::<Vec<_>>()))
}

fn xfa_non_dataset_packet_fingerprint(input: &[u8]) -> Result<Value> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = xfa_inventory(&engine, &XfaLimits::default())?;
    Ok(json!(inventory
        .packets
        .into_iter()
        .filter(|packet| !packet.name.eq_ignore_ascii_case("datasets"))
        .map(|packet| json!({
            "order": packet.order,
            "name": packet.name,
            "decoded_byte_length": packet.decoded_byte_length,
            "content_sha256": packet.content_sha256,
        }))
        .collect::<Vec<_>>()))
}

fn import_static_xfa_datasets_pdf(input: &[u8], datasets_xml: &str) -> Result<(Vec<u8>, Value)> {
    const MAX_DATASETS_BYTES: usize = 8 * 1024 * 1024;
    let datasets = datasets_xml.trim();
    if datasets.is_empty()
        || datasets.len() > MAX_DATASETS_BYTES
        || !datasets.starts_with('<')
        || !datasets.contains("datasets")
        || datasets
            .chars()
            .any(|ch| ch == '\0' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: XFA datasets import requires bounded XML with a datasets root"
                .to_string(),
        ));
    }
    let before_engine = ContentEngine::open_bytes(input.to_vec())?;
    let before = extract_xfa(&before_engine, &XfaLimits::default())?;
    if !before.inventory.present {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no XFA datasets packet".to_string(),
        ));
    }
    if before.inventory.classification.dynamic_xfa {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems dynamic_xfa_unsupported: datasets import is limited to resolved static XFA"
                .to_string(),
        ));
    }
    if !before.template_parsed || !before.datasets_parsed {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems xfa_conversion_lossy: static template and existing datasets packet must both parse before import"
                .to_string(),
        ));
    }
    let preserved_before = xfa_non_dataset_packet_fingerprint(input)?;
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm/XFA dictionary"
                .to_string(),
        )
    })?)?;
    let xfa = acroform
        .as_dict()
        .and_then(|dict| dict.get("XFA"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems field_not_found: document has no XFA packet array".to_string(),
            )
        })?;
    let packet_items = resolve_pdf_array(reader, Some(xfa));
    if packet_items.len() < 2 || !packet_items.len().is_multiple_of(2) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems xfa_conversion_lossy: XFA datasets import requires an even packet array"
                .to_string(),
        ));
    }
    let dataset_reference = packet_items
        .chunks_exact(2)
        .find(|pair| {
            form_scalar_name(&pair[0])
                .is_some_and(|name| name.eq_ignore_ascii_case("datasets"))
        })
        .and_then(|pair| pair[1].as_reference())
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems xfa_conversion_lossy: datasets import requires an indirect datasets packet"
                    .to_string(),
            )
        })?;
    let raw = datasets.as_bytes().to_vec();
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != dataset_reference.0 {
            return;
        }
        if let PdfObject::Stream { dict, raw: stream } = object {
            dict.remove("Filter");
            dict.remove("DecodeParms");
            dict.remove("Length");
            *stream = raw.clone();
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: resolved XFA datasets packet was not a writable stream"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    let after_engine = ContentEngine::open_bytes(output.clone())?;
    let after = extract_xfa(&after_engine, &XfaLimits::default())?;
    if !after.datasets_parsed || after.inventory.classification.dynamic_xfa {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems structure_update_failed: imported static XFA datasets did not reopen as a resolved static packet"
                .to_string(),
        ));
    }
    let preserved_after = xfa_non_dataset_packet_fingerprint(&output)?;
    if preserved_before != preserved_after {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems structure_update_failed: static XFA datasets import changed a non-datasets packet"
                .to_string(),
        ));
    }
    Ok((
        output,
        json!({
            "operation": "static_xfa_datasets_packet_source_rewrite",
            "datasets_packet": format!("{} {} R", dataset_reference.0, dataset_reference.1),
            "datasets_byte_length": raw.len(),
            "template_preserved": true,
            "non_datasets_packets_preserved": true,
            "datasets_nodes_after": after.datasets.len(),
            "dynamic_xfa": "dynamic_xfa_unsupported",
        }),
    ))
}

fn table_cell_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    table_id: &str,
    row: usize,
    col: usize,
    replacement_text: &str,
) -> Result<(GeometricReflowRequest, EditableTableGraph)> {
    let graph = editable_tables(input)?
        .into_iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: source-linked table {table_id} was not found in the current snapshot"
            ))
        })?;
    let cell = graph
        .source
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: origin cell ({row},{col}) is not present in {table_id}"
            ))
        })?;
    if cell.text.trim().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_not_resolved: an empty cell has no source text instruction to rewrite"
                .to_string(),
        ));
    }
    let mut reflow = reflow_required(request, "table_not_resolved")?.clone();
    reflow.page = graph.page;
    reflow.source_text = cell.text.clone();
    reflow.replacement_text = replacement_text.to_string();
    reflow.region = Some(cell.bbox);
    Ok((reflow, graph))
}

fn table_cell_alignment_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    table_id: &str,
    row: usize,
    col: usize,
    alignment: &str,
) -> Result<(GeometricReflowRequest, EditableTableGraph)> {
    if !matches!(
        alignment,
        "left" | "right" | "center" | "justify" | "start" | "end"
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: table cell alignment must be left, right, center, justify, start, or end"
                .to_string(),
        ));
    }
    let graph = editable_tables(input)?
        .into_iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: source-linked table {table_id} was not found in the current snapshot"
            ))
        })?;
    let cell = graph
        .source
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: origin cell ({row},{col}) is not present in {table_id}"
            ))
        })?;
    if cell.text.trim().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_not_resolved: an empty cell has no source text instruction to realign"
                .to_string(),
        ));
    }
    let mut reflow = reflow_required(request, "table_not_resolved")?.clone();
    reflow.page = graph.page;
    reflow.source_text = cell.text.clone();
    reflow.replacement_text = cell.text.clone();
    reflow.region = Some(cell.bbox);
    reflow.alignment = alignment.to_string();
    Ok((reflow, graph))
}

fn table_cell_padding_reflow(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    table_id: &str,
    row: usize,
    col: usize,
    padding: [f64; 4],
) -> Result<(GeometricReflowRequest, EditableTableGraph)> {
    if padding
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_geometry: table cell padding must be finite and nonnegative"
                .to_string(),
        ));
    }
    let graph = editable_tables(input)?
        .into_iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: source-linked table {table_id} was not found in the current snapshot"
            ))
        })?;
    let cell = graph
        .source
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: origin cell ({row},{col}) is not present in {table_id}"
            ))
        })?;
    if cell.text.trim().is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_not_resolved: an empty cell has no source text instruction to reflow"
                .to_string(),
        ));
    }
    let [left, bottom, right, top] = padding;
    let x0 = cell.bbox[0].min(cell.bbox[2]) + left;
    let y0 = cell.bbox[1].min(cell.bbox[3]) + bottom;
    let x1 = cell.bbox[0].max(cell.bbox[2]) - right;
    let y1 = cell.bbox[1].max(cell.bbox[3]) - top;
    if x1 - x0 < 4.0 || y1 - y0 < 4.0 {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: requested cell padding leaves no usable source reflow region"
                .to_string(),
        ));
    }
    let mut reflow = reflow_required(request, "table_not_resolved")?.clone();
    reflow.page = graph.page;
    reflow.source_text = cell.text.clone();
    reflow.replacement_text = cell.text.clone();
    reflow.region = Some([x0, y0, x1, y1]);
    Ok((reflow, graph))
}

fn append_simple_ruled_table_row(
    input: &[u8],
    table_id: &str,
    values: &[String],
    requested_height: Option<f64>,
) -> Result<(Vec<u8>, Value, usize)> {
    let table = editable_tables(input)?
        .into_iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: source-linked table {table_id} was not found in the current snapshot"
            ))
        })?;
    if table.source.source != crate::analysis::tables::TableSource::Ruled {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems decorative_layout_not_table: row insertion requires a resolved ruled grid"
                .to_string(),
        ));
    }
    let columns = table.source.num_cols();
    if columns == 0 || values.len() != columns {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems table_not_resolved: appended row requires exactly {columns} values"
        )));
    }
    if table
        .source
        .cells
        .iter()
        .any(|cell| cell.rowspan != 1 || cell.colspan != 1)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems merged_cell_conflict: appended rows are not safe for a grid with merged cells"
                .to_string(),
        ));
    }
    let mut column_bounds = vec![None::<(f64, f64)>; columns];
    let mut row_heights = Vec::new();
    for cell in &table.source.cells {
        if cell.col < columns {
            let x0 = cell.bbox[0].min(cell.bbox[2]);
            let x1 = cell.bbox[0].max(cell.bbox[2]);
            if x1 > x0 && x0.is_finite() && x1.is_finite() {
                let slot = &mut column_bounds[cell.col];
                *slot = Some(match *slot {
                    Some((left, right)) => (left.min(x0), right.max(x1)),
                    None => (x0, x1),
                });
            }
        }
        let height = (cell.bbox[3] - cell.bbox[1]).abs();
        if height.is_finite() && height > 4.0 {
            row_heights.push(height);
        }
    }
    let column_bounds = column_bounds
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems grid_ambiguous: table columns do not have finite source bounds"
                    .to_string(),
            )
        })?;
    let smallest_source_height = row_heights.into_iter().fold(f64::INFINITY, f64::min);
    let inferred_height = if smallest_source_height.is_finite() {
        smallest_source_height.max(18.0)
    } else {
        ((table.source.bbox[3] - table.source.bbox[1]).abs()
            / table.source.num_rows().max(1) as f64)
            .max(18.0)
    };
    let row_height = requested_height.unwrap_or(inferred_height);
    if !row_height.is_finite() || !(12.0..=144.0).contains(&row_height) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: row height must be finite and within 12..=144 points"
                .to_string(),
        ));
    }
    if values
        .iter()
        .any(|value| value.contains('\n') || value.contains('\r'))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: appended row values must be single-line; use cell reflow for wrapped content"
                .to_string(),
        ));
    }
    let y_top = table.source.bbox[1].min(table.source.bbox[3]);
    let y_bottom = y_top - row_height;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.get_page(table.page)?;
    let page_bottom = page.crop_box[1].min(page.crop_box[3]);
    if y_bottom < page_bottom {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: no explicit page-space capacity exists below the ruled table"
                .to_string(),
        ));
    }
    let intersects =
        |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1];
    let new_row_bounds = [
        column_bounds.first().expect("nonempty").0,
        y_bottom,
        column_bounds.last().expect("nonempty").1,
        y_top,
    ];
    if engine
        .collect_page_text_chunks(table.page)?
        .iter()
        .any(|chunk| {
            intersects(
                [
                    chunk.x,
                    chunk.y,
                    chunk.x + chunk.width,
                    chunk.y + chunk.font_size,
                ],
                new_row_bounds,
            )
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: source text occupies the requested appended-row region"
                .to_string(),
        ));
    }
    let interactive = interactive_report(&engine)?;
    if interactive
        .annotations
        .annotations
        .iter()
        .any(|annotation| {
            annotation.page == table.page
                && annotation
                    .rect
                    .is_some_and(|rect| intersects(rect, new_row_bounds))
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: an annotation occupies the requested appended-row region"
                .to_string(),
        ));
    }
    let classification = crate::classify_document(&engine, &[], &crate::ClassifyConfig::default())?
        .into_iter()
        .find(|item| item.page as usize == table.page);
    if classification.is_some_and(|item| item.image_coverage > 0.0) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: pages with image occurrences require an explicit scene dependency before table expansion"
                .to_string(),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    for (column, ((x0, x1), value)) in column_bounds.iter().zip(values).enumerate() {
        let width = x1 - x0;
        if width <= 0.0 {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems grid_ambiguous: appended table cell has nonpositive width"
                    .to_string(),
            ));
        }
        // A bounded fixed-width source insertion is intentional here: wrapped
        // text belongs to `table_edit_cell` and text reflow reflow, not an
        // implicit font-size reduction during row insertion.
        if value.chars().count() as f64 * 6.0 > width - 8.0 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_overflow: appended value for column {column} exceeds the resolved single-line cell width"
            )));
        }
        editor.draw_rect(
            table.page,
            ImageRect::new(*x0, y_bottom, width, row_height),
            EditRectStyle::default(),
            OverlayLayer::Overlay,
        )?;
        if !value.is_empty() {
            editor.draw_text(
                table.page,
                value,
                x0 + 4.0,
                y_bottom + (row_height - 12.0).max(4.0),
                EditTextStyle::new(10.0),
                OverlayLayer::Overlay,
            )?;
        }
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    Ok((
        output,
        json!({
            "table_id": table.table_id,
            "row_index": table.source.num_rows(),
            "row_height": row_height,
            "source_cells": columns,
            "inserted_page_content": ["vector_borders", "text_instructions"],
            "constraints": "ruled_unmerged_table + explicit_empty_space + no_text_annotation_or_image_conflict",
        }),
        table.page,
    ))
}

fn append_simple_ruled_table_column(
    input: &[u8],
    table_id: &str,
    values: &[String],
    requested_width: Option<f64>,
) -> Result<(Vec<u8>, Value, usize)> {
    let table = editable_tables(input)?
        .into_iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_not_resolved: source-linked table {table_id} was not found in the current snapshot"
            ))
        })?;
    if table.source.source != crate::analysis::tables::TableSource::Ruled {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems decorative_layout_not_table: column insertion requires a resolved ruled grid"
                .to_string(),
        ));
    }
    let rows = table.source.num_rows();
    if rows == 0 || values.len() != rows {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems table_not_resolved: appended column requires exactly {rows} values"
        )));
    }
    if table
        .source
        .cells
        .iter()
        .any(|cell| cell.rowspan != 1 || cell.colspan != 1)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems merged_cell_conflict: appended columns are not safe for a grid with merged cells"
                .to_string(),
        ));
    }
    if values
        .iter()
        .any(|value| value.contains('\n') || value.contains('\r'))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: appended column values must be single-line; use cell reflow for wrapped content"
                .to_string(),
        ));
    }
    let mut row_bounds = vec![None::<(f64, f64)>; rows];
    let mut widths = Vec::new();
    for cell in &table.source.cells {
        if cell.row < rows {
            let y0 = cell.bbox[1].min(cell.bbox[3]);
            let y1 = cell.bbox[1].max(cell.bbox[3]);
            if y1 > y0 && y0.is_finite() && y1.is_finite() {
                let slot = &mut row_bounds[cell.row];
                *slot = Some(match *slot {
                    Some((bottom, top)) => (bottom.min(y0), top.max(y1)),
                    None => (y0, y1),
                });
            }
        }
        let width = (cell.bbox[2] - cell.bbox[0]).abs();
        if width.is_finite() && width > 4.0 {
            widths.push(width);
        }
    }
    let row_bounds = row_bounds
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems grid_ambiguous: table rows do not have finite source bounds"
                    .to_string(),
            )
        })?;
    let smallest_width = widths.into_iter().fold(f64::INFINITY, f64::min);
    let inferred_width = if smallest_width.is_finite() {
        smallest_width.max(24.0)
    } else {
        ((table.source.bbox[2] - table.source.bbox[0]).abs()
            / table.source.num_cols().max(1) as f64)
            .max(24.0)
    };
    let column_width = requested_width.unwrap_or(inferred_width);
    if !column_width.is_finite() || !(18.0..=288.0).contains(&column_width) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: column width must be finite and within 18..=288 points"
                .to_string(),
        ));
    }
    let x_left = table.source.bbox[0].max(table.source.bbox[2]);
    let x_right = x_left + column_width;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.get_page(table.page)?;
    let page_right = page.crop_box[0].max(page.crop_box[2]);
    if x_right > page_right {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems table_overflow: no explicit page-space capacity exists beside the ruled table"
                .to_string(),
        ));
    }
    let row_bottom = row_bounds
        .iter()
        .map(|(bottom, _)| *bottom)
        .fold(f64::INFINITY, f64::min);
    let row_top = row_bounds
        .iter()
        .map(|(_, top)| *top)
        .fold(f64::NEG_INFINITY, f64::max);
    let new_column_bounds = [x_left, row_bottom, x_right, row_top];
    let intersects =
        |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1];
    if engine
        .collect_page_text_chunks(table.page)?
        .iter()
        .any(|chunk| {
            intersects(
                [
                    chunk.x,
                    chunk.y,
                    chunk.x + chunk.width,
                    chunk.y + chunk.font_size,
                ],
                new_column_bounds,
            )
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: source text occupies the requested appended-column region"
                .to_string(),
        ));
    }
    let interactive = interactive_report(&engine)?;
    if interactive
        .annotations
        .annotations
        .iter()
        .any(|annotation| {
            annotation.page == table.page
                && annotation
                    .rect
                    .is_some_and(|rect| intersects(rect, new_column_bounds))
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: an annotation occupies the requested appended-column region"
                .to_string(),
        ));
    }
    let classification = crate::classify_document(&engine, &[], &crate::ClassifyConfig::default())?
        .into_iter()
        .find(|item| item.page as usize == table.page);
    if classification.is_some_and(|item| item.image_coverage > 0.0) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems locked_object_conflict: pages with image occurrences require an explicit scene dependency before table expansion"
                .to_string(),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    for (row, ((y0, y1), value)) in row_bounds.iter().zip(values).enumerate() {
        let height = y1 - y0;
        if height <= 0.0 {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems grid_ambiguous: appended table cell has nonpositive height"
                    .to_string(),
            ));
        }
        if value.chars().count() as f64 * 6.0 > column_width - 8.0 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems table_overflow: appended value for row {row} exceeds the resolved single-line cell width"
            )));
        }
        editor.draw_rect(
            table.page,
            ImageRect::new(x_left, *y0, column_width, height),
            EditRectStyle::default(),
            OverlayLayer::Overlay,
        )?;
        if !value.is_empty() {
            editor.draw_text(
                table.page,
                value,
                x_left + 4.0,
                y0 + (height - 12.0).max(4.0),
                EditTextStyle::new(10.0),
                OverlayLayer::Overlay,
            )?;
        }
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    Ok((
        output,
        json!({
            "table_id": table.table_id,
            "column_index": table.source.num_cols(),
            "column_width": column_width,
            "source_rows": rows,
            "inserted_page_content": ["vector_borders", "text_instructions"],
            "constraints": "ruled_unmerged_table + explicit_empty_space + no_text_annotation_or_image_conflict",
        }),
        table.page,
    ))
}

fn rect_from_pdf_bounds(bounds: [f64; 4]) -> Result<ImageRect> {
    let [x0, y0, x1, y1] = bounds;
    if !x0.is_finite()
        || !y0.is_finite()
        || !x1.is_finite()
        || !y1.is_finite()
        || x1 <= x0
        || y1 <= y0
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_geometry: annotation geometry must be finite with positive extent"
                .to_string(),
        ));
    }
    Ok(ImageRect::new(x0, y0, x1 - x0, y1 - y0))
}

fn supported_annotation_create(
    input: &[u8],
    page: usize,
    subtype: &str,
    bounds: [f64; 4],
    contents: &str,
    uri: Option<&str>,
) -> Result<(Vec<u8>, Value)> {
    let rect = rect_from_pdf_bounds(bounds)?;
    // annotation/media redaction owns the source-level writer for annotations whose appearance
    // is a real form XObject.  Keep this convenience action intentionally
    // narrow: complex geometry such as quads, ink lists, replies, and popups
    // must arrive through `AnnotationXfdf`, where their full source evidence
    // is explicit rather than guessed from a rectangle.
    if let Some(element) = direct_xfdf_annotation_element(subtype) {
        if page == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems invalid_geometry: annotation pages are one-based".to_string(),
            ));
        }
        let fingerprint = format!(
            "{}:{page}:{subtype}:{:.6}:{:.6}:{:.6}:{:.6}:{contents}",
            digest(input),
            bounds[0],
            bounds[1],
            bounds[2],
            bounds[3]
        );
        let annotation_id = format!("wf-p34-{}", &digest(fingerprint.as_bytes())[..24]);
        let xfdf = format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                "<xfdf xmlns=\"http://ns.adobe.com/xfdf/\"><annots>",
                "<{element} page=\"{page_zero}\" name=\"{annotation_id}\" rect=\"{rect}\">",
                "<contents>{contents}</contents></{element}>",
                "</annots></xfdf>"
            ),
            element = element,
            page_zero = page - 1,
            annotation_id = xml_escape_document_subsystems(&annotation_id),
            rect = xfdf_rect(bounds),
            contents = xml_escape_document_subsystems(contents),
        );
        let options = AnnotationXfdfImportOptions {
            fail_on_unsupported: true,
            ..AnnotationXfdfImportOptions::default()
        };
        let (output, appearance) = import_annotation_xfdf_pdf(input, xfdf.as_bytes(), &options)?;
        return Ok((
            output,
            json!({
                "source_edit": "canonical_annotation_media_redaction_xfdf_import",
                "annotation_id": annotation_id,
                "appearance": appearance,
            }),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    let options = AnnotationOptions::default().contents(contents.to_string());
    match subtype.to_ascii_lowercase().as_str() {
        "highlight" => {
            editor.add_highlight_annotation(page, rect, options)?;
        }
        "text" | "text_note" => {
            editor.add_text_note_annotation(page, rect, contents.to_string(), options)?;
        }
        "stamp" => {
            editor.add_stamp_annotation(page, rect, contents.to_string(), options)?;
        }
        "link" => {
            let target = uri.ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "document_subsystems invalid_geometry: Link creation requires an explicit URI"
                        .to_string(),
                )
            })?;
            editor.add_link_uri(page, rect, target.to_string())?;
        }
        unsupported => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems unsupported_annotation_type: {unsupported}"
            )))
        }
    }
    let edited = editor.save_to_bytes(EditMode::FullRewrite)?;
    let (output, appearance) =
        generate_annotation_appearances_pdf(&edited, &AnnotationAppearanceOptions::default())?;
    Ok((
        output,
        json!({"source_edit": "PdfEditor", "appearance": appearance}),
    ))
}

fn direct_xfdf_annotation_element(subtype: &str) -> Option<&'static str> {
    match subtype.trim().to_ascii_lowercase().as_str() {
        "free_text" | "freetext" => Some("freetext"),
        "square" => Some("square"),
        "circle" => Some("circle"),
        "caret" => Some("caret"),
        "redact" | "redaction" => Some("redact"),
        _ => None,
    }
}

fn supported_annotation_create_reply(
    input: &[u8],
    parent_annotation_id: &str,
    page: usize,
    bounds: [f64; 4],
    contents: &str,
) -> Result<(Vec<u8>, Value)> {
    if parent_annotation_id.trim().is_empty() || page == 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems reply_relationship_invalid: replies require a stable parent ID and one-based page"
                .to_string(),
        ));
    }
    rect_from_pdf_bounds(bounds)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let (existing, _) = export_annotation_xfdf(&engine)?;
    if !parse_annotation_xfdf(&existing)?
        .annotations
        .iter()
        .any(|annotation| annotation.id == parent_annotation_id)
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems reply_relationship_invalid: parent annotation {parent_annotation_id} is not in the current snapshot"
        )));
    }
    let fingerprint = format!(
        "{}:{parent_annotation_id}:{page}:{:.6}:{:.6}:{:.6}:{:.6}:{contents}",
        digest(input),
        bounds[0],
        bounds[1],
        bounds[2],
        bounds[3]
    );
    let reply_id = format!("wf-p34-reply-{}", &digest(fingerprint.as_bytes())[..24]);
    let xfdf = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<xfdf xmlns=\"http://ns.adobe.com/xfdf/\"><annots>",
            "<text page=\"{page_zero}\" name=\"{reply_id}\" rect=\"{rect}\" inreplyto=\"{parent}\" replyType=\"R\">",
            "<contents>{contents}</contents></text></annots></xfdf>"
        ),
        page_zero = page - 1,
        reply_id = xml_escape_document_subsystems(&reply_id),
        rect = xfdf_rect(bounds),
        parent = xml_escape_document_subsystems(parent_annotation_id),
        contents = xml_escape_document_subsystems(contents),
    );
    let options = AnnotationXfdfImportOptions {
        fail_on_unsupported: true,
        ..AnnotationXfdfImportOptions::default()
    };
    let (output, report) = import_annotation_xfdf_pdf(input, xfdf.as_bytes(), &options)?;
    Ok((
        output,
        json!({
            "source_edit": "canonical_annotation_media_redaction_xfdf_reply_import",
            "reply_id": reply_id,
            "parent_annotation_id": parent_annotation_id,
            "appearance": report,
        }),
    ))
}

fn xfdf_rect(bounds: [f64; 4]) -> String {
    bounds
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn xml_escape_document_subsystems(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            ch if ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control() => out.push(ch),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

fn form_scalar_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(crate::info::decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn resolve_pdf_array(reader: &PdfReader, object: Option<&PdfObject>) -> Vec<PdfObject> {
    object
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_array().map(<[PdfObject]>::to_vec))
        .unwrap_or_default()
}

fn form_field_reference_by_name(
    reader: &PdfReader,
    source: &PdfObject,
    parent_name: &str,
    target: &str,
    depth: usize,
) -> Result<Option<(u32, u16)>> {
    if depth > 32 {
        return Err(WellfriendError::ResourceLimit(
            "document_subsystems resource_limit_exceeded: AcroForm field hierarchy exceeds depth 32"
                .to_string(),
        ));
    }
    let reference = source.as_reference();
    let resolved = reader.resolve(source.clone())?;
    let Some(dict) = resolved.as_dict() else {
        return Ok(None);
    };
    let local_name = dict.get("T").and_then(form_scalar_name);
    let full_name = match (parent_name.is_empty(), local_name.as_deref()) {
        (_, None | Some("")) => parent_name.to_string(),
        (true, Some(local)) => local.to_string(),
        (false, Some(local)) => format!("{parent_name}.{local}"),
    };
    if full_name == target && dict.get("T").is_some() {
        return Ok(reference);
    }
    for child in resolve_pdf_array(reader, dict.get("Kids")) {
        if let Some(reference) =
            form_field_reference_by_name(reader, &child, &full_name, target, depth + 1)?
        {
            return Ok(Some(reference));
        }
    }
    Ok(None)
}

fn rename_form_field_pdf(
    input: &[u8],
    field_name: &str,
    new_name: &str,
) -> Result<(Vec<u8>, Value)> {
    if new_name.trim().is_empty()
        || new_name.contains('.')
        || !new_name.is_ascii()
        || new_name.chars().any(char::is_control)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: a field rename requires a nonempty ASCII terminal name without hierarchy separators"
                .to_string(),
        ));
    }
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = catalog.get("AcroForm").ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?;
    let acroform = reader.resolve(acroform.clone())?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let mut target = None;
    for field in &fields {
        if let Some(reference) = form_field_reference_by_name(reader, field, "", field_name, 0)? {
            target = Some(reference);
            break;
        }
    }
    let target = target.ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
        ))
    })?;
    let parent_prefix = field_name.rsplit_once('.').map(|(parent, _)| parent);
    let output_name = parent_prefix
        .map(|parent| format!("{parent}.{new_name}"))
        .unwrap_or_else(|| new_name.to_string());
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if crate::forms_report(&engine)?
        .fields
        .iter()
        .any(|field| field.full_name == output_name)
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems duplicate_field_name: {output_name} already exists in the current AcroForm field tree"
        )));
    }
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != target.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("T", PdfObject::String(new_name.as_bytes().to_vec()));
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: resolved field object was not a mutable dictionary"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "rename_terminal_acroform_field",
            "field_object": format!("{} {} R", target.0, target.1),
            "old_full_name": field_name,
            "new_full_name": output_name,
            "appearance_state": "preserved; field name mutation does not repaint the widget",
        }),
    ))
}

fn set_form_default_pdf(input: &[u8], field_name: &str, value: &str) -> Result<(Vec<u8>, Value)> {
    if !value.is_ascii() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: default values currently require at most 4096 printable ASCII characters"
                .to_string(),
        ));
    }
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = catalog.get("AcroForm").ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?;
    let acroform = reader.resolve(acroform.clone())?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let target = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
            ))
        })?;
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != target.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("DV", PdfObject::String(value.as_bytes().to_vec()));
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: resolved field object was not a mutable dictionary"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "set_acroform_default_value",
            "field_object": format!("{} {} R", target.0, target.1),
            "field_name": field_name,
            "appearance_state": "unchanged; /DV changes reset behavior but not the live widget appearance",
        }),
    ))
}

fn set_form_button_default_pdf(
    input: &[u8],
    field_name: &str,
    checked: bool,
) -> Result<(Vec<u8>, Value)> {
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let target = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
            ))
        })?;
    let field = reader.get_and_resolve(target.0, target.1)?;
    let widgets = resolve_pdf_array(reader, field.as_dict().and_then(|dict| dict.get("Kids")));
    if widgets.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems appearance_state_mismatch: button default editing requires exactly one resolved widget"
                .to_string(),
        ));
    }
    let widget = reader.resolve(widgets[0].clone())?;
    let normal_states = widget
        .as_dict()
        .and_then(|dict| dict.get("AP"))
        .and_then(|appearance| reader.resolve(appearance.clone()).ok())
        .and_then(|appearance| appearance.as_dict().and_then(|dict| dict.get("N")).cloned())
        .and_then(|normal| reader.resolve(normal).ok())
        .and_then(|normal| normal.as_dict().cloned())
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "document_subsystems appearance_state_mismatch: button has no named normal appearance states"
                    .to_string(),
            )
        })?;
    let state = if checked {
        normal_states
            .entries()
            .find_map(|(name, _)| (name != "Off").then_some(name.to_string()))
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "document_subsystems export_value_mismatch: button has no non-Off export appearance state"
                        .to_string(),
                )
            })?
    } else {
        "Off".to_string()
    };
    if normal_states.get(&state).is_none() {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems appearance_state_mismatch: requested button default state is absent from /AP /N"
                .to_string(),
        ));
    }
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != target.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("DV", PdfObject::Name(state.clone()));
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: button default did not update the resolved field dictionary"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "set_acroform_button_default_state",
            "field_object": format!("{} {} R", target.0, target.1),
            "field_name": field_name,
            "default_state": state,
            "live_value_unchanged": true,
        }),
    ))
}

fn set_form_calculation_order_pdf(
    input: &[u8],
    field_names: &[String],
) -> Result<(Vec<u8>, Value)> {
    const MAX_CALCULATION_FIELDS: usize = 512;
    if field_names.is_empty()
        || field_names.len() > MAX_CALCULATION_FIELDS
        || field_names.iter().any(|name| name.trim().is_empty())
        || field_names.iter().collect::<BTreeSet<_>>().len() != field_names.len()
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: calculation order requires 1..=512 unique nonempty resolved field names"
                .to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let form_report = crate::forms_report(&engine)?;
    for field_name in field_names {
        let field = form_report
            .fields
            .iter()
            .find(|field| field.full_name == *field_name)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
                ))
            })?;
        if field.is_signature {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems signature_permission_violation: signature fields cannot enter a calculation order"
                    .to_string(),
            ));
        }
    }
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform_source = catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?;
    let acroform_reference = acroform_source.as_reference().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems structure_update_failed: calculation-order mutation requires an indirect AcroForm dictionary"
                .to_string(),
        )
    })?;
    let acroform = reader.resolve(acroform_source)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let references = field_names
        .iter()
        .map(|field_name| {
            fields
                .iter()
                .find_map(|field| {
                    form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
                })
                .transpose()?
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!(
                        "document_subsystems structure_update_failed: calculation-order field {field_name} disappeared during resolution"
                    ))
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let order = PdfObject::Array(
        references
            .iter()
            .map(|(number, generation)| PdfObject::Reference {
                number: *number,
                generation: *generation,
            })
            .collect(),
    );
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != acroform_reference.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("CO", order.clone());
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: calculation-order update did not reach the AcroForm dictionary"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let forms = crate::forms_report(&reopened)?;
    if forms.calculation_order_len != field_names.len() {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: calculation-order cardinality did not survive reopen"
                .to_string(),
        ));
    }
    Ok((
        output,
        json!({
            "operation": "set_acroform_calculation_order",
            "field_count": field_names.len(),
            "field_references": references.iter().map(|(number, generation)| format!("{number} {generation} R")).collect::<Vec<_>>(),
            "event_execution": "not_triggered_by_order_mutation",
            "appearance_regeneration": "not_required_without_value_change",
        }),
    ))
}

fn collect_form_subtree_references(
    reader: &PdfReader,
    source: &PdfObject,
    depth: usize,
    references: &mut BTreeSet<(u32, u16)>,
    array_references: &mut BTreeSet<(u32, u16)>,
) -> Result<()> {
    if depth > 32 {
        return Err(WellfriendError::ResourceLimit(
            "document_subsystems resource_limit_exceeded: AcroForm field hierarchy exceeds depth 32"
                .to_string(),
        ));
    }
    let Some(reference) = source.as_reference() else {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: field subtree must use indirect objects"
                .to_string(),
        ));
    };
    if !references.insert(reference) {
        return Ok(());
    }
    let resolved = reader.resolve(source.clone())?;
    let Some(dict) = resolved.as_dict() else {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: field subtree object is not a dictionary"
                .to_string(),
        ));
    };
    if let Some(reference) = dict.get("Kids").and_then(PdfObject::as_reference) {
        array_references.insert(reference);
    }
    for child in resolve_pdf_array(reader, dict.get("Kids")) {
        collect_form_subtree_references(reader, &child, depth + 1, references, array_references)?;
    }
    Ok(())
}

fn remove_form_references(items: &mut Vec<PdfObject>, deleted: &BTreeSet<(u32, u16)>) {
    items.retain(|item| {
        item.as_reference()
            .is_none_or(|reference| !deleted.contains(&reference))
    });
}

fn delete_form_field_pdf(input: &[u8], field_name: &str) -> Result<(Vec<u8>, Value)> {
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform_object = catalog.get("AcroForm").ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?;
    let acroform = reader.resolve(acroform_object.clone())?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let mut target = None;
    for field in &fields {
        if let Some(reference) = form_field_reference_by_name(reader, field, "", field_name, 0)? {
            target = Some(reference);
            break;
        }
    }
    let target = target.ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
        ))
    })?;
    let target_source = PdfObject::Reference {
        number: target.0,
        generation: target.1,
    };
    let mut deleted = BTreeSet::new();
    let mut array_references = BTreeSet::new();
    collect_form_subtree_references(
        reader,
        &target_source,
        0,
        &mut deleted,
        &mut array_references,
    )?;
    if let Some(reference) = acroform
        .as_dict()
        .and_then(|dict| dict.get("Fields"))
        .and_then(PdfObject::as_reference)
    {
        array_references.insert(reference);
    }
    for page in document.get_pages()? {
        let page_object = reader.get_and_resolve(page.object_number, page.generation_number)?;
        if let Some(reference) = page_object
            .as_dict()
            .and_then(|dict| dict.get("Annots"))
            .and_then(PdfObject::as_reference)
        {
            array_references.insert(reference);
        }
    }
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if deleted.iter().any(|(candidate, _)| *candidate == number) {
            *object = PdfObject::Null;
            changed = true;
            return;
        }
        if array_references
            .iter()
            .any(|(candidate, _)| *candidate == number)
        {
            if let PdfObject::Array(items) = object {
                let before = items.len();
                remove_form_references(items, &deleted);
                changed |= items.len() != before;
            }
        }
        if let PdfObject::Dictionary(dict) = object {
            for key in ["Fields", "Kids", "Annots", "CO"] {
                if let Some(PdfObject::Array(items)) = dict.get_mut(key) {
                    let before = items.len();
                    remove_form_references(items, &deleted);
                    changed |= items.len() != before;
                }
            }
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: field deletion did not alter any source field-tree or annotation object"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "delete_acroform_field_subtree",
            "field_object": format!("{} {} R", target.0, target.1),
            "deleted_object_count": deleted.len(),
            "widget_annotations_removed": true,
        }),
    ))
}

fn document_subsystems_object_remap(reader: &PdfReader) -> BTreeMap<u32, u32> {
    let mut remap = BTreeMap::new();
    let mut next = 1u32;
    for (number, _) in reader.object_ids() {
        remap.entry(number).or_insert_with(|| {
            let current = next;
            next = next.saturating_add(1);
            current
        });
    }
    remap
}

fn document_subsystems_pdf_numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}

fn document_subsystems_pdf_literal(value: &str) -> Result<String> {
    if !value.is_ascii() || value.chars().any(char::is_control) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems unsupported_script: viewer-independent text-field creation currently requires exact ASCII values"
                .to_string(),
        ));
    }
    Ok(value
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)"))
}

fn create_text_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
    value: &str,
) -> Result<(Vec<u8>, Value)> {
    if field_name.trim().is_empty()
        || field_name.contains('.')
        || !field_name.is_ascii()
        || field_name.chars().any(char::is_control)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems duplicate_field_name: text-field creation requires a unique nonempty ASCII root field name"
                .to_string(),
        ));
    }
    let rect = rect_from_pdf_bounds(bounds)?;
    let value_literal = document_subsystems_pdf_literal(value)?;
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if crate::forms_report(&engine)?
        .fields
        .iter()
        .any(|field| field.full_name == field_name)
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems duplicate_field_name: {field_name} already exists in the current AcroForm field tree"
        )));
    }
    let page_source = document.get_page(page)?;
    let page_source_number = page_source.object_number;
    let page_annots = reader
        .get_and_resolve(page_source.object_number, page_source.generation_number)?
        .as_dict()
        .and_then(|dict| dict.get("Annots"))
        .cloned();
    let catalog = document.get_catalog()?;
    let root_source_number = reader
        .root_reference()
        .ok_or_else(|| WellfriendError::MalformedPdf("catalog root is missing".to_string()))?
        .0;
    let acroform_source = catalog.get("AcroForm").cloned();
    let acroform_dict = acroform_source
        .as_ref()
        .map(|object| reader.resolve(object.clone()))
        .transpose()?
        .and_then(|object| object.as_dict().cloned());
    if acroform_source.is_some() && acroform_dict.is_none() {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: catalog AcroForm is not a dictionary"
                .to_string(),
        ));
    }
    let remap = document_subsystems_object_remap(reader);
    let mut next = remap.values().copied().max().unwrap_or(0).saturating_add(1);
    let acroform_number = if acroform_source.is_some() {
        acroform_source
            .as_ref()
            .and_then(PdfObject::as_reference)
            .and_then(|reference| remap.get(&reference.0).copied())
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "document_subsystems structure_update_failed: indirect AcroForm is required for field creation"
                        .to_string(),
                )
            })?
    } else {
        let number = next;
        next = next.saturating_add(1);
        number
    };
    let field_number = next;
    next = next.saturating_add(1);
    let widget_number = next;
    next = next.saturating_add(1);
    let appearance_number = next;
    next = next.saturating_add(1);
    let font_number = next;
    let page_output_number = remap.get(&page_source_number).copied().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: page object is missing from canonical remap"
                .to_string(),
        )
    })?;
    let fields_array_source = acroform_dict
        .as_ref()
        .and_then(|dict| dict.get("Fields"))
        .and_then(PdfObject::as_reference)
        .map(|reference| reference.0);
    let annots_array_source = page_annots
        .as_ref()
        .and_then(PdfObject::as_reference)
        .map(|reference| reference.0);
    let mut changed = false;
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number == root_source_number && acroform_source.is_none() {
            if let PdfObject::Dictionary(dict) = object {
                dict.insert(
                    "AcroForm",
                    PdfObject::Reference {
                        number: acroform_number,
                        generation: 0,
                    },
                );
                changed = true;
            }
        }
        if acroform_source
            .as_ref()
            .and_then(PdfObject::as_reference)
            .is_some_and(|reference| number == reference.0)
        {
            if let PdfObject::Dictionary(dict) = object {
                if fields_array_source.is_none() {
                    let fields = dict
                        .get("Fields")
                        .and_then(PdfObject::as_array)
                        .map(<[PdfObject]>::to_vec)
                        .unwrap_or_default();
                    let mut fields = fields;
                    fields.push(PdfObject::Reference {
                        number: field_number,
                        generation: 0,
                    });
                    dict.insert("Fields", PdfObject::Array(fields));
                    changed = true;
                }
                dict.insert("NeedAppearances", PdfObject::Boolean(false));
            }
        }
        if fields_array_source == Some(number) {
            if let PdfObject::Array(fields) = object {
                fields.push(PdfObject::Reference {
                    number: field_number,
                    generation: 0,
                });
                changed = true;
            }
        }
        if number == page_source_number {
            if let PdfObject::Dictionary(dict) = object {
                if annots_array_source.is_none() {
                    let annots = dict
                        .get("Annots")
                        .and_then(PdfObject::as_array)
                        .map(<[PdfObject]>::to_vec)
                        .unwrap_or_default();
                    let mut annots = annots;
                    annots.push(PdfObject::Reference {
                        number: widget_number,
                        generation: 0,
                    });
                    dict.insert("Annots", PdfObject::Array(annots));
                    changed = true;
                }
            }
        }
        if annots_array_source == Some(number) {
            if let PdfObject::Array(annots) = object {
                annots.push(PdfObject::Reference {
                    number: widget_number,
                    generation: 0,
                });
                changed = true;
            }
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: field creation could not update catalog, field tree, or page annotations"
                .to_string(),
        ));
    }
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
    let mut appearance_resources = PdfDictionary::empty();
    let mut appearance_fonts = PdfDictionary::empty();
    appearance_fonts.insert(
        "F0",
        PdfObject::Reference {
            number: font_number,
            generation: 0,
        },
    );
    appearance_resources.insert("Font", PdfObject::Dictionary(appearance_fonts));
    let mut appearance = PdfDictionary::empty();
    appearance.insert("Type", PdfObject::Name("XObject".to_string()));
    appearance.insert("Subtype", PdfObject::Name("Form".to_string()));
    appearance.insert("FormType", PdfObject::Integer(1));
    appearance.insert(
        "BBox",
        document_subsystems_pdf_numbers(&[0.0, 0.0, rect.width, rect.height]),
    );
    appearance.insert("Resources", PdfObject::Dictionary(appearance_resources));
    let baseline = (rect.height - 12.0).max(3.0);
    let appearance_content = format!(
        "q 0 0 {} {} re W n BT /F0 10 Tf 0 g 3 {} Td ({}) Tj ET Q\n",
        rect.width, rect.height, baseline, value_literal
    );
    let mut field = PdfDictionary::empty();
    field.insert("FT", PdfObject::Name("Tx".to_string()));
    field.insert("T", PdfObject::String(field_name.as_bytes().to_vec()));
    field.insert("V", PdfObject::String(value.as_bytes().to_vec()));
    field.insert("DV", PdfObject::String(value.as_bytes().to_vec()));
    field.insert("DA", PdfObject::String(b"/F0 10 Tf 0 g".to_vec()));
    field.insert(
        "Kids",
        PdfObject::Array(vec![PdfObject::Reference {
            number: widget_number,
            generation: 0,
        }]),
    );
    let mut widget = PdfDictionary::empty();
    widget.insert("Type", PdfObject::Name("Annot".to_string()));
    widget.insert("Subtype", PdfObject::Name("Widget".to_string()));
    widget.insert("Rect", document_subsystems_pdf_numbers(&bounds));
    widget.insert(
        "P",
        PdfObject::Reference {
            number: page_output_number,
            generation: 0,
        },
    );
    widget.insert(
        "Parent",
        PdfObject::Reference {
            number: field_number,
            generation: 0,
        },
    );
    widget.insert("F", PdfObject::Integer(4));
    let mut appearance_dict = PdfDictionary::empty();
    appearance_dict.insert(
        "N",
        PdfObject::Reference {
            number: appearance_number,
            generation: 0,
        },
    );
    widget.insert("AP", PdfObject::Dictionary(appearance_dict));
    if acroform_source.is_none() {
        let mut acroform = PdfDictionary::empty();
        acroform.insert(
            "Fields",
            PdfObject::Array(vec![PdfObject::Reference {
                number: field_number,
                generation: 0,
            }]),
        );
        acroform.insert("NeedAppearances", PdfObject::Boolean(false));
        let mut dr = PdfDictionary::empty();
        let mut dr_fonts = PdfDictionary::empty();
        dr_fonts.insert(
            "F0",
            PdfObject::Reference {
                number: font_number,
                generation: 0,
            },
        );
        dr.insert("Font", PdfObject::Dictionary(dr_fonts));
        acroform.insert("DR", PdfObject::Dictionary(dr));
        objects.push(OutputObject {
            number: acroform_number,
            object: PdfObject::Dictionary(acroform),
        });
    }
    objects.extend([
        OutputObject {
            number: field_number,
            object: PdfObject::Dictionary(field),
        },
        OutputObject {
            number: widget_number,
            object: PdfObject::Dictionary(widget),
        },
        OutputObject {
            number: appearance_number,
            object: PdfObject::Stream {
                dict: appearance,
                raw: appearance_content.into_bytes(),
            },
        },
        OutputObject {
            number: font_number,
            object: PdfObject::Dictionary(font),
        },
    ]);
    objects.sort_by_key(|object| object.number);
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "create_acroform_text_field_with_normal_appearance",
            "field_name": field_name,
            "page": page,
            "field_object": field_number,
            "widget_object": widget_number,
            "appearance_object": appearance_number,
            "viewer_independent_appearance": true,
        }),
    ))
}

fn create_signature_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
) -> Result<(Vec<u8>, Value)> {
    // Reuse the canonical field-tree, widget, resource, and blank appearance
    // construction path, then change only the terminal field dictionary to a
    // real unsigned `/Sig` field. This never touches an existing signature.
    let (seed, _) = create_text_form_field_pdf(input, field_name, page, bounds, "")?;
    let document = PdfDocument::open_bytes(seed)?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("created AcroForm is missing".to_string())
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let field_reference = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created signature field cannot be resolved"
                    .to_string(),
            )
        })?;
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != field_reference.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("FT", PdfObject::Name("Sig".to_string()));
            dict.remove("V");
            dict.remove("DV");
            dict.remove("DA");
            dict.remove("Ff");
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: signature creation did not update its terminal field"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let field = crate::forms_report(&reopened)?
        .fields
        .into_iter()
        .find(|field| field.full_name == field_name)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created signature field is absent after reopen"
                    .to_string(),
            )
        })?;
    if !field.is_signature {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created field is not reported as a signature field"
                .to_string(),
        ));
    }
    Ok((
        output,
        json!({
            "operation": "create_unsigned_acroform_signature_field",
            "field_name": field_name,
            "page": page,
            "field_object": format!("{} {} R", field_reference.0, field_reference.1),
            "viewer_independent_blank_appearance": true,
            "signature_value_policy": "unsigned_field_created_existing_signature_values_immutable",
        }),
    ))
}

fn checkbox_appearance(width: f64, height: f64, marked: bool) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".to_string()));
    dict.insert("Subtype", PdfObject::Name("Form".to_string()));
    dict.insert("FormType", PdfObject::Integer(1));
    dict.insert(
        "BBox",
        document_subsystems_pdf_numbers(&[0.0, 0.0, width, height]),
    );
    let right = (width - 1.0).max(1.0);
    let top = (height - 1.0).max(1.0);
    let mut content = format!("q 0 0 0 RG 1 w 0.5 0.5 {right} {top} re S Q\n");
    if marked {
        let x = (width * 0.25).max(2.0);
        let y = (height * 0.45).max(2.0);
        let x2 = (width * 0.45).max(3.0);
        let y2 = (height * 0.2).max(2.0);
        let x3 = (width * 0.78).max(4.0);
        let y3 = (height * 0.78).max(4.0);
        content.push_str(&format!(
            "q 0 0 0 RG 1.5 w {x} {y} m {x2} {y2} l {x3} {y3} l S Q\n"
        ));
    }
    PdfObject::Stream {
        dict,
        raw: content.into_bytes(),
    }
}

fn create_checkbox_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
    checked: bool,
) -> Result<(Vec<u8>, Value)> {
    let rect = rect_from_pdf_bounds(bounds)?;
    if rect.width < 8.0 || rect.height < 8.0 {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_geometry: checkbox creation requires at least an 8 by 8 point rectangle"
                .to_string(),
        ));
    }
    // Reuse the canonical field-tree, widget, annotation, and page-array
    // insertion routine.  The second transaction changes the real field and
    // replaces its normal appearance with named button states.
    let (seed, _) = create_text_form_field_pdf(input, field_name, page, bounds, "")?;
    let document = PdfDocument::open_bytes(seed)?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("created AcroForm is missing".to_string())
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let field_reference = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created checkbox field cannot be resolved"
                    .to_string(),
            )
        })?;
    let field = reader.get_and_resolve(field_reference.0, field_reference.1)?;
    let widgets = resolve_pdf_array(reader, field.as_dict().and_then(|dict| dict.get("Kids")));
    if widgets.len() != 1 {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created checkbox needs exactly one indirect widget"
                .to_string(),
        ));
    }
    let widget_reference = widgets[0].as_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created checkbox needs exactly one indirect widget"
                .to_string(),
        )
    })?;
    let widget = reader.get_and_resolve(widget_reference.0, widget_reference.1)?;
    let off_source = widget
        .as_dict()
        .and_then(|dict| dict.get("AP"))
        .and_then(|appearance| reader.resolve(appearance.clone()).ok())
        .and_then(|appearance| appearance.as_dict().and_then(|dict| dict.get("N")).cloned())
        .and_then(|normal| normal.as_reference())
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created checkbox normal appearance is not indirect"
                    .to_string(),
            )
        })?;
    let remap = document_subsystems_object_remap(reader);
    let off_number = remap.get(&off_source.0).copied().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: checkbox normal appearance is absent from canonical remap"
                .to_string(),
        )
    })?;
    let yes_number = remap.values().copied().max().unwrap_or(0).saturating_add(1);
    let state = if checked { "Yes" } else { "Off" };
    let mut changed = false;
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number == field_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                dict.insert("FT", PdfObject::Name("Btn".to_string()));
                dict.insert("V", PdfObject::Name(state.to_string()));
                dict.insert("DV", PdfObject::Name(state.to_string()));
                dict.insert("Ff", PdfObject::Integer(0));
                changed = true;
            }
        }
        if number == widget_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                let mut normal_states = PdfDictionary::empty();
                normal_states.insert(
                    "Off",
                    PdfObject::Reference {
                        number: off_number,
                        generation: 0,
                    },
                );
                normal_states.insert(
                    "Yes",
                    PdfObject::Reference {
                        number: yes_number,
                        generation: 0,
                    },
                );
                let mut appearance = PdfDictionary::empty();
                appearance.insert("N", PdfObject::Dictionary(normal_states));
                dict.insert("AP", PdfObject::Dictionary(appearance));
                dict.insert("AS", PdfObject::Name(state.to_string()));
                changed = true;
            }
        }
        if number == off_source.0 {
            *object = checkbox_appearance(rect.width, rect.height, false);
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: checkbox creation did not update field, widget, and appearance"
                .to_string(),
        ));
    }
    objects.push(OutputObject {
        number: yes_number,
        object: checkbox_appearance(rect.width, rect.height, true),
    });
    objects.sort_by_key(|object| object.number);
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "create_acroform_checkbox_with_named_normal_appearances",
            "field_name": field_name,
            "page": page,
            "field_object": field_reference.0,
            "widget_object": widget_reference.0,
            "normal_states": ["Off", "Yes"],
            "selected_state": state,
            "viewer_independent_appearance": true,
        }),
    ))
}

fn create_radio_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
    export_value: &str,
    selected: bool,
) -> Result<(Vec<u8>, Value)> {
    if export_value.is_empty()
        || export_value.len() > 64
        || !export_value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems export_value_mismatch: radio export values require 1..=64 ASCII name-safe characters"
                .to_string(),
        ));
    }
    let (seed, _) = create_checkbox_form_field_pdf(input, field_name, page, bounds, selected)?;
    let document = PdfDocument::open_bytes(seed)?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("created AcroForm is missing".to_string())
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let field_reference = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created radio field cannot be resolved"
                    .to_string(),
            )
        })?;
    let field = reader.get_and_resolve(field_reference.0, field_reference.1)?;
    let widgets = resolve_pdf_array(reader, field.as_dict().and_then(|dict| dict.get("Kids")));
    if widgets.len() != 1 {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created radio field needs exactly one indirect widget"
                .to_string(),
        ));
    }
    let widget_reference = widgets[0].as_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created radio field needs exactly one indirect widget"
                .to_string(),
        )
    })?;
    let widget = reader.get_and_resolve(widget_reference.0, widget_reference.1)?;
    let normal = widget
        .as_dict()
        .and_then(|dict| dict.get("AP"))
        .and_then(|appearance| reader.resolve(appearance.clone()).ok())
        .and_then(|appearance| appearance.as_dict().and_then(|dict| dict.get("N")).cloned())
        .and_then(|normal| reader.resolve(normal).ok())
        .and_then(|normal| normal.as_dict().cloned())
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created radio normal appearance states are missing"
                    .to_string(),
            )
        })?;
    let off_source = normal
        .get("Off")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created radio off appearance is not indirect"
                    .to_string(),
            )
        })?;
    let on_source = normal
        .get("Yes")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created radio on appearance is not indirect"
                    .to_string(),
            )
        })?;
    let remap = document_subsystems_object_remap(reader);
    let off_number = remap.get(&off_source.0).copied().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: radio off appearance is absent from canonical remap"
                .to_string(),
        )
    })?;
    let on_number = remap.get(&on_source.0).copied().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: radio on appearance is absent from canonical remap"
                .to_string(),
        )
    })?;
    let state = if selected { export_value } else { "Off" };
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number == field_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                dict.insert("FT", PdfObject::Name("Btn".to_string()));
                dict.insert("Ff", PdfObject::Integer(1_i64 << 15));
                dict.insert("V", PdfObject::Name(state.to_string()));
                dict.insert("DV", PdfObject::Name(state.to_string()));
                changed = true;
            }
        }
        if number == widget_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                let mut states = PdfDictionary::empty();
                states.insert(
                    "Off",
                    PdfObject::Reference {
                        number: off_number,
                        generation: 0,
                    },
                );
                states.insert(
                    export_value,
                    PdfObject::Reference {
                        number: on_number,
                        generation: 0,
                    },
                );
                let mut appearance = PdfDictionary::empty();
                appearance.insert("N", PdfObject::Dictionary(states));
                dict.insert("AP", PdfObject::Dictionary(appearance));
                dict.insert("AS", PdfObject::Name(state.to_string()));
                changed = true;
            }
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: radio creation did not update field and widget state"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "create_acroform_radio_with_export_state",
            "field_name": field_name,
            "page": page,
            "field_object": field_reference.0,
            "widget_object": widget_reference.0,
            "export_value": export_value,
            "selected": selected,
        }),
    ))
}

fn create_choice_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
    options: &[String],
    selected: Option<&str>,
    editable_combo: bool,
) -> Result<(Vec<u8>, Value)> {
    if options.is_empty()
        || options.len() > 100
        || options.iter().any(|option| {
            option.is_empty()
                || !option.is_ascii()
                || option.len() > 256
                || option.chars().any(char::is_control)
        })
        || options.iter().collect::<BTreeSet<_>>().len() != options.len()
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: choice creation requires 1..=100 unique printable ASCII options"
                .to_string(),
        ));
    }
    let selected = selected.unwrap_or(&options[0]);
    if !options.iter().any(|option| option == selected) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: selected choice value must occur in the declared option array"
                .to_string(),
        ));
    }
    let (seed, _) = create_text_form_field_pdf(input, field_name, page, bounds, selected)?;
    let document = PdfDocument::open_bytes(seed)?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("created AcroForm is missing".to_string())
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let field_reference = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created choice field cannot be resolved"
                    .to_string(),
            )
        })?;
    let flags = if editable_combo {
        (1_i64 << 17) | (1_i64 << 18)
    } else {
        0
    };
    let option_objects = options
        .iter()
        .map(|option| PdfObject::String(option.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != field_reference.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("FT", PdfObject::Name("Ch".to_string()));
            dict.insert("Ff", PdfObject::Integer(flags));
            dict.insert("Opt", PdfObject::Array(option_objects.clone()));
            dict.insert("V", PdfObject::String(selected.as_bytes().to_vec()));
            dict.insert("DV", PdfObject::String(selected.as_bytes().to_vec()));
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: choice creation did not update the source field dictionary"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "create_acroform_choice_field_with_normal_appearance",
            "field_name": field_name,
            "page": page,
            "field_object": field_reference.0,
            "option_count": options.len(),
            "editable_combo": editable_combo,
            "viewer_independent_appearance": true,
        }),
    ))
}

fn set_choice_options_pdf(
    input: &[u8],
    field_name: &str,
    options: &[String],
    selected: Option<&str>,
    editable_combo: bool,
) -> Result<(Vec<u8>, Value)> {
    if options.is_empty()
        || options.len() > 100
        || options.iter().any(|option| {
            option.is_empty()
                || !option.is_ascii()
                || option.len() > 256
                || option.chars().any(char::is_control)
        })
        || options.iter().collect::<BTreeSet<_>>().len() != options.len()
    {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: choice options require 1..=100 unique printable ASCII values"
                .to_string(),
        ));
    }
    let selected = selected.unwrap_or(&options[0]);
    if !options.iter().any(|option| option == selected) {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems invalid_field_value: selected choice value must occur in the replacement option array"
                .to_string(),
        ));
    }
    let document = PdfDocument::open_bytes(input.to_vec())?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "document_subsystems field_not_found: document has no AcroForm field tree".to_string(),
        )
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let target = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
            ))
        })?;
    let flags = if editable_combo {
        (1_i64 << 17) | (1_i64 << 18)
    } else {
        0
    };
    let option_objects = options
        .iter()
        .map(|option| PdfObject::String(option.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number != target.0 {
            return;
        }
        if let PdfObject::Dictionary(dict) = object {
            dict.insert("FT", PdfObject::Name("Ch".to_string()));
            dict.insert("Ff", PdfObject::Integer(flags));
            dict.insert("Opt", PdfObject::Array(option_objects.clone()));
            dict.insert("V", PdfObject::String(selected.as_bytes().to_vec()));
            dict.insert("DV", PdfObject::String(selected.as_bytes().to_vec()));
            changed = true;
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: choice option update did not reach the resolved field dictionary"
                .to_string(),
        ));
    }
    let staged = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    // The existing editor owns canonical choice appearance regeneration. It
    // writes the selected source value without replacing the field topology.
    let mut editor = PdfEditor::open_bytes(staged)?;
    editor.set_form_choice(field_name, selected);
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "set_acroform_choice_options_and_selection",
            "field_object": format!("{} {} R", target.0, target.1),
            "field_name": field_name,
            "option_count": options.len(),
            "editable_combo": editable_combo,
            "appearance_regeneration": "canonical_form_writer",
        }),
    ))
}

fn create_push_button_form_field_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    bounds: [f64; 4],
    caption: &str,
) -> Result<(Vec<u8>, Value)> {
    // The seed routine gives the button an indirect Form XObject with the
    // shared canonical font resources already owned by the document.
    let (seed, _) = create_text_form_field_pdf(input, field_name, page, bounds, caption)?;
    let document = PdfDocument::open_bytes(seed)?;
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let acroform = reader.resolve(catalog.get("AcroForm").cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("created AcroForm is missing".to_string())
    })?)?;
    let fields = resolve_pdf_array(
        reader,
        acroform.as_dict().and_then(|dict| dict.get("Fields")),
    );
    let field_reference = fields
        .iter()
        .find_map(|field| {
            form_field_reference_by_name(reader, field, "", field_name, 0).transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created push button cannot be resolved"
                    .to_string(),
            )
        })?;
    let field = reader.get_and_resolve(field_reference.0, field_reference.1)?;
    let widgets = resolve_pdf_array(reader, field.as_dict().and_then(|dict| dict.get("Kids")));
    if widgets.len() != 1 {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created push button needs exactly one indirect widget"
                .to_string(),
        ));
    }
    let widget_reference = widgets[0].as_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: created push button needs exactly one indirect widget"
                .to_string(),
        )
    })?;
    let widget = reader.get_and_resolve(widget_reference.0, widget_reference.1)?;
    let appearance_reference = widget
        .as_dict()
        .and_then(|dict| dict.get("AP"))
        .and_then(|appearance| reader.resolve(appearance.clone()).ok())
        .and_then(|appearance| appearance.as_dict().and_then(|dict| dict.get("N")).cloned())
        .and_then(|normal| normal.as_reference())
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: created push-button normal appearance is not indirect"
                    .to_string(),
            )
        })?;
    let output_appearance = document_subsystems_object_remap(reader)
        .get(&appearance_reference.0)
        .copied()
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "document_subsystems structure_update_failed: push-button appearance is absent from canonical remap"
                    .to_string(),
            )
        })?;
    let mut changed = false;
    let (objects, root, info) = rewrite_document_objects(reader, &mut |number, object| {
        if number == field_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                dict.insert("FT", PdfObject::Name("Btn".to_string()));
                dict.insert("Ff", PdfObject::Integer(1_i64 << 16));
                changed = true;
            }
        }
        if number == widget_reference.0 {
            if let PdfObject::Dictionary(dict) = object {
                let appearance_ref = PdfObject::Reference {
                    number: output_appearance,
                    generation: 0,
                };
                let mut appearance = PdfDictionary::empty();
                appearance.insert("N", appearance_ref.clone());
                appearance.insert("R", appearance_ref.clone());
                appearance.insert("D", appearance_ref);
                dict.insert("AP", PdfObject::Dictionary(appearance));
                changed = true;
            }
        }
    })?;
    if !changed {
        return Err(WellfriendError::MalformedPdf(
            "document_subsystems structure_update_failed: push-button creation did not update field and widget appearances"
                .to_string(),
        ));
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": "create_acroform_push_button_with_normal_rollover_down_appearances",
            "field_name": field_name,
            "page": page,
            "field_object": field_reference.0,
            "widget_object": widget_reference.0,
            "appearance_states": ["N", "R", "D"],
            "restricted_actions": "none_created",
        }),
    ))
}

fn move_resize_form_widget_pdf(
    input: &[u8],
    field_name: &str,
    page: usize,
    rect: [f64; 4],
) -> Result<(Vec<u8>, Value)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let (xfdf, _) = export_annotation_xfdf(&engine)?;
    let widgets = parse_annotation_xfdf(&xfdf)?
        .annotations
        .into_iter()
        .filter(|annotation| {
            annotation.subtype == "Widget"
                && annotation.page == page
                && annotation.widget_field.as_deref() == Some(field_name)
        })
        .collect::<Vec<_>>();
    if widgets.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "document_subsystems field_not_found: field {field_name} requires exactly one resolved widget on page {page}"
        )));
    }
    let widget = widgets
        .into_iter()
        .next()
        .expect("checked widget cardinality");
    let (output, report) = move_resize_annotation_pdf(input, &widget.id, page, rect)?;
    Ok((
        output,
        json!({
            "operation": "move_resize_form_widget",
            "field_name": field_name,
            "widget_annotation_id": widget.id,
            "canonical_annotation_geometry": report,
            "valid_existing_appearance": "preserved_or_regenerated_under_annotation_media_redaction_policy",
        }),
    ))
}

fn supported_form_edit(
    input: &[u8],
    action: &DocumentSubsystemsAction,
) -> Result<(Vec<u8>, Value)> {
    if let DocumentSubsystemsAction::FormCreateTextInTableCell {
        table_id,
        row,
        col,
        field_name,
        value,
    } = action
    {
        let table = editable_tables(input)?
            .into_iter()
            .find(|candidate| candidate.table_id == *table_id)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                ))
            })?;
        let cell = table
            .source
            .cells
            .iter()
            .find(|candidate| candidate.row == *row && candidate.col == *col)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                ))
            })?;
        if !cell.bbox.iter().all(|coordinate| coordinate.is_finite())
            || cell.bbox[2] <= cell.bbox[0]
            || cell.bbox[3] <= cell.bbox[1]
        {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems invalid_geometry: resolved table-cell bounds are not usable for a form widget".into(),
            ));
        }
        return create_text_form_field_pdf(input, field_name, table.page, cell.bbox, value);
    }
    if let DocumentSubsystemsAction::FormCreateCheckboxInTableCell {
        table_id,
        row,
        col,
        field_name,
        checked,
    } = action
    {
        let table = editable_tables(input)?
            .into_iter()
            .find(|candidate| candidate.table_id == *table_id)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                ))
            })?;
        let cell = table
            .source
            .cells
            .iter()
            .find(|candidate| candidate.row == *row && candidate.col == *col)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                ))
            })?;
        if !cell.bbox.iter().all(|coordinate| coordinate.is_finite())
            || cell.bbox[2] <= cell.bbox[0]
            || cell.bbox[3] <= cell.bbox[1]
        {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems invalid_geometry: resolved table-cell bounds are not usable for a form widget".into(),
            ));
        }
        return create_checkbox_form_field_pdf(input, field_name, table.page, cell.bbox, *checked);
    }
    if let DocumentSubsystemsAction::FormCreateChoiceInTableCell {
        table_id,
        row,
        col,
        field_name,
        options,
        selected,
        editable_combo,
    } = action
    {
        let table = editable_tables(input)?
            .into_iter()
            .find(|candidate| candidate.table_id == *table_id)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                ))
            })?;
        let cell = table
            .source
            .cells
            .iter()
            .find(|candidate| candidate.row == *row && candidate.col == *col)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                ))
            })?;
        if !cell.bbox.iter().all(|coordinate| coordinate.is_finite())
            || cell.bbox[2] <= cell.bbox[0]
            || cell.bbox[3] <= cell.bbox[1]
        {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems invalid_geometry: resolved table-cell bounds are not usable for a form widget".into(),
            ));
        }
        return create_choice_form_field_pdf(
            input,
            field_name,
            table.page,
            cell.bbox,
            options,
            selected.as_deref(),
            *editable_combo,
        );
    }
    if let DocumentSubsystemsAction::FormCreateSignature {
        field_name,
        page,
        rect,
    } = action
    {
        return create_signature_form_field_pdf(input, field_name, *page, *rect);
    }
    if let DocumentSubsystemsAction::FormSetCalculationOrder { field_names } = action {
        return set_form_calculation_order_pdf(input, field_names);
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let form_report = crate::forms_report(&engine)?;
    let requested_field = match action {
        DocumentSubsystemsAction::FormSetText { field_name, .. }
        | DocumentSubsystemsAction::FormSetChoice { field_name, .. }
        | DocumentSubsystemsAction::FormSetChoiceOptions { field_name, .. }
        | DocumentSubsystemsAction::FormSetCheckbox { field_name, .. }
        | DocumentSubsystemsAction::FormSetDefault { field_name, .. }
        | DocumentSubsystemsAction::FormSetButtonDefault { field_name, .. }
        | DocumentSubsystemsAction::FormRename { field_name, .. }
        | DocumentSubsystemsAction::FormDelete { field_name }
        | DocumentSubsystemsAction::FormMoveResizeWidget { field_name, .. } => {
            Some(field_name.as_str())
        }
        DocumentSubsystemsAction::FormImportData { .. }
        | DocumentSubsystemsAction::FormSetCalculationOrder { .. }
        | DocumentSubsystemsAction::FormReset { .. }
        | DocumentSubsystemsAction::FormFlatten
        | DocumentSubsystemsAction::FormCreateText { .. }
        | DocumentSubsystemsAction::FormCreateTextInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateCheckbox { .. }
        | DocumentSubsystemsAction::FormCreateCheckboxInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateChoiceInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateChoice { .. }
        | DocumentSubsystemsAction::FormCreatePushButton { .. }
        | DocumentSubsystemsAction::FormCreateRadio { .. }
        | DocumentSubsystemsAction::FormCreateSignature { .. } => None,
        _ => None,
    };
    if let Some(field_name) = requested_field {
        let field = form_report
            .fields
            .iter()
            .find(|field| field.full_name == field_name)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "document_subsystems field_not_found: {field_name} is not in the current AcroForm field tree"
                ))
            })?;
        if field.is_signature {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems signature_permission_violation: signature field values are not mutable"
                    .to_string(),
            ));
        }
        let compatible = matches!(
            (action, field.field_type.as_str()),
            (DocumentSubsystemsAction::FormSetText { .. }, "text")
                | (DocumentSubsystemsAction::FormSetChoice { .. }, "choice")
                | (
                    DocumentSubsystemsAction::FormSetChoiceOptions { .. },
                    "choice"
                )
                | (
                    DocumentSubsystemsAction::FormSetCheckbox { .. },
                    "checkbox" | "radio"
                )
                | (
                    DocumentSubsystemsAction::FormSetDefault { .. },
                    "text" | "choice"
                )
                | (
                    DocumentSubsystemsAction::FormSetButtonDefault { .. },
                    "checkbox" | "radio"
                )
                | (DocumentSubsystemsAction::FormRename { .. }, _)
                | (DocumentSubsystemsAction::FormDelete { .. }, _)
                | (DocumentSubsystemsAction::FormMoveResizeWidget { .. }, _)
        );
        if !compatible {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems invalid_field_value: action is incompatible with {field_name} field type {}",
                field.field_type
            )));
        }
    }
    if let DocumentSubsystemsAction::FormRename {
        field_name,
        new_name,
    } = action
    {
        return rename_form_field_pdf(input, field_name, new_name);
    }
    if let DocumentSubsystemsAction::FormSetDefault { field_name, value } = action {
        return set_form_default_pdf(input, field_name, value);
    }
    if let DocumentSubsystemsAction::FormSetButtonDefault {
        field_name,
        checked,
    } = action
    {
        return set_form_button_default_pdf(input, field_name, *checked);
    }
    if let DocumentSubsystemsAction::FormSetChoiceOptions {
        field_name,
        options,
        selected,
        editable_combo,
    } = action
    {
        return set_choice_options_pdf(
            input,
            field_name,
            options,
            selected.as_deref(),
            *editable_combo,
        );
    }
    if let DocumentSubsystemsAction::FormDelete { field_name } = action {
        return delete_form_field_pdf(input, field_name);
    }
    if let DocumentSubsystemsAction::FormCreateText {
        field_name,
        page,
        rect,
        value,
    } = action
    {
        return create_text_form_field_pdf(input, field_name, *page, *rect, value);
    }
    if let DocumentSubsystemsAction::FormCreateCheckbox {
        field_name,
        page,
        rect,
        checked,
    } = action
    {
        return create_checkbox_form_field_pdf(input, field_name, *page, *rect, *checked);
    }
    if let DocumentSubsystemsAction::FormCreateChoice {
        field_name,
        page,
        rect,
        options,
        selected,
        editable_combo,
    } = action
    {
        return create_choice_form_field_pdf(
            input,
            field_name,
            *page,
            *rect,
            options,
            selected.as_deref(),
            *editable_combo,
        );
    }
    if let DocumentSubsystemsAction::FormCreatePushButton {
        field_name,
        page,
        rect,
        caption,
    } = action
    {
        return create_push_button_form_field_pdf(input, field_name, *page, *rect, caption);
    }
    if let DocumentSubsystemsAction::FormCreateRadio {
        field_name,
        page,
        rect,
        export_value,
        selected,
    } = action
    {
        return create_radio_form_field_pdf(
            input,
            field_name,
            *page,
            *rect,
            export_value,
            *selected,
        );
    }
    if let DocumentSubsystemsAction::FormMoveResizeWidget {
        field_name,
        page,
        rect,
    } = action
    {
        return move_resize_form_widget_pdf(input, field_name, *page, *rect);
    }
    if let DocumentSubsystemsAction::FormImportData { data, format } = action {
        let format = FormDataFormat::parse(format).ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "document_subsystems unsupported_exact: unsupported form data format {format}"
            ))
        })?;
        let (output, report) = apply_form_data_pdf(input.to_vec(), data.as_bytes(), format)?;
        return Ok((
            output,
            json!({
                "operation": "import_form_data",
                "canonical_exchange": report,
                "appearance_regeneration": "canonical_form_writer",
            }),
        ));
    }
    if let DocumentSubsystemsAction::FormReset { field_name } = action {
        let selected = form_report
            .fields
            .iter()
            .filter(|field| {
                field_name
                    .as_deref()
                    .map(|name| name == field.full_name)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "document_subsystems field_not_found: {} is not in the current AcroForm field tree",
                field_name.as_deref().unwrap_or("requested field set")
            )));
        }
        let mut editor = PdfEditor::open_bytes(input.to_vec())?;
        for field in selected {
            if field.is_signature {
                continue;
            }
            let value = field.default_value.as_deref().unwrap_or_default();
            match field.field_type.as_str() {
                "text" => {
                    editor.set_form_text(&field.full_name, value);
                }
                "choice" => {
                    editor.set_form_choice(&field.full_name, value);
                }
                "checkbox" | "radio" => {
                    let checked = matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    );
                    editor.set_form_checkbox(&field.full_name, checked);
                }
                _ => {}
            }
        }
        let output = editor.save_to_bytes(EditMode::FullRewrite)?;
        return Ok((
            output,
            json!({
                "operation": "reset_to_default_value",
                "field_name": field_name,
                "appearance_regeneration": "canonical_form_writer",
            }),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    let operation = match action {
        DocumentSubsystemsAction::FormSetText { field_name, value } => {
            editor.set_form_text(field_name, value);
            "set_text"
        }
        DocumentSubsystemsAction::FormSetChoice { field_name, value } => {
            editor.set_form_choice(field_name, value);
            "set_choice"
        }
        DocumentSubsystemsAction::FormSetChoiceOptions { .. } => {
            unreachable!("choice options handled above")
        }
        DocumentSubsystemsAction::FormSetCheckbox {
            field_name,
            checked,
        } => {
            editor.set_form_checkbox(field_name, *checked);
            "set_checkbox"
        }
        DocumentSubsystemsAction::FormSetDefault { .. } => unreachable!("default value handled above"),
        DocumentSubsystemsAction::FormSetButtonDefault { .. } => {
            unreachable!("button default handled above")
        }
        DocumentSubsystemsAction::FormRename { .. } => unreachable!("rename handled above"),
        DocumentSubsystemsAction::FormDelete { .. } => unreachable!("delete handled above"),
        DocumentSubsystemsAction::FormCreateText { .. } => unreachable!("create handled above"),
        DocumentSubsystemsAction::FormCreateCheckbox { .. } => unreachable!("create handled above"),
        DocumentSubsystemsAction::FormCreateChoice { .. } => unreachable!("create handled above"),
        DocumentSubsystemsAction::FormCreatePushButton { .. } => unreachable!("create handled above"),
        DocumentSubsystemsAction::FormCreateRadio { .. } => unreachable!("create handled above"),
        DocumentSubsystemsAction::FormMoveResizeWidget { .. } => unreachable!("widget move handled above"),
        DocumentSubsystemsAction::FormReset { .. } => unreachable!("reset handled above"),
        DocumentSubsystemsAction::FormFlatten => {
            editor.flatten_forms();
            "flatten"
        }
        _ => {
            return Err(WellfriendError::UnsupportedFeature(
                "document_subsystems invalid_field_value: request action is not a canonical form operation"
                    .to_string(),
            ))
        }
    };
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    Ok((
        output,
        json!({
            "operation": operation,
            "writer": "canonical_pdf_editor_full_rewrite",
            "appearance_regeneration": "canonical_form_writer",
        }),
    ))
}

fn action_subsystem(action: &DocumentSubsystemsAction) -> DocumentSubsystemsSubsystem {
    match action {
        DocumentSubsystemsAction::TableEditCell { .. }
        | DocumentSubsystemsAction::TableEditMathCell { .. }
        | DocumentSubsystemsAction::TableMoveLinkedAnnotation { .. }
        | DocumentSubsystemsAction::TableSetCellAlignment { .. }
        | DocumentSubsystemsAction::TableSetCellPadding { .. }
        | DocumentSubsystemsAction::TableAddCellBorder { .. }
        | DocumentSubsystemsAction::TableSetCellFill { .. }
        | DocumentSubsystemsAction::TableAppendRow { .. }
        | DocumentSubsystemsAction::TableAppendColumn { .. } => DocumentSubsystemsSubsystem::Table,
        DocumentSubsystemsAction::MathReplace { .. }
        | DocumentSubsystemsAction::MathMoveResize { .. }
        | DocumentSubsystemsAction::MathEditMatrixCell { .. }
        | DocumentSubsystemsAction::MathEditMatrixStructure { .. }
        | DocumentSubsystemsAction::MathEditFractionPart { .. }
        | DocumentSubsystemsAction::MathEditScript { .. }
        | DocumentSubsystemsAction::MathEditFencedInner { .. }
        | DocumentSubsystemsAction::MathEditRadicand { .. } => DocumentSubsystemsSubsystem::Math,
        DocumentSubsystemsAction::OcrCorrectText { .. }
        | DocumentSubsystemsAction::OcrCorrectGeometry { .. }
        | DocumentSubsystemsAction::OcrAddSearchableText { .. }
        | DocumentSubsystemsAction::OcrAddSearchableTextWithLink { .. }
        | DocumentSubsystemsAction::OcrAddSearchableWords { .. } => {
            DocumentSubsystemsSubsystem::OcrSearchableLayer
        }
        DocumentSubsystemsAction::AnnotationCreate { .. }
        | DocumentSubsystemsAction::AnnotationEditContents { .. }
        | DocumentSubsystemsAction::AnnotationMoveResize { .. }
        | DocumentSubsystemsAction::AnnotationCreateReply { .. }
        | DocumentSubsystemsAction::AnnotationDeleteInRect { .. }
        | DocumentSubsystemsAction::AnnotationXfdf { .. }
        | DocumentSubsystemsAction::AnnotationFlatten { .. } => {
            DocumentSubsystemsSubsystem::AnnotationAppearance
        }
        DocumentSubsystemsAction::FormSetText { .. }
        | DocumentSubsystemsAction::FormSetChoice { .. }
        | DocumentSubsystemsAction::FormSetChoiceOptions { .. }
        | DocumentSubsystemsAction::FormSetCheckbox { .. }
        | DocumentSubsystemsAction::FormSetDefault { .. }
        | DocumentSubsystemsAction::FormSetButtonDefault { .. }
        | DocumentSubsystemsAction::FormRename { .. }
        | DocumentSubsystemsAction::FormDelete { .. }
        | DocumentSubsystemsAction::FormCreateText { .. }
        | DocumentSubsystemsAction::FormCreateTextInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateCheckbox { .. }
        | DocumentSubsystemsAction::FormCreateCheckboxInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateChoiceInTableCell { .. }
        | DocumentSubsystemsAction::FormCreateChoice { .. }
        | DocumentSubsystemsAction::FormCreatePushButton { .. }
        | DocumentSubsystemsAction::FormCreateRadio { .. }
        | DocumentSubsystemsAction::FormCreateSignature { .. }
        | DocumentSubsystemsAction::FormMoveResizeWidget { .. }
        | DocumentSubsystemsAction::FormImportData { .. }
        | DocumentSubsystemsAction::FormSetCalculationOrder { .. }
        | DocumentSubsystemsAction::FormReset { .. }
        | DocumentSubsystemsAction::FormFlatten => DocumentSubsystemsSubsystem::FormData,
        DocumentSubsystemsAction::XfaInventory
        | DocumentSubsystemsAction::XfaImportDatasets { .. }
        | DocumentSubsystemsAction::XfaFlattenStatic { .. } => {
            DocumentSubsystemsSubsystem::XfaPreservation
        }
    }
}

fn apply_explicit_action(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
    action: &DocumentSubsystemsAction,
) -> Result<(Vec<u8>, DocumentSubsystemsOperationReport)> {
    if action_subsystem(action) != request.subsystem {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems unsupported_exact: action does not match the selected subsystem"
                .to_string(),
        ));
    }
    let source_sha256 = digest(input);
    let (output, operation, transaction, appearance_effect, xfa_effect, changed_pages) =
        match action {
            DocumentSubsystemsAction::TableMoveLinkedAnnotation {
                table_id,
                row,
                col,
                annotation_id,
            } => {
                let graph = editable_tables(input)?
                    .into_iter()
                    .find(|candidate| candidate.table_id == *table_id)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                        ))
                    })?;
                let cell = graph
                    .source
                    .cells
                    .iter()
                    .find(|candidate| candidate.row == *row && candidate.col == *col)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                        ))
                    })?;
                let rect = cell.bbox;
                if !rect.iter().all(|value| value.is_finite())
                    || rect[2] <= rect[0]
                    || rect[3] <= rect[1]
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems invalid_geometry: resolved table-cell bounds are not usable for annotation movement".into(),
                    ));
                }
                let (output, report) =
                    move_resize_annotation_pdf(input, annotation_id, graph.page, rect)?;
                (
                    output,
                    "table_linked_annotation_source_move_and_appearance_regeneration",
                    json!({
                        "annotation": value(&report)?,
                        "table_id": table_id,
                        "cell": {"row": row, "col": col, "bounds": rect},
                        "relationship": "table_cell_to_annotation_geometry"
                    }),
                    json!({
                        "normal_rollover_down": "canonical_annotation_media_redaction_generation",
                        "geometry": "cell_bounds_linked_annotation"
                    }),
                    json!({"preserved": true}),
                    vec![graph.page],
                )
            }
            DocumentSubsystemsAction::TableEditMathCell {
                table_id,
                row,
                col,
                replacement_text,
                approved,
            } => {
                if !approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: table-cell mathematical replacement requires explicit approval".into(),
                    ));
                }
                let (reflow, table) =
                    table_cell_reflow(input, request, table_id, *row, *col, replacement_text)?;
                if !math_like(&reflow.source_text) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems math_structure_not_resolved: resolved table cell is not born-digital mathematical source".into(),
                    ));
                }
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, &reflow)?
                } else {
                    apply_reflow_region(input, &reflow)?
                };
                (
                    output,
                    "table_math_cell_source_rewrite",
                    json!({
                        "editing_transactions_transaction": report,
                        "table_id": table.table_id,
                        "cell": {"row": row, "col": col},
                        "math": {
                            "source_text": reflow.source_text,
                            "replacement_text": replacement_text,
                            "review_approved": true,
                            "layout": "editing_transactions_shaped_text_reflow_cell_reflow"
                        }
                    }),
                    json!({}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableEditCell {
                table_id,
                row,
                col,
                replacement_text,
            } => {
                let (reflow, table) =
                    table_cell_reflow(input, request, table_id, *row, *col, replacement_text)?;
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, &reflow)?
                } else {
                    apply_reflow_region(input, &reflow)?
                };
                (
                    output,
                    "table_cell_source_rewrite",
                    json!({
                        "editing_transactions_transaction": report,
                        "table_id": table.table_id,
                        "cell": {"row": row, "col": col},
                        "grid_source": table.source.source,
                    }),
                    json!({}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableSetCellAlignment {
                table_id,
                row,
                col,
                alignment,
            } => {
                let (reflow, table) =
                    table_cell_alignment_reflow(input, request, table_id, *row, *col, alignment)?;
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, &reflow)?
                } else {
                    apply_reflow_region(input, &reflow)?
                };
                (
                    output,
                    "table_cell_alignment_source_rewrite",
                    json!({
                        "editing_transactions_transaction": report,
                        "table_id": table.table_id,
                        "cell": {"row": row, "col": col},
                        "alignment": alignment,
                    }),
                    json!({"caption_footnote": "no_associated_caption_or_footnote_moved"}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableSetCellPadding {
                table_id,
                row,
                col,
                padding,
            } => {
                let (reflow, table) =
                    table_cell_padding_reflow(input, request, table_id, *row, *col, *padding)?;
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, &reflow)?
                } else {
                    apply_reflow_region(input, &reflow)?
                };
                (
                    output,
                    "table_cell_padding_source_rewrite",
                    json!({
                        "editing_transactions_transaction": report,
                        "table_id": table.table_id,
                        "cell": {"row": row, "col": col},
                        "padding": padding,
                    }),
                    json!({"caption_footnote": "no_associated_caption_or_footnote_moved"}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableSetCellFill {
                table_id,
                row,
                col,
                color_rgb,
                opacity,
            } => {
                if !opacity.is_finite()
                    || *opacity < 0.0
                    || *opacity > 1.0
                    || !color_rgb.iter().all(|component| component.is_finite() && (0.0..=1.0).contains(component))
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems invalid_geometry: table-cell fill color and opacity must be finite values in 0..=1".into(),
                    ));
                }
                let table = editable_tables(input)?
                    .into_iter()
                    .find(|candidate| candidate.table_id == *table_id)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                        ))
                    })?;
                let cell = table
                    .source
                    .cells
                    .iter()
                    .find(|candidate| candidate.row == *row && candidate.col == *col)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                        ))
                    })?;
                let mut editor = PdfEditor::open_bytes(input.to_vec())?;
                let style = EditRectStyle {
                    stroke: None,
                    fill: Some(Color::device_rgb(color_rgb[0], color_rgb[1], color_rgb[2])),
                    opacity: *opacity,
                    ..EditRectStyle::default()
                };
                editor.draw_rect(
                    table.page,
                    rect_from_pdf_bounds(cell.bbox)?,
                    style,
                    OverlayLayer::Underlay,
                )?;
                let output = editor.save_to_bytes(EditMode::FullRewrite)?;
                ContentEngine::open_bytes(output.clone())?;
                (
                    output,
                    "table_cell_fill_source_instruction_append",
                    json!({
                        "table_id": table_id,
                        "cell": {"row": row, "col": col, "bounds": cell.bbox},
                        "color_rgb": color_rgb,
                        "opacity": opacity,
                        "source_edit": "canonical_pdf_editor_rect_underlay"
                    }),
                    json!({"fill": "canonical_underlay_rect"}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableAddCellBorder {
                table_id,
                row,
                col,
                line_width,
            } => {
                if !line_width.is_finite() || *line_width <= 0.0 || *line_width > 36.0 {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems invalid_geometry: table-cell border width must be finite and within 0..=36 PDF units".into(),
                    ));
                }
                let table = editable_tables(input)?
                    .into_iter()
                    .find(|candidate| candidate.table_id == *table_id)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems table_not_resolved: table {table_id} is not in the current source snapshot"
                        ))
                    })?;
                let cell = table
                    .source
                    .cells
                    .iter()
                    .find(|candidate| candidate.row == *row && candidate.col == *col)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems grid_ambiguous: table {table_id} has no uniquely resolved cell at row {row}, column {col}"
                        ))
                    })?;
                let mut editor = PdfEditor::open_bytes(input.to_vec())?;
                let style = EditRectStyle {
                    line_width: *line_width,
                    ..EditRectStyle::default()
                };
                editor.draw_rect(
                    table.page,
                    rect_from_pdf_bounds(cell.bbox)?,
                    style,
                    OverlayLayer::Overlay,
                )?;
                let output = editor.save_to_bytes(EditMode::FullRewrite)?;
                ContentEngine::open_bytes(output.clone())?;
                (
                    output,
                    "table_cell_border_source_instruction_append",
                    json!({
                        "table_id": table_id,
                        "cell": {"row": row, "col": col, "bounds": cell.bbox},
                        "line_width": line_width,
                        "source_edit": "canonical_pdf_editor_rect_stroke"
                    }),
                    json!({"border": "canonical_stroked_rect"}),
                    json!({"preserved": true}),
                    vec![table.page],
                )
            }
            DocumentSubsystemsAction::TableAppendRow {
                table_id,
                values,
                row_height,
            } => {
                let (output, transaction, page) =
                    append_simple_ruled_table_row(input, table_id, values, *row_height)?;
                (
                    output,
                    "table_append_row_source_instructions",
                    transaction,
                    json!({"caption_footnote": "no_associated_caption_or_footnote_moved"}),
                    json!({"preserved": true}),
                    vec![page],
                )
            }
            DocumentSubsystemsAction::TableAppendColumn {
                table_id,
                values,
                column_width,
            } => {
                let (output, transaction, page) =
                    append_simple_ruled_table_column(input, table_id, values, *column_width)?;
                (
                    output,
                    "table_append_column_source_instructions",
                    transaction,
                    json!({"caption_footnote": "no_associated_caption_or_footnote_moved"}),
                    json!({"preserved": true}),
                    vec![page],
                )
            }
            DocumentSubsystemsAction::MathMoveResize { bounds } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source movement requires explicit approval"
                            .to_string(),
                    ));
                }
                if !bounds.iter().all(|value| value.is_finite())
                    || bounds[2] <= bounds[0]
                    || bounds[3] <= bounds[1]
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems invalid_geometry: mathematical destination bounds are not usable"
                            .to_string(),
                    ));
                }
                let source_text = reflow_required(request, "math_structure_not_resolved")?
                    .source_text
                    .clone();
                let mut reflow = resolved_math_reflow(input, request, &source_text)?;
                reflow.region = Some(*bounds);
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, &reflow)?
                } else {
                    apply_reflow_region(input, &reflow)?
                };
                (
                    output,
                    "math_source_move_resize_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": source_text,
                        "destination_bounds": bounds,
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "geometry": "text_reflow_source_region_rewrite",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathReplace { replacement_text } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = resolved_math_reflow(input, request, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_source_rewrite_with_editing_transactions_shaping",
                    json!({"editing_transactions_transaction": report, "source_text": reflow.source_text}),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "unresolved_raster_or_outlined_formula": "formula_review_required"
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditMatrixCell {
                row,
                col,
                replacement_text,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = matrix_cell_reflow(input, request, *row, *col, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_matrix_cell_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                        "cell": {"row": row, "col": col},
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "matrix_structure": "resolved_bracket_matrix",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditMatrixStructure {
                operation,
                index,
                values,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = matrix_structure_reflow(input, request, operation, *index, values)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_matrix_structure_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                        "operation": operation,
                        "index": index,
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "matrix_structure": "resolved_bracket_matrix",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditFractionPart {
                part,
                replacement_text,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = fraction_part_reflow(input, request, part, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_fraction_part_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                        "part": part,
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "fraction_structure": "resolved_single_slash_fraction",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditScript {
                script_kind,
                replacement_text,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = math_script_reflow(input, request, script_kind, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_script_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                        "script_kind": script_kind,
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "script_structure": "resolved_single_source_script",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditFencedInner { replacement_text } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = math_fenced_inner_reflow(input, request, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_fenced_inner_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                        "delimiter_pair": "preserved_resolved_pair",
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "fenced_structure": "resolved_single_layer_fence",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::MathEditRadicand { replacement_text } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems formula_review_required: mathematical source replacement requires explicit approval"
                            .to_string(),
                    ));
                }
                let reflow = math_radicand_reflow(input, request, replacement_text)?;
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "math_radicand_source_rewrite_with_editing_transactions_shaping",
                    json!({
                        "editing_transactions_transaction": report,
                        "source_text": reflow.source_text,
                    }),
                    json!({
                        "math_layout": "source_text_shaped_by_editing_transactions",
                        "radical_structure": "resolved_source_radical",
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::OcrCorrectText { replacement_text } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems reconstruction_review_required: searchable-layer correction requires explicit approval"
                            .to_string(),
                    ));
                }
                let mut reflow = reflow_required(request, "scan_not_resolved")?.clone();
                reflow.replacement_text = replacement_text.clone();
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "ocr_searchable_layer_source_correction",
                    json!({"editing_transactions_transaction": report, "source_text": reflow.source_text}),
                    json!({
                        "original_scan_preserved": true,
                        "text_rendering": "source-linked_existing_searchable_layer"
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::OcrCorrectGeometry { bounds } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems reconstruction_review_required: searchable-layer geometry correction requires explicit approval"
                            .to_string(),
                    ));
                }
                rect_from_pdf_bounds(*bounds)?;
                let mut reflow = reflow_required(request, "scan_not_resolved")?.clone();
                reflow.region = Some(*bounds);
                reflow.replacement_text = reflow.source_text.clone();
                let (output, report) = apply_reflow_region(input, &reflow)?;
                (
                    output,
                    "ocr_searchable_layer_source_geometry_correction",
                    value(&report)?,
                    json!({
                        "source_scan_preserved": true,
                        "text_rendering": "source-linked_existing_searchable_layer",
                        "target_bounds": bounds,
                    }),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsAction::OcrAddSearchableText {
                page,
                text,
                rect,
                font_size,
                provider_id,
                provider_version,
                confidence,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems reconstruction_review_required: searchable-layer creation requires explicit approval"
                            .to_string(),
                    ));
                }
                let (output, report) = supported_ocr_add_searchable_text(
                    input,
                    *page,
                    text,
                    *rect,
                    *font_size,
                    provider_id,
                    provider_version.as_deref(),
                    *confidence,
                )?;
                (
                    output,
                    "ocr_searchable_layer_creation",
                    report,
                    json!({
                        "original_scan_preserved": true,
                        "text_rendering": "invisible_searchable_text"
                    }),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::OcrAddSearchableTextWithLink {
                page,
                text,
                rect,
                font_size,
                provider_id,
                provider_version,
                confidence,
                uri,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems reconstruction_review_required: OCR searchable text with link creation requires explicit approval"
                            .to_string(),
                    ));
                }
                if uri.trim().is_empty() || uri.len() > 4_096 {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems invalid_geometry: OCR-linked annotation URI must be a bounded nonempty value"
                            .to_string(),
                    ));
                }
                let (ocr_output, ocr_report) = supported_ocr_add_searchable_text(
                    input,
                    *page,
                    text,
                    *rect,
                    *font_size,
                    provider_id,
                    provider_version.as_deref(),
                    *confidence,
                )?;
                let mut editor = PdfEditor::open_bytes(ocr_output)?;
                editor.add_link_uri(*page, rect_from_pdf_bounds(*rect)?, uri)?;
                let output = editor.save_to_bytes(EditMode::FullRewrite)?;
                ContentEngine::open_bytes(output.clone())?;
                (
                    output,
                    "ocr_searchable_layer_with_source_link_annotation",
                    json!({
                        "ocr": ocr_report,
                        "provider": {"id": provider_id, "version": provider_version},
                        "source_image_geometry": rect,
                        "uri": uri,
                    }),
                    json!({
                        "original_scan_preserved": true,
                        "text_rendering": "invisible_searchable_text",
                        "annotation": "canonical_uri_link"
                    }),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::OcrAddSearchableWords {
                page,
                words,
                provider_id,
                provider_version,
                language,
            } => {
                let (output, report) = supported_ocr_add_searchable_words(
                    input,
                    *page,
                    words,
                    provider_id,
                    provider_version.as_deref(),
                    language.as_deref(),
                )?;
                (
                    output,
                    "ocr_searchable_layer_creation",
                    report,
                    json!({
                        "original_scan": "preserved",
                        "editable_reconstruction": "not_implicitly_applied",
                    }),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationCreate {
                page,
                subtype,
                rect,
                contents,
                uri,
            } => {
                let (output, report) = supported_annotation_create(
                    input,
                    *page,
                    subtype,
                    *rect,
                    contents,
                    uri.as_deref(),
                )?;
                (
                    output,
                    "annotation_create_and_appearance_regeneration",
                    report,
                    json!({"normal_rollover_down": "generated_for_supported_annotation"}),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationEditContents {
                page,
                annotation_index,
                contents,
            } => {
                let mut editor = PdfEditor::open_bytes(input.to_vec())?;
                editor.edit_annotation_contents(*page, *annotation_index, contents)?;
                let edited = editor.save_to_bytes(EditMode::FullRewrite)?;
                let (output, appearance) = generate_annotation_appearances_pdf(
                    &edited,
                    &AnnotationAppearanceOptions::default(),
                )?;
                (
                    output,
                    "annotation_contents_and_appearance_regeneration",
                    json!({"source_edit": "PdfEditor", "appearance": appearance}),
                    json!({"normal_rollover_down": "regenerated_for_supported_annotation"}),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationMoveResize {
                annotation_id,
                page,
                rect,
            } => {
                let (output, report) =
                    move_resize_annotation_pdf(input, annotation_id, *page, *rect)?;
                (
                    output,
                    "annotation_move_resize_source_update_and_appearance_regeneration",
                    value(&report)?,
                    json!({
                        "normal_rollover_down": "canonical_annotation_media_redaction_generation",
                        "geometry": "rect_linked_quads_vertices_lines_callouts_ink",
                    }),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationCreateReply {
                parent_annotation_id,
                page,
                rect,
                contents,
            } => {
                let (output, report) = supported_annotation_create_reply(
                    input,
                    parent_annotation_id,
                    *page,
                    *rect,
                    contents,
                )?;
                (
                    output,
                    "annotation_reply_source_update_and_appearance_regeneration",
                    report,
                    json!({"reply_relationship": "canonical_irt_reference"}),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationDeleteInRect { page, rect } => {
                let mut editor = PdfEditor::open_bytes(input.to_vec())?;
                editor.delete_annotations_in_rect(*page, rect_from_pdf_bounds(*rect)?)?;
                let edited = editor.save_to_bytes(EditMode::FullRewrite)?;
                let (output, appearance) = generate_annotation_appearances_pdf(
                    &edited,
                    &AnnotationAppearanceOptions::default(),
                )?;
                (
                    output,
                    "annotation_delete_source_update",
                    json!({"source_edit": "PdfEditor", "appearance": appearance}),
                    json!({"stale_appearance_references": "removed_with_annotation"}),
                    json!({"preserved": true}),
                    vec![*page],
                )
            }
            DocumentSubsystemsAction::AnnotationXfdf { xfdf, delete_ids } => {
                let mut options = AnnotationXfdfImportOptions {
                    fail_on_unsupported: true,
                    ..AnnotationXfdfImportOptions::default()
                };
                if !delete_ids.is_empty() {
                    options.delete_policy = AnnotationDeletePolicy::ExplicitIds;
                    options.delete_ids = delete_ids.clone();
                }
                let (output, report) = import_annotation_xfdf_pdf(input, xfdf.as_bytes(), &options)?;
                (
                    output,
                    "annotation_xfdf_source_update_and_appearance_regeneration",
                    value(&report)?,
                    json!({
                        "normal_rollover_down": "canonical_annotation_media_redaction_generation",
                        "reply_popup_relationships": "source_linked_xfdf_records"
                    }),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            DocumentSubsystemsAction::AnnotationFlatten { subtypes } => {
                let mut editor = PdfEditor::open_bytes(input.to_vec())?;
                if subtypes.is_empty() {
                    editor.flatten_annotations();
                } else {
                    editor.flatten_annotation_subtypes(subtypes.clone());
                }
                let output = editor.save_to_bytes(EditMode::FullRewrite)?;
                (
                    output,
                    "annotation_flatten",
                    json!({
                        "source_edit": "PdfEditor",
                        "selected_subtypes": subtypes,
                        "live_annotation_removed_under_policy": true,
                    }),
                    json!({"flattened": true}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            DocumentSubsystemsAction::FormSetText { .. }
            | DocumentSubsystemsAction::FormSetChoice { .. }
            | DocumentSubsystemsAction::FormSetChoiceOptions { .. }
            | DocumentSubsystemsAction::FormSetCheckbox { .. }
            | DocumentSubsystemsAction::FormSetDefault { .. }
            | DocumentSubsystemsAction::FormSetButtonDefault { .. }
            | DocumentSubsystemsAction::FormRename { .. }
            | DocumentSubsystemsAction::FormDelete { .. }
            | DocumentSubsystemsAction::FormCreateText { .. }
            | DocumentSubsystemsAction::FormCreateTextInTableCell { .. }
            | DocumentSubsystemsAction::FormCreateCheckbox { .. }
            | DocumentSubsystemsAction::FormCreateCheckboxInTableCell { .. }
            | DocumentSubsystemsAction::FormCreateChoiceInTableCell { .. }
            | DocumentSubsystemsAction::FormCreateChoice { .. }
            | DocumentSubsystemsAction::FormCreatePushButton { .. }
            | DocumentSubsystemsAction::FormCreateRadio { .. }
            | DocumentSubsystemsAction::FormCreateSignature { .. }
            | DocumentSubsystemsAction::FormMoveResizeWidget { .. }
            | DocumentSubsystemsAction::FormImportData { .. }
            | DocumentSubsystemsAction::FormSetCalculationOrder { .. }
            | DocumentSubsystemsAction::FormReset { .. }
            | DocumentSubsystemsAction::FormFlatten => {
                let (output, report) = supported_form_edit(input, action)?;
                (
                    output,
                    "acroform_source_edit",
                    report,
                    json!({"viewer_independent_appearance": true}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            DocumentSubsystemsAction::XfaInventory => {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems dynamic_xfa_unsupported: inventory is analyze-only; destructive conversion requires an explicit future conversion policy"
                        .to_string(),
                ))
            }
            DocumentSubsystemsAction::XfaImportDatasets { datasets_xml } => {
                let (output, report) = import_static_xfa_datasets_pdf(input, datasets_xml)?;
                (
                    output,
                    "static_xfa_datasets_source_import",
                    report,
                    json!({}),
                    json!({
                        "datasets_imported": true,
                        "template_and_non_datasets_packets_preserved": true,
                        "dynamic_xfa": "dynamic_xfa_unsupported",
                    }),
                    Vec::new(),
                )
            }
            DocumentSubsystemsAction::XfaFlattenStatic {
                remove_original_packets,
            } => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems replacement_not_approved: static XFA flattening requires explicit approval"
                            .to_string(),
                    ));
                }
                let mode = if *remove_original_packets {
                    XfaFlattenMode::FlattenAndRemoveXfa
                } else {
                    XfaFlattenMode::FlattenSupportedStatic
                };
                let (output, report) = xfa_flatten_pdf(
                    input,
                    &XfaFlattenOptions {
                        mode,
                        ..XfaFlattenOptions::default()
                    },
                )?;
                (
                    output,
                    "static_xfa_flatten_with_canonical_runtime",
                    value(&report)?,
                    json!({}),
                    json!({
                        "retained_original_packets": !remove_original_packets,
                        "dynamic_xfa": "dynamic_xfa_unsupported",
                        "conversion_report": "canonical_xfa_flatten_report"
                    }),
                    Vec::new(),
                )
            }
        };
    let reopened = ContentEngine::open_bytes(output.clone()).map_err(|error| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems output_reopen_failed: {error}"
        ))
    })?;
    Ok((
        output.clone(),
        DocumentSubsystemsOperationReport {
            schema_version: DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION.into(),
            subsystem: request.subsystem.clone(),
            action: Some(action.clone()),
            operation: operation.into(),
            source_sha256,
            output_sha256: digest(&output),
            changed_pages,
            source_links: json!({
                "provenance": "source_editing",
                "scene_transaction": "editing_transactions",
                "reflow": "text_reflow",
                "output_pages": reopened.page_count()?,
            }),
            transaction,
            appearance_effect,
            xfa_effect,
            undo_available: true,
            exact_limits: no_change_limit(&request.subsystem),
        },
    ))
}

pub fn analyze_document_subsystems(input: &[u8]) -> Result<DocumentSubsystemsAnalysisReport> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let interactive = interactive_report(&engine)?;
    let xfa = xfa_inventory(&engine, &XfaLimits::default())?;
    let xfa_extraction = extract_xfa(&engine, &XfaLimits::default())?;
    let xfa_runtime = xfa_runtime_report(&engine, &XfaRuntimeOptions::default())?;
    let semantic = analyze_semantic_layout(input, None)?;
    let tables = editable_tables(input)?;
    let mathematical_expressions = analyze_math_expressions(input)?;
    let ocr_layers = analyze_ocr_layers(input)?;
    let semantic_nodes_sample = semantic
        .nodes
        .iter()
        .take(MAX_DOCUMENT_SUBSYSTEM_SAMPLE_ITEMS)
        .cloned()
        .collect::<Vec<_>>();
    let semantic_edges_sample = semantic
        .edges
        .iter()
        .take(MAX_DOCUMENT_SUBSYSTEM_SAMPLE_ITEMS)
        .cloned()
        .collect::<Vec<_>>();
    let table_evidence = json!({
        "canonical_module": "analysis::tables + table_intelligence + text_reflow semantic regions",
        "semantic_regions": {
            "node_count": semantic.nodes.len(),
            "edge_count": semantic.edges.len(),
            "nodes_sample": semantic_nodes_sample,
            "edges_sample": semantic_edges_sample,
            "sample_limit": MAX_DOCUMENT_SUBSYSTEM_SAMPLE_ITEMS,
            "reading_order": semantic.reading_order,
            "flow_graph": semantic.flow_graph,
            "region_graph_invariants": semantic.region_graph_invariants,
            "review_required": semantic.review_required,
            "analysis_scope": {
                "max_public_report_pages": crate::text_reflow::MAX_PUBLIC_SEMANTIC_REPORT_PAGES,
                "document_wide_reports_use_word_line_paragraph_detail": true
            }
        },
        "tables": tables,
        "supported_detection": ["ruled", "borderless", "partially_ruled", "repeated_header_candidates"],
        "status": "source_linked_analysis"
    });
    Ok(DocumentSubsystemsAnalysisReport {
        schema_version: DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION.into(),
        source_sha256: digest(input),
        table_evidence,
        mathematical_content: json!({
            "canonical_primitives": "editing_transactions shaping, fonts, subsets, provenance",
            "expressions": mathematical_expressions,
            "review_required_for_unresolved_formula": true,
            "outlined_or_raster_formula": "formula_review_required"
        }),
        ocr_layers,
        annotations: value(&interactive.annotations)?,
        forms: value(&interactive.forms)?,
        xfa: json!({
            "inventory": xfa,
            "data_extraction": xfa_extraction,
            "static_conversion_plan": {
                "canonical_runtime": xfa_runtime,
                "apply_action": "xfa_flatten_static",
                "dynamic_xfa": "dynamic_xfa_unsupported",
                "original_packet_retention": "default_true",
            }
        }),
        exact_limits: vec![
            "all mutations require exact provenance and canonical transaction-compatible output".into(),
            "unsupported source geometry, providers, dynamic XFA, and unsafe appearances return typed no-change failures".into(),
        ],
    })
}

pub fn plan_document_subsystems(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
) -> Result<Value> {
    let analysis = analyze_document_subsystems(input)?;
    let reflow = request
        .reflow
        .as_ref()
        .map(|item| analyze_geometric_region(input, item))
        .transpose()?;
    Ok(json!({
        "schema_version": DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION,
        "kind": "document_subsystems_plan",
        "subsystem": request.subsystem,
        "action": request.action,
        "approved": request.approved,
        "analysis": analysis,
        "reflow_plan": reflow,
        "typed_limits": no_change_limit(&request.subsystem)
    }))
}

pub fn apply_document_subsystems(
    input: &[u8],
    request: &DocumentSubsystemsRequest,
) -> Result<(Vec<u8>, DocumentSubsystemsOperationReport)> {
    if let Some(action) = request.action.as_ref() {
        return apply_explicit_action(input, request, action);
    }
    let source_sha256 = digest(input);
    let (output, operation, transaction, appearance_effect, xfa_effect, changed_pages) =
        match request.subsystem {
            DocumentSubsystemsSubsystem::Table => {
                let reflow = reflow_required(request, "table_not_resolved")?;
                let (output, report) = if request.use_semantic_document_flow {
                    apply_reflow_document(input, reflow)?
                } else {
                    apply_reflow_region(input, reflow)?
                };
                (
                    output,
                    "table_cell_source_rewrite",
                    value(&report)?,
                    json!({}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsSubsystem::Math => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems formula_review_required: inferred or unresolved mathematical content requires explicit approval".into(),
                ));
                }
                let reflow = reflow_required(request, "math_structure_not_resolved")?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                (
                    output,
                    "math_source_rewrite_with_shaping",
                    value(&report)?,
                    json!({"math_layout": "editing_transactions_shaping_subset_path"}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsSubsystem::OcrSearchableLayer
            | DocumentSubsystemsSubsystem::OcrReconstruction => {
                if !request.approved {
                    return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems reconstruction_review_required: OCR correction or reconstruction needs explicit approval".into(),
                ));
                }
                let reflow = reflow_required(request, "scan_not_resolved")?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                (
                    output,
                    "ocr_approved_source_linked_reconstruction",
                    value(&report)?,
                    json!({"original_scan_preserved": true, "text_rendering": "canonical_invisible_or_visible_policy"}),
                    json!({"preserved": true}),
                    vec![reflow.page],
                )
            }
            DocumentSubsystemsSubsystem::AnnotationAppearance => {
                let (output, report) = generate_annotation_appearances_pdf(
                    input,
                    &AnnotationAppearanceOptions::default(),
                )?;
                (
                    output,
                    "annotation_appearance_regeneration",
                    value(&report)?,
                    json!({"normal_rollover_down": "canonical_supported_states"}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            DocumentSubsystemsSubsystem::FormData => {
                let data = request.form_data.as_deref().ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "document_subsystems field_not_found: canonical FDF or XFDF form data is required"
                            .into(),
                    )
                })?;
                let format = match request.form_data_format.as_deref().unwrap_or("fdf") {
                    "json" => FormDataFormat::Json,
                    "fdf" => FormDataFormat::Fdf,
                    "xfdf" => FormDataFormat::Xfdf,
                    other => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "document_subsystems unsupported_exact: form data format {other}"
                        )))
                    }
                };
                let (output, report) =
                    apply_form_data_pdf(input.to_vec(), data.as_bytes(), format)?;
                (
                    output,
                    "acroform_value_and_appearance_update",
                    value(&report)?,
                    json!({"viewer_independent_appearance": true}),
                    json!({"preserved": true}),
                    Vec::new(),
                )
            }
            DocumentSubsystemsSubsystem::XfaPreservation => {
                let reflow = reflow_required(request, "dynamic_xfa_unsupported")?;
                let before_packets = xfa_packet_fingerprint(input)?;
                let (output, report) = apply_reflow_region(input, reflow)?;
                let after_packets = xfa_packet_fingerprint(&output)?;
                if before_packets != after_packets {
                    return Err(WellfriendError::UnsupportedFeature(
                        "document_subsystems structure_update_failed: unrelated edit changed byte-preserved XFA packet content"
                            .to_string(),
                    ));
                }
                (
                    output,
                    "unrelated_edit_with_xfa_packet_preservation",
                    value(&report)?,
                    json!({}),
                    json!({
                        "packet_bytes_preserved_by_canonical_writer": true,
                        "packet_fingerprint": before_packets,
                        "dynamic_conversion": "unsupported_exact"
                    }),
                    vec![reflow.page],
                )
            }
        };
    let reopen = ContentEngine::open_bytes(output.clone()).map_err(|error| {
        WellfriendError::UnsupportedFeature(format!(
            "document_subsystems output_reopen_failed: {error}"
        ))
    })?;
    let output_sha256 = digest(&output);
    let reopened_page_count = reopen.page_count()?;
    Ok((
        output,
        DocumentSubsystemsOperationReport {
            schema_version: DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION.into(),
            subsystem: request.subsystem.clone(),
            action: None,
            operation: operation.into(),
            source_sha256,
            output_sha256,
            changed_pages,
            source_links: json!({"provenance": "source_editing", "scene_transaction": "editing_transactions", "reflow": "text_reflow", "output_pages": reopened_page_count}),
            transaction,
            appearance_effect,
            xfa_effect,
            undo_available: true,
            exact_limits: no_change_limit(&request.subsystem),
        },
    ))
}

pub fn undo_document_subsystems(
    original: &[u8],
    output: &[u8],
    request: &DocumentSubsystemsRequest,
) -> Result<(Vec<u8>, Value)> {
    if original == output {
        return Err(WellfriendError::UnsupportedFeature(
            "document_subsystems undo_failed: output must be a distinct committed transaction result".into(),
        ));
    }
    let proof = match request.action.as_ref() {
        Some(DocumentSubsystemsAction::TableEditMathCell {
            table_id,
            row,
            col,
            replacement_text,
            approved,
        }) => {
            if !approved {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems formula_review_required: table-cell mathematical replacement requires explicit approval".into(),
                ));
            }
            let (reflow, _) =
                table_cell_reflow(original, request, table_id, *row, *col, replacement_text)?;
            if !math_like(&reflow.source_text) {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems math_structure_not_resolved: resolved table cell is not born-digital mathematical source".into(),
                ));
            }
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::UnsupportedFeature(
                    "document_subsystems undo_failed: table-cell math replay did not restore original bytes"
                        .into(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::TableEditCell {
            table_id,
            row,
            col,
            replacement_text,
        }) => {
            let (reflow, _) =
                table_cell_reflow(original, request, table_id, *row, *col, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: table reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::TableSetCellAlignment {
            table_id,
            row,
            col,
            alignment,
        }) => {
            let (reflow, _) =
                table_cell_alignment_reflow(original, request, table_id, *row, *col, alignment)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: table alignment inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::TableSetCellPadding {
            table_id,
            row,
            col,
            padding,
        }) => {
            let (reflow, _) =
                table_cell_padding_reflow(original, request, table_id, *row, *col, *padding)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: table padding inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathReplace { replacement_text })
        | Some(DocumentSubsystemsAction::OcrCorrectText { replacement_text }) => {
            let typed = if matches!(
                request.action.as_ref(),
                Some(DocumentSubsystemsAction::MathReplace { .. })
            ) {
                "math_structure_not_resolved"
            } else {
                "scan_not_resolved"
            };
            let mut reflow = if typed == "math_structure_not_resolved" {
                resolved_math_reflow(original, request, replacement_text)?
            } else {
                reflow_required(request, typed)?.clone()
            };
            reflow.replacement_text = replacement_text.clone();
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::OcrCorrectGeometry { bounds }) => {
            let mut reflow = reflow_required(request, "scan_not_resolved")?.clone();
            rect_from_pdf_bounds(*bounds)?;
            reflow.region = Some(*bounds);
            reflow.replacement_text = reflow.source_text.clone();
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: OCR geometry source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditMatrixCell {
            row,
            col,
            replacement_text,
        }) => {
            let reflow = matrix_cell_reflow(original, request, *row, *col, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: matrix source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditMatrixStructure {
            operation,
            index,
            values,
        }) => {
            let reflow = matrix_structure_reflow(original, request, operation, *index, values)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: matrix structure reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditFractionPart {
            part,
            replacement_text,
        }) => {
            let reflow = fraction_part_reflow(original, request, part, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: fraction source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditScript {
            script_kind,
            replacement_text,
        }) => {
            let reflow = math_script_reflow(original, request, script_kind, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: script source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditFencedInner { replacement_text }) => {
            let reflow = math_fenced_inner_reflow(original, request, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: fenced-expression source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(DocumentSubsystemsAction::MathEditRadicand { replacement_text }) => {
            let reflow = math_radicand_reflow(original, request, replacement_text)?;
            let (restored, undo) = undo_reflow_from_replay(original, output, &reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: radical source reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
        Some(_) => {
            let (replayed, _) = apply_explicit_action(
                original,
                request,
                request.action.as_ref().expect("checked"),
            )?;
            if replayed != output {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems stale_snapshot_conflict: output does not match the deterministic action replay"
                        .to_string(),
                ));
            }
            json!({
                "inverse": "verified_canonical_action_preimage",
                "replay_matched_output": true,
                "atomic": true,
            })
        }
        None => {
            let reflow = reflow_required(request, "undo_failed")?;
            let (restored, undo) = undo_reflow_from_replay(original, output, reflow)?;
            if restored != original {
                return Err(WellfriendError::MalformedPdf(
                    "document_subsystems undo_failed: legacy reflow inverse did not restore the input snapshot"
                        .to_string(),
                ));
            }
            json!({"inverse": "text_reflow_reflow_replay", "undo": undo})
        }
    };
    ContentEngine::open_bytes(original.to_vec())?;
    Ok((
        original.to_vec(),
        json!({
            "schema_version": DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION,
            "kind": "document_subsystems_undo",
            "subsystem": request.subsystem,
            "byte_exact_restoration": true,
            "proof": proof,
        }),
    ))
}

pub fn document_subsystems_feature_matrix() -> Value {
    json!({
        "schema_version": DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION,
        "tables": "source-linked text, math-cell, alignment, padding, border, fill, linked-annotation, and in-cell-widget edits plus conservative ruled row/column append in verified empty space; ambiguous structural edits fail closed",
        "math": "approved born-digital source-linked shaping edits including move/resize, resolved bracket-matrix cells and dimensions, fractions, scripts, fenced-expression inners, and radicals; unresolved formulas require review",
        "ocr": "scan-preserving approved existing-searchable text and geometry correction plus atomic provider-recorded invisible word layers and source-linked URI annotations",
        "annotations": "PdfEditor source edits, AnnotationMediaRedaction XFDF/reply/move-resize import, appearance regeneration, and explicit flattening",
        "forms": "type-checked AcroForm text/choice/check/radio edits, unsigned signature-field creation, terminal rename, calculation-order mutation, reset, JSON/FDF/XFDF data application, and flattening",
        "xfa": "inventory/extraction/runtime conversion plan, static datasets-packet import with non-datasets preservation proof, unrelated-edit packet preservation, and approved static flattening",
        "undo": "atomic preimage restoration"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_page_pdf(content: &str) -> Vec<u8> {
        let stream = format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        );
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            stream,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut output = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(output.len());
            output
                .extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref = output.len();
        output.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in offsets {
            output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        output.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                objects.len() + 1
            )
            .as_bytes(),
        );
        output
    }

    fn static_xfa_pdf() -> Vec<u8> {
        let template = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form1" layout="position"><field name="name" x="20pt" y="20pt" w="120pt" h="20pt"><value><text>Initial</text></value><ui><textEdit/></ui></field></subform></template>"#;
        let datasets = r#"<datasets xmlns="http://www.xfa.org/schema/xfa-data/1.0/"><data><person><name>Initial</name></person></data></datasets>"#;
        let content = "q Q\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            "<< /Fields [] /XFA [(template) 7 0 R (datasets) 8 0 R] >>".to_string(),
            format!("<< /Length {} >>\nstream\n{template}\nendstream", template.len()),
            format!("<< /Length {} >>\nstream\n{datasets}\nendstream", datasets.len()),
        ];
        let mut output = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(output.len());
            output
                .extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref = output.len();
        output.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in offsets {
            output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        output.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                objects.len() + 1
            )
            .as_bytes(),
        );
        output
    }

    fn ruled_table_pdf() -> Vec<u8> {
        let mut content = String::from("1 w 0 0 0 RG\n");
        for y in [600.0, 640.0, 680.0] {
            content.push_str(&format!("50 {y} m 300 {y} l S\n"));
        }
        for x in [50.0, 175.0, 300.0] {
            content.push_str(&format!("{x} 600 m {x} 680 l S\n"));
        }
        for (x, y, text) in [
            (60.0, 650.0, "Alpha"),
            (190.0, 650.0, "Beta"),
            (60.0, 610.0, "Gamma"),
            (190.0, 610.0, "Delta"),
        ] {
            content.push_str(&format!("BT /F1 12 Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"));
        }
        one_page_pdf(&content)
    }

    fn reflow() -> GeometricReflowRequest {
        serde_json::from_value(json!({
            "requested_mode": "geometric_block",
            "page": 1,
            "source_text": "Hello",
            "replacement_text": "World",
            "region": [10.0, 10.0, 260.0, 90.0],
            "language": "en",
            "hyphenation": true
        }))
        .expect("DocumentSubsystems fixture reflow request")
    }

    fn formula_reflow() -> GeometricReflowRequest {
        serde_json::from_value(json!({
            "requested_mode": "geometric_block",
            "page": 1,
            "source_text": "x=1",
            "replacement_text": "x=2",
            "region": [50.0, 700.0, 300.0, 760.0],
            "language": "en"
        }))
        .expect("DocumentSubsystems formula reflow request")
    }

    fn matrix_reflow() -> GeometricReflowRequest {
        serde_json::from_value(json!({
            "requested_mode": "geometric_block",
            "page": 1,
            "source_text": "[[a,b];[c,d]]",
            "replacement_text": "[[a,b];[c,d]]",
            "region": [50.0, 700.0, 300.0, 760.0],
            "language": "en"
        }))
        .expect("DocumentSubsystems matrix reflow request")
    }

    #[test]
    fn source_linked_table_math_and_ocr_edits_reopen_and_undo() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        for (subsystem, approved) in [
            (DocumentSubsystemsSubsystem::Table, true),
            (DocumentSubsystemsSubsystem::Math, true),
            (DocumentSubsystemsSubsystem::OcrSearchableLayer, true),
        ] {
            let request = DocumentSubsystemsRequest {
                subsystem,
                action: None,
                reflow: Some(reflow()),
                approved,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            let (output, report) = apply_document_subsystems(&input, &request)
                .expect("DocumentSubsystems source edit");
            assert_ne!(input, output);
            assert!(report.undo_available);
            let (restored, undo) = undo_document_subsystems(&input, &output, &request)
                .expect("DocumentSubsystems undo");
            assert_eq!(input, restored);
            assert_eq!(undo["byte_exact_restoration"], Value::Bool(true));
        }
    }

    #[test]
    fn inferred_math_and_ocr_refuse_without_approval() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        for subsystem in [
            DocumentSubsystemsSubsystem::Math,
            DocumentSubsystemsSubsystem::OcrReconstruction,
        ] {
            let request = DocumentSubsystemsRequest {
                subsystem,
                action: None,
                reflow: Some(reflow()),
                approved: false,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            assert!(apply_document_subsystems(&input, &request).is_err());
        }
    }

    #[test]
    fn analysis_uses_canonical_interactive_and_xfa_models() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let report = analyze_document_subsystems(&input).expect("DocumentSubsystems analysis");
        assert_eq!(report.schema_version, DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION);
        assert!(report.table_evidence["semantic_regions"].is_object());
    }

    #[test]
    fn table_math_cell_uses_math_review_and_cell_reflow() {
        let mut input = ruled_table_pdf();
        let initial_table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let initial_text = initial_table
            .source
            .cells
            .first()
            .expect("first table cell")
            .text
            .as_bytes();
        let source = [b"(".as_slice(), initial_text, b")".as_slice()].concat();
        let offset = input
            .windows(source.len())
            .position(|window| window == source.as_slice())
            .expect("ruled fixture cell source");
        assert!(
            source.len() >= 4,
            "fixture cell supports a two-byte math source"
        );
        input[offset + 1] = b'1';
        input[offset + 2] = b'/';
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableEditMathCell {
                table_id: table.table_id,
                row: 0,
                col: 0,
                replacement_text: "x/2".to_string(),
                approved: true,
            }),
            reflow: Some(reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("table math rewrite");
        assert_eq!(report.operation, "table_math_cell_source_rewrite");
        assert_ne!(output, input, "table math must rewrite source output");
        ContentEngine::open_bytes(output.clone()).expect("reopen table math output");
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("undo table math");
        assert_eq!(restored, input);
    }

    #[test]
    fn source_linked_table_cell_action_rewrites_real_cell_source_and_undoes() {
        let input = ruled_table_pdf();
        let tables = editable_tables(&input).expect("source-linked table analysis");
        assert_eq!(tables.len(), 1);
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableEditCell {
                table_id: tables[0].table_id.clone(),
                row: 0,
                col: 0,
                replacement_text: "Renamed".to_string(),
            }),
            reflow: Some(reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("cell source rewrite");
        assert_eq!(report.operation, "table_cell_source_rewrite");
        assert!(ContentEngine::open_bytes(output.clone())
            .expect("reopen output")
            .get_page_text(1)
            .expect("extract output")
            .contains("Renamed"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("table undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn source_linked_table_cell_alignment_rewrites_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("source-linked table analysis")
            .into_iter()
            .next()
            .expect("table graph");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableSetCellAlignment {
                table_id: table.table_id,
                row: 0,
                col: 0,
                alignment: "right".to_string(),
            }),
            reflow: Some(reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("cell alignment rewrite");
        assert_eq!(report.operation, "table_cell_alignment_source_rewrite");
        assert!(ContentEngine::open_bytes(output.clone()).is_ok());
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("alignment undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn source_linked_table_cell_padding_rewrites_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("source-linked table analysis")
            .into_iter()
            .next()
            .expect("table graph");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableSetCellPadding {
                table_id: table.table_id,
                row: 0,
                col: 0,
                padding: [8.0, 4.0, 8.0, 4.0],
            }),
            reflow: Some(reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("cell padding rewrite");
        assert_eq!(report.operation, "table_cell_padding_source_rewrite");
        assert!(ContentEngine::open_bytes(output.clone()).is_ok());
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("padding undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn source_linked_table_cell_fill_writes_real_underlay_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableSetCellFill {
                table_id: table.table_id,
                row: 0,
                col: 0,
                color_rgb: [0.9, 0.9, 0.4],
                opacity: 0.8,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("append table fill");
        assert_eq!(
            report.operation,
            "table_cell_fill_source_instruction_append"
        );
        assert_ne!(output, input, "table fill must append real source content");
        ContentEngine::open_bytes(output.clone()).expect("reopen filled table");
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("table fill undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn source_linked_table_cell_border_writes_real_path_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableAddCellBorder {
                table_id: table.table_id,
                row: 0,
                col: 0,
                line_width: 1.5,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("append table border");
        assert_eq!(
            report.operation,
            "table_cell_border_source_instruction_append"
        );
        assert_ne!(
            output, input,
            "table border must append real source content"
        );
        ContentEngine::open_bytes(output.clone()).expect("reopen bordered table");
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("table border undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn simple_ruled_table_append_row_writes_borders_text_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("detect ruled table")
            .into_iter()
            .next()
            .expect("table graph");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableAppendRow {
                table_id: table.table_id,
                values: vec!["Epsilon".to_string(), "Zeta".to_string()],
                row_height: Some(40.0),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("append table row");
        assert_eq!(report.operation, "table_append_row_source_instructions");
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen appended table")
            .get_page_text(1)
            .expect("extract appended table");
        assert!(extracted.contains("Epsilon"));
        assert!(extracted.contains("Zeta"));
        assert!(editable_tables(&output)
            .expect("reanalyze appended table")
            .iter()
            .any(|graph| graph.source.num_rows() >= 3));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("append row undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn simple_ruled_table_append_column_writes_borders_text_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("detect ruled table")
            .into_iter()
            .next()
            .expect("table graph");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableAppendColumn {
                table_id: table.table_id,
                values: vec!["North".to_string(), "South".to_string()],
                column_width: Some(90.0),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("append table column");
        assert_eq!(report.operation, "table_append_column_source_instructions");
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen appended table")
            .get_page_text(1)
            .expect("extract appended table");
        assert!(extracted.contains("North"));
        assert!(extracted.contains("South"));
        assert!(editable_tables(&output)
            .expect("reanalyze appended table")
            .iter()
            .any(|graph| graph.source.num_cols() >= 3));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("append column undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn math_move_resize_rewrites_source_geometry_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm (x=1) Tj ET");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathMoveResize {
                bounds: [180.0, 540.0, 300.0, 580.0],
            }),
            reflow: Some(formula_reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("move mathematical source");
        assert_eq!(
            report.operation,
            "math_source_move_resize_with_editing_transactions_shaping"
        );
        assert_ne!(output, input, "math movement must rewrite source output");
        ContentEngine::open_bytes(output.clone()).expect("reopen moved math");
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("math movement undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn math_fenced_inner_edit_preserves_delimiters_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm ((x=1)) Tj ET");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditFencedInner {
                replacement_text: "y=2".to_string(),
            }),
            reflow: Some(
                serde_json::from_value(json!({
                    "requested_mode": "geometric_block",
                    "page": 1,
                    "source_text": "(x=1)",
                    "replacement_text": "(x=1)",
                    "region": [50.0, 700.0, 300.0, 760.0],
                    "language": "en"
                }))
                .expect("fenced math reflow"),
            ),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("edit fenced math");
        assert_eq!(
            report.operation,
            "math_fenced_inner_source_rewrite_with_editing_transactions_shaping"
        );
        assert!(ContentEngine::open_bytes(output.clone())
            .expect("reopen fenced math")
            .get_page_text(1)
            .expect("extract fenced math")
            .contains("(y=2)"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("fenced math undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn math_radicand_edit_uses_resolved_source_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm (sqrt(x)) Tj ET");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditRadicand {
                replacement_text: "y+1".to_string(),
            }),
            reflow: Some(
                serde_json::from_value(json!({
                    "requested_mode": "geometric_block",
                    "page": 1,
                    "source_text": "sqrt(x)",
                    "replacement_text": "sqrt(x)",
                    "region": [50.0, 700.0, 300.0, 760.0],
                    "language": "en"
                }))
                .expect("radical math reflow"),
            ),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) = apply_document_subsystems(&input, &request).expect("edit radical");
        assert_eq!(
            report.operation,
            "math_radicand_source_rewrite_with_editing_transactions_shaping"
        );
        assert!(ContentEngine::open_bytes(output.clone())
            .expect("reopen radical math")
            .get_page_text(1)
            .expect("extract radical math")
            .contains("sqrt(y+1)"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("radical undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn explicit_math_ocr_and_annotation_actions_use_canonical_editors() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let formula = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm (x=1) Tj ET");
        for (source, action, reflow_request) in [
            (
                formula.as_slice(),
                DocumentSubsystemsAction::MathReplace {
                    replacement_text: "x=2".to_string(),
                },
                formula_reflow(),
            ),
            (
                input.as_slice(),
                DocumentSubsystemsAction::OcrCorrectText {
                    replacement_text: "World".to_string(),
                },
                reflow(),
            ),
        ] {
            let subsystem = action_subsystem(&action);
            let request = DocumentSubsystemsRequest {
                subsystem,
                action: Some(action),
                reflow: Some(reflow_request),
                approved: true,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            let (output, _) =
                apply_document_subsystems(source, &request).expect("source-linked action");
            assert!(ContentEngine::open_bytes(output).is_ok());
        }
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreate {
                page: 1,
                subtype: "text".to_string(),
                rect: [72.0, 72.0, 120.0, 112.0],
                contents: "review".to_string(),
                uri: None,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("annotation action");
        assert_eq!(
            report.operation,
            "annotation_create_and_appearance_regeneration"
        );
        assert!(ContentEngine::open_bytes(output).is_ok());

        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreate {
                page: 1,
                subtype: "free_text".to_string(),
                rect: [72.0, 144.0, 216.0, 184.0],
                contents: "review & approve".to_string(),
                uri: None,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("free text action");
        assert_eq!(
            report.operation,
            "annotation_create_and_appearance_regeneration"
        );
        let interactive = interactive_report(
            &ContentEngine::open_bytes(output).expect("reopen free text output"),
        )
        .expect("inspect free text annotation");
        assert!(interactive
            .annotations
            .annotations
            .iter()
            .any(|annotation| annotation.subtype == "FreeText"));
    }

    #[test]
    fn matrix_cell_edit_rewrites_source_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm ([[a,b];[c,d]]) Tj ET");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditMatrixCell {
                row: 1,
                col: 0,
                replacement_text: "z".to_string(),
            }),
            reflow: Some(matrix_reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("matrix cell source edit");
        assert_eq!(
            report.operation,
            "math_matrix_cell_source_rewrite_with_editing_transactions_shaping"
        );
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen matrix output")
            .get_page_text(1)
            .expect("extract matrix output");
        // The canonical extractor normalizes punctuation spacing, so assert
        // the replaced source atom rather than a viewer-dependent bracket
        // serialization.
        assert!(extracted.contains('z'));
        assert!(!extracted.contains('c'));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("matrix undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn matrix_structure_edit_rewrites_source_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 72 720 Tm ([[a,b];[c,d]]) Tj ET");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditMatrixStructure {
                operation: "insert_column".to_string(),
                index: 1,
                values: vec!["x".to_string(), "y".to_string()],
            }),
            reflow: Some(matrix_reflow()),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("matrix structure edit");
        assert_eq!(
            report.operation,
            "math_matrix_structure_source_rewrite_with_editing_transactions_shaping"
        );
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen matrix structure output")
            .get_page_text(1)
            .expect("extract matrix structure output");
        assert!(extracted.contains('x'));
        assert!(extracted.contains('y'));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("matrix structure undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn fraction_part_edit_rewrites_resolved_math_source_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 50 720 Tm (1/2) Tj ET\n");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditFractionPart {
                part: "denominator".to_string(),
                replacement_text: "3".to_string(),
            }),
            reflow: Some(
                serde_json::from_value(json!({
                    "requested_mode": "geometric_block",
                    "page": 1,
                    "source_text": "1/2",
                    "replacement_text": "1/2",
                    "region": [50.0, 700.0, 300.0, 760.0],
                    "language": "en"
                }))
                .expect("fraction reflow"),
            ),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("fraction part source edit");
        assert_eq!(
            report.operation,
            "math_fraction_part_source_rewrite_with_editing_transactions_shaping"
        );
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen fraction output")
            .get_page_text(1)
            .expect("extract fraction output");
        assert!(extracted.contains("1/3"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("fraction undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn script_edit_rewrites_resolved_math_source_and_undoes() {
        let input = one_page_pdf("BT /F1 12 Tf 1 0 0 1 50 720 Tm (x^2) Tj ET\n");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Math,
            action: Some(DocumentSubsystemsAction::MathEditScript {
                script_kind: "superscript".to_string(),
                replacement_text: "3".to_string(),
            }),
            reflow: Some(
                serde_json::from_value(json!({
                    "requested_mode": "geometric_block",
                    "page": 1,
                    "source_text": "x^2",
                    "replacement_text": "x^2",
                    "region": [50.0, 700.0, 300.0, 760.0],
                    "language": "en"
                }))
                .expect("script reflow"),
            ),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("script source edit");
        assert_eq!(
            report.operation,
            "math_script_source_rewrite_with_editing_transactions_shaping"
        );
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen script output")
            .get_page_text(1)
            .expect("extract script output");
        assert!(extracted.contains("x^3"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("script undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn table_linked_annotation_moves_only_to_resolved_cell_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let cell_bounds = table.source.cells[0].bbox;
        let create = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreate {
                page: table.page,
                subtype: "square".to_string(),
                rect: [8.0, 8.0, 20.0, 20.0],
                contents: "table linked".to_string(),
                uri: None,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (created, _) =
            apply_document_subsystems(&input, &create).expect("create source annotation");
        let (xfdf, _) = crate::annotation_media_redaction::export_annotation_xfdf(
            &ContentEngine::open_bytes(created.clone()).expect("open created annotation"),
        )
        .expect("export source annotation");
        let annotation_id = crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf)
            .expect("parse source annotation")
            .annotations
            .into_iter()
            .find(|annotation| annotation.subtype == "Square")
            .expect("square identity")
            .id;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableMoveLinkedAnnotation {
                table_id: table.table_id,
                row: 0,
                col: 0,
                annotation_id: annotation_id.clone(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&created, &request).expect("move linked table annotation");
        assert_eq!(
            report.operation,
            "table_linked_annotation_source_move_and_appearance_regeneration"
        );
        let (xfdf, _) = crate::annotation_media_redaction::export_annotation_xfdf(
            &ContentEngine::open_bytes(output.clone()).expect("reopen moved annotation"),
        )
        .expect("export moved annotation");
        let moved = crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf)
            .expect("parse moved annotation")
            .annotations
            .into_iter()
            .find(|annotation| annotation.id == annotation_id)
            .expect("moved identity");
        assert_eq!(moved.rect, Some(cell_bounds));
        let (restored, _) =
            undo_document_subsystems(&created, &output, &request).expect("table annotation undo");
        assert_eq!(restored, created);
    }

    #[test]
    fn annotation_move_resize_preserves_source_identity_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let create = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreate {
                page: 1,
                subtype: "square".to_string(),
                rect: [72.0, 72.0, 120.0, 108.0],
                contents: "move me".to_string(),
                uri: None,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (created, _) =
            apply_document_subsystems(&input, &create).expect("create source annotation");
        let (xfdf, _) = crate::annotation_media_redaction::export_annotation_xfdf(
            &ContentEngine::open_bytes(created.clone()).expect("open created annotation"),
        )
        .expect("export source annotation");
        let annotation_id = crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf)
            .expect("parse source annotation")
            .annotations
            .into_iter()
            .find(|annotation| annotation.subtype == "Square")
            .expect("square identity")
            .id;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationMoveResize {
                annotation_id: annotation_id.clone(),
                page: 1,
                rect: [180.0, 120.0, 260.0, 180.0],
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&created, &request).expect("move source annotation");
        assert_eq!(
            report.operation,
            "annotation_move_resize_source_update_and_appearance_regeneration"
        );
        let (xfdf, _) = crate::annotation_media_redaction::export_annotation_xfdf(
            &ContentEngine::open_bytes(output.clone()).expect("reopen moved annotation"),
        )
        .expect("export moved annotation");
        let moved = crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf)
            .expect("parse moved annotation")
            .annotations
            .into_iter()
            .find(|annotation| annotation.id == annotation_id)
            .expect("moved identity");
        assert_eq!(moved.rect, Some([180.0, 120.0, 260.0, 180.0]));
        let (restored, _) =
            undo_document_subsystems(&created, &output, &request).expect("annotation undo");
        assert_eq!(restored, created);
    }

    #[test]
    fn annotation_reply_uses_canonical_parent_relationship_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let create = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreate {
                page: 1,
                subtype: "text".to_string(),
                rect: [72.0, 72.0, 108.0, 108.0],
                contents: "parent".to_string(),
                uri: None,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (created, _) =
            apply_document_subsystems(&input, &create).expect("create parent annotation");
        let (xfdf, _) = export_annotation_xfdf(
            &ContentEngine::open_bytes(created.clone()).expect("open parent annotation"),
        )
        .expect("export parent annotation");
        let parent_annotation_id = parse_annotation_xfdf(&xfdf)
            .expect("parse parent annotation")
            .annotations
            .into_iter()
            .find(|annotation| annotation.subtype == "Text")
            .expect("parent identity")
            .id;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
            action: Some(DocumentSubsystemsAction::AnnotationCreateReply {
                parent_annotation_id: parent_annotation_id.clone(),
                page: 1,
                rect: [120.0, 72.0, 180.0, 108.0],
                contents: "reply".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&created, &request).expect("create reply annotation");
        assert_eq!(
            report.operation,
            "annotation_reply_source_update_and_appearance_regeneration"
        );
        let (xfdf, _) = export_annotation_xfdf(
            &ContentEngine::open_bytes(output.clone()).expect("open reply annotation"),
        )
        .expect("export reply annotation");
        assert!(parse_annotation_xfdf(&xfdf)
            .expect("parse reply annotation")
            .annotations
            .iter()
            .any(|annotation| annotation.reply_to.as_deref() == Some(&parent_annotation_id)));
        let (restored, _) =
            undo_document_subsystems(&created, &output, &request).expect("reply undo");
        assert_eq!(restored, created);
    }

    #[test]
    fn provider_recorded_ocr_searchable_layer_preserves_scan_and_undoes() {
        let input = include_bytes!("../tests/fixtures/image_only.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrAddSearchableText {
                page: 1,
                text: "Scanned".to_string(),
                rect: [72.0, 72.0, 144.0, 96.0],
                font_size: 12.0,
                provider_id: "fixture_provider".to_string(),
                provider_version: Some("1".to_string()),
                confidence: 0.95,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("searchable layer");
        assert_eq!(report.operation, "ocr_searchable_layer_creation");
        assert!(ContentEngine::open_bytes(output.clone())
            .expect("reopen searchable output")
            .get_page_text(1)
            .expect("extract searchable layer")
            .contains("Scanned"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("searchable undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn searchable_ocr_geometry_correction_rewrites_source_and_undoes() {
        let input = include_bytes!("../tests/fixtures/image_only.pdf").to_vec();
        let create = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrAddSearchableText {
                page: 1,
                text: "Scanned".to_string(),
                rect: [72.0, 72.0, 144.0, 96.0],
                font_size: 12.0,
                provider_id: "fixture_provider".to_string(),
                provider_version: Some("1".to_string()),
                confidence: 0.95,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (searchable, _) =
            apply_document_subsystems(&input, &create).expect("create searchable layer");
        let geometry = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrCorrectGeometry {
                bounds: [180.0, 120.0, 252.0, 144.0],
            }),
            reflow: Some(
                serde_json::from_value(json!({
                    "requested_mode": "geometric_block",
                    "page": 1,
                    "source_text": "Scanned",
                    "replacement_text": "Scanned",
                    "region": [72.0, 72.0, 144.0, 96.0],
                    "language": "en"
                }))
                .expect("OCR geometry reflow"),
            ),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&searchable, &geometry).expect("correct searchable geometry");
        assert_eq!(
            report.operation,
            "ocr_searchable_layer_source_geometry_correction"
        );
        assert!(ContentEngine::open_bytes(output.clone())
            .expect("reopen OCR geometry output")
            .get_page_text(1)
            .expect("extract OCR geometry output")
            .contains("Scanned"));
        let (restored, _) = undo_document_subsystems(&searchable, &output, &geometry)
            .expect("geometry correction undo");
        assert_eq!(restored, searchable);
    }

    #[test]
    fn provider_recorded_ocr_text_with_link_preserves_scan_and_undoes() {
        let input = include_bytes!("../tests/fixtures/image_only.pdf").to_vec();
        let rect = [72.0, 72.0, 132.0, 96.0];
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrAddSearchableTextWithLink {
                page: 1,
                text: "Scanned".to_string(),
                rect,
                font_size: 12.0,
                provider_id: "fixture_provider".to_string(),
                provider_version: Some("1".to_string()),
                confidence: 0.96,
                uri: "https://example.invalid/ocr".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("OCR text with link");
        assert_eq!(
            report.operation,
            "ocr_searchable_layer_with_source_link_annotation"
        );
        let reopened = ContentEngine::open_bytes(output.clone()).expect("reopen OCR link output");
        assert!(reopened
            .get_page_text(1)
            .expect("extract OCR layer")
            .contains("Scanned"));
        let (xfdf, _) = crate::annotation_media_redaction::export_annotation_xfdf(&reopened)
            .expect("export OCR link annotation");
        assert!(
            crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf)
                .expect("parse OCR link annotation")
                .annotations
                .iter()
                .any(|annotation| annotation.subtype == "Link" && annotation.rect == Some(rect))
        );
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("OCR link undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn provider_recorded_ocr_word_layer_is_atomic_and_searchable() {
        let input = include_bytes!("../tests/fixtures/image_only.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrAddSearchableWords {
                page: 1,
                words: vec![
                    OcrSearchableWord {
                        text: "Scanned".to_string(),
                        rect: [72.0, 72.0, 132.0, 96.0],
                        font_size: 12.0,
                        confidence: 0.96,
                        line_id: Some(1),
                    },
                    OcrSearchableWord {
                        text: "Page".to_string(),
                        rect: [138.0, 72.0, 180.0, 96.0],
                        font_size: 12.0,
                        confidence: 0.94,
                        line_id: Some(1),
                    },
                ],
                provider_id: "fixture_provider".to_string(),
                provider_version: Some("1".to_string()),
                language: Some("en".to_string()),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) = apply_document_subsystems(&input, &request).expect("word layer");
        assert_eq!(report.operation, "ocr_searchable_layer_creation");
        let extracted = ContentEngine::open_bytes(output.clone())
            .expect("reopen searchable output")
            .get_page_text(1)
            .expect("extract searchable layer");
        assert!(extracted.contains("Scanned"));
        assert!(extracted.contains("Page"));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("word layer undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn canonical_form_action_reopens_and_exposes_inverse_proof() {
        let input = include_bytes!("../tests/fixtures/form_160f.pdf").to_vec();
        let engine = ContentEngine::open_bytes(input.clone()).expect("open form fixture");
        let field = crate::forms_report(&engine)
            .expect("form report")
            .fields
            .into_iter()
            .find(|field| field.field_type == "text")
            .expect("text field fixture");
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetText {
                field_name: field.full_name.clone(),
                value: "DocumentSubsystems".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("form source edit");
        assert_eq!(report.operation, "acroform_source_edit");
        assert!(ContentEngine::open_bytes(output.clone()).is_ok());
        let (restored, inverse) =
            undo_document_subsystems(&input, &output, &request).expect("form inverse");
        assert_eq!(restored, input);
        assert_eq!(inverse["byte_exact_restoration"], Value::Bool(true));

        let default_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetDefault {
                field_name: field.full_name.clone(),
                value: "DocumentSubsystemsDefault".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (defaulted, default_report) =
            apply_document_subsystems(&input, &default_request).expect("form default value");
        assert_eq!(default_report.operation, "acroform_source_edit");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(defaulted.clone()).expect("open defaulted form"),
        )
        .expect("inspect defaulted form")
        .fields
        .iter()
        .any(|candidate| {
            candidate.full_name == field.full_name
                && candidate.default_value.as_deref() == Some("DocumentSubsystemsDefault")
        }));
        let (restored, _) = undo_document_subsystems(&input, &defaulted, &default_request)
            .expect("form default undo");
        assert_eq!(restored, input);

        let import_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormImportData {
                data: serde_json::to_string(&json!({
                    "fields": [{"name": field.full_name.clone(), "value": "Imported"}]
                }))
                .expect("serialize form import"),
                format: "json".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (imported, import_report) =
            apply_document_subsystems(&input, &import_request).expect("form data import");
        assert_eq!(import_report.operation, "acroform_source_edit");
        assert!(ContentEngine::open_bytes(imported.clone()).is_ok());
        let (restored, _) =
            undo_document_subsystems(&input, &imported, &import_request).expect("form import undo");
        assert_eq!(restored, input);

        let rename_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormRename {
                field_name: field.full_name.clone(),
                new_name: "DocumentSubsystemsRenamed".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (renamed, rename_report) =
            apply_document_subsystems(&input, &rename_request).expect("form rename");
        assert_eq!(rename_report.operation, "acroform_source_edit");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(renamed.clone()).expect("open renamed form"),
        )
        .expect("inspect renamed form")
        .fields
        .iter()
        .any(|candidate| candidate.full_name.ends_with("DocumentSubsystemsRenamed")));
        let (restored, _) =
            undo_document_subsystems(&input, &renamed, &rename_request).expect("form rename undo");
        assert_eq!(restored, input);

        let delete_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormDelete {
                field_name: field.full_name.clone(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (deleted, delete_report) =
            apply_document_subsystems(&input, &delete_request).expect("form delete");
        assert_eq!(delete_report.operation, "acroform_source_edit");
        assert!(!crate::forms_report(
            &ContentEngine::open_bytes(deleted.clone()).expect("open deleted form"),
        )
        .expect("inspect deleted form")
        .fields
        .iter()
        .any(|candidate| candidate.full_name == field.full_name));
        let (restored, _) =
            undo_document_subsystems(&input, &deleted, &delete_request).expect("form delete undo");
        assert_eq!(restored, input);

        let reset_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormReset {
                field_name: Some(field.full_name),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (reset, reset_report) =
            apply_document_subsystems(&input, &reset_request).expect("form reset");
        assert_eq!(reset_report.operation, "acroform_source_edit");
        assert!(ContentEngine::open_bytes(reset.clone()).is_ok());
        let (restored, _) =
            undo_document_subsystems(&input, &reset, &reset_request).expect("form reset undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_calculation_order_rewrites_co_and_undoes() {
        let input = include_bytes!("../tests/fixtures/form_160f.pdf").to_vec();
        let field_name = crate::forms_report(
            &ContentEngine::open_bytes(input.clone()).expect("open calculation-order fixture"),
        )
        .expect("inspect calculation-order fixture")
        .fields
        .into_iter()
        .find(|field| !field.is_signature)
        .expect("non-signature field")
        .full_name;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetCalculationOrder {
                field_names: vec![field_name],
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("set calculation order");
        assert_eq!(report.operation, "acroform_source_edit");
        assert_eq!(
            crate::forms_report(
                &ContentEngine::open_bytes(output.clone()).expect("open calculation-order output"),
            )
            .expect("inspect calculation-order output")
            .calculation_order_len,
            1
        );
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("calculation-order undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_choice_widget_in_table_cell_uses_resolved_cell_bounds_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let cell_bounds = table.source.cells[0].bbox;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateChoiceInTableCell {
                table_id: table.table_id,
                row: 0,
                col: 0,
                field_name: "DocumentSubsystemsTableChoice".to_string(),
                options: vec!["One".to_string(), "Two".to_string()],
                selected: Some("Two".to_string()),
                editable_combo: false,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create table cell choice");
        assert_eq!(report.operation, "acroform_source_edit");
        let interactive = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created table choice"),
        )
        .expect("inspect created table choice");
        let created = interactive
            .fields
            .iter()
            .find(|field| field.full_name == "DocumentSubsystemsTableChoice")
            .expect("created table choice");
        assert_eq!(created.field_type, "choice");
        assert!(created.widgets.iter().any(|widget| {
            widget.has_appearance && widget.rect.is_some_and(|rect| rect == cell_bounds)
        }));
        let (restored, _) = undo_document_subsystems(&input, &output, &request)
            .expect("table choice creation undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_checkbox_widget_in_table_cell_uses_resolved_cell_bounds_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let cell_bounds = table.source.cells[0].bbox;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateCheckboxInTableCell {
                table_id: table.table_id,
                row: 0,
                col: 0,
                field_name: "DocumentSubsystemsTableCheck".to_string(),
                checked: true,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create table cell checkbox");
        assert_eq!(report.operation, "acroform_source_edit");
        let interactive = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created table checkbox"),
        )
        .expect("inspect created table checkbox");
        let created = interactive
            .fields
            .iter()
            .find(|field| field.full_name == "DocumentSubsystemsTableCheck")
            .expect("created table checkbox");
        assert_eq!(created.field_type, "checkbox");
        assert!(created.widgets.iter().any(|widget| {
            widget.has_appearance && widget.rect.is_some_and(|rect| rect == cell_bounds)
        }));
        let (restored, _) = undo_document_subsystems(&input, &output, &request)
            .expect("table checkbox creation undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_text_widget_in_table_cell_uses_resolved_cell_bounds_and_undoes() {
        let input = ruled_table_pdf();
        let table = editable_tables(&input)
            .expect("table analysis")
            .into_iter()
            .next()
            .expect("table");
        let cell_bounds = table.source.cells[0].bbox;
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateTextInTableCell {
                table_id: table.table_id,
                row: 0,
                col: 0,
                field_name: "DocumentSubsystemsTableCell".to_string(),
                value: "Cell field".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create table cell field");
        assert_eq!(report.operation, "acroform_source_edit");
        let interactive = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created table field"),
        )
        .expect("inspect created table field");
        let created = interactive
            .fields
            .iter()
            .find(|field| field.full_name == "DocumentSubsystemsTableCell")
            .expect("created table field");
        assert!(created.widgets.iter().any(|widget| {
            widget.has_appearance && widget.rect.is_some_and(|rect| rect == cell_bounds)
        }));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("table field creation undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_text_creates_widget_appearance_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateText {
                field_name: "DocumentSubsystemsText".to_string(),
                page: 1,
                rect: [72.0, 72.0, 216.0, 104.0],
                value: "Initial".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create text field");
        assert_eq!(report.operation, "acroform_source_edit");
        let interactive = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created form"),
        )
        .expect("inspect created form");
        let created = interactive
            .fields
            .iter()
            .find(|field| field.full_name == "DocumentSubsystemsText")
            .expect("created field");
        assert_eq!(created.field_type, "text");
        assert!(created.widgets.iter().any(|widget| widget.has_appearance));
        let move_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormMoveResizeWidget {
                field_name: "DocumentSubsystemsText".to_string(),
                page: 1,
                rect: [240.0, 72.0, 360.0, 104.0],
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (moved, _) =
            apply_document_subsystems(&output, &move_request).expect("move created widget");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(moved.clone()).expect("open moved widget"),
        )
        .expect("inspect moved widget")
        .fields
        .iter()
        .find(|field| field.full_name == "DocumentSubsystemsText")
        .and_then(|field| field.widgets.first())
        .and_then(|widget| widget.rect)
        .is_some_and(|rect| rect == [240.0, 72.0, 360.0, 104.0]));
        let (restored_created, _) = undo_document_subsystems(&output, &moved, &move_request)
            .expect("created widget move undo");
        assert_eq!(restored_created, output);
        let update_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetText {
                field_name: "DocumentSubsystemsText".to_string(),
                value: "Updated".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (updated, _) =
            apply_document_subsystems(&output, &update_request).expect("update created field");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(updated.clone()).expect("open updated field"),
        )
        .expect("inspect updated field")
        .fields
        .iter()
        .any(|field| field.full_name == "DocumentSubsystemsText"
            && field.value.as_deref() == Some("Updated")));
        let (restored_created, _) = undo_document_subsystems(&output, &updated, &update_request)
            .expect("created field update undo");
        assert_eq!(restored_created, output);
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("created field undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_checkbox_writes_named_appearances_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateCheckbox {
                field_name: "DocumentSubsystemsCheck".to_string(),
                page: 1,
                rect: [72.0, 120.0, 90.0, 138.0],
                checked: true,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create checkbox");
        assert_eq!(report.operation, "acroform_source_edit");
        let created = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created checkbox"),
        )
        .expect("inspect created checkbox")
        .fields
        .into_iter()
        .find(|field| field.full_name == "DocumentSubsystemsCheck")
        .expect("created checkbox field");
        assert_eq!(created.field_type, "checkbox");
        assert_eq!(created.value.as_deref(), Some("Yes"));
        assert!(created.widgets.iter().any(|widget| widget.has_appearance));
        let default_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetButtonDefault {
                field_name: "DocumentSubsystemsCheck".to_string(),
                checked: false,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (defaulted, _) =
            apply_document_subsystems(&output, &default_request).expect("set checkbox default");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(defaulted.clone()).expect("open defaulted checkbox"),
        )
        .expect("inspect defaulted checkbox")
        .fields
        .iter()
        .any(|field| {
            field.full_name == "DocumentSubsystemsCheck"
                && field.value.as_deref() == Some("Yes")
                && field.default_value.as_deref() == Some("Off")
        }));
        let (restored_default, _) = undo_document_subsystems(&output, &defaulted, &default_request)
            .expect("checkbox default undo");
        assert_eq!(restored_default, output);
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("checkbox undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_choice_writes_options_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateChoice {
                field_name: "DocumentSubsystemsChoice".to_string(),
                page: 1,
                rect: [108.0, 120.0, 216.0, 144.0],
                options: vec!["One".to_string(), "Two".to_string()],
                selected: Some("Two".to_string()),
                editable_combo: true,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) = apply_document_subsystems(&input, &request).expect("create choice");
        assert_eq!(report.operation, "acroform_source_edit");
        let created = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created choice"),
        )
        .expect("inspect created choice")
        .fields
        .into_iter()
        .find(|field| field.full_name == "DocumentSubsystemsChoice")
        .expect("created choice field");
        assert_eq!(created.field_type, "choice");
        assert_eq!(created.value.as_deref(), Some("Two"));
        assert!(created.widgets.iter().any(|widget| widget.has_appearance));
        let option_request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormSetChoiceOptions {
                field_name: "DocumentSubsystemsChoice".to_string(),
                options: vec!["Three".to_string(), "Four".to_string()],
                selected: Some("Four".to_string()),
                editable_combo: false,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (updated, _) =
            apply_document_subsystems(&output, &option_request).expect("replace choice options");
        assert!(crate::forms_report(
            &ContentEngine::open_bytes(updated.clone()).expect("open updated choice"),
        )
        .expect("inspect updated choice")
        .fields
        .iter()
        .any(|field| field.full_name == "DocumentSubsystemsChoice"
            && field.value.as_deref() == Some("Four")));
        let (restored_choice, _) = undo_document_subsystems(&output, &updated, &option_request)
            .expect("choice options undo");
        assert_eq!(restored_choice, output);
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("choice undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_push_button_writes_all_appearance_states_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreatePushButton {
                field_name: "DocumentSubsystemsButton".to_string(),
                page: 1,
                rect: [240.0, 120.0, 336.0, 144.0],
                caption: "Apply".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create push button");
        assert_eq!(report.operation, "acroform_source_edit");
        let created = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created button"),
        )
        .expect("inspect created button")
        .fields
        .into_iter()
        .find(|field| field.full_name == "DocumentSubsystemsButton")
        .expect("created push button field");
        assert_eq!(created.field_type, "push_button");
        assert!(created.widgets.iter().any(|widget| widget.has_appearance));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("button undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_radio_writes_export_state_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateRadio {
                field_name: "DocumentSubsystemsRadio".to_string(),
                page: 1,
                rect: [360.0, 120.0, 378.0, 138.0],
                export_value: "Selected".to_string(),
                selected: true,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) = apply_document_subsystems(&input, &request).expect("create radio");
        assert_eq!(report.operation, "acroform_source_edit");
        let created = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open created radio"),
        )
        .expect("inspect created radio")
        .fields
        .into_iter()
        .find(|field| field.full_name == "DocumentSubsystemsRadio")
        .expect("created radio field");
        assert_eq!(created.field_type, "radio");
        assert_eq!(created.value.as_deref(), Some("Selected"));
        assert!(created.widgets.iter().any(|widget| widget.has_appearance));
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("radio undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn form_create_signature_field_is_unsigned_and_undoes() {
        let input = include_bytes!("../tests/fixtures/multi_stream.pdf").to_vec();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::FormData,
            action: Some(DocumentSubsystemsAction::FormCreateSignature {
                field_name: "DocumentSubsystemsSignature".to_string(),
                page: 1,
                rect: [396.0, 120.0, 516.0, 150.0],
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("create signature field");
        assert_eq!(report.operation, "acroform_source_edit");
        let field = crate::forms_report(
            &ContentEngine::open_bytes(output.clone()).expect("open signature field"),
        )
        .expect("inspect signature field")
        .fields
        .into_iter()
        .find(|field| field.full_name == "DocumentSubsystemsSignature")
        .expect("created signature field");
        assert!(field.is_signature);
        assert!(field.value.is_none());
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("signature undo");
        assert_eq!(restored, input);
    }

    #[test]
    fn static_xfa_datasets_import_rewrites_only_datasets_packet_and_undoes() {
        let input = static_xfa_pdf();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::XfaPreservation,
            action: Some(DocumentSubsystemsAction::XfaImportDatasets {
                datasets_xml: "<datasets xmlns=\"http://www.xfa.org/schema/xfa-data/1.0/\"><data><person><name>Updated</name></person></data></datasets>".to_string(),
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (output, report) =
            apply_document_subsystems(&input, &request).expect("import datasets");
        assert_eq!(report.operation, "static_xfa_datasets_source_import");
        let extraction = extract_xfa(
            &ContentEngine::open_bytes(output.clone()).expect("open XFA output"),
            &XfaLimits::default(),
        )
        .expect("extract imported datasets");
        assert!(extraction.template_parsed);
        assert!(extraction.datasets_parsed);
        assert_ne!(
            xfa_packet_fingerprint(&input).expect("input XFA fingerprint"),
            xfa_packet_fingerprint(&output).expect("output XFA fingerprint")
        );
        assert_eq!(
            xfa_non_dataset_packet_fingerprint(&input).expect("input preserved fingerprint"),
            xfa_non_dataset_packet_fingerprint(&output).expect("output preserved fingerprint")
        );
        let (restored, _) =
            undo_document_subsystems(&input, &output, &request).expect("datasets undo");
        assert_eq!(restored, input);
    }
}
